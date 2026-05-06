//! Marker timeout task for startup UNKNOWN and long-running BUSY STUCK fallback.

use crate::db::{self, Db};
use crate::marker::parser_registry::ParserHandle;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{oneshot, watch};

pub const STARTUP_TIMEOUT: Duration = Duration::from_secs(10);
// mvp13 Stage 3E: PaneDiffWatcher is the primary BUSY hang detector. This wide
// deadline is only a final fallback when pane diff detection fails entirely.
pub const BUSY_TIMEOUT: Duration = Duration::from_secs(10_800);

/// Timer mode for startup readiness or command completion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimerKind {
    Startup,
    Busy,
}

/// Channels used by readers and terminal paths to reset or cancel a marker timer.
pub struct MarkerTimerHandle {
    pub reset_tx: watch::Sender<Instant>,
    pub cancel_tx: oneshot::Sender<()>,
}

/// Spawn a marker timeout task with the default MVP3 timeout.
pub fn spawn_marker_timer_task(
    agent_id: String,
    kind: TimerKind,
    db: Arc<Db>,
    parser_handle: ParserHandle,
) -> MarkerTimerHandle {
    let timeout = match kind {
        TimerKind::Startup => STARTUP_TIMEOUT,
        TimerKind::Busy => BUSY_TIMEOUT,
    };
    spawn_marker_timer_task_with_timeout(agent_id, kind, timeout, db, parser_handle)
}

fn spawn_marker_timer_task_with_timeout(
    agent_id: String,
    kind: TimerKind,
    timeout: Duration,
    db: Arc<Db>,
    parser_handle: ParserHandle,
) -> MarkerTimerHandle {
    let (reset_tx, mut reset_rx) = watch::channel(Instant::now());
    let (cancel_tx, mut cancel_rx) = oneshot::channel();
    tokio::spawn(async move {
        loop {
            let deadline = *reset_rx.borrow_and_update() + timeout;
            let sleep_for = deadline.saturating_duration_since(Instant::now());
            tokio::select! {
                _ = &mut cancel_rx => return,
                _ = tokio::time::sleep(sleep_for) => break,
                changed = reset_rx.changed() => {
                    if changed.is_err() {
                        return;
                    }
                }
            }
        }

        match kind {
            TimerKind::Busy => {
                // mvp13 Stage 3E: BUSY timeout means business-level hang, not
                // system-level UNKNOWN. The normal path should be PaneDiffWatcher;
                // this 3h timer is only the last fallback.
                match db::state_machine::mark_agent_stuck(db.as_ref().clone(), agent_id.clone())
                    .await
                {
                    Ok(changes) if changes > 0 => {
                        tracing::info!(agent_id = %agent_id, "BUSY timeout marked agent STUCK");
                        crate::orchestrator::wake_up();
                    }
                    Ok(_) => {}
                    Err(err) => {
                        tracing::warn!(agent_id = %agent_id, error = %err, "failed to mark agent STUCK after BUSY marker timeout");
                    }
                }
            }
            TimerKind::Startup => {
                let pane_bytes = match parser_handle.lock() {
                    Ok(parser) => parser.screen().contents().into_bytes(),
                    Err(err) => {
                        tracing::warn!(agent_id = %agent_id, error = %err, "parser mutex poisoned during marker timeout snapshot");
                        Vec::new()
                    }
                };
                let failed_rules = serde_json::json!(["[\\$#>✦]\\s*$"]);
                match db::state_machine::mark_agent_unknown(
                    db.as_ref().clone(),
                    agent_id.clone(),
                    "STARTUP_MARKER_TIMEOUT".to_string(),
                    pane_bytes,
                    failed_rules,
                )
                .await
                {
                    Ok((changes, affected_job)) if changes > 0 => {
                        // R-2 (mvp12): notify hoisted from state_machine wrapper for clearer dispatcher boundary.
                        if let Some(job_id) = affected_job {
                            crate::orchestrator::pubsub::notify_job_update(&job_id);
                        }
                        crate::orchestrator::wake_up();
                    }
                    Ok(_) => {}
                    Err(err) => {
                        tracing::warn!(agent_id = %agent_id, error = %err, "failed to mark agent UNKNOWN after STARTUP marker timeout");
                    }
                }
            }
        }
    });

    MarkerTimerHandle {
        reset_tx,
        cancel_tx,
    }
}

