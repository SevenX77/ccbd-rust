use crate::db::Db;
use crate::db::common::{map_db_error, spawn_db};
use crate::db::jobs::{
    collect_reply_for_dispatched_job_sync, mark_dispatched_jobs_failed_for_agent_conn_sync,
    mark_job_cancelled_conn_sync, mark_job_completed_conn_sync,
    query_dispatched_job_for_agent_sync, strip_ansi_escapes,
};
use crate::error::CcbdError;
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};

/// 状态字符串常量 (避免散落字面量).
pub const STATE_SPAWNING: &str = "SPAWNING";
pub const STATE_IDLE: &str = "IDLE";
pub const STATE_WAITING_FOR_ACK: &str = "WAITING_FOR_ACK";
pub const STATE_BUSY: &str = "BUSY";
/// Agent is blocked on an interactive prompt and waiting for master resolution.
///
/// This is neither an active execution state nor a terminal state: orchestrator must not
/// dispatch new jobs to it, and lifecycle/reconcile paths must not silently crash or kill it.
pub const STATE_PROMPT_PENDING: &str = "PROMPT_PENDING";
pub const STATE_STUCK: &str = "STUCK";
pub const STATE_CRASHED: &str = "CRASHED";
pub const STATE_KILLED: &str = "KILLED";
pub const STATE_UNKNOWN: &str = "UNKNOWN";

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
    let allowed = [STATE_IDLE, STATE_WAITING_FOR_ACK, STATE_BUSY];
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
            "UPDATE agents SET state = 'PROMPT_PENDING', state_version = state_version + 1, updated_at = unixepoch() WHERE id = ? AND state IN ('IDLE', 'WAITING_FOR_ACK', 'BUSY') AND state_version = ?",
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

pub(crate) fn mark_agent_idle_matched_sync(db: &Db, agent_id: &str) -> Result<usize, CcbdError> {
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
        return Ok(0);
    };
    if !is_active_state(previous_state.as_str()) {
        tracing::trace!(agent_id, state = %previous_state, "marker match swallowed: agent not in active state");
        return Ok(0);
    }

    let dispatched_job_reply = if let Some(job) =
        query_dispatched_job_for_agent_sync(&tx, agent_id)?
    {
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
            return Ok(0);
        }
        Some((job.id, reply_text, job.cancel_requested))
    } else {
        None
    };

    let changes = tx
        .execute(
            "UPDATE agents SET state = 'IDLE', sub_state = 'Matched', state_version = state_version + 1, updated_at = unixepoch() WHERE id = ? AND state IN ('SPAWNING', 'WAITING_FOR_ACK', 'BUSY') AND state_version = ?",
            params![agent_id, state_version],
        )
        .map_err(|err| map_db_error("mark agent idle matched", err))?;

    if changes == 1 {
        if let Some((job_id, reply_text, cancel_requested)) = dispatched_job_reply {
            if cancel_requested {
                mark_job_cancelled_conn_sync(&tx, &job_id, &reply_text)?;
            } else {
                mark_job_completed_conn_sync(&tx, &job_id, &reply_text)?;
            }
        }

        let payload = serde_json::json!({
            "from": previous_state,
            "to": "IDLE",
            "sub_state": "Matched",
            "reason": "MARKER_MATCHED",
        })
        .to_string();
        tx.execute(
            "INSERT INTO events (agent_id, request_id, event_type, payload) VALUES (?, NULL, 'state_change', ?)",
            params![agent_id, payload],
        )
        .map_err(|err| map_db_error("insert marker matched state_change", err))?;
    } else {
        tracing::trace!(agent_id, "marker match swallowed: state_version CAS failed");
    }

    tx.commit()
        .map_err(|err| map_db_error("commit mark agent idle matched", err))?;
    Ok(changes)
}

fn is_prompt_only_reply(reply_text: &str) -> bool {
    let stripped = strip_ansi_escapes(reply_text);
    let text = stripped.trim();
    if text.is_empty() {
        return true;
    }
    text.len() <= 4 && matches!(text, "$" | "#" | ">" | "✦" | "❯" | "▌" | "%")
}

pub(crate) fn mark_agent_stuck_sync(db: &Db, agent_id: &str) -> Result<usize, CcbdError> {
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
        return Ok(0);
    };
    if previous_state != STATE_BUSY && previous_state != STATE_WAITING_FOR_ACK {
        tracing::trace!(agent_id, state = %previous_state, "pane diff stuck swallowed: agent not busy or waiting for ack");
        tx.rollback()
            .map_err(|err| map_db_error("rollback mark stuck ignored state", err))?;
        return Ok(0);
    }

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
        })
        .to_string();
        tx.execute(
            "INSERT INTO events (agent_id, request_id, event_type, payload) VALUES (?, NULL, 'state_change', ?)",
            params![agent_id, payload],
        )
        .map_err(|err| map_db_error("insert stuck state_change", err))?;
    } else {
        tracing::trace!(
            agent_id,
            "pane diff stuck swallowed: state_version CAS failed"
        );
        tx.rollback()
            .map_err(|err| map_db_error("rollback mark stuck CAS failed", err))?;
        return Ok(0);
    }

    tx.commit()
        .map_err(|err| map_db_error("commit mark agent stuck", err))?;
    Ok(changes)
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

