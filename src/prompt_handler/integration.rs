//! Integration helpers that connect prompt-handler scans to ccbd state and events.

use crate::db::Db;
use crate::db::common::{map_db_error, spawn_db};
use crate::error::CcbdError;
use crate::marker::MarkerMatcher;
use crate::prompt_handler::events::{UNKNOWN_PROMPT_DETECTED, UnknownPromptPayload, hex_hash};
use crate::prompt_handler::kb::load_or_bootstrap_kb;
use crate::prompt_handler::llm_client::RealHaikuClassifier;
use crate::prompt_handler::matcher::PromptScanPurpose;
use crate::prompt_handler::runner::{
    PromptRunOutcome, RunnerContext, TmuxPromptIo, handle_prompt_chain,
};
use crate::tmux::{TmuxPaneId, TmuxServer};
use rusqlite::{OptionalExtension, params};
use serde_json::json;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, LazyLock, Mutex};

static TRANSIENT_UNKNOWN_PROMPTS: LazyLock<Mutex<HashMap<String, TransientUnknownPrompt>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

#[derive(Debug, Clone)]
struct TransientUnknownPrompt {
    state: String,
    capture_hash: String,
    seen_count: usize,
}

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
    pub scan_purpose: PromptScanPurpose,
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
        PromptRunOutcome::Pending {
            snapshot,
            depth,
            block_reason,
        } => {
            let current_state = query_agent_state(db.clone(), agent_id.clone()).await?;
            if is_active_dispatch_prompt_demote_deferred_state(&current_state) {
                tracing::info!(
                    agent_id,
                    depth,
                    state = %current_state,
                    "prompt integration deferred unknown prompt during active dispatch"
                );
                return Ok(PromptScanDisposition::Deferred {
                    depth,
                    block_reason: "active_dispatch_prompt_scan".to_string(),
                });
            }
            if is_prompt_demote_deferred_state(&current_state) {
                let capture_hash = hex_hash(&snapshot.sanitized_hash);
                if transient_unknown_prompt_should_defer(
                    &agent_id,
                    &current_state,
                    &capture_hash,
                    transient_unknown_stable_scan_threshold(),
                ) {
                    tracing::info!(
                        agent_id,
                        depth,
                        state = %current_state,
                        capture_hash = %capture_hash,
                        "prompt integration deferred unknown prompt while transient screen stabilizes"
                    );
                    return Ok(PromptScanDisposition::Deferred {
                        depth,
                        block_reason: "unknown_prompt_stabilizing".to_string(),
                    });
                }
            }
            let payload = UnknownPromptPayload::new(&snapshot, &block_reason, depth, &provider);
            mark_prompt_pending_and_emit_unknown(db, agent_id, payload).await?;
            Ok(PromptScanDisposition::Pending {
                depth,
                block_reason,
            })
        }
        PromptRunOutcome::DepthExceeded { snapshot, depth } => {
            let current_state = query_agent_state(db.clone(), agent_id.clone()).await?;
            if is_active_dispatch_prompt_demote_deferred_state(&current_state) {
                tracing::info!(
                    agent_id,
                    depth,
                    state = %current_state,
                    "prompt integration deferred depth-exceeded prompt during active dispatch"
                );
                return Ok(PromptScanDisposition::Deferred {
                    depth,
                    block_reason: "active_dispatch_prompt_scan".to_string(),
                });
            }
            if is_prompt_demote_deferred_state(&current_state) {
                let capture_hash = hex_hash(&snapshot.sanitized_hash);
                if transient_unknown_prompt_should_defer(
                    &agent_id,
                    &current_state,
                    &capture_hash,
                    transient_unknown_stable_scan_threshold(),
                ) {
                    tracing::info!(
                        agent_id,
                        depth,
                        state = %current_state,
                        capture_hash = %capture_hash,
                        "prompt integration deferred depth-exceeded prompt while transient screen stabilizes"
                    );
                    return Ok(PromptScanDisposition::Deferred {
                        depth,
                        block_reason: "unknown_prompt_stabilizing".to_string(),
                    });
                }
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

fn is_active_dispatch_prompt_demote_deferred_state(state: &str) -> bool {
    matches!(
        state,
        crate::db::state_machine::STATE_WAITING_FOR_ACK | crate::db::state_machine::STATE_BUSY
    )
}

fn is_prompt_demote_deferred_state(state: &str) -> bool {
    matches!(
        state,
        crate::db::state_machine::STATE_SPAWNING
            | crate::db::state_machine::STATE_WAITING_FOR_ACK
            | crate::db::state_machine::STATE_BUSY
    )
}

fn transient_unknown_stable_scan_threshold() -> usize {
    std::env::var("AH_PROMPT_TRANSIENT_UNKNOWN_STABLE_SCANS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(3)
}

#[cfg(test)]
fn transient_unknown_prompt_disposition(
    agent_id: &str,
    state: &str,
    capture_hash: &str,
    stable_scan_threshold: usize,
) -> PromptScanDisposition {
    if transient_unknown_prompt_should_defer(agent_id, state, capture_hash, stable_scan_threshold) {
        PromptScanDisposition::Deferred {
            depth: 0,
            block_reason: "unknown_prompt_stabilizing".to_string(),
        }
    } else {
        PromptScanDisposition::Pending {
            depth: 0,
            block_reason: "unknown_prompt".to_string(),
        }
    }
}

fn transient_unknown_prompt_should_defer(
    agent_id: &str,
    state: &str,
    capture_hash: &str,
    stable_scan_threshold: usize,
) -> bool {
    if !is_prompt_demote_deferred_state(state) {
        if let Ok(mut prompts) = TRANSIENT_UNKNOWN_PROMPTS.lock() {
            prompts.remove(agent_id);
        }
        return false;
    }
    let threshold = stable_scan_threshold.max(1);
    let Ok(mut prompts) = TRANSIENT_UNKNOWN_PROMPTS.lock() else {
        return threshold > 1;
    };
    let entry = prompts
        .entry(agent_id.to_string())
        .and_modify(|entry| {
            if entry.state == state && entry.capture_hash == capture_hash {
                entry.seen_count += 1;
            } else {
                entry.state = state.to_string();
                entry.capture_hash = capture_hash.to_string();
                entry.seen_count = 1;
            }
        })
        .or_insert_with(|| TransientUnknownPrompt {
            state: state.to_string(),
            capture_hash: capture_hash.to_string(),
            seen_count: 1,
        });
    if entry.seen_count >= threshold {
        prompts.remove(agent_id);
        false
    } else {
        true
    }
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
    .with_scan_purpose(request.scan_purpose)
    .with_prompt_experience(&request.db)
    .with_llm_classifier(&RealHaikuClassifier)
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
        crate::db::state_machine::STATE_SPAWNING,
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
            "UPDATE agents SET state = 'PROMPT_PENDING', state_version = state_version + 1, updated_at = unixepoch() WHERE id = ? AND state IN ('IDLE', 'SPAWNING') AND state_version = ?",
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
        mark_prompt_pending_and_emit_unknown_sync, transient_unknown_prompt_disposition,
    };
    use crate::db::agents::{insert_agent_sync, query_agent_sync};
    use crate::db::events::query_events_since_sync;
    use crate::db::sessions::insert_session_sync;
    use crate::db::state_machine::{
        STATE_BUSY, STATE_IDLE, STATE_PROMPT_PENDING, STATE_SPAWNING, STATE_WAITING_FOR_ACK,
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
    fn pending_unknown_event_preserves_llm_block_reasons() {
        for reason in [
            "missing_api_key",
            "llm_error",
            "llm_timeout",
            "llm_low_confidence",
            "llm_unsafe",
            "unknown_prompt",
        ] {
            with_test_db(|db| {
                let payload = UnknownPromptPayload::new(
                    &PromptSnapshot {
                        sanitized_hash: [4; 32],
                        sanitized_text: format!("Prompt for {reason}"),
                    },
                    reason,
                    0,
                    "codex",
                );

                mark_prompt_pending_and_emit_unknown_sync(db, "a1", &payload).unwrap();

                let conn = db.conn();
                let events = query_events_since_sync(&conn, "a1", 0).unwrap();
                let unknown_payload: serde_json::Value =
                    serde_json::from_str(&events[1].payload).unwrap();
                assert_eq!(unknown_payload["block_reason"], reason);
                let state_payload: serde_json::Value =
                    serde_json::from_str(&events[0].payload).unwrap();
                assert_eq!(state_payload["reason"], reason);
            });
        }
    }

    #[test]
    fn pending_state_and_unknown_event_allows_persistent_spawning_agent() {
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

        let changes = mark_prompt_pending_and_emit_unknown_sync(&db, "a_spawn", &payload).unwrap();

        let conn = db.conn();
        let agent = query_agent_sync(&conn, "a_spawn").unwrap().unwrap();
        assert_eq!(changes, 1);
        assert_eq!(agent.state, STATE_PROMPT_PENDING);
        let events = query_events_since_sync(&conn, "a_spawn", 0).unwrap();
        assert_eq!(events[0].event_type, "state_change");
        assert_eq!(events[1].event_type, UNKNOWN_PROMPT_DETECTED);
    }

    #[test]
    fn pending_state_and_unknown_event_rejects_persistent_waiting_for_ack_agent() {
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
        assert!(matches!(err, CcbdError::AgentWrongState { .. }));
        assert_eq!(agent.state, STATE_WAITING_FOR_ACK);
        let events = query_events_since_sync(&conn, "a_waiting", 0).unwrap();
        assert!(events.is_empty());
    }

    #[test]
    fn busy_unknown_prompt_scan_is_deferred_and_agent_stays_busy() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let db = init(file.path()).unwrap();
        {
            let conn = db.conn();
            insert_session_sync(&conn, "s_busy", "p_busy", "/tmp/busy").unwrap();
            insert_agent_sync(&conn, "a_busy", "s_busy", "codex", STATE_BUSY, Some(789)).unwrap();
        }
        let payload = UnknownPromptPayload::new(
            &PromptSnapshot {
                sanitized_hash: [5; 32],
                sanitized_text: "Active job banner".into(),
            },
            "unknown_prompt",
            0,
            "codex",
        );

        let err = mark_prompt_pending_and_emit_unknown_sync(&db, "a_busy", &payload).unwrap_err();

        assert!(matches!(err, CcbdError::AgentWrongState { .. }));
        let conn = db.conn();
        let agent = query_agent_sync(&conn, "a_busy").unwrap().unwrap();
        assert_eq!(agent.state, STATE_BUSY);
        let events = query_events_since_sync(&conn, "a_busy", 0).unwrap();
        assert!(events.is_empty());
        assert_eq!(
            transient_unknown_prompt_disposition("a_busy", STATE_BUSY, "hash-busy", 2),
            PromptScanDisposition::Deferred {
                depth: 0,
                block_reason: "unknown_prompt_stabilizing".to_string(),
            }
        );
    }

    #[test]
    fn waiting_for_ack_unknown_prompt_scan_is_deferred_and_agent_stays_waiting() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let db = init(file.path()).unwrap();
        {
            let conn = db.conn();
            insert_session_sync(&conn, "s_wait", "p_wait", "/tmp/wait").unwrap();
            insert_agent_sync(
                &conn,
                "a_wait",
                "s_wait",
                "codex",
                STATE_WAITING_FOR_ACK,
                Some(789),
            )
            .unwrap();
        }
        let payload = UnknownPromptPayload::new(
            &PromptSnapshot {
                sanitized_hash: [6; 32],
                sanitized_text: "ACK repaint".into(),
            },
            "unknown_prompt",
            0,
            "codex",
        );

        let err = mark_prompt_pending_and_emit_unknown_sync(&db, "a_wait", &payload).unwrap_err();

        assert!(matches!(err, CcbdError::AgentWrongState { .. }));
        let conn = db.conn();
        let agent = query_agent_sync(&conn, "a_wait").unwrap().unwrap();
        assert_eq!(agent.state, STATE_WAITING_FOR_ACK);
        let events = query_events_since_sync(&conn, "a_wait", 0).unwrap();
        assert!(events.is_empty());
        assert_eq!(
            transient_unknown_prompt_disposition("a_wait", STATE_WAITING_FOR_ACK, "hash-wait", 2),
            PromptScanDisposition::Deferred {
                depth: 0,
                block_reason: "unknown_prompt_stabilizing".to_string(),
            }
        );
    }

    #[test]
    fn idle_unknown_prompt_still_enters_prompt_pending() {
        with_test_db(|db| {
            let payload = UnknownPromptPayload::new(
                &PromptSnapshot {
                    sanitized_hash: [7; 32],
                    sanitized_text: "Idle prompt".into(),
                },
                "unknown_prompt",
                0,
                "codex",
            );

            let changes = mark_prompt_pending_and_emit_unknown_sync(db, "a1", &payload).unwrap();

            let conn = db.conn();
            let agent = query_agent_sync(&conn, "a1").unwrap().unwrap();
            assert_eq!(changes, 1);
            assert_eq!(agent.state, STATE_PROMPT_PENDING);
        });
    }

    #[test]
    fn spawning_unknown_prompt_still_enters_prompt_pending() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let db = init(file.path()).unwrap();
        {
            let conn = db.conn();
            insert_session_sync(&conn, "s_spawn_new", "p_spawn_new", "/tmp/spawn-new").unwrap();
            insert_agent_sync(
                &conn,
                "a_spawn_new",
                "s_spawn_new",
                "codex",
                STATE_SPAWNING,
                Some(789),
            )
            .unwrap();
        }
        let payload = UnknownPromptPayload::new(
            &PromptSnapshot {
                sanitized_hash: [8; 32],
                sanitized_text: "Startup prompt".into(),
            },
            "unknown_prompt",
            0,
            "codex",
        );

        let changes =
            mark_prompt_pending_and_emit_unknown_sync(&db, "a_spawn_new", &payload).unwrap();

        let conn = db.conn();
        let agent = query_agent_sync(&conn, "a_spawn_new").unwrap().unwrap();
        assert_eq!(changes, 1);
        assert_eq!(agent.state, STATE_PROMPT_PENDING);
    }

    #[test]
    fn transient_unknown_prompt_defers_once_then_promotes_when_stable() {
        let first =
            transient_unknown_prompt_disposition("a_spawn_stable", STATE_SPAWNING, "hash-a", 2);
        let second =
            transient_unknown_prompt_disposition("a_spawn_stable", STATE_SPAWNING, "hash-a", 2);

        assert_eq!(
            first,
            PromptScanDisposition::Deferred {
                depth: 0,
                block_reason: "unknown_prompt_stabilizing".to_string(),
            }
        );
        assert!(matches!(second, PromptScanDisposition::Pending { .. }));
    }

    #[test]
    fn transient_unknown_prompt_keeps_deferring_when_snapshot_changes() {
        let first = transient_unknown_prompt_disposition(
            "a_spawn_changing",
            STATE_WAITING_FOR_ACK,
            "hash-a",
            2,
        );
        let second = transient_unknown_prompt_disposition(
            "a_spawn_changing",
            STATE_WAITING_FOR_ACK,
            "hash-b",
            2,
        );

        assert!(matches!(first, PromptScanDisposition::Deferred { .. }));
        assert!(matches!(second, PromptScanDisposition::Deferred { .. }));
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
