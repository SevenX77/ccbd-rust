use crate::db::Db;
use crate::db::common::{map_db_error, spawn_db};
use crate::db::evidence::has_job_evidence_sync;
use crate::db::jobs::{
    collect_reply_for_dispatched_job_sync, contains_prompt_text, distill_reply,
    mark_dispatched_jobs_failed_for_agent_conn_sync, mark_job_cancelled_conn_sync,
    mark_job_completed_conn_sync, query_dispatched_job_for_agent_sync, strip_ansi_escapes,
};
use crate::error::CcbdError;
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};

/// 状态字符串常量 (避免散落字面量).
pub const STATE_SPAWNING: &str = "SPAWNING";
pub const STATE_SPAWNING_INTERVENTION: &str = "SPAWNING_INTERVENTION";
pub const STATE_IDLE: &str = "IDLE";
pub const STATE_WAITING_FOR_ACK: &str = "WAITING_FOR_ACK";
pub const STATE_BUSY: &str = "BUSY";
/// Agent is blocked on an interactive prompt and waiting for master resolution.
///
/// This is neither an active execution state nor a terminal state: orchestrator must not
/// dispatch new jobs to it, and lifecycle/reconcile paths must not silently crash or kill it.
pub const STATE_PROMPT_PENDING: &str = "PROMPT_PENDING";
pub const STATE_STUCK: &str = "STUCK";
pub const STATE_FAILED: &str = "FAILED";
pub const STATE_CRASHED: &str = "CRASHED";
pub const STATE_KILLED: &str = "KILLED";
pub const STATE_UNKNOWN: &str = "UNKNOWN";

