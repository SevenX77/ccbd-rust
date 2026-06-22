use std::path::Path;
use std::time::{Duration, Instant};

use crate::completion::parser::LogParseResult;
use crate::completion::reader::{LogCursorMap, LogReadState, read_provider_log_tail_with_state};
use crate::db;
use crate::error::CcbdError;

pub const LOG_MONITOR_POLL_INTERVAL: Duration = Duration::from_millis(250);
pub const MAX_LOG_MONITOR_WAIT: Duration = crate::pane_diff::DEFAULT_STUCK_THRESHOLD;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogMonitorTickOutcome {
    pub completed: bool,
    pub woke_orchestrator: bool,
    pub cursors: LogCursorMap,
    pub state: LogReadState,
}

pub async fn run_log_monitor_tick(
    db: db::Db,
    agent_id: &str,
    provider: &str,
    log_root: &Path,
    state: LogReadState,
) -> Result<LogMonitorTickOutcome, CcbdError> {
    let read = read_provider_log_tail_with_state(provider, log_root, &state)
        .map_err(|err| CcbdError::PtyIoError(format!("read provider log tail: {err}")))?;
    let updated_cursors = read.cursors.clone();
    let updated_state = read.state.clone();

    for completion in read.completions {
        let LogParseResult::TurnComplete { turn_id, reply } = completion.parsed else {
            continue;
        };
        let raw_path = completion.raw_path.to_string_lossy().to_string();
        match db::state_machine::mark_agent_idle_log_event(
            db.clone(),
            agent_id.to_string(),
            provider.to_string(),
            reply,
            raw_path,
            completion.raw_offset,
            turn_id,
        )
        .await
        {
            Ok((changes, affected_job)) if changes > 0 => {
                if let Some(job_id) = affected_job {
                    crate::orchestrator::pubsub::notify_job_update(&job_id);
                }
                crate::orchestrator::wake_up();
                return Ok(LogMonitorTickOutcome {
                    completed: true,
                    woke_orchestrator: true,
                    cursors: updated_cursors,
                    state: updated_state,
                });
            }
            Ok(_) => {}
            Err(err) => {
                tracing::warn!(agent_id, provider, error = %err, "failed to mark agent IDLE from log event");
            }
        }
    }

    Ok(LogMonitorTickOutcome {
        completed: false,
        woke_orchestrator: false,
        cursors: updated_cursors,
        state: updated_state,
    })
}

