//! Marker timeout task that moves waiting agents to UNKNOWN.

use crate::db::{self, Db};
use crate::marker::parser_registry::ParserHandle;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{oneshot, watch};

pub const STARTUP_TIMEOUT: Duration = Duration::from_secs(10);
pub const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

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

        let reason = match kind {
            TimerKind::Startup => "STARTUP_MARKER_TIMEOUT",
            TimerKind::Busy => "PTY_MARKER_TIMEOUT",
        };
        let pane_bytes = match parser_handle.lock() {
            Ok(parser) => parser.screen().contents().into_bytes(),
            Err(err) => {
                tracing::warn!(agent_id = %agent_id, error = %err, "parser mutex poisoned during marker timeout snapshot");
                Vec::new()
            }
        };
        let failed_rules = serde_json::json!(["[\\$#>✦]\\s*$"]);
        if let Err(err) = db::state_machine::mark_agent_unknown(
            db.as_ref().clone(),
            agent_id.clone(),
            reason.to_string(),
            pane_bytes,
            failed_rules,
        )
        .await
        {
            tracing::warn!(agent_id = %agent_id, error = %err, "failed to mark agent UNKNOWN after marker timeout");
        }
    });

    MarkerTimerHandle {
        reset_tx,
        cancel_tx,
    }
}

#[cfg(test)]
mod tests {
    use super::{TimerKind, spawn_marker_timer_task_with_timeout};
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
            insert_session_sync(&conn, "s1", "p1", "/tmp/foo", 999).unwrap();
            insert_agent_sync(&conn, "a1", "s1", "bash", state, Some(1)).unwrap();
        }
        db
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_timer_timeout_marks_unknown() {
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
        assert_eq!(state, "UNKNOWN");
        assert_eq!(error_code.as_deref(), Some("PTY_MARKER_TIMEOUT"));
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
        assert_eq!(state, "UNKNOWN");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_timer_timeout_writes_evidence_snapshot() {
        let db = test_db_with_agent("BUSY");
        let parser = Arc::new(Mutex::new(vt100::Parser::new(200, 200, 0)));
        parser.lock().unwrap().process(b"no prompt yet\n");
        let _handle = spawn_marker_timer_task_with_timeout(
            "a1".into(),
            TimerKind::Busy,
            Duration::from_millis(20),
            Arc::new(db.clone()),
            parser,
        );

        sleep_ms(80).await;

        let (pane_bytes, failed_rules): (Vec<u8>, String) = db
            .conn()
            .query_row(
                "SELECT pane_bytes, failed_rules FROM evidence WHERE agent_id = 'a1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert!(!pane_bytes.is_empty());
        assert!(failed_rules.contains("[\\\\$#>✦]\\\\s*$"));
    }
}