pub const EVIDENCE_DENY_MESSAGE: &str = "SYSTEM DENY: Missing physical evidence. You must output a git diff or test result before finishing.";
const LOG_EVENT_TASK_COMPLETE_REASON: &str = "LOG_EVENT_TASK_COMPLETE";
const LOG_EVENT_SUB_STATE: &str = "LogEvent";
const HOOK_EVENT_SUB_STATE: &str = "HookEvent";
const UI_COMPLETION_RECAPTURE_MATCHED_REASON: &str = "UI_COMPLETION_RECAPTURE_MATCHED";
const UI_COMPLETION_RECAPTURE_PROMPT_ONLY_REASON: &str = "UI_COMPLETION_RECAPTURE_PROMPT_ONLY";
const UI_COMPLETION_RECAPTURE_SOURCE: &str = "pane_diff_ui_completion_recapture";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkerMatchedOutcome {
    pub changes: usize,
    pub affected_job: Option<String>,
    pub denial_message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StuckOutcome {
    pub changes: usize,
    pub agent_id: String,
    pub job_id: Option<String>,
    pub from_state: Option<String>,
    pub event_seq_id: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UiCompletionRecaptureDisposition {
    MarkedIdle,
    MarkedStuck,
    NoOp,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiCompletionRecaptureOutcome {
    pub changes: usize,
    pub affected_job: Option<String>,
    pub disposition: UiCompletionRecaptureDisposition,
}

#[derive(Debug)]
struct MarkerMatchedSyncOutcome {
    changes: usize,
    denial_message: Option<String>,
}

#[derive(Debug)]
struct UiCompletionRecaptureSyncOutcome {
    changes: usize,
    affected_job: Option<String>,
    disposition: UiCompletionRecaptureDisposition,
    stuck_event_seq_id: Option<i64>,
    stuck_payload: Option<Value>,
}

/// 是否处于 "正在工作 / 待 ACK" 中 (后续 marker / stuck / unknown guard 复用).
///
/// 包括: SPAWNING (启动中), WAITING_FOR_ACK (T2.1.2 引入), BUSY (执行中).
pub fn is_active_state(state: &str) -> bool {
    matches!(state, STATE_SPAWNING | STATE_WAITING_FOR_ACK | STATE_BUSY)
}

/// 是否处于"等待 ACK"状态.
pub fn is_waiting_for_ack(state: &str) -> bool {
    state == STATE_WAITING_FOR_ACK
}

pub fn transit_agent_state_sync(
    conn: &mut Connection,
    agent_id: &str,
    from_state_list: &[&str],
    to_state: &str,
    reason: Option<&str>,
) -> Result<(), CcbdError> {
    let tx = conn
        .transaction()
        .map_err(|err| map_db_error("begin transit agent state", err))?;
    transit_agent_state_conn_inner(&tx, agent_id, from_state_list, to_state, reason, None, true)?;
    tx.commit()
        .map_err(|err| map_db_error("commit transit agent state", err))?;
    Ok(())
}

pub(crate) fn transit_agent_state_conn_sync(
    conn: &Connection,
    agent_id: &str,
    from_state_list: &[&str],
    to_state: &str,
    reason: Option<&str>,
) -> Result<(), CcbdError> {
    transit_agent_state_conn_inner(
        conn,
        agent_id,
        from_state_list,
        to_state,
        reason,
        None,
        true,
    )
    .map(|_| ())
}

fn transit_agent_state_conn_inner(
    conn: &Connection,
    agent_id: &str,
    from_state_list: &[&str],
    to_state: &str,
    reason: Option<&str>,
    expected_version: Option<i64>,
    strict: bool,
) -> Result<bool, CcbdError> {
    let current = conn
        .query_row(
            "SELECT state, state_version FROM agents WHERE id = ?",
            params![agent_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
        )
        .optional()
        .map_err(|err| map_db_error("query state for transit", err))?;

    let Some((previous_state, state_version)) = current else {
        return if strict {
            Err(CcbdError::AgentNotFound(agent_id.to_string()))
        } else {
            Ok(false)
        };
    };

    let version_matches = expected_version.is_none_or(|expected| expected == state_version);
    let from_matches =
        from_state_list.is_empty() || from_state_list.contains(&previous_state.as_str());
    if !version_matches || !from_matches {
        return if strict {
            Err(CcbdError::AgentWrongState {
                current_state: previous_state,
            })
        } else {
            Ok(false)
        };
    }

    let changes = conn
        .execute(
            "UPDATE agents SET state = ?, state_version = state_version + 1, updated_at = unixepoch() WHERE id = ? AND state_version = ?",
            params![to_state, agent_id, state_version],
        )
        .map_err(|err| map_db_error("transit agent state", err))?;
    if changes != 1 {
        return if strict {
            Err(CcbdError::AgentWrongState {
                current_state: previous_state,
            })
        } else {
            Ok(false)
        };
    }

    let payload = json!({
        "from": previous_state,
        "to": to_state,
        "reason": reason,
    })
    .to_string();
    conn.execute(
        "INSERT INTO events (agent_id, request_id, event_type, payload) VALUES (?, NULL, 'state_change', ?)",
        params![agent_id, payload],
    )
    .map_err(|err| map_db_error("insert transit state_change", err))?;

    Ok(true)
}

/// Atomic CAS: IDLE -> WAITING_FOR_ACK.
///
/// Returns true if transition succeeded (rowcount == 1), false otherwise
/// (agent in a non-IDLE state, version mismatch, or row missing).
pub(crate) fn mark_agent_waiting_for_ack_sync(
    conn: &mut Connection,
    agent_id: &str,
    expected_version: i64,
) -> Result<bool, CcbdError> {
    let tx = conn
        .transaction()
        .map_err(|err| map_db_error("begin mark agent waiting for ack", err))?;
    let changed = transit_agent_state_conn_inner(
        &tx,
        agent_id,
        &[STATE_IDLE],
        STATE_WAITING_FOR_ACK,
        Some("ack_pending"),
        Some(expected_version),
        false,
    )?;
    tx.commit()
        .map_err(|err| map_db_error("commit mark agent waiting for ack", err))?;
    Ok(changed)
}

pub(crate) fn mark_agent_prompt_pending_sync(
    db: &Db,
    agent_id: &str,
    reason: &str,
) -> Result<usize, CcbdError> {
    let mut conn = db.conn();
    let tx = conn
        .transaction()
        .map_err(|err| map_db_error("begin mark agent prompt pending", err))?;
    let current = tx
        .query_row(
            "SELECT state, state_version FROM agents WHERE id = ?",
            params![agent_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
        )
        .optional()
        .map_err(|err| map_db_error("query state for prompt pending", err))?;

    let Some((previous_state, state_version)) = current else {
        tx.rollback()
            .map_err(|err| map_db_error("rollback mark prompt pending missing agent", err))?;
        return Err(CcbdError::AgentNotFound(agent_id.to_string()));
    };
    let allowed = [STATE_IDLE, STATE_SPAWNING];
    if !allowed.contains(&previous_state.as_str()) {
        tracing::warn!(
            agent_id,
            state = %previous_state,
            reason,
            impact = "agent remains in its current state and no prompt resolution is pending",
            "prompt pending transition rejected"
        );
        tx.rollback()
            .map_err(|err| map_db_error("rollback mark prompt pending invalid state", err))?;
        return Err(CcbdError::AgentWrongState {
            current_state: previous_state,
        });
    }

    tracing::info!(
        agent_id,
        from = %previous_state,
        to = STATE_PROMPT_PENDING,
        reason,
        "mark agent prompt pending start"
    );
    let changes = tx
        .execute(
            "UPDATE agents SET state = 'PROMPT_PENDING', state_version = state_version + 1, updated_at = unixepoch() WHERE id = ? AND state IN ('IDLE', 'SPAWNING') AND state_version = ?",
            params![agent_id, state_version],
        )
        .map_err(|err| map_db_error("mark agent prompt pending", err))?;

    if changes == 1 {
        let payload = json!({
            "from": previous_state,
            "to": STATE_PROMPT_PENDING,
            "reason": reason,
        })
        .to_string();
        tx.execute(
            "INSERT INTO events (agent_id, request_id, event_type, payload) VALUES (?, NULL, 'state_change', ?)",
            params![agent_id, payload],
        )
        .map_err(|err| map_db_error("insert prompt pending state_change", err))?;
    } else {
        tracing::warn!(
            agent_id,
            reason,
            impact = "prompt pending transition lost a state_version race",
            "prompt pending CAS failed"
        );
        tx.rollback()
            .map_err(|err| map_db_error("rollback mark prompt pending CAS failed", err))?;
        return Ok(0);
    }

    tx.commit()
        .map_err(|err| map_db_error("commit mark agent prompt pending", err))?;
    tracing::info!(
        agent_id,
        to = STATE_PROMPT_PENDING,
        reason,
        "mark agent prompt pending complete"
    );
    Ok(changes)
}

#[cfg(test)]
pub(crate) fn mark_agent_idle_matched_sync(db: &Db, agent_id: &str) -> Result<usize, CcbdError> {
    mark_agent_idle_matched_outcome_sync(db, agent_id).map(|outcome| outcome.changes)
}

fn mark_agent_idle_matched_outcome_sync(
    db: &Db,
    agent_id: &str,
) -> Result<MarkerMatchedSyncOutcome, CcbdError> {
    mark_agent_idle_matched_outcome_sync_inner(db, agent_id, "MARKER_MATCHED")
}

#[cfg(test)]
pub(crate) fn mark_agent_idle_recaptured_sync(db: &Db, agent_id: &str) -> Result<usize, CcbdError> {
    mark_agent_idle_recaptured_outcome_sync_with_pane(db, agent_id, "")
        .map(|outcome| outcome.changes)
}

fn mark_agent_idle_recaptured_outcome_sync_with_pane(
    db: &Db,
    agent_id: &str,
    pane_snapshot: &str,
) -> Result<UiCompletionRecaptureSyncOutcome, CcbdError> {
    let mut conn = db.conn();
    let tx = conn
        .transaction()
        .map_err(|err| map_db_error("begin mark agent idle recaptured", err))?;
    let current = tx
        .query_row(
            "SELECT state, state_version FROM agents WHERE id = ?",
            params![agent_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
        )
        .optional()
        .map_err(|err| map_db_error("query state for UI recapture", err))?;

    let Some((previous_state, state_version)) = current else {
        return Ok(ui_recapture_noop());
    };
    let stuck_recapture_allowed = previous_state.as_str() == STATE_STUCK;
    if !is_active_state(previous_state.as_str()) && !stuck_recapture_allowed {
        tracing::trace!(agent_id, state = %previous_state, "UI recapture swallowed: agent not active or stuck");
        return Ok(ui_recapture_noop());
    }

    let dispatched_job_reply = if let Some(job) =
        query_dispatched_job_for_agent_sync(&tx, agent_id)?
    {
        if crate::completion::registry::contains(agent_id) {
            tracing::trace!(
                agent_id,
                job_id = %job.id,
                "UI recapture deferred: completion log monitor is authoritative while active"
            );
            tx.commit()
                .map_err(|err| map_db_error("commit deferred UI recapture", err))?;
            return Ok(ui_recapture_noop());
        }
        if let Some(denial_message) = evidence_denial_for_job(&tx, agent_id, &job)? {
            insert_evidence_denied_event(&tx, agent_id, &job.id, &denial_message)?;
            tx.commit()
                .map_err(|err| map_db_error("commit evidence denied UI recapture", err))?;
            return Ok(ui_recapture_noop());
        }
        if let Some(marker_job_id) =
            latest_ah_idle_marker_job_id(&tx, agent_id, job.dispatched_at_seq_id)?
            && marker_job_id != job.id
        {
            tracing::warn!(
                agent_id,
                marker_job_id,
                dispatched_job_id = %job.id,
                "UI recapture swallowed: ah idle marker job-id mismatch"
            );
            tx.rollback()
                .map_err(|err| map_db_error("rollback UI recapture marker job-id mismatch", err))?;
            return Ok(ui_recapture_noop());
        }

        let mut reply_text = collect_reply_for_dispatched_job_sync(
            &tx,
            agent_id,
            job.dispatched_at_seq_id,
            &job.prompt_text,
        )?;
        if !job.cancel_requested
            && (previous_state == STATE_BUSY || previous_state == STATE_STUCK)
            && is_prompt_only_reply(&reply_text)
        {
            let pane_reply = if contains_prompt_text(pane_snapshot, &job.prompt_text) {
                distill_reply(pane_snapshot, &job.prompt_text)
            } else {
                String::new()
            };
            if is_prompt_only_reply(&pane_reply) {
                if previous_state == STATE_STUCK {
                    tracing::info!(
                        agent_id,
                        job_id = %job.id,
                        "UI recapture prompt-only pane observed while agent already STUCK"
                    );
                    tx.commit()
                        .map_err(|err| map_db_error("commit already-stuck UI recapture", err))?;
                    return Ok(ui_recapture_noop());
                }
                let changes = tx
                    .execute(
                        "UPDATE agents SET state = 'STUCK', state_version = state_version + 1, updated_at = unixepoch() WHERE id = ? AND state = 'BUSY' AND state_version = ?",
                        params![agent_id, state_version],
                    )
                    .map_err(|err| map_db_error("mark UI recapture prompt-only stuck", err))?;
                if changes == 1 {
                    let payload = json!({
                        "from": previous_state,
                        "to": "STUCK",
                        "reason": UI_COMPLETION_RECAPTURE_PROMPT_ONLY_REASON,
                        "job_id": job.id.clone(),
                        "pane_reply_len": pane_reply.len(),
                        "content_hash": recapture_content_hash(pane_snapshot),
                        "source": UI_COMPLETION_RECAPTURE_SOURCE,
                    });
                    tx.execute(
                        "INSERT INTO events (agent_id, request_id, event_type, payload) VALUES (?, NULL, 'state_change', ?)",
                        params![agent_id, payload.to_string()],
                    )
                    .map_err(|err| map_db_error("insert UI recapture prompt-only state_change", err))?;
                    let event_seq_id = tx.last_insert_rowid();
                    tx.commit().map_err(|err| {
                        map_db_error("commit UI recapture prompt-only stuck", err)
                    })?;
                    return Ok(UiCompletionRecaptureSyncOutcome {
                        changes,
                        affected_job: Some(job.id),
                        disposition: UiCompletionRecaptureDisposition::MarkedStuck,
                        stuck_event_seq_id: Some(event_seq_id),
                        stuck_payload: Some(payload),
                    });
                }
                tracing::trace!(
                    agent_id,
                    "UI recapture prompt-only stuck swallowed: state_version CAS failed"
                );
                tx.rollback().map_err(|err| {
                    map_db_error("rollback UI recapture prompt-only CAS failed", err)
                })?;
                return Ok(ui_recapture_noop());
            }
            reply_text = pane_reply;
        }
        Some((job.id, reply_text, job.cancel_requested))
    } else {
        None
    };

    let affected_job = dispatched_job_reply
        .as_ref()
        .map(|(job_id, _, _)| job_id.clone());
    let changes = mark_agent_idle_matched_conn_inner(
        &tx,
        agent_id,
        state_version,
        &previous_state,
        dispatched_job_reply,
        UI_COMPLETION_RECAPTURE_MATCHED_REASON,
        true,
    )?;

    tx.commit()
        .map_err(|err| map_db_error("commit mark agent idle recaptured", err))?;
    Ok(UiCompletionRecaptureSyncOutcome {
        changes,
        affected_job: if changes > 0 { affected_job } else { None },
        disposition: if changes > 0 {
            UiCompletionRecaptureDisposition::MarkedIdle
        } else {
            UiCompletionRecaptureDisposition::NoOp
        },
        stuck_event_seq_id: None,
        stuck_payload: None,
    })
}

fn mark_agent_idle_matched_outcome_sync_inner(
    db: &Db,
    agent_id: &str,
    reason: &str,
) -> Result<MarkerMatchedSyncOutcome, CcbdError> {
    let mut conn = db.conn();
    let tx = conn
        .transaction()
        .map_err(|err| map_db_error("begin mark agent idle matched", err))?;
    let current = tx
        .query_row(
            "SELECT state, state_version FROM agents WHERE id = ?",
            params![agent_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
        )
        .optional()
        .map_err(|err| map_db_error("query state for marker matched", err))?;

    let Some((previous_state, state_version)) = current else {
        return Ok(MarkerMatchedSyncOutcome {
            changes: 0,
            denial_message: None,
        });
    };
    if !is_active_state(previous_state.as_str()) {
        tracing::trace!(agent_id, state = %previous_state, "marker match swallowed: agent not in active state");
        return Ok(MarkerMatchedSyncOutcome {
            changes: 0,
            denial_message: None,
        });
    }

    let dispatched_job_reply = if let Some(job) =
        query_dispatched_job_for_agent_sync(&tx, agent_id)?
    {
        if crate::completion::registry::contains(agent_id) {
            tracing::trace!(
                agent_id,
                job_id = %job.id,
                "marker match deferred: completion log monitor is authoritative while active"
            );
            tx.commit()
                .map_err(|err| map_db_error("commit deferred marker match", err))?;
            return Ok(MarkerMatchedSyncOutcome {
                changes: 0,
                denial_message: None,
            });
        }
        if let Some(denial_message) = evidence_denial_for_job(&tx, agent_id, &job)? {
            insert_evidence_denied_event(&tx, agent_id, &job.id, &denial_message)?;
            tx.commit()
                .map_err(|err| map_db_error("commit evidence denied marker match", err))?;
            return Ok(MarkerMatchedSyncOutcome {
                changes: 0,
                denial_message: Some(denial_message),
            });
        }
        if let Some(marker_job_id) =
            latest_ah_idle_marker_job_id(&tx, agent_id, job.dispatched_at_seq_id)?
            && marker_job_id != job.id
        {
            tracing::warn!(
                agent_id,
                marker_job_id,
                dispatched_job_id = %job.id,
                "marker match swallowed: ah idle marker job-id mismatch"
            );
            tx.rollback()
                .map_err(|err| map_db_error("rollback marker job-id mismatch", err))?;
            return Ok(MarkerMatchedSyncOutcome {
                changes: 0,
                denial_message: None,
            });
        }
        let reply_text = collect_reply_for_dispatched_job_sync(
            &tx,
            agent_id,
            job.dispatched_at_seq_id,
            &job.prompt_text,
        )?;
        if previous_state == "BUSY" && !job.cancel_requested && is_prompt_only_reply(&reply_text) {
            tracing::trace!(agent_id, "marker match swallowed: prompt-only job reply");
            tx.rollback()
                .map_err(|err| map_db_error("rollback prompt-only marker match", err))?;
            return Ok(MarkerMatchedSyncOutcome {
                changes: 0,
                denial_message: None,
            });
        }
        Some((job.id, reply_text, job.cancel_requested))
    } else {
        None
    };

    let changes = mark_agent_idle_matched_conn_inner(
        &tx,
        agent_id,
        state_version,
        &previous_state,
        dispatched_job_reply,
        reason,
        false,
    )?;

    tx.commit()
        .map_err(|err| map_db_error("commit mark agent idle matched", err))?;
    Ok(MarkerMatchedSyncOutcome {
        changes,
        denial_message: None,
    })
}

fn mark_agent_idle_matched_conn_inner(
    conn: &Connection,
    agent_id: &str,
    state_version: i64,
    previous_state: &str,
    dispatched_job_reply: Option<(String, String, bool)>,
    reason: &str,
    allow_from_stuck: bool,
) -> Result<usize, CcbdError> {
    let changes = if allow_from_stuck {
        conn.execute(
            "UPDATE agents SET state = 'IDLE', sub_state = 'Matched', state_version = state_version + 1, updated_at = unixepoch() WHERE id = ? AND state IN ('SPAWNING', 'WAITING_FOR_ACK', 'BUSY', 'STUCK') AND state_version = ?",
            params![agent_id, state_version],
        )
    } else {
        conn.execute(
            "UPDATE agents SET state = 'IDLE', sub_state = 'Matched', state_version = state_version + 1, updated_at = unixepoch() WHERE id = ? AND state IN ('SPAWNING', 'WAITING_FOR_ACK', 'BUSY') AND state_version = ?",
            params![agent_id, state_version],
        )
    }
    .map_err(|err| map_db_error("mark agent idle matched", err))?;

    if changes == 1 {
        if let Some((job_id, reply_text, cancel_requested)) = dispatched_job_reply {
            if cancel_requested {
                mark_job_cancelled_conn_sync(conn, &job_id, &reply_text)?;
            } else {
                mark_job_completed_conn_sync(conn, &job_id, &reply_text)?;
            }
        }

        let payload = serde_json::json!({
            "from": previous_state,
            "to": "IDLE",
            "sub_state": "Matched",
            "reason": reason,
        })
        .to_string();
        conn.execute(
            "INSERT INTO events (agent_id, request_id, event_type, payload) VALUES (?, NULL, 'state_change', ?)",
            params![agent_id, payload],
        )
        .map_err(|err| map_db_error("insert marker matched state_change", err))?;
    } else {
        tracing::trace!(agent_id, "marker match swallowed: state_version CAS failed");
    }
    Ok(changes)
}

#[cfg(test)]
pub(crate) fn mark_agent_idle_log_event_sync(
    db: &Db,
    agent_id: &str,
    provider: &str,
    reply: Option<&str>,
    raw_path: &str,
    raw_offset: u64,
    provider_turn_id: Option<&str>,
) -> Result<usize, CcbdError> {
    mark_agent_idle_log_event_outcome_sync(
        db,
        agent_id,
        provider,
        reply,
        raw_path,
        raw_offset,
        provider_turn_id,
    )
    .map(|outcome| outcome.changes)
}

pub(crate) fn mark_agent_idle_hook_event_sync(
    db: &Db,
    agent_id: &str,
    provider: &str,
    hook_event: &str,
    event_id: Option<&str>,
    reply: Option<&str>,
) -> Result<usize, CcbdError> {
    mark_agent_idle_hook_event_outcome_sync(
        db, agent_id, provider, hook_event, event_id, reply, None,
    )
    .map(|outcome| outcome.changes)
}

#[cfg(test)]
pub(crate) fn mark_agent_idle_hook_event_at_version_sync(
    db: &Db,
    agent_id: &str,
    provider: &str,
    hook_event: &str,
    event_id: Option<&str>,
    reply: Option<&str>,
    expected_state_version: i64,
) -> Result<usize, CcbdError> {
    mark_agent_idle_hook_event_outcome_sync(
        db,
        agent_id,
        provider,
        hook_event,
        event_id,
        reply,
        Some(expected_state_version),
    )
    .map(|outcome| outcome.changes)
}

fn mark_agent_idle_hook_event_outcome_sync(
    db: &Db,
    agent_id: &str,
    provider: &str,
    hook_event: &str,
    event_id: Option<&str>,
    reply: Option<&str>,
    expected_state_version: Option<i64>,
) -> Result<MarkerMatchedSyncOutcome, CcbdError> {
    let mut conn = db.conn();
    let tx = conn
        .transaction()
        .map_err(|err| map_db_error("begin mark agent idle hook event", err))?;
    let current = tx
        .query_row(
            "SELECT state, state_version FROM agents WHERE id = ?",
            params![agent_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
        )
        .optional()
        .map_err(|err| map_db_error("query state for hook event matched", err))?;

    let Some((previous_state, state_version)) = current else {
        return Ok(MarkerMatchedSyncOutcome {
            changes: 0,
            denial_message: None,
        });
    };
    if previous_state != STATE_WAITING_FOR_ACK && previous_state != STATE_BUSY {
        tracing::trace!(agent_id, state = %previous_state, "hook completion swallowed: agent not busy or waiting for ack");
        return Ok(MarkerMatchedSyncOutcome {
            changes: 0,
            denial_message: None,
        });
    }
    let Some(cas_version) = expected_state_version.or(Some(state_version)) else {
        unreachable!("state_version is always present for existing agents")
    };

    let dispatched_job_reply =
        if let Some(job) = query_dispatched_job_for_agent_sync(&tx, agent_id)? {
            if let Some(denial_message) = evidence_denial_for_job(&tx, agent_id, &job)? {
                insert_evidence_denied_event(&tx, agent_id, &job.id, &denial_message)?;
                tx.commit()
                    .map_err(|err| map_db_error("commit evidence denied hook event", err))?;
                return Ok(MarkerMatchedSyncOutcome {
                    changes: 0,
                    denial_message: Some(denial_message),
                });
            }
            let (reply_text, reply_source) = if let Some(reply) = reply {
                (reply.to_string(), "hook")
            } else {
                (
                    collect_reply_for_dispatched_job_sync(
                        &tx,
                        agent_id,
                        job.dispatched_at_seq_id,
                        &job.prompt_text,
                    )?,
                    "screen",
                )
            };
            Some((job.id, reply_text, reply_source, job.cancel_requested))
        } else {
            None
        };
    let reply_source = dispatched_job_reply
        .as_ref()
        .map(|(_, _, reply_source, _)| *reply_source)
        .unwrap_or_else(|| if reply.is_some() { "hook" } else { "screen" });

    let changes = tx
        .execute(
            "UPDATE agents SET state = 'IDLE', sub_state = 'HookEvent', state_version = state_version + 1, updated_at = unixepoch() WHERE id = ? AND state IN ('WAITING_FOR_ACK', 'BUSY') AND state_version = ?",
            params![agent_id, cas_version],
        )
        .map_err(|err| map_db_error("mark agent idle hook event", err))?;

    if changes == 1 {
        if let Some((job_id, reply_text, _, cancel_requested)) = dispatched_job_reply {
            if cancel_requested {
                mark_job_cancelled_conn_sync(&tx, &job_id, &reply_text)?;
            } else {
                mark_job_completed_conn_sync(&tx, &job_id, &reply_text)?;
            }
        }

        let payload = json!({
            "from": previous_state,
            "to": STATE_IDLE,
            "sub_state": HOOK_EVENT_SUB_STATE,
            "source": "hook",
            "hook_event": hook_event,
            "provider": provider,
            "event_id": event_id,
            "schema_version": 1,
            "reply_source": reply_source,
        })
        .to_string();
        tx.execute(
            "INSERT INTO events (agent_id, request_id, event_type, payload) VALUES (?, NULL, 'state_change', ?)",
            params![agent_id, payload],
        )
        .map_err(|err| map_db_error("insert hook event state_change", err))?;
    } else {
        tracing::trace!(
            agent_id,
            "hook completion swallowed: state_version CAS failed"
        );
    }

    tx.commit()
        .map_err(|err| map_db_error("commit mark agent idle hook event", err))?;
    Ok(MarkerMatchedSyncOutcome {
        changes,
        denial_message: None,
    })
}

fn mark_agent_idle_log_event_outcome_sync(
    db: &Db,
    agent_id: &str,
    provider: &str,
    reply: Option<&str>,
    raw_path: &str,
    raw_offset: u64,
    provider_turn_id: Option<&str>,
) -> Result<MarkerMatchedSyncOutcome, CcbdError> {
    let mut conn = db.conn();
    let tx = conn
        .transaction()
        .map_err(|err| map_db_error("begin mark agent idle log event", err))?;
    let current = tx
        .query_row(
            "SELECT state, state_version FROM agents WHERE id = ?",
            params![agent_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
        )
        .optional()
        .map_err(|err| map_db_error("query state for log event matched", err))?;

    let Some((previous_state, state_version)) = current else {
        return Ok(MarkerMatchedSyncOutcome {
            changes: 0,
            denial_message: None,
        });
    };
    if previous_state != STATE_WAITING_FOR_ACK && previous_state != STATE_BUSY {
        tracing::trace!(agent_id, state = %previous_state, "log completion swallowed: agent not busy or waiting for ack");
        return Ok(MarkerMatchedSyncOutcome {
            changes: 0,
            denial_message: None,
        });
    }

    let dispatched_job_reply =
        if let Some(job) = query_dispatched_job_for_agent_sync(&tx, agent_id)? {
            if let Some(denial_message) = evidence_denial_for_job(&tx, agent_id, &job)? {
                insert_evidence_denied_event(&tx, agent_id, &job.id, &denial_message)?;
                tx.commit()
                    .map_err(|err| map_db_error("commit evidence denied log event", err))?;
                return Ok(MarkerMatchedSyncOutcome {
                    changes: 0,
                    denial_message: Some(denial_message),
                });
            }
            let (reply_text, reply_source) = if let Some(reply) = reply {
                (reply.to_string(), "log")
            } else {
                (
                    collect_reply_for_dispatched_job_sync(
                        &tx,
                        agent_id,
                        job.dispatched_at_seq_id,
                        &job.prompt_text,
                    )?,
                    "screen",
                )
            };
            Some((job.id, reply_text, reply_source, job.cancel_requested))
        } else {
            None
        };
    let reply_source = dispatched_job_reply
        .as_ref()
        .map(|(_, _, reply_source, _)| *reply_source)
        .unwrap_or_else(|| if reply.is_some() { "log" } else { "screen" });

    let changes = tx
        .execute(
            "UPDATE agents SET state = 'IDLE', sub_state = 'LogEvent', state_version = state_version + 1, updated_at = unixepoch() WHERE id = ? AND state IN ('WAITING_FOR_ACK', 'BUSY') AND state_version = ?",
            params![agent_id, state_version],
        )
        .map_err(|err| map_db_error("mark agent idle log event", err))?;

    if changes == 1 {
        if let Some((job_id, reply_text, _, cancel_requested)) = dispatched_job_reply {
            if cancel_requested {
                mark_job_cancelled_conn_sync(&tx, &job_id, &reply_text)?;
            } else {
                mark_job_completed_conn_sync(&tx, &job_id, &reply_text)?;
            }
        }

        let payload = json!({
            "from": previous_state,
            "to": STATE_IDLE,
            "sub_state": LOG_EVENT_SUB_STATE,
            "reason": LOG_EVENT_TASK_COMPLETE_REASON,
            "provider": provider,
            "raw_path": raw_path,
            "raw_offset": raw_offset,
            "provider_turn_id": provider_turn_id,
            "schema_version": 1,
            "reply_source": reply_source,
        })
        .to_string();
        tx.execute(
            "INSERT INTO events (agent_id, request_id, event_type, payload) VALUES (?, NULL, 'state_change', ?)",
            params![agent_id, payload],
        )
        .map_err(|err| map_db_error("insert log event state_change", err))?;
    } else {
        tracing::trace!(
            agent_id,
            "log completion swallowed: state_version CAS failed"
        );
    }

    tx.commit()
        .map_err(|err| map_db_error("commit mark agent idle log event", err))?;
    Ok(MarkerMatchedSyncOutcome {
        changes,
        denial_message: None,
    })
}

fn latest_ah_idle_marker_job_id(
    conn: &Connection,
    agent_id: &str,
    dispatched_at_seq_id: Option<i64>,
) -> Result<Option<String>, CcbdError> {
    let since_seq_id = dispatched_at_seq_id.unwrap_or(0);
    let mut stmt = conn
        .prepare(
            "SELECT payload FROM events WHERE agent_id = ? AND event_type = 'output_chunk' AND seq_id > ? ORDER BY seq_id ASC",
        )
        .map_err(|err| map_db_error("prepare query ah idle marker", err))?;
    let rows = stmt
        .query_map(params![agent_id, since_seq_id], |row| {
            row.get::<_, String>(0)
        })
        .map_err(|err| map_db_error("query ah idle marker", err))?;

    let mut marker = None;
    for row in rows {
        let payload = row.map_err(|err| map_db_error("read ah idle marker payload", err))?;
        let text = serde_json::from_str::<Value>(&payload)
            .ok()
            .and_then(|value| {
                value
                    .get("text")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .unwrap_or(payload);
        if let Some(job_id) = extract_ah_idle_marker_job_id(&text) {
            marker = Some(job_id);
        }
    }
    Ok(marker)
}

pub fn extract_ah_idle_marker_job_id(text: &str) -> Option<String> {
    let start = text.find("<<ah-idle:job-id=")? + "<<ah-idle:job-id=".len();
    let rest = &text[start..];
    let end = rest.find(">>")?;
    let job_id = rest[..end].trim();
    if job_id.is_empty() {
        None
    } else {
        Some(job_id.to_string())
    }
}

fn evidence_denial_for_job(
    conn: &Connection,
    agent_id: &str,
    job: &crate::db::schema::Job,
) -> Result<Option<String>, CcbdError> {
    if job.requires_physical_evidence
        && !has_job_evidence_sync(
            conn,
            agent_id,
            &job.id,
            &["mtime_changed", "diff_generated"],
        )?
    {
        return Ok(Some(EVIDENCE_DENY_MESSAGE.to_string()));
    }
    if job.requires_test_evidence
        && !has_job_evidence_sync(conn, agent_id, &job.id, &["test_passed"])?
    {
        return Ok(Some(format!(
            "{EVIDENCE_DENY_MESSAGE} Missing required evidence_type=test_passed."
        )));
    }
    Ok(None)
}

fn insert_evidence_denied_event(
    conn: &Connection,
    agent_id: &str,
    job_id: &str,
    message: &str,
) -> Result<(), CcbdError> {
    let payload = json!({
        "job_id": job_id,
        "reason": "missing_physical_evidence",
        "message": message,
    })
    .to_string();
    conn.execute(
        "INSERT INTO events (agent_id, request_id, event_type, payload) VALUES (?, NULL, 'evidence_denied', ?)",
        params![agent_id, payload],
    )
    .map_err(|err| map_db_error("insert evidence denied event", err))?;
    Ok(())
}

fn is_prompt_only_reply(reply_text: &str) -> bool {
    let stripped = strip_ansi_escapes(reply_text);
    let text = stripped.trim();
    if text.is_empty() {
        return true;
    }
    text.len() <= 4 && matches!(text, "$" | "#" | ">" | "✦" | "❯" | "▌" | "%")
}

fn ui_recapture_noop() -> UiCompletionRecaptureSyncOutcome {
    UiCompletionRecaptureSyncOutcome {
        changes: 0,
        affected_job: None,
        disposition: UiCompletionRecaptureDisposition::NoOp,
        stuck_event_seq_id: None,
        stuck_payload: None,
    }
}

fn recapture_content_hash(content: &str) -> String {
    format!("{:016x}", crate::pane_diff::compute_content_hash(content))
}

#[cfg(test)]
pub(crate) fn mark_agent_stuck_sync(db: &Db, agent_id: &str) -> Result<usize, CcbdError> {
    mark_agent_stuck_outcome_sync(db, agent_id).map(|outcome| outcome.changes)
}

fn mark_agent_stuck_outcome_sync(db: &Db, agent_id: &str) -> Result<StuckOutcome, CcbdError> {
    let mut conn = db.conn();
    let tx = conn
        .transaction()
        .map_err(|err| map_db_error("begin mark agent stuck", err))?;
    let current = tx
        .query_row(
            "SELECT state, state_version FROM agents WHERE id = ?",
            params![agent_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
        )
        .optional()
        .map_err(|err| map_db_error("query state for stuck", err))?;

    let Some((previous_state, state_version)) = current else {
        tx.rollback()
            .map_err(|err| map_db_error("rollback mark stuck missing agent", err))?;
        return Ok(StuckOutcome {
            changes: 0,
            agent_id: agent_id.to_string(),
            job_id: None,
            from_state: None,
            event_seq_id: None,
        });
    };
    if previous_state != STATE_BUSY && previous_state != STATE_WAITING_FOR_ACK {
        tracing::trace!(agent_id, state = %previous_state, "pane diff stuck swallowed: agent not busy or waiting for ack");
        tx.rollback()
            .map_err(|err| map_db_error("rollback mark stuck ignored state", err))?;
        return Ok(StuckOutcome {
            changes: 0,
            agent_id: agent_id.to_string(),
            job_id: None,
            from_state: Some(previous_state),
            event_seq_id: None,
        });
    }
    let affected_job = query_dispatched_job_for_agent_sync(&tx, agent_id)?.map(|job| job.id);

    let changes = tx
        .execute(
            "UPDATE agents SET state = 'STUCK', state_version = state_version + 1, updated_at = unixepoch() WHERE id = ? AND state IN ('BUSY', 'WAITING_FOR_ACK') AND state_version = ?",
            params![agent_id, state_version],
        )
        .map_err(|err| map_db_error("mark agent stuck", err))?;

    if changes == 1 {
        let payload = json!({
            "from": previous_state,
            "to": "STUCK",
            "reason": "PANE_DIFF_STUCK",
            "job_id": affected_job,
            "signal_kinds": ["state_machine"],
            "elapsed_secs": 0,
        })
        .to_string();
        tx.execute(
            "INSERT INTO events (agent_id, request_id, event_type, payload) VALUES (?, NULL, 'state_change', ?)",
            params![agent_id, payload],
        )
        .map_err(|err| map_db_error("insert stuck state_change", err))?;
        let event_seq_id = tx.last_insert_rowid();
        tx.commit()
            .map_err(|err| map_db_error("commit mark agent stuck", err))?;
        return Ok(StuckOutcome {
            changes,
            agent_id: agent_id.to_string(),
            job_id: affected_job,
            from_state: Some(previous_state),
            event_seq_id: Some(event_seq_id),
        });
    } else {
        tracing::trace!(
            agent_id,
            "pane diff stuck swallowed: state_version CAS failed"
        );
        tx.rollback()
            .map_err(|err| map_db_error("rollback mark stuck CAS failed", err))?;
        return Ok(StuckOutcome {
            changes: 0,
            agent_id: agent_id.to_string(),
            job_id: affected_job,
            from_state: Some(previous_state),
            event_seq_id: None,
        });
    }
}

pub(crate) fn mark_agent_unknown_sync(
    db: &Db,
    agent_id: &str,
    reason: &str,
    pane_bytes: Vec<u8>,
    failed_rules: Value,
) -> Result<usize, CcbdError> {
    let mut conn = db.conn();
    let tx = conn
        .transaction()
        .map_err(|err| map_db_error("begin mark agent unknown", err))?;
    let current = tx
        .query_row(
            "SELECT state, state_version FROM agents WHERE id = ?",
            params![agent_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
        )
        .optional()
        .map_err(|err| map_db_error("query state for unknown", err))?;

    let Some((previous_state, state_version)) = current else {
        tx.rollback()
            .map_err(|err| map_db_error("rollback mark unknown missing agent", err))?;
        return Ok(0);
    };
    if !is_active_state(previous_state.as_str()) {
        tracing::trace!(agent_id, state = %previous_state, "marker timeout swallowed: agent not in active state");
        tx.rollback()
            .map_err(|err| map_db_error("rollback mark unknown ignored state", err))?;
        return Ok(0);
    }

    let changes = tx
        .execute(
            "UPDATE agents SET state = 'UNKNOWN', error_code = ?, state_version = state_version + 1, updated_at = unixepoch() WHERE id = ? AND state IN ('SPAWNING', 'WAITING_FOR_ACK', 'BUSY') AND state_version = ?",
            params![reason, agent_id, state_version],
        )
        .map_err(|err| map_db_error("mark agent unknown", err))?;

    if changes == 1 {
        tx.execute(
            "UPDATE evidence SET status = 'SEALED' WHERE agent_id = ? AND status = 'PENDING'",
            params![agent_id],
        )
        .map_err(|err| map_db_error("seal pending evidence", err))?;

        let payload = json!({
            "from": previous_state,
            "to": "UNKNOWN",
            "reason": reason,
        })
        .to_string();
        tx.execute(
            "INSERT INTO events (agent_id, request_id, event_type, payload) VALUES (?, NULL, 'state_change', ?)",
            params![agent_id, payload],
        )
        .map_err(|err| map_db_error("insert unknown state_change", err))?;
        let event_seq_id = tx.last_insert_rowid();
        let evidence_id = format!("evi_{}", uuid::Uuid::new_v4().simple());
        tx.execute(
            "INSERT INTO evidence (id, agent_id, event_seq_id, pane_bytes, failed_rules) VALUES (?, ?, ?, ?, ?)",
            params![evidence_id, agent_id, event_seq_id, pane_bytes, failed_rules.to_string()],
        )
        .map_err(|err| map_db_error("insert evidence", err))?;
        if let Some(job) = query_dispatched_job_for_agent_sync(&tx, agent_id)? {
            if job.cancel_requested {
                mark_job_cancelled_conn_sync(&tx, &job.id, "")?;
            } else {
                mark_dispatched_jobs_failed_for_agent_conn_sync(&tx, agent_id, reason)?;
            }
        }
    } else {
        tracing::trace!(
            agent_id,
            "marker timeout swallowed: state_version CAS failed"
        );
        tx.rollback()
            .map_err(|err| map_db_error("rollback mark unknown CAS failed", err))?;
        return Ok(0);
    }

    tx.commit()
        .map_err(|err| map_db_error("commit mark agent unknown", err))?;
    Ok(changes)
}

pub(crate) fn mark_agent_failed_from_intervention_sync(
    db: &Db,
    agent_id: &str,
    reason: &str,
) -> Result<usize, CcbdError> {
    let mut conn = db.conn();
    let tx = conn
        .transaction()
        .map_err(|err| map_db_error("begin mark agent failed", err))?;
    let current = tx
        .query_row(
            "SELECT state, state_version FROM agents WHERE id = ?",
            params![agent_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
        )
        .optional()
        .map_err(|err| map_db_error("query state for failed", err))?;

    let Some((previous_state, state_version)) = current else {
        tx.rollback()
            .map_err(|err| map_db_error("rollback mark failed missing agent", err))?;
        return Ok(0);
    };
    if previous_state != STATE_SPAWNING_INTERVENTION {
        tx.rollback()
            .map_err(|err| map_db_error("rollback mark failed ignored state", err))?;
        return Ok(0);
    }

    let changes = tx
        .execute(
            "UPDATE agents SET state = ?, error_code = ?, state_version = state_version + 1, updated_at = unixepoch() WHERE id = ? AND state = ? AND state_version = ?",
            params![
                STATE_FAILED,
                reason,
                agent_id,
                STATE_SPAWNING_INTERVENTION,
                state_version
            ],
        )
        .map_err(|err| map_db_error("mark agent failed", err))?;

    if changes == 1 {
        let payload = json!({
            "from": previous_state,
            "to": STATE_FAILED,
            "reason": reason,
        })
        .to_string();
        tx.execute(
            "INSERT INTO events (agent_id, request_id, event_type, payload) VALUES (?, NULL, 'state_change', ?)",
            params![agent_id, payload],
        )
        .map_err(|err| map_db_error("insert failed state_change", err))?;
    }

    tx.commit()
        .map_err(|err| map_db_error("commit mark agent failed", err))?;
    Ok(changes)
}

pub async fn mark_agent_unknown(
    db: Db,
    agent_id: String,
    reason: String,
    pane_bytes: Vec<u8>,
    failed_rules: Value,
) -> Result<(usize, Option<String>), CcbdError> {
    let affected_job =
        crate::db::jobs::query_dispatched_job_for_agent(db.clone(), agent_id.clone())
            .await?
            .map(|job| job.id);
    spawn_db("state_machine::mark_agent_unknown", move || {
        mark_agent_unknown_sync(&db, &agent_id, &reason, pane_bytes, failed_rules)
    })
    .await
    .map(|changes| {
        let affected_job = if changes > 0 { affected_job } else { None };
        (changes, affected_job)
    })
}

pub async fn mark_agent_failed_from_intervention(
    db: Db,
    agent_id: String,
    reason: String,
) -> Result<usize, CcbdError> {
    spawn_db(
        "state_machine::mark_agent_failed_from_intervention",
        move || mark_agent_failed_from_intervention_sync(&db, &agent_id, &reason),
    )
    .await
}

pub async fn mark_agent_stuck(db: Db, agent_id: String) -> Result<usize, CcbdError> {
    let outcome = spawn_db("state_machine::mark_agent_stuck", move || {
        mark_agent_stuck_outcome_sync(&db, &agent_id)
    })
    .await?;
    if outcome.changes > 0 {
        let payload = json!({
            "job_id": outcome.job_id,
            "signal_kinds": ["state_machine"],
            "elapsed_secs": 0,
        });
        crate::orchestrator::pubsub::notify_event(crate::orchestrator::pubsub::EventFrame {
            event_id: outcome.event_seq_id.unwrap_or(0),
            kind: "stuck".to_string(),
            agent_id: outcome.agent_id.clone(),
            job_id: outcome.job_id.clone(),
            state: Some(STATE_STUCK.to_string()),
            ts_unix_micro: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_micros() as i64)
                .unwrap_or(0),
            payload: Some(payload),
        });
    }
    Ok(outcome.changes)
}

pub async fn mark_agent_waiting_for_ack(
    db: Db,
    agent_id: String,
    expected_version: i64,
) -> Result<bool, CcbdError> {
    spawn_db("state_machine::mark_agent_waiting_for_ack", move || {
        let mut conn = db.conn();
        mark_agent_waiting_for_ack_sync(&mut conn, &agent_id, expected_version)
    })
    .await
}

pub async fn mark_agent_prompt_pending(
    db: Db,
    agent_id: String,
    reason: String,
) -> Result<usize, CcbdError> {
    spawn_db("state_machine::mark_agent_prompt_pending", move || {
        mark_agent_prompt_pending_sync(&db, &agent_id, &reason)
    })
    .await
}

pub async fn transit_agent_state(
    db: Db,
    agent_id: String,
    from_state_list: Vec<String>,
    to_state: String,
    reason: Option<String>,
) -> Result<(), CcbdError> {
    spawn_db("state_machine::transit_agent_state", move || {
        let mut conn = db.conn();
        let from_states = from_state_list
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>();
        transit_agent_state_sync(
            &mut conn,
            &agent_id,
            &from_states,
            &to_state,
            reason.as_deref(),
        )
    })
    .await
}

pub async fn mark_agent_idle_matched(
    db: Db,
    agent_id: String,
) -> Result<(usize, Option<String>), CcbdError> {
    let outcome = mark_agent_idle_matched_outcome(db, agent_id).await?;
    Ok((outcome.changes, outcome.affected_job))
}

pub async fn mark_agent_idle_recaptured(
    db: Db,
    agent_id: String,
) -> Result<UiCompletionRecaptureOutcome, CcbdError> {
    mark_agent_idle_recaptured_with_pane(db, agent_id, String::new()).await
}

pub async fn mark_agent_idle_recaptured_with_pane(
    db: Db,
    agent_id: String,
    pane_snapshot: String,
) -> Result<UiCompletionRecaptureOutcome, CcbdError> {
    let publish_agent_id = agent_id.clone();
    let outcome = spawn_db("state_machine::mark_agent_idle_recaptured", move || {
        mark_agent_idle_recaptured_outcome_sync_with_pane(&db, &agent_id, &pane_snapshot)
    })
    .await?;
    if outcome.changes > 0 && outcome.disposition == UiCompletionRecaptureDisposition::MarkedStuck {
        crate::orchestrator::pubsub::notify_event(crate::orchestrator::pubsub::EventFrame {
            event_id: outcome.stuck_event_seq_id.unwrap_or(0),
            kind: "stuck".to_string(),
            agent_id: publish_agent_id,
            job_id: outcome.affected_job.clone(),
            state: Some(STATE_STUCK.to_string()),
            ts_unix_micro: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_micros() as i64)
                .unwrap_or(0),
            payload: outcome.stuck_payload.clone(),
        });
    }
    Ok(UiCompletionRecaptureOutcome {
        changes: outcome.changes,
        affected_job: outcome.affected_job,
        disposition: outcome.disposition,
    })
}

pub async fn mark_agent_idle_log_event(
    db: Db,
    agent_id: String,
    provider: String,
    reply: Option<String>,
    raw_path: String,
    raw_offset: u64,
    provider_turn_id: Option<String>,
) -> Result<(usize, Option<String>), CcbdError> {
    let outcome = mark_agent_idle_log_event_outcome(
        db,
        agent_id,
        provider,
        reply,
        raw_path,
        raw_offset,
        provider_turn_id,
    )
    .await?;
    Ok((outcome.changes, outcome.affected_job))
}

pub async fn mark_agent_idle_hook_event(
    db: Db,
    agent_id: String,
    provider: String,
    hook_event: String,
    event_id: Option<String>,
    reply: Option<String>,
) -> Result<(usize, Option<String>), CcbdError> {
    let affected_job =
        crate::db::jobs::query_dispatched_job_for_agent(db.clone(), agent_id.clone())
            .await?
            .map(|job| job.id);
    let agent_id_for_denial = agent_id.clone();
    spawn_db("state_machine::mark_agent_idle_hook_event", move || {
        mark_agent_idle_hook_event_outcome_sync(
            &db,
            &agent_id,
            &provider,
            &hook_event,
            event_id.as_deref(),
            reply.as_deref(),
            None,
        )
    })
    .await
    .and_then(|outcome| {
        if let Some(message) = &outcome.denial_message {
            let message = message.clone();
            let agent_id = agent_id_for_denial.clone();
            tokio::spawn(send_evidence_denial_nudge(agent_id, message));
        }
        let affected_job = if outcome.changes > 0 {
            affected_job
        } else {
            None
        };
        Ok((outcome.changes, affected_job))
    })
}

async fn send_evidence_denial_nudge(agent_id: String, message: String) {
    #[cfg(test)]
    record_test_denial_nudge(&agent_id, &message);
    if let Err(err) = crate::agent_io::send_text_to_registered_pane(&agent_id, message).await {
        tracing::warn!(agent_id = %agent_id, error = %err, "failed to inject evidence denial");
    }
}

#[cfg(test)]
static TEST_DENIAL_NUDGES: std::sync::LazyLock<std::sync::Mutex<Vec<(String, String)>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(Vec::new()));

#[cfg(test)]
fn record_test_denial_nudge(agent_id: &str, message: &str) {
    if let Ok(mut nudges) = TEST_DENIAL_NUDGES.lock() {
        nudges.push((agent_id.to_string(), message.to_string()));
    }
}

#[cfg(test)]
fn clear_test_denial_nudges() {
    TEST_DENIAL_NUDGES.lock().unwrap().clear();
}

#[cfg(test)]
fn test_denial_nudges() -> Vec<(String, String)> {
    TEST_DENIAL_NUDGES.lock().unwrap().clone()
}

pub async fn mark_agent_idle_matched_outcome(
    db: Db,
    agent_id: String,
) -> Result<MarkerMatchedOutcome, CcbdError> {
    let affected_job =
        crate::db::jobs::query_dispatched_job_for_agent(db.clone(), agent_id.clone())
            .await?
            .map(|job| job.id);
    let agent_id_for_denial = agent_id.clone();
    spawn_db("state_machine::mark_agent_idle_matched", move || {
        mark_agent_idle_matched_outcome_sync(&db, &agent_id)
    })
    .await
    .and_then(|outcome| {
        if let Some(message) = &outcome.denial_message {
            let message = message.clone();
            let agent_id = agent_id_for_denial.clone();
            tokio::spawn(send_evidence_denial_nudge(agent_id, message));
        }
        let affected_job = if outcome.changes > 0 {
            affected_job
        } else {
            None
        };
        Ok(MarkerMatchedOutcome {
            changes: outcome.changes,
            affected_job,
            denial_message: outcome.denial_message,
        })
    })
}

pub async fn mark_agent_idle_log_event_outcome(
    db: Db,
    agent_id: String,
    provider: String,
    reply: Option<String>,
    raw_path: String,
    raw_offset: u64,
    provider_turn_id: Option<String>,
) -> Result<MarkerMatchedOutcome, CcbdError> {
    let affected_job =
        crate::db::jobs::query_dispatched_job_for_agent(db.clone(), agent_id.clone())
            .await?
            .map(|job| job.id);
    let agent_id_for_denial = agent_id.clone();
    spawn_db("state_machine::mark_agent_idle_log_event", move || {
        mark_agent_idle_log_event_outcome_sync(
            &db,
            &agent_id,
            &provider,
            reply.as_deref(),
            &raw_path,
            raw_offset,
            provider_turn_id.as_deref(),
        )
    })
    .await
    .and_then(|outcome| {
        if let Some(message) = &outcome.denial_message {
            let message = message.clone();
            let agent_id = agent_id_for_denial.clone();
            tokio::spawn(send_evidence_denial_nudge(agent_id, message));
        }
        let affected_job = if outcome.changes > 0 {
            affected_job
        } else {
            None
        };
        Ok(MarkerMatchedOutcome {
            changes: outcome.changes,
            affected_job,
            denial_message: outcome.denial_message,
        })
    })
}

#[cfg(test)]
mod tests {
    use super::{
        STATE_BUSY, STATE_CRASHED, STATE_FAILED, STATE_IDLE, STATE_KILLED, STATE_PROMPT_PENDING,
        STATE_SPAWNING, STATE_SPAWNING_INTERVENTION, STATE_STUCK, STATE_UNKNOWN,
        STATE_WAITING_FOR_ACK, is_active_state, mark_agent_idle_log_event_sync,
        mark_agent_idle_matched_sync, mark_agent_idle_recaptured_sync,
        mark_agent_prompt_pending_sync, mark_agent_stuck_sync, mark_agent_unknown_sync,
        transit_agent_state_sync,
    };
    use crate::db::agents::insert_agent_sync;
    use crate::db::events::insert_event_sync;
    use crate::db::jobs::{
        claim_next_job_sync, insert_job_sync, query_job_sync, request_dispatched_job_cancel_sync,
        set_job_evidence_requirements_sync, update_dispatched_seq_id_sync,
    };
    use crate::db::sessions::insert_session_sync;
    use crate::db::{Db, init};
    use crate::error::CcbdError;

    fn with_test_db_handle<T>(test: impl FnOnce(&Db) -> T) -> T {
        let file = tempfile::NamedTempFile::new().unwrap();
        let db = init(file.path()).unwrap();
        test(&db)
    }

    fn seed_busy_agent(db: &Db, agent_id: &str) {
        {
            let conn = db.conn();
            insert_session_sync(&conn, "s_assert", "p_assert", "/tmp/assert").unwrap();
            insert_agent_sync(&conn, agent_id, "s_assert", "bash", "BUSY", Some(1)).unwrap();
        }
    }

    fn seed_dispatched_agent_job(db: &Db, agent_id: &str, state: &str, job_id: &str) -> i64 {
        {
            let conn = db.conn();
            insert_session_sync(
                &conn,
                &format!("s_{agent_id}"),
                &format!("p_{agent_id}"),
                "/tmp/log-event",
            )
            .unwrap();
            insert_agent_sync(
                &conn,
                agent_id,
                &format!("s_{agent_id}"),
                "bash",
                STATE_IDLE,
                Some(1),
            )
            .unwrap();
            insert_job_sync(&conn, job_id, agent_id, None, "echo PONG\n").unwrap();
        }
        claim_next_job_sync(db, agent_id).unwrap().unwrap();
        let conn = db.conn();
        let dispatch_seq = insert_event_sync(
            &conn,
            agent_id,
            None,
            "command_received",
            r#"{"cmd":"echo PONG\n","status":"SENT"}"#,
        )
        .unwrap();
        update_dispatched_seq_id_sync(&conn, job_id, dispatch_seq).unwrap();
        conn.execute(
            "UPDATE agents SET state = ?, state_version = state_version + 1 WHERE id = ?",
            rusqlite::params![state, agent_id],
        )
        .unwrap();
        dispatch_seq
    }

    fn state_change_count(db: &Db, agent_id: &str) -> i64 {
        db.conn()
            .query_row(
                "SELECT COUNT(*) FROM events WHERE agent_id = ? AND event_type = 'state_change'",
                [agent_id],
                |row| row.get(0),
            )
            .unwrap()
    }

    #[test]
    fn test_transit_agent_state_sync_updates_agent_and_emits_event() {
        with_test_db_handle(|db| {
            {
                let conn = db.conn();
                insert_session_sync(&conn, "s_transit", "p_transit", "/tmp/transit").unwrap();
                insert_agent_sync(
                    &conn,
                    "a_transit",
                    "s_transit",
                    "bash",
                    STATE_SPAWNING,
                    Some(1),
                )
                .unwrap();
            }

            let mut conn = db.conn();
            transit_agent_state_sync(
                &mut conn,
                "a_transit",
                &[STATE_SPAWNING],
                STATE_IDLE,
                Some("INIT_READY"),
            )
            .unwrap();
            let (state, version, payload): (String, i64, String) = conn
                .query_row(
                    "SELECT agents.state, agents.state_version, events.payload \
                     FROM agents JOIN events ON events.agent_id = agents.id \
                     WHERE agents.id = 'a_transit' AND events.event_type = 'state_change'",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .unwrap();
            let payload: serde_json::Value = serde_json::from_str(&payload).unwrap();

            assert_eq!(state, STATE_IDLE);
            assert_eq!(version, 2);
            assert_eq!(payload["from"], STATE_SPAWNING);
            assert_eq!(payload["to"], STATE_IDLE);
            assert_eq!(payload["reason"], "INIT_READY");
        });
    }

    #[test]
    fn test_transit_agent_state_sync_rejects_invalid_from_state_without_event() {
        with_test_db_handle(|db| {
            seed_busy_agent(db, "a_invalid_from");

            let mut conn = db.conn();
            let err = transit_agent_state_sync(
                &mut conn,
                "a_invalid_from",
                &[STATE_IDLE],
                STATE_SPAWNING,
                Some("invalid_test_transition"),
            )
            .unwrap_err();
            let state: String = conn
                .query_row(
                    "SELECT state FROM agents WHERE id = 'a_invalid_from'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();

            assert!(matches!(err, CcbdError::AgentWrongState { .. }));
            assert_eq!(state, STATE_BUSY);
            drop(conn);
            assert_eq!(state_change_count(db, "a_invalid_from"), 0);
        });
    }

    #[test]
    fn test_transit_agent_state_sync_missing_agent_errors_without_event() {
        with_test_db_handle(|db| {
            let mut conn = db.conn();
            let err = transit_agent_state_sync(
                &mut conn,
                "a_missing",
                &[STATE_IDLE],
                STATE_SPAWNING,
                Some("missing_test_transition"),
            )
            .unwrap_err();

            assert!(matches!(err, CcbdError::AgentNotFound(agent) if agent == "a_missing"));
            drop(conn);
            assert_eq!(state_change_count(db, "a_missing"), 0);
        });
    }

    #[test]
    fn test_mark_agent_stuck_from_busy_succeeds() {
        with_test_db_handle(|db| {
            seed_busy_agent(db, "a_stuck");

            let changes = mark_agent_stuck_sync(db, "a_stuck").unwrap();
            let state: String = db
                .conn()
                .query_row("SELECT state FROM agents WHERE id = 'a_stuck'", [], |row| {
                    row.get(0)
                })
                .unwrap();

            assert_eq!(changes, 1);
            assert_eq!(state, "STUCK");
        });
    }

    #[test]
    fn test_mark_agent_stuck_from_idle_swallowed() {
        with_test_db_handle(|db| {
            {
                let conn = db.conn();
                insert_session_sync(&conn, "s_idle", "p_idle", "/tmp/idle").unwrap();
                insert_agent_sync(&conn, "a_idle", "s_idle", "bash", "IDLE", Some(1)).unwrap();
            }

            let changes = mark_agent_stuck_sync(db, "a_idle").unwrap();
            let state: String = db
                .conn()
                .query_row("SELECT state FROM agents WHERE id = 'a_idle'", [], |row| {
                    row.get(0)
                })
                .unwrap();

            assert_eq!(changes, 0);
            assert_eq!(state, "IDLE");
        });
    }

    #[test]
    fn test_mark_agent_stuck_state_version_cas() {
        with_test_db_handle(|db| {
            seed_busy_agent(db, "a_cas");

            assert_eq!(mark_agent_stuck_sync(db, "a_cas").unwrap(), 1);
            assert_eq!(mark_agent_stuck_sync(db, "a_cas").unwrap(), 0);

            let (state, state_version, event_count): (String, i64, i64) = db
                .conn()
                .query_row(
                    "SELECT agents.state, agents.state_version, COUNT(events.seq_id) \
                     FROM agents \
                     LEFT JOIN events ON events.agent_id = agents.id AND events.event_type = 'state_change' \
                     WHERE agents.id = 'a_cas' \
                     GROUP BY agents.id",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .unwrap();

            assert_eq!(state, "STUCK");
            assert_eq!(state_version, 2);
            assert_eq!(event_count, 1);
        });
    }

    #[test]
    fn test_mark_agent_stuck_emits_state_change_event() {
        with_test_db_handle(|db| {
            seed_busy_agent(db, "a_event");

            mark_agent_stuck_sync(db, "a_event").unwrap();
            let payload: String = db
                .conn()
                .query_row(
                    "SELECT payload FROM events WHERE agent_id = 'a_event' AND event_type = 'state_change'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            let payload: serde_json::Value = serde_json::from_str(&payload).unwrap();

            assert_eq!(payload["from"], "BUSY");
            assert_eq!(payload["to"], "STUCK");
            assert_eq!(payload["reason"], "PANE_DIFF_STUCK");
        });
    }

    #[test]
    fn test_ui_recapture_can_mark_stuck_agent_idle_without_opening_live_marker_guard() {
        with_test_db_handle(|db| {
            seed_dispatched_agent_job(db, "a_recapture_stuck", STATE_STUCK, "job_recapture_stuck");
            insert_event_sync(
                &db.conn(),
                "a_recapture_stuck",
                None,
                "output_chunk",
                r#"{"text":"recaptured reply\n"}"#,
            )
            .unwrap();

            let live_changes = mark_agent_idle_matched_sync(db, "a_recapture_stuck").unwrap();
            let recapture_changes =
                mark_agent_idle_recaptured_sync(db, "a_recapture_stuck").unwrap();
            let (state, sub_state, status, payload): (String, String, String, String) = db
                .conn()
                .query_row(
                    "SELECT agents.state, agents.sub_state, jobs.status, events.payload \
                     FROM agents \
                     JOIN jobs ON jobs.agent_id = agents.id \
                     JOIN events ON events.agent_id = agents.id AND events.event_type = 'state_change' \
                     WHERE agents.id = 'a_recapture_stuck' \
                     ORDER BY events.seq_id DESC LIMIT 1",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
                )
                .unwrap();
            let payload: serde_json::Value = serde_json::from_str(&payload).unwrap();

            assert_eq!(live_changes, 0);
            assert_eq!(recapture_changes, 1);
            assert_eq!(state, STATE_IDLE);
            assert_eq!(sub_state, "Matched");
            assert_eq!(status, "COMPLETED");
            assert_eq!(payload["from"], STATE_STUCK);
            assert_eq!(payload["reason"], "UI_COMPLETION_RECAPTURE_MATCHED");
        });
    }

    #[test]
    fn test_mark_agent_unknown_writes_evidence() {
        with_test_db_handle(|db| {
            seed_busy_agent(db, "a_assert");
            mark_agent_unknown_sync(
                db,
                "a_assert",
                "PTY_MARKER_TIMEOUT",
                b"pane".to_vec(),
                serde_json::json!(["rule"]),
            )
            .unwrap();
            let (state, evidence_status): (String, String) = db
                .conn()
                .query_row(
                    "SELECT agents.state, evidence.status FROM agents JOIN evidence ON evidence.agent_id = agents.id WHERE agents.id='a_assert'",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .unwrap();
            assert_eq!(state, "UNKNOWN");
            assert_eq!(evidence_status, "PENDING");
        });
    }

    #[test]
    fn log_event_completes_busy_job_with_log_reply() {
        with_test_db_handle(|db| {
            seed_dispatched_agent_job(db, "a_log_busy", STATE_BUSY, "job_log_busy");

            let changes = mark_agent_idle_log_event_sync(
                db,
                "a_log_busy",
                "codex",
                Some("LOG PONG"),
                "/tmp/rollout.jsonl",
                42,
                Some("turn-1"),
            )
            .unwrap();
            let (state, sub_state, status, reply): (String, String, String, String) = db
                .conn()
                .query_row(
                    "SELECT agents.state, agents.sub_state, jobs.status, jobs.reply_text \
                     FROM agents JOIN jobs ON jobs.agent_id = agents.id \
                     WHERE agents.id = 'a_log_busy'",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
                )
                .unwrap();

            assert_eq!(changes, 1);
            assert_eq!(state, STATE_IDLE);
            assert_eq!(sub_state, "LogEvent");
            assert_eq!(status, "COMPLETED");
            assert_eq!(reply, "LOG PONG");
        });
    }

    #[test]
    fn ui_marker_does_not_complete_dispatched_job_while_log_monitor_active() {
        with_test_db_handle(|db| {
            seed_dispatched_agent_job(
                db,
                "a_log_monitor_active",
                STATE_BUSY,
                "job_log_monitor_active",
            );
            insert_event_sync(
                &db.conn(),
                "a_log_monitor_active",
                None,
                "output_chunk",
                r#"{"text":"REAL UI FALLBACK REPLY"}"#,
            )
            .unwrap();
            let (cancel_tx, _cancel_rx) = tokio::sync::oneshot::channel();
            crate::completion::registry::register(
                "a_log_monitor_active".to_string(),
                crate::completion::registry::LogMonitorEntry {
                    provider: "claude".to_string(),
                    log_root: std::path::PathBuf::from("/tmp/claude-log-root"),
                    state: crate::completion::reader::LogReadState::default(),
                    cancel_tx,
                },
            );

            let changes = mark_agent_idle_matched_sync(db, "a_log_monitor_active").unwrap();
            let (state, status): (String, String) = db
                .conn()
                .query_row(
                    "SELECT agents.state, jobs.status \
                     FROM agents JOIN jobs ON jobs.agent_id = agents.id \
                     WHERE agents.id = 'a_log_monitor_active'",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .unwrap();
            crate::completion::registry::cancel("a_log_monitor_active");

            assert_eq!(changes, 0);
            assert_eq!(state, STATE_BUSY);
            assert_eq!(status, "DISPATCHED");
        });
    }

    #[test]
    fn log_event_does_not_complete_spawning_agent() {
        with_test_db_handle(|db| {
            seed_dispatched_agent_job(db, "a_log_spawn", STATE_SPAWNING, "job_log_spawn");

            let changes = mark_agent_idle_log_event_sync(
                db,
                "a_log_spawn",
                "codex",
                Some("LOG PONG"),
                "/tmp/rollout.jsonl",
                42,
                Some("turn-1"),
            )
            .unwrap();
            let (state, job_status): (String, String) = db
                .conn()
                .query_row(
                    "SELECT agents.state, jobs.status FROM agents JOIN jobs ON jobs.agent_id = agents.id WHERE agents.id = 'a_log_spawn'",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .unwrap();

            assert_eq!(changes, 0);
            assert_eq!(state, STATE_SPAWNING);
            assert_eq!(job_status, "DISPATCHED");
        });
    }

    #[test]
    fn log_event_state_change_reason_is_log_event_task_complete() {
        with_test_db_handle(|db| {
            seed_dispatched_agent_job(db, "a_log_reason", STATE_BUSY, "job_log_reason");

            mark_agent_idle_log_event_sync(
                db,
                "a_log_reason",
                "codex",
                Some("LOG PONG"),
                "/tmp/rollout.jsonl",
                42,
                Some("turn-1"),
            )
            .unwrap();
            let payload: String = db
                .conn()
                .query_row(
                    "SELECT payload FROM events WHERE agent_id = 'a_log_reason' AND event_type = 'state_change' ORDER BY seq_id DESC LIMIT 1",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            let payload: serde_json::Value = serde_json::from_str(&payload).unwrap();

            assert_eq!(payload["reason"], "LOG_EVENT_TASK_COMPLETE");
            assert_eq!(payload["provider"], "codex");
            assert_eq!(payload["raw_path"], "/tmp/rollout.jsonl");
            assert_eq!(payload["raw_offset"], 42);
            assert_eq!(payload["provider_turn_id"], "turn-1");
            assert_eq!(payload["reply_source"], "log");
        });
    }

    #[test]
    fn log_event_missing_reply_uses_screen_collection() {
        with_test_db_handle(|db| {
            let dispatch_seq =
                seed_dispatched_agent_job(db, "a_log_screen", STATE_BUSY, "job_log_screen");
            insert_event_sync(
                &db.conn(),
                "a_log_screen",
                None,
                "output_chunk",
                r#"{"text":"echo PONG\nSCREEN PONG\n"}"#,
            )
            .unwrap();

            let changes = mark_agent_idle_log_event_sync(
                db,
                "a_log_screen",
                "codex",
                None,
                "/tmp/rollout.jsonl",
                43,
                None,
            )
            .unwrap();
            let (reply, payload): (String, String) = db
                .conn()
                .query_row(
                    "SELECT jobs.reply_text, events.payload \
                     FROM jobs JOIN events ON events.agent_id = jobs.agent_id \
                     WHERE jobs.id = 'job_log_screen' AND events.event_type = 'state_change' \
                     ORDER BY events.seq_id DESC LIMIT 1",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .unwrap();
            let payload: serde_json::Value = serde_json::from_str(&payload).unwrap();

            assert!(dispatch_seq > 0);
            assert_eq!(changes, 1);
            assert!(reply.contains("SCREEN PONG"), "reply was {reply:?}");
            assert_eq!(payload["reply_source"], "screen");
        });
    }

    #[test]
    fn hook_push_waiting_for_ack_to_idle_uses_hook_source_not_log_payload() {
        with_test_db_handle(|db| {
            seed_dispatched_agent_job(
                db,
                "a_hook_waiting",
                STATE_WAITING_FOR_ACK,
                "job_hook_waiting",
            );

            let changes = super::mark_agent_idle_hook_event_sync(
                db,
                "a_hook_waiting",
                "codex",
                "stop",
                Some("evt-hook-waiting"),
                Some("HOOK PONG"),
            )
            .unwrap();
            let (state, sub_state, payload): (String, String, String) = db
                .conn()
                .query_row(
                    "SELECT agents.state, agents.sub_state, events.payload \
                     FROM agents JOIN events ON events.agent_id = agents.id \
                     WHERE agents.id = 'a_hook_waiting' AND events.event_type = 'state_change' \
                     ORDER BY events.seq_id DESC LIMIT 1",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .unwrap();
            let payload: serde_json::Value = serde_json::from_str(&payload).unwrap();

            assert_eq!(changes, 1);
            assert_eq!(state, STATE_IDLE);
            assert_eq!(sub_state, "HookEvent");
            assert_eq!(payload["from"], STATE_WAITING_FOR_ACK);
            assert_eq!(payload["to"], STATE_IDLE);
            assert_eq!(payload["sub_state"], "HookEvent");
            assert_eq!(payload["source"], "hook");
            assert_eq!(payload["hook_event"], "stop");
            assert_eq!(payload["provider"], "codex");
            assert_eq!(payload["event_id"], "evt-hook-waiting");
            assert_eq!(payload["reply_source"], "hook");
            assert!(payload.get("raw_path").is_none());
            assert!(payload.get("raw_offset").is_none());
            assert!(payload.get("provider_turn_id").is_none());
        });
    }

    #[test]
    fn hook_push_busy_to_idle_completes_dispatched_job() {
        with_test_db_handle(|db| {
            seed_dispatched_agent_job(db, "a_hook_busy", STATE_BUSY, "job_hook_busy");

            let changes = super::mark_agent_idle_hook_event_sync(
                db,
                "a_hook_busy",
                "codex",
                "stop",
                Some("evt-hook-busy"),
                Some("HOOK PONG"),
            )
            .unwrap();
            let (state, status, reply): (String, String, String) = db
                .conn()
                .query_row(
                    "SELECT agents.state, jobs.status, jobs.reply_text \
                     FROM agents JOIN jobs ON jobs.agent_id = agents.id \
                     WHERE agents.id = 'a_hook_busy'",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .unwrap();

            assert_eq!(changes, 1);
            assert_eq!(state, STATE_IDLE);
            assert_eq!(status, "COMPLETED");
            assert_eq!(reply, "HOOK PONG");
        });
    }

    #[test]
    fn hook_push_duplicate_idle_returns_zero() {
        with_test_db_handle(|db| {
            seed_dispatched_agent_job(db, "a_hook_idle", STATE_IDLE, "job_hook_idle");
            let before_version: i64 = db
                .conn()
                .query_row(
                    "SELECT state_version FROM agents WHERE id = 'a_hook_idle'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();

            let changes = super::mark_agent_idle_hook_event_sync(
                db,
                "a_hook_idle",
                "codex",
                "stop",
                Some("evt-hook-idle"),
                Some("HOOK PONG"),
            )
            .unwrap();
            let (state, state_version, event_count, job_status): (String, i64, i64, String) = db
                .conn()
                .query_row(
                    "SELECT agents.state, agents.state_version, COUNT(events.seq_id), jobs.status \
                     FROM agents \
                     JOIN jobs ON jobs.agent_id = agents.id \
                     LEFT JOIN events ON events.agent_id = agents.id AND events.event_type = 'state_change' \
                     WHERE agents.id = 'a_hook_idle' \
                     GROUP BY agents.id",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
                )
                .unwrap();

            assert_eq!(changes, 0);
            assert_eq!(state, STATE_IDLE);
            assert_eq!(state_version, before_version);
            assert_eq!(event_count, 0);
            assert_eq!(job_status, "DISPATCHED");
        });
    }

    #[test]
    fn hook_push_stale_state_version_loses_without_event() {
        with_test_db_handle(|db| {
            seed_dispatched_agent_job(db, "a_hook_stale", STATE_BUSY, "job_hook_stale");
            let stale_version: i64 = db
                .conn()
                .query_row(
                    "SELECT state_version FROM agents WHERE id = 'a_hook_stale'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            db.conn()
                .execute(
                    "UPDATE agents SET state_version = state_version + 1 WHERE id = 'a_hook_stale'",
                    [],
                )
                .unwrap();

            let changes = super::mark_agent_idle_hook_event_at_version_sync(
                db,
                "a_hook_stale",
                "codex",
                "stop",
                Some("evt-hook-stale"),
                Some("HOOK PONG"),
                stale_version,
            )
            .unwrap();
            let (state, event_count, job_status): (String, i64, String) = db
                .conn()
                .query_row(
                    "SELECT agents.state, COUNT(events.seq_id), jobs.status \
                     FROM agents \
                     JOIN jobs ON jobs.agent_id = agents.id \
                     LEFT JOIN events ON events.agent_id = agents.id AND events.event_type = 'state_change' \
                     WHERE agents.id = 'a_hook_stale' \
                     GROUP BY agents.id",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .unwrap();

            assert_eq!(changes, 0);
            assert_eq!(state, STATE_BUSY);
            assert_eq!(event_count, 0);
            assert_eq!(job_status, "DISPATCHED");
        });
    }

    #[test]
    fn hook_push_cancel_requested_job_is_cancelled_like_pull_v2() {
        with_test_db_handle(|db| {
            seed_dispatched_agent_job(db, "a_hook_cancel", STATE_BUSY, "job_hook_cancel");
            request_dispatched_job_cancel_sync(db, "job_hook_cancel").unwrap();

            let changes = super::mark_agent_idle_hook_event_sync(
                db,
                "a_hook_cancel",
                "codex",
                "stop",
                Some("evt-hook-cancel"),
                Some("HOOK CANCELLED"),
            )
            .unwrap();
            let (state, status, reply): (String, String, String) = db
                .conn()
                .query_row(
                    "SELECT agents.state, jobs.status, jobs.reply_text \
                     FROM agents JOIN jobs ON jobs.agent_id = agents.id \
                     WHERE agents.id = 'a_hook_cancel'",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .unwrap();

            assert_eq!(changes, 1);
            assert_eq!(state, STATE_IDLE);
            assert_eq!(status, "CANCELLED");
            assert_eq!(reply, "HOOK CANCELLED");
        });
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn f4_hook_push_evidence_denial_enters_pane_nudge_path() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let db = init(file.path()).unwrap();
        let agent_id = "a_hook_denial_nudge";
        super::clear_test_denial_nudges();
        seed_dispatched_agent_job(&db, agent_id, STATE_BUSY, "job_hook_denial_nudge");
        set_job_evidence_requirements_sync(&db.conn(), "job_hook_denial_nudge", true, false)
            .unwrap();

        let outcome = super::mark_agent_idle_hook_event(
            db.clone(),
            agent_id.to_string(),
            "codex".to_string(),
            "stop".to_string(),
            Some("evt-hook-denial".to_string()),
            Some("HOOK PONG".to_string()),
        )
        .await
        .unwrap();

        assert_eq!(outcome, (0, None));
        let mut nudges = Vec::new();
        for _ in 0..50 {
            nudges = super::test_denial_nudges();
            if nudges
                .iter()
                .any(|(agent, message)| agent == agent_id && message.contains("SYSTEM DENY"))
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }

        assert!(
            nudges
                .iter()
                .any(|(agent, message)| agent == agent_id && message.contains("SYSTEM DENY")),
            "hook denial must be sent through the pane nudge path: {nudges:?}"
        );
    }

    #[test]
    fn test_cancel_requested_prompt_only_reply_marks_cancelled() {
        with_test_db_handle(|db| {
            seed_busy_agent(db, "a_cancel");
            let before = {
                let conn = db.conn();
                insert_job_sync(&conn, "job_cancel", "a_cancel", None, "sleep 10\n").unwrap();
                let before = insert_event_sync(
                    &conn,
                    "a_cancel",
                    None,
                    "command_received",
                    r#"{"cmd":"sleep 10\n","status":"SENT"}"#,
                )
                .unwrap();
                insert_event_sync(&conn, "a_cancel", None, "output_chunk", r#"{"text":"$ "}"#)
                    .unwrap();
                conn.execute("UPDATE agents SET state = 'IDLE' WHERE id = 'a_cancel'", [])
                    .unwrap();
                before
            };
            claim_next_job_sync(db, "a_cancel").unwrap().unwrap();
            {
                let conn = db.conn();
                update_dispatched_seq_id_sync(&conn, "job_cancel", before).unwrap();
                conn.execute("UPDATE agents SET state = 'BUSY' WHERE id = 'a_cancel'", [])
                    .unwrap();
            }
            request_dispatched_job_cancel_sync(db, "job_cancel").unwrap();

            let changes = mark_agent_idle_matched_sync(db, "a_cancel").unwrap();
            let job = query_job_sync(&db.conn(), "job_cancel").unwrap().unwrap();
            let state: String = db
                .conn()
                .query_row(
                    "SELECT state FROM agents WHERE id = 'a_cancel'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();

            assert_eq!(changes, 1);
            assert_eq!(state, "IDLE");
            assert_eq!(job.status, "CANCELLED");
        });
    }

    #[test]
    fn test_is_prompt_only_reply_recognizes_known_prompts() {
        use super::is_prompt_only_reply;
        assert!(is_prompt_only_reply(""));
        assert!(is_prompt_only_reply("  "));
        assert!(is_prompt_only_reply(">"));
        assert!(is_prompt_only_reply("$"));
        assert!(is_prompt_only_reply("✦"));
        assert!(is_prompt_only_reply("❯"));
        assert!(is_prompt_only_reply("  > "));
    }

    #[test]
    fn test_is_prompt_only_reply_does_not_swallow_real_content() {
        use super::is_prompt_only_reply;
        assert!(!is_prompt_only_reply("done"));
        assert!(!is_prompt_only_reply("Hello, here is your answer"));
        assert!(!is_prompt_only_reply("> quoted text"));
        assert!(!is_prompt_only_reply("pong"));
    }

    #[test]
    fn test_state_constants_match_strings() {
        assert_eq!(STATE_SPAWNING, "SPAWNING");
        assert_eq!(STATE_SPAWNING_INTERVENTION, "SPAWNING_INTERVENTION");
        assert_eq!(STATE_IDLE, "IDLE");
        assert_eq!(STATE_WAITING_FOR_ACK, "WAITING_FOR_ACK");
        assert_eq!(STATE_BUSY, "BUSY");
        assert_eq!(STATE_PROMPT_PENDING, "PROMPT_PENDING");
        assert_eq!(STATE_STUCK, "STUCK");
        assert_eq!(STATE_FAILED, "FAILED");
        assert_eq!(STATE_CRASHED, "CRASHED");
        assert_eq!(STATE_KILLED, "KILLED");
        assert_eq!(STATE_UNKNOWN, "UNKNOWN");
    }

    #[test]
    fn test_is_active_state_covers_spawning_waiting_busy() {
        assert!(is_active_state("SPAWNING"));
        assert!(is_active_state("WAITING_FOR_ACK"));
        assert!(is_active_state("BUSY"));
        assert!(!is_active_state("IDLE"));
        assert!(!is_active_state("SPAWNING_INTERVENTION"));
        assert!(!is_active_state("PROMPT_PENDING"));
        assert!(!is_active_state("STUCK"));
        assert!(!is_active_state("FAILED"));
        assert!(!is_active_state("CRASHED"));
        assert!(!is_active_state("KILLED"));
        assert!(!is_active_state("UNKNOWN"));
    }

    #[test]
    fn test_mark_agent_prompt_pending_accepts_idle_and_spawning_rejects_active_dispatch() {
        with_test_db_handle(|db| {
            {
                let conn = db.conn();
                insert_session_sync(&conn, "s_pending", "p_pending", "/tmp/pending").unwrap();
                insert_agent_sync(
                    &conn,
                    "a_spawning",
                    "s_pending",
                    "bash",
                    STATE_SPAWNING,
                    Some(0),
                )
                .unwrap();
                insert_agent_sync(&conn, "a_idle", "s_pending", "bash", STATE_IDLE, Some(1))
                    .unwrap();
                insert_agent_sync(
                    &conn,
                    "a_waiting",
                    "s_pending",
                    "bash",
                    STATE_WAITING_FOR_ACK,
                    Some(2),
                )
                .unwrap();
                insert_agent_sync(&conn, "a_busy", "s_pending", "bash", STATE_BUSY, Some(3))
                    .unwrap();
            }

            for agent_id in ["a_waiting", "a_busy"] {
                let err =
                    mark_agent_prompt_pending_sync(db, agent_id, "UNKNOWN_PROMPT").unwrap_err();
                let state: String = db
                    .conn()
                    .query_row("SELECT state FROM agents WHERE id = ?", [agent_id], |row| {
                        row.get(0)
                    })
                    .unwrap();
                assert!(matches!(err, CcbdError::AgentWrongState { .. }));
                assert!(matches!(state.as_str(), STATE_WAITING_FOR_ACK | STATE_BUSY));
                assert_eq!(state_change_count(db, agent_id), 0);
            }

            for agent_id in ["a_idle", "a_spawning"] {
                let changes =
                    mark_agent_prompt_pending_sync(db, agent_id, "UNKNOWN_PROMPT").unwrap();
                let state: String = db
                    .conn()
                    .query_row("SELECT state FROM agents WHERE id = ?", [agent_id], |row| {
                        row.get(0)
                    })
                    .unwrap();
                assert_eq!(changes, 1);
                assert_eq!(state, STATE_PROMPT_PENDING);
                assert_eq!(state_change_count(db, agent_id), 1);
            }
        });
    }

    #[test]
    fn test_mark_agent_prompt_pending_rejects_terminal_without_event() {
        with_test_db_handle(|db| {
            {
                let conn = db.conn();
                insert_session_sync(&conn, "s_terminal", "p_terminal", "/tmp/terminal").unwrap();
                insert_agent_sync(
                    &conn,
                    "a_crashed",
                    "s_terminal",
                    "bash",
                    STATE_CRASHED,
                    Some(1),
                )
                .unwrap();
            }

            let err =
                mark_agent_prompt_pending_sync(db, "a_crashed", "UNKNOWN_PROMPT").unwrap_err();
            let state: String = db
                .conn()
                .query_row(
                    "SELECT state FROM agents WHERE id = 'a_crashed'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();

            assert!(matches!(err, CcbdError::AgentWrongState { .. }));
            assert_eq!(state, STATE_CRASHED);
            assert_eq!(state_change_count(db, "a_crashed"), 0);
        });
    }

    #[test]
    fn test_prompt_pending_can_resolve_to_idle_or_busy() {
        with_test_db_handle(|db| {
            {
                let conn = db.conn();
                insert_session_sync(&conn, "s_resolve", "p_resolve", "/tmp/resolve").unwrap();
                insert_agent_sync(
                    &conn,
                    "a_idle_resolve",
                    "s_resolve",
                    "bash",
                    STATE_PROMPT_PENDING,
                    Some(1),
                )
                .unwrap();
                insert_agent_sync(
                    &conn,
                    "a_busy_resolve",
                    "s_resolve",
                    "bash",
                    STATE_PROMPT_PENDING,
                    Some(2),
                )
                .unwrap();
            }
            let mut conn = db.conn();
            transit_agent_state_sync(
                &mut conn,
                "a_idle_resolve",
                &[STATE_PROMPT_PENDING],
                STATE_IDLE,
                Some("prompt_resolved"),
            )
            .unwrap();
            transit_agent_state_sync(
                &mut conn,
                "a_busy_resolve",
                &[STATE_PROMPT_PENDING],
                STATE_BUSY,
                Some("prompt_resolved_job_in_flight"),
            )
            .unwrap();

            let states: Vec<String> = ["a_idle_resolve", "a_busy_resolve"]
                .into_iter()
                .map(|agent_id| {
                    conn.query_row("SELECT state FROM agents WHERE id = ?", [agent_id], |row| {
                        row.get(0)
                    })
                    .unwrap()
                })
                .collect();
            assert_eq!(states, [STATE_IDLE, STATE_BUSY]);
        });
    }
}
