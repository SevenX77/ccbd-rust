use crate::db::Db;
use crate::db::common::{map_db_error, spawn_db};
use crate::db::jobs::{
    collect_reply_for_dispatched_job_sync, mark_dispatched_jobs_failed_for_agent_conn_sync,
    mark_job_cancelled_conn_sync, mark_job_completed_conn_sync,
    query_dispatched_job_for_agent_sync, strip_ansi_escapes,
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
            "UPDATE agents SET state = 'IDLE', sub_state = 'Matched', state_version = state_version + 1, updated_at = unixepoch() WHERE id = ? AND state IN ('SPAWNING', 'BUSY') AND state_version = ?",
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
    text.is_empty() || matches!(text, "$" | "#" | ">" | "✦")
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
    if previous_state != "BUSY" {
        tracing::trace!(agent_id, state = %previous_state, "pane diff stuck swallowed: agent not busy");
        tx.rollback()
            .map_err(|err| map_db_error("rollback mark stuck ignored state", err))?;
        return Ok(0);
    }

    let changes = tx
        .execute(
            "UPDATE agents SET state = 'STUCK', state_version = state_version + 1, updated_at = unixepoch() WHERE id = ? AND state = 'BUSY' AND state_version = ?",
            params![agent_id, state_version],
        )
        .map_err(|err| map_db_error("mark agent stuck", err))?;

    if changes == 1 {
        let payload = json!({
            "from": "BUSY",
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
    use super::{mark_agent_idle_matched_sync, mark_agent_stuck_sync, mark_agent_unknown_sync};
    use crate::db::agents::insert_agent_sync;
    use crate::db::events::insert_event_sync;
    use crate::db::jobs::{
        claim_next_job_sync, insert_job_sync, query_job_sync, request_dispatched_job_cancel_sync,
        update_dispatched_seq_id_sync,
    };
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
            insert_session_sync(&conn, "s_assert", "p_assert", "/tmp/assert").unwrap();
            insert_agent_sync(&conn, agent_id, "s_assert", "bash", "BUSY", Some(1)).unwrap();
        }
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
}