#[cfg(test)]
mod tests {
    use super::{BUSY_TIMEOUT, TimerKind, spawn_marker_timer_task_with_timeout};
    use crate::db;
    use crate::db::agents::insert_agent_sync;
    use crate::db::sessions::insert_session_sync;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    async fn sleep_ms(ms: u64) {
        tokio::time::sleep(Duration::from_millis(ms)).await;
    }

    fn test_db_with_agent(state: &str) -> db::Db {
        let file = tempfile::NamedTempFile::new().unwrap();
        let db = db::init(file.path()).unwrap();
        {
            let conn = db.conn();
            insert_session_sync(&conn, "s1", "p1", "/tmp/foo").unwrap();
            insert_agent_sync(&conn, "a1", "s1", "bash", state, Some(1)).unwrap();
        }
        db
    }

    #[test]
    fn test_busy_timeout_default_is_three_hours() {
        assert_eq!(BUSY_TIMEOUT, Duration::from_secs(10_800));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_busy_timeout_marks_stuck() {
        let db = test_db_with_agent("BUSY");
        let _handle = spawn_marker_timer_task_with_timeout(
            "a1".into(),
            TimerKind::Busy,
            Duration::from_millis(20),
            Arc::new(db.clone()),
            Arc::new(Mutex::new(vt100::Parser::new(200, 200, 0))),
        );

        sleep_ms(80).await;

        let (state, error_code): (String, Option<String>) = db
            .conn()
            .query_row(
                "SELECT state, error_code FROM agents WHERE id = 'a1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(state, "STUCK");
        assert_eq!(error_code, None);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_timer_cancel_prevents_unknown() {
        let db = test_db_with_agent("BUSY");
        let handle = spawn_marker_timer_task_with_timeout(
            "a1".into(),
            TimerKind::Busy,
            Duration::from_millis(20),
            Arc::new(db.clone()),
            Arc::new(Mutex::new(vt100::Parser::new(200, 200, 0))),
        );
        let _ = handle.cancel_tx.send(());

        sleep_ms(80).await;

        let state: String = db
            .conn()
            .query_row("SELECT state FROM agents WHERE id = 'a1'", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(state, "BUSY");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_timer_reset_extends_deadline() {
        let db = test_db_with_agent("BUSY");
        let handle = spawn_marker_timer_task_with_timeout(
            "a1".into(),
            TimerKind::Busy,
            Duration::from_millis(80),
            Arc::new(db.clone()),
            Arc::new(Mutex::new(vt100::Parser::new(200, 200, 0))),
        );

        sleep_ms(50).await;
        handle.reset_tx.send(Instant::now()).unwrap();
        sleep_ms(50).await;
        let state: String = db
            .conn()
            .query_row("SELECT state FROM agents WHERE id = 'a1'", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(state, "BUSY");

        sleep_ms(80).await;
        let state: String = db
            .conn()
            .query_row("SELECT state FROM agents WHERE id = 'a1'", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(state, "STUCK");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_startup_timeout_still_marks_unknown_and_writes_evidence_snapshot() {
        let db = test_db_with_agent("SPAWNING");
        let parser = Arc::new(Mutex::new(vt100::Parser::new(200, 200, 0)));
        parser.lock().unwrap().process(b"no prompt yet\n");
        let _handle = spawn_marker_timer_task_with_timeout(
            "a1".into(),
            TimerKind::Startup,
            Duration::from_millis(20),
            Arc::new(db.clone()),
            parser,
        );

        sleep_ms(80).await;

        let (state, error_code, pane_bytes, failed_rules): (
            String,
            Option<String>,
            Vec<u8>,
            String,
        ) = db
            .conn()
            .query_row(
                "SELECT agents.state, agents.error_code, evidence.pane_bytes, evidence.failed_rules \
                 FROM agents JOIN evidence ON evidence.agent_id = agents.id WHERE agents.id = 'a1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(state, "UNKNOWN");
        assert_eq!(error_code.as_deref(), Some("STARTUP_MARKER_TIMEOUT"));
        assert!(!pane_bytes.is_empty());
        assert!(failed_rules.contains("[\\\\$#>✦]\\\\s*$"));
    }
}