pub async fn mark_agent_stuck(db: Db, agent_id: String) -> Result<usize, CcbdError> {
    spawn_db("state_machine::mark_agent_stuck", move || {
        mark_agent_stuck_sync(&db, &agent_id)
    })
    .await
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
    let affected_job =
        crate::db::jobs::query_dispatched_job_for_agent(db.clone(), agent_id.clone())
            .await?
            .map(|job| job.id);
    spawn_db("state_machine::mark_agent_idle_matched", move || {
        mark_agent_idle_matched_sync(&db, &agent_id)
    })
    .await
    .map(|changes| {
        let affected_job = if changes > 0 { affected_job } else { None };
        (changes, affected_job)
    })
}

#[cfg(test)]
mod tests {
    use super::{
        STATE_BUSY, STATE_CRASHED, STATE_IDLE, STATE_KILLED, STATE_PROMPT_PENDING, STATE_SPAWNING,
        STATE_STUCK, STATE_UNKNOWN, STATE_WAITING_FOR_ACK, is_active_state, is_waiting_for_ack,
        mark_agent_idle_matched_sync, mark_agent_prompt_pending_sync, mark_agent_stuck_sync,
        mark_agent_unknown_sync, mark_agent_waiting_for_ack_sync, transit_agent_state_sync,
    };
    use crate::db::agents::insert_agent_sync;
    use crate::db::events::insert_event_sync;
    use crate::db::jobs::{
        claim_next_job_sync, insert_job_sync, query_job_sync, request_dispatched_job_cancel_sync,
        update_dispatched_seq_id_sync,
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

    fn seed_waiting_for_ack_agent(db: &Db, agent_id: &str) {
        {
            let conn = db.conn();
            let session_id = format!("s_{agent_id}");
            insert_session_sync(&conn, &session_id, "p_ack", "/tmp/ack").unwrap();
            insert_agent_sync(
                &conn,
                agent_id,
                &session_id,
                "bash",
                STATE_WAITING_FOR_ACK,
                Some(1),
            )
            .unwrap();
        }
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
                    STATE_WAITING_FOR_ACK,
                    Some(1),
                )
                .unwrap();
            }

            let mut conn = db.conn();
            transit_agent_state_sync(
                &mut conn,
                "a_transit",
                &[STATE_WAITING_FOR_ACK],
                STATE_BUSY,
                Some("ACK_VISUAL_DIFF"),
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

            assert_eq!(state, STATE_BUSY);
            assert_eq!(version, 2);
            assert_eq!(payload["from"], STATE_WAITING_FOR_ACK);
            assert_eq!(payload["to"], STATE_BUSY);
            assert_eq!(payload["reason"], "ACK_VISUAL_DIFF");
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
                STATE_WAITING_FOR_ACK,
                Some("ack_pending"),
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
                STATE_WAITING_FOR_ACK,
                Some("ack_pending"),
            )
            .unwrap_err();

