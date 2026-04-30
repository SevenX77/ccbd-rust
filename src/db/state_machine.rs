use crate::db::Db;
use crate::db::common::{map_db_error, spawn_db};
use crate::db::jobs::{
    collect_reply_for_dispatched_job_sync, mark_dispatched_jobs_failed_for_agent_conn_sync,
    mark_job_completed_conn_sync, query_dispatched_job_for_agent_sync,
};
use crate::error::CcbdError;
use rusqlite::{OptionalExtension, params};
use serde_json::{Value, json};

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
    if !matches!(previous_state.as_str(), "SPAWNING" | "BUSY") {
        tracing::trace!(agent_id, state = %previous_state, "marker match swallowed: agent not waiting for marker");
        return Ok(0);
    }

    let dispatched_job_reply =
        if let Some(job) = query_dispatched_job_for_agent_sync(&tx, agent_id)? {
            let reply_text =
                collect_reply_for_dispatched_job_sync(&tx, agent_id, job.dispatched_at_seq_id)?;
            if previous_state == "BUSY" && is_prompt_only_reply(&reply_text) {
                tracing::trace!(agent_id, "marker match swallowed: prompt-only job reply");
                tx.rollback()
                    .map_err(|err| map_db_error("rollback prompt-only marker match", err))?;
                return Ok(0);
            }
            Some((job.id, reply_text))
        } else {
            None
        };

    let changes = tx
        .execute(
            "UPDATE agents SET state = 'IDLE', sub_state = 'Matched', state_version = state_version + 1, updated_at = unixepoch() WHERE id = ? AND state IN ('SPAWNING', 'BUSY') AND state_version = ?",
            params![agent_id, state_version],
        )
        .map_err(|err| map_db_error("mark agent idle matched", err))?;

    if changes == 1 {
        if let Some((job_id, reply_text)) = dispatched_job_reply {
            mark_job_completed_conn_sync(&tx, &job_id, &reply_text)?;
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
    let stripped = strip_ansi_csi(reply_text);
    let text = stripped.trim();
    text.is_empty() || matches!(text, "$" | "#" | ">" | "✦")
}

fn strip_ansi_csi(input: &str) -> String {
    let mut out = String::new();
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' && chars.peek() == Some(&'[') {
            let _ = chars.next();
            for c in chars.by_ref() {
                if ('@'..='~').contains(&c) {
                    break;
                }
            }
        } else {
            out.push(ch);
        }
    }
    out
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
    if !matches!(previous_state.as_str(), "SPAWNING" | "BUSY") {
        tracing::trace!(agent_id, state = %previous_state, "marker timeout swallowed: agent not waiting for marker");
        tx.rollback()
            .map_err(|err| map_db_error("rollback mark unknown ignored state", err))?;
        return Ok(0);
    }

    let changes = tx
        .execute(
            "UPDATE agents SET state = 'UNKNOWN', error_code = ?, state_version = state_version + 1, updated_at = unixepoch() WHERE id = ? AND state IN ('SPAWNING', 'BUSY') AND state_version = ?",
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
        mark_dispatched_jobs_failed_for_agent_conn_sync(&tx, agent_id, reason)?;
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
) -> Result<usize, CcbdError> {
    let affected_job =
        crate::db::jobs::query_dispatched_job_for_agent(db.clone(), agent_id.clone())
            .await?
            .map(|job| job.id);
    let result = spawn_db("state_machine::mark_agent_unknown", move || {
        mark_agent_unknown_sync(&db, &agent_id, &reason, pane_bytes, failed_rules)
    })
    .await;
    if matches!(result, Ok(changes) if changes > 0) {
        if let Some(job_id) = affected_job {
            crate::orchestrator::pubsub::notify_job_update(&job_id);
        }
        crate::orchestrator::wake_up();
    }
    result
}

pub async fn mark_agent_idle_matched(db: Db, agent_id: String) -> Result<usize, CcbdError> {
    let affected_job =
        crate::db::jobs::query_dispatched_job_for_agent(db.clone(), agent_id.clone())
            .await?
            .map(|job| job.id);
    let result = spawn_db("state_machine::mark_agent_idle_matched", move || {
        mark_agent_idle_matched_sync(&db, &agent_id)
    })
    .await;
    if matches!(result, Ok(changes) if changes > 0) {
        if let Some(job_id) = affected_job {
            crate::orchestrator::pubsub::notify_job_update(&job_id);
        }
        crate::orchestrator::wake_up();
    }
    result
}

#[cfg(test)]
mod tests {
    use super::mark_agent_unknown_sync;
    use crate::db::agents::insert_agent_sync;
    use crate::db::sessions::insert_session_sync;
    use crate::db::{Db, init};

    fn with_test_db_handle<T>(test: impl FnOnce(&Db) -> T) -> T {
        let file = tempfile::NamedTempFile::new().unwrap();
        let db = init(file.path()).unwrap();
        test(&db)
    }

    fn seed_busy_agent(db: &Db, agent_id: &str) {
        {
            let conn = db.conn();
            insert_session_sync(&conn, "s_assert", "p_assert", "/tmp/assert", 999).unwrap();
            insert_agent_sync(&conn, agent_id, "s_assert", "bash", "BUSY", Some(1)).unwrap();
        }
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
}
