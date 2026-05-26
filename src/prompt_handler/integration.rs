//! Integration helpers that connect prompt-handler scans to ccbd state and events.

use crate::db::Db;
use crate::db::common::{map_db_error, spawn_db};
use crate::error::CcbdError;
use crate::marker::MarkerMatcher;
use crate::prompt_handler::events::{UNKNOWN_PROMPT_DETECTED, UnknownPromptPayload};
use crate::prompt_handler::kb::load_or_bootstrap_kb;
use crate::prompt_handler::runner::{
    PromptRunOutcome, RunnerContext, TmuxPromptIo, handle_prompt_chain,
};
use crate::tmux::{TmuxPaneId, TmuxServer};
use rusqlite::{OptionalExtension, params};
use serde_json::json;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Clone)]
pub struct PromptScanRequest {
    pub db: Db,
    pub agent_id: String,
    pub provider: String,
    pub pane_id: TmuxPaneId,
    pub tmux: Arc<TmuxServer>,
    pub state_dir: PathBuf,
    pub marker_matcher: Arc<MarkerMatcher>,
    pub max_depth: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PromptScanDisposition {
    NoActionNeeded { depth: usize },
    Handled { depth: usize },
    Deferred { depth: usize, block_reason: String },
    Pending { depth: usize, block_reason: String },
}

pub fn is_prompt_handling_provider(provider: &str) -> bool {
    matches!(provider, "codex" | "claude" | "gemini")
}

pub async fn scan_prompt_and_apply_outcome(
    request: PromptScanRequest,
) -> Result<PromptScanDisposition, CcbdError> {
    tracing::info!(
        agent_id = %request.agent_id,
        provider = %request.provider,
        max_depth = request.max_depth,
        "prompt integration scan start"
    );

    let db = request.db.clone();
    let agent_id = request.agent_id.clone();
    let provider = request.provider.clone();
    let outcome = tokio::task::spawn_blocking(move || run_prompt_scan(request))
        .await
        .map_err(|err| CcbdError::DatabaseRuntimePanic {
            details: format!("prompt scan worker join failed: {err}"),
        })??;

    match outcome {
        PromptRunOutcome::NoActionNeeded { depth } => {
            if depth > 0 {
                tracing::info!(
                    agent_id,
                    depth,
                    "prompt integration auto-handled known prompt chain"
                );
                Ok(PromptScanDisposition::Handled { depth })
            } else {
                tracing::info!(agent_id, "prompt integration no prompt action needed");
                Ok(PromptScanDisposition::NoActionNeeded { depth })
            }
        }
        PromptRunOutcome::RetryLater { depth } => {
            tracing::info!(
                agent_id,
                depth,
                "prompt integration deferred scan until pane capture is non-empty"
            );
            Ok(PromptScanDisposition::Deferred {
                depth,
                block_reason: "retry_later".to_string(),
            })
        }
        PromptRunOutcome::Pending { snapshot, depth } => {
            let current_state = query_agent_state(db.clone(), agent_id.clone()).await?;
            if is_prompt_demote_deferred_state(&current_state) {
                tracing::info!(
                    agent_id,
                    depth,
                    state = %current_state,
                    "prompt integration deferred unknown prompt while agent is transient"
                );
                return Ok(PromptScanDisposition::Deferred {
                    depth,
                    block_reason: "unknown_prompt".to_string(),
                });
            }
            let payload = UnknownPromptPayload::new(&snapshot, "unknown_prompt", depth, &provider);
            mark_prompt_pending_and_emit_unknown(db, agent_id, payload).await?;
            Ok(PromptScanDisposition::Pending {
                depth,
                block_reason: "unknown_prompt".to_string(),
            })
        }
        PromptRunOutcome::DepthExceeded { snapshot, depth } => {
            let current_state = query_agent_state(db.clone(), agent_id.clone()).await?;
            if is_prompt_demote_deferred_state(&current_state) {
                tracing::info!(
                    agent_id,
                    depth,
                    state = %current_state,
                    "prompt integration deferred depth-exceeded prompt while agent is transient"
                );
                return Ok(PromptScanDisposition::Deferred {
                    depth,
                    block_reason: "depth_exceeded".to_string(),
                });
            }
            let payload = UnknownPromptPayload::new(&snapshot, "depth_exceeded", depth, &provider);
            mark_prompt_pending_and_emit_unknown(db, agent_id, payload).await?;
            Ok(PromptScanDisposition::Pending {
                depth,
                block_reason: "depth_exceeded".to_string(),
            })
        }
        PromptRunOutcome::ExecutorFailed { error, depth } => {
            tracing::error!(
                agent_id,
                depth,
                reason = %error,
                impact = "caller should keep existing ACK/STUCK fallback behavior",
                "prompt integration scan failed"
            );
            Err(error)
        }
    }
}

fn is_prompt_demote_deferred_state(state: &str) -> bool {
    matches!(
        state,
        crate::db::state_machine::STATE_SPAWNING | crate::db::state_machine::STATE_WAITING_FOR_ACK
    )
}

async fn query_agent_state(db: Db, agent_id: String) -> Result<String, CcbdError> {
    spawn_db("prompt_handler::query_agent_state", move || {
        db.conn()
            .query_row(
                "SELECT state FROM agents WHERE id = ?",
                params![&agent_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|err| map_db_error("query agent state for prompt disposition", err))?
            .ok_or(CcbdError::AgentNotFound(agent_id))
    })
    .await
}

fn run_prompt_scan(request: PromptScanRequest) -> Result<PromptRunOutcome, CcbdError> {
    let kb_path = request.state_dir.join("prompt-cases.json");
    let kb = load_or_bootstrap_kb(&kb_path).map_err(CcbdError::from)?;
    let current_state: String = request
        .db
        .conn()
        .query_row(
            "SELECT state FROM agents WHERE id = ?",
            params![&request.agent_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|err| map_db_error("query agent state for prompt scan", err))?
        .ok_or_else(|| CcbdError::AgentNotFound(request.agent_id.clone()))?;
    let io = TmuxPromptIo::new((*request.tmux).clone());
    let ctx = RunnerContext::new(
        &request.agent_id,
        &request.pane_id,
        &request.provider,
        &io,
        &kb,
    )
    .with_current_state(&current_state)
    .with_marker_matcher(request.marker_matcher.as_ref());
    Ok(handle_prompt_chain(ctx, request.max_depth))
}

pub async fn mark_prompt_pending_and_emit_unknown(
    db: Db,
    agent_id: String,
    payload: UnknownPromptPayload,
) -> Result<usize, CcbdError> {
    spawn_db("prompt_handler::mark_pending_emit_unknown", move || {
        mark_prompt_pending_and_emit_unknown_sync(&db, &agent_id, &payload)
    })
    .await
}

pub(crate) fn mark_prompt_pending_and_emit_unknown_sync(
    db: &Db,
    agent_id: &str,
    payload: &UnknownPromptPayload,
) -> Result<usize, CcbdError> {
    let mut conn = db.conn();
    let tx = conn
        .transaction()
        .map_err(|err| map_db_error("begin prompt pending event transaction", err))?;
    let current = tx
        .query_row(
            "SELECT state, state_version FROM agents WHERE id = ?",
            params![agent_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
        )
        .optional()
        .map_err(|err| map_db_error("query state for prompt pending event", err))?;

    let Some((previous_state, state_version)) = current else {
        tx.rollback()
            .map_err(|err| map_db_error("rollback prompt pending missing agent", err))?;
        return Err(CcbdError::AgentNotFound(agent_id.to_string()));
    };

    let allowed = [
        crate::db::state_machine::STATE_IDLE,
        crate::db::state_machine::STATE_BUSY,
    ];
    if !allowed.contains(&previous_state.as_str()) {
        tx.rollback()
            .map_err(|err| map_db_error("rollback prompt pending invalid state", err))?;
        return Err(CcbdError::AgentWrongState {
            current_state: previous_state,
        });
    }

    tracing::info!(
        agent_id,
        from = %previous_state,
        block_reason = %payload.block_reason,
        "prompt pending transaction start"
    );
    let changes = tx
        .execute(
            "UPDATE agents SET state = 'PROMPT_PENDING', state_version = state_version + 1, updated_at = unixepoch() WHERE id = ? AND state IN ('IDLE', 'BUSY') AND state_version = ?",
            params![agent_id, state_version],
        )
        .map_err(|err| map_db_error("mark prompt pending for unknown prompt", err))?;
    if changes == 0 {
        tx.rollback()
            .map_err(|err| map_db_error("rollback prompt pending CAS miss", err))?;
        return Ok(0);
    }

    let state_payload = json!({
        "from": previous_state,
        "to": crate::db::state_machine::STATE_PROMPT_PENDING,
        "reason": payload.block_reason,
    })
    .to_string();
    tx.execute(
        "INSERT INTO events (agent_id, request_id, event_type, payload) VALUES (?, NULL, 'state_change', ?)",
        params![agent_id, state_payload],
    )
    .map_err(|err| map_db_error("insert prompt pending state_change", err))?;

    let unknown_payload = serde_json::to_string(payload).map_err(|err| {
        CcbdError::IpcInvalidRequest(format!("serialize unknown prompt payload: {err}"))
    })?;
    tx.execute(
        "INSERT INTO events (agent_id, request_id, event_type, payload) VALUES (?, NULL, ?, ?)",
        params![agent_id, UNKNOWN_PROMPT_DETECTED, unknown_payload],
    )
    .map_err(|err| {
        tracing::error!(
            agent_id,
            reason = %err,
            impact = "agent would be pending without master-visible prompt event; transaction rolls back",
            "unknown prompt event insert failed"
        );
        map_db_error("insert unknown prompt event", err)
    })?;

    tx.commit()
        .map_err(|err| map_db_error("commit prompt pending event transaction", err))?;
    tracing::info!(
        agent_id,
        block_reason = %payload.block_reason,
        "prompt pending transaction complete"
    );
    Ok(changes)
}

#[cfg(test)]
mod tests {
    use super::{
        PromptScanDisposition, is_prompt_handling_provider,
        mark_prompt_pending_and_emit_unknown_sync,
    };
    use crate::db::agents::{insert_agent_sync, query_agent_sync};
    use crate::db::events::query_events_since_sync;
    use crate::db::sessions::insert_session_sync;
    use crate::db::state_machine::{
        STATE_IDLE, STATE_PROMPT_PENDING, STATE_SPAWNING, STATE_WAITING_FOR_ACK,
    };
    use crate::db::{Db, init};
    use crate::error::CcbdError;
    use crate::prompt_handler::events::{UNKNOWN_PROMPT_DETECTED, UnknownPromptPayload};
    use crate::prompt_handler::runner::PromptSnapshot;

    fn with_test_db<T>(test: impl FnOnce(&Db) -> T) -> T {
        let file = tempfile::NamedTempFile::new().unwrap();
        let db = init(file.path()).unwrap();
        {
            let conn = db.conn();
            insert_session_sync(&conn, "s1", "p1", "/tmp/p1").unwrap();
            insert_agent_sync(&conn, "a1", "s1", "codex", STATE_IDLE, Some(456)).unwrap();
        }
        test(&db)
    }

    #[test]
    fn pending_state_and_unknown_event_are_written_together() {
        with_test_db(|db| {
            let payload = UnknownPromptPayload::new(
                &PromptSnapshot {
                    sanitized_hash: [1; 32],
                    sanitized_text: "Unknown prompt".into(),
                },
                "unknown_prompt",
                0,
                "codex",
            );

            let changes = mark_prompt_pending_and_emit_unknown_sync(db, "a1", &payload).unwrap();

            assert_eq!(changes, 1);
            let conn = db.conn();
            let agent = query_agent_sync(&conn, "a1").unwrap().unwrap();
            assert_eq!(agent.state, STATE_PROMPT_PENDING);
            let events = query_events_since_sync(&conn, "a1", 0).unwrap();
            assert_eq!(events[0].event_type, "state_change");
            assert_eq!(events[1].event_type, UNKNOWN_PROMPT_DETECTED);
        });
    }

    #[test]
    fn pending_state_and_unknown_event_reject_spawning_agent() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let db = init(file.path()).unwrap();
        {
            let conn = db.conn();
            insert_session_sync(&conn, "s_spawn", "p_spawn", "/tmp/spawn").unwrap();
            insert_agent_sync(
                &conn,
                "a_spawn",
                "s_spawn",
                "codex",
                STATE_SPAWNING,
                Some(789),
            )
            .unwrap();
        }
        let payload = UnknownPromptPayload::new(
            &PromptSnapshot {
                sanitized_hash: [2; 32],
                sanitized_text: "Startup prompt".into(),
            },
            "unknown_prompt",
            0,
            "codex",
        );

        let err = mark_prompt_pending_and_emit_unknown_sync(&db, "a_spawn", &payload).unwrap_err();

        let conn = db.conn();
        let agent = query_agent_sync(&conn, "a_spawn").unwrap().unwrap();
        assert_eq!(agent.state, STATE_SPAWNING);
        let events = query_events_since_sync(&conn, "a_spawn", 0).unwrap();
        assert!(matches!(err, CcbdError::AgentWrongState { .. }));
        assert!(events.is_empty());
    }

    #[test]
    fn pending_state_and_unknown_event_reject_waiting_for_ack_agent() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let db = init(file.path()).unwrap();
        {
            let conn = db.conn();
            insert_session_sync(&conn, "s_waiting", "p_waiting", "/tmp/waiting").unwrap();
            insert_agent_sync(
                &conn,
                "a_waiting",
                "s_waiting",
                "codex",
                STATE_WAITING_FOR_ACK,
                Some(789),
            )
            .unwrap();
        }
        let payload = UnknownPromptPayload::new(
            &PromptSnapshot {
                sanitized_hash: [3; 32],
                sanitized_text: "Banner repaint".into(),
            },
            "unknown_prompt",
            0,
            "codex",
        );

        let err =
            mark_prompt_pending_and_emit_unknown_sync(&db, "a_waiting", &payload).unwrap_err();

        let conn = db.conn();
        let agent = query_agent_sync(&conn, "a_waiting").unwrap().unwrap();
        assert_eq!(agent.state, STATE_WAITING_FOR_ACK);
        let events = query_events_since_sync(&conn, "a_waiting", 0).unwrap();
        assert!(matches!(err, CcbdError::AgentWrongState { .. }));
        assert!(events.is_empty());
    }

    #[test]
    fn disposition_pending_shape_is_stable() {
        assert_eq!(
            PromptScanDisposition::Pending {
                depth: 3,
                block_reason: "depth_exceeded".into()
            },
            PromptScanDisposition::Pending {
                depth: 3,
                block_reason: "depth_exceeded".into()
            }
        );
    }

    #[test]
    fn prompt_handling_provider_gate_excludes_shell_providers() {
        assert!(is_prompt_handling_provider("codex"));
        assert!(is_prompt_handling_provider("claude"));
        assert!(is_prompt_handling_provider("gemini"));
        assert!(!is_prompt_handling_provider("bash"));
    }
}