            assert!(matches!(err, CcbdError::AgentNotFound(agent) if agent == "a_missing"));
            drop(conn);
            assert_eq!(state_change_count(db, "a_missing"), 0);
        });
    }

    #[test]
    fn test_mark_idle_matched_accepts_waiting_for_ack() {
        with_test_db_handle(|db| {
            seed_waiting_for_ack_agent(db, "a_ack_idle");

            let changes = mark_agent_idle_matched_sync(db, "a_ack_idle").unwrap();
            let state: String = db
                .conn()
                .query_row(
                    "SELECT state FROM agents WHERE id = 'a_ack_idle'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();

            assert_eq!(changes, 1);
            assert_eq!(state, STATE_IDLE);
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
    fn test_mark_agent_stuck_accepts_waiting_for_ack() {
        with_test_db_handle(|db| {
            seed_waiting_for_ack_agent(db, "a_ack_stuck");

            let changes = mark_agent_stuck_sync(db, "a_ack_stuck").unwrap();
            let (state, payload): (String, String) = db
                .conn()
                .query_row(
                    "SELECT agents.state, events.payload \
                     FROM agents JOIN events ON events.agent_id = agents.id \
                     WHERE agents.id = 'a_ack_stuck' AND events.event_type = 'state_change'",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .unwrap();
            let payload: serde_json::Value = serde_json::from_str(&payload).unwrap();

            assert_eq!(changes, 1);
            assert_eq!(state, STATE_STUCK);
            assert_eq!(payload["from"], STATE_WAITING_FOR_ACK);
            assert_eq!(payload["reason"], "PANE_DIFF_STUCK");
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
    fn test_mark_agent_unknown_accepts_waiting_for_ack() {
        with_test_db_handle(|db| {
            seed_waiting_for_ack_agent(db, "a_ack_unknown");

            let changes = mark_agent_unknown_sync(
                db,
                "a_ack_unknown",
                "PTY_MARKER_TIMEOUT",
                b"pane".to_vec(),
                serde_json::json!(["rule"]),
            )
            .unwrap();
            let (state, evidence_status): (String, String) = db
                .conn()
                .query_row(
                    "SELECT agents.state, evidence.status \
                     FROM agents JOIN evidence ON evidence.agent_id = agents.id \
                     WHERE agents.id='a_ack_unknown'",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .unwrap();

            assert_eq!(changes, 1);
            assert_eq!(state, STATE_UNKNOWN);
            assert_eq!(evidence_status, "PENDING");
        });
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
        assert_eq!(STATE_IDLE, "IDLE");
        assert_eq!(STATE_WAITING_FOR_ACK, "WAITING_FOR_ACK");
        assert_eq!(STATE_BUSY, "BUSY");
        assert_eq!(STATE_PROMPT_PENDING, "PROMPT_PENDING");
        assert_eq!(STATE_STUCK, "STUCK");
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
        assert!(!is_active_state("PROMPT_PENDING"));
        assert!(!is_active_state("STUCK"));
        assert!(!is_active_state("CRASHED"));
        assert!(!is_active_state("KILLED"));
        assert!(!is_active_state("UNKNOWN"));
    }

    #[test]
    fn test_is_waiting_for_ack_only_matches_waiting() {
        assert!(is_waiting_for_ack("WAITING_FOR_ACK"));
        assert!(!is_waiting_for_ack("IDLE"));
        assert!(!is_waiting_for_ack("BUSY"));
        assert!(!is_waiting_for_ack("PROMPT_PENDING"));
        assert!(!is_waiting_for_ack("SPAWNING"));
    }

    #[test]
    fn test_mark_agent_prompt_pending_accepts_idle_waiting_and_busy() {
        with_test_db_handle(|db| {
            {
                let conn = db.conn();
                insert_session_sync(&conn, "s_pending", "p_pending", "/tmp/pending").unwrap();
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

            for agent_id in ["a_idle", "a_waiting", "a_busy"] {
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

    #[test]
    fn test_mark_waiting_for_ack_succeeds_from_idle() {
        with_test_db_handle(|db| {
            {
                let conn = db.conn();
                insert_session_sync(&conn, "s_idle", "p_idle", "/tmp/idle").unwrap();
                insert_agent_sync(&conn, "a_idle", "s_idle", "bash", STATE_IDLE, Some(1)).unwrap();
            }

            let mut conn = db.conn();
            let ok = mark_agent_waiting_for_ack_sync(&mut conn, "a_idle", 1).unwrap();
            let (state, version): (String, i64) = conn
                .query_row(
                    "SELECT state, state_version FROM agents WHERE id='a_idle'",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .unwrap();

            assert!(ok);
            assert_eq!(state, STATE_WAITING_FOR_ACK);
            assert_eq!(version, 2);
            drop(conn);
            assert_eq!(state_change_count(db, "a_idle"), 1);
        });
    }

    #[test]
    fn test_mark_waiting_for_ack_fails_from_busy() {
        with_test_db_handle(|db| {
            seed_busy_agent(db, "a_busy");

            let mut conn = db.conn();
            let ok = mark_agent_waiting_for_ack_sync(&mut conn, "a_busy", 1).unwrap();

            assert!(!ok);
            drop(conn);
            assert_eq!(state_change_count(db, "a_busy"), 0);
        });
    }

    #[test]
    fn test_mark_waiting_for_ack_fails_on_version_mismatch() {
        with_test_db_handle(|db| {
            {
                let conn = db.conn();
                insert_session_sync(&conn, "s_idle2", "p_idle2", "/tmp/idle2").unwrap();
                insert_agent_sync(&conn, "a_idle2", "s_idle2", "bash", STATE_IDLE, Some(1))
                    .unwrap();
                conn.execute(
                    "UPDATE agents SET state_version = 2 WHERE id = 'a_idle2'",
                    [],
                )
                .unwrap();
            }

            let mut conn = db.conn();
            let ok = mark_agent_waiting_for_ack_sync(&mut conn, "a_idle2", 1).unwrap();

            assert!(!ok);
            drop(conn);
            assert_eq!(state_change_count(db, "a_idle2"), 0);
        });
    }

    #[test]
    fn test_mark_waiting_for_ack_concurrent_only_one_wins() {
        with_test_db_handle(|db| {
            {
                let conn = db.conn();
                insert_session_sync(&conn, "s_race", "p_race", "/tmp/race").unwrap();
                insert_agent_sync(&conn, "a_race", "s_race", "bash", STATE_IDLE, Some(1)).unwrap();
            }

            let mut conn = db.conn();
            let ok1 = mark_agent_waiting_for_ack_sync(&mut conn, "a_race", 1).unwrap();
            let ok2 = mark_agent_waiting_for_ack_sync(&mut conn, "a_race", 1).unwrap();

            assert!(ok1);
            assert!(!ok2);
            drop(conn);
            assert_eq!(state_change_count(db, "a_race"), 1);
        });
    }
}