pub fn spawn_log_monitor_task(
    db: db::Db,
    agent_id: String,
    provider: String,
    log_root: std::path::PathBuf,
    initial_state: LogReadState,
    mut cancel_rx: tokio::sync::oneshot::Receiver<()>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut state = initial_state;
        let deadline = Instant::now() + MAX_LOG_MONITOR_WAIT;
        loop {
            tokio::select! {
                _ = &mut cancel_rx => break,
                _ = tokio::time::sleep(LOG_MONITOR_POLL_INTERVAL) => {}
            }

            if Instant::now() >= deadline {
                tracing::warn!(agent_id = %agent_id, provider = %provider, "log monitor reached max wait; UI/stuck fallback remains authoritative");
                crate::completion::registry::cancel(&agent_id);
                break;
            }

            match db::agents::query_agent_state(db.clone(), agent_id.clone()).await {
                Ok(Some((state, _)))
                    if state == db::state_machine::STATE_WAITING_FOR_ACK
                        || state == db::state_machine::STATE_BUSY => {}
                Ok(_) => {
                    crate::completion::registry::cancel(&agent_id);
                    break;
                }
                Err(err) => {
                    tracing::warn!(agent_id = %agent_id, error = %err, "log monitor failed to query agent state");
                    break;
                }
            }

            match run_log_monitor_tick(db.clone(), &agent_id, &provider, &log_root, state.clone())
                .await
            {
                Ok(outcome) => {
                    state = outcome.state;
                    crate::completion::registry::update_state(&agent_id, state.clone());
                    if outcome.completed {
                        crate::completion::registry::cancel(&agent_id);
                        break;
                    }
                }
                Err(err) => {
                    tracing::warn!(agent_id = %agent_id, provider = %provider, error = %err, "log monitor tick failed; keeping UI fallback active");
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::Duration;

    use crate::completion::reader::LogReadState;
    use crate::db::agents::insert_agent_sync;
    use crate::db::events::insert_event_sync;
    use crate::db::jobs::{insert_job_sync, query_job_sync, update_dispatched_seq_id_sync};
    use crate::db::sessions::insert_session_sync;
    use crate::db::{self, Db};

    use super::{LOG_MONITOR_POLL_INTERVAL, MAX_LOG_MONITOR_WAIT, run_log_monitor_tick};

    fn codex_complete(turn_id: &str, reply: &str) -> String {
        format!(
            r#"{{"type":"event_msg","payload":{{"type":"task_complete","turn_id":"{turn_id}","last_agent_message":"{reply}"}}}}"#
        )
    }

    fn seed_busy_dispatched_job(db: &Db, agent_id: &str, job_id: &str) {
        let conn = db.conn();
        insert_session_sync(&conn, "s_log", "p_log", "/tmp/log").unwrap();
        insert_agent_sync(&conn, agent_id, "s_log", "codex", "BUSY", Some(123)).unwrap();
        insert_job_sync(&conn, job_id, agent_id, None, "echo PONG\n").unwrap();
        conn.execute(
            "UPDATE jobs SET status='DISPATCHED', dispatched_at=unixepoch() WHERE id=?",
            [job_id],
        )
        .unwrap();
        let seq = insert_event_sync(
            &conn,
            agent_id,
            None,
            "command_received",
            r#"{"cmd":"echo PONG\n","status":"SENT"}"#,
        )
        .unwrap();
        update_dispatched_seq_id_sync(&conn, job_id, seq).unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn monitor_wakes_orchestrator_and_notifies_job_update_on_complete() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let db = db::init(file.path()).unwrap();
        seed_busy_dispatched_job(&db, "a_log", "job_log");
        let root = tempfile::TempDir::new().unwrap();
        let log = root.path().join("rollout-session.jsonl");
        fs::write(&log, format!("{}\n", codex_complete("turn-1", "PONG"))).unwrap();
        let mut updates = crate::orchestrator::pubsub::subscribe_job_updates();

        let outcome = run_log_monitor_tick(
            db.clone(),
            "a_log",
            "codex",
            root.path(),
            LogReadState::default(),
        )
        .await
        .unwrap();

        assert!(outcome.completed);
        assert!(outcome.woke_orchestrator);
        assert_eq!(updates.try_recv().unwrap(), "job_log");
        let job = query_job_sync(&db.conn(), "job_log").unwrap().unwrap();
        assert_eq!(job.status, "COMPLETED");
        assert_eq!(job.reply_text.as_deref(), Some("PONG"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn pull_fallback_completes_when_hook_push_never_transitions() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let db = db::init(file.path()).unwrap();
        seed_busy_dispatched_job(
            &db,
            "a_hook_failed_pull_fallback",
            "job_hook_failed_pull_fallback",
        );
        let root = tempfile::TempDir::new().unwrap();
        let log = root.path().join("rollout-session.jsonl");
        fs::write(
            &log,
            format!("{}\n", codex_complete("turn-fallback", "PULL PONG")),
        )
        .unwrap();

        let outcome = run_log_monitor_tick(
            db.clone(),
            "a_hook_failed_pull_fallback",
            "codex",
            root.path(),
            LogReadState::default(),
        )
        .await
        .unwrap();
        let job = query_job_sync(&db.conn(), "job_hook_failed_pull_fallback")
            .unwrap()
            .unwrap();
        let state: String = db
            .conn()
            .query_row(
                "SELECT state FROM agents WHERE id='a_hook_failed_pull_fallback'",
                [],
                |row| row.get(0),
            )
            .unwrap();

        assert!(outcome.completed);
        assert!(outcome.woke_orchestrator);
        assert_eq!(state, "IDLE");
        assert_eq!(job.status, "COMPLETED");
        assert_eq!(job.reply_text.as_deref(), Some("PULL PONG"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn schema_error_keeps_ui_fallback_enabled() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let db = db::init(file.path()).unwrap();
        seed_busy_dispatched_job(&db, "a_schema", "job_schema");
        let root = tempfile::TempDir::new().unwrap();
        let log = root.path().join("project/session.jsonl");
        fs::create_dir_all(log.parent().unwrap()).unwrap();
        fs::write(
            &log,
            r#"{"type":"assistant","message":{"type":"message","role":"assistant","content":[{"type":"text","text":"PONG"}],"stop_reason":"schema_drift"}}"#,
        )
        .unwrap();

        let outcome = run_log_monitor_tick(
            db.clone(),
            "a_schema",
            "claude",
            root.path(),
            LogReadState::default(),
        )
        .await
        .unwrap();

        assert!(!outcome.completed);
        assert!(!outcome.woke_orchestrator);
        let state: String = db
            .conn()
            .query_row("SELECT state FROM agents WHERE id='a_schema'", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(state, "BUSY");
    }

    #[test]
    fn log_wait_timeout_is_not_above_stuck_threshold() {
        assert!(LOG_MONITOR_POLL_INTERVAL < Duration::from_secs(1));
        assert!(MAX_LOG_MONITOR_WAIT <= crate::pane_diff::DEFAULT_STUCK_THRESHOLD);
    }
}
