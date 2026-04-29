use crate::db::Db;
use crate::db::common::map_db_error;
use crate::error::CcbdError;
use rusqlite::{OptionalExtension, TransactionBehavior, params};
use serde_json::{Value, json};

#[derive(Debug)]
pub struct AssertStateOutcome {
    pub state_change_seq_id: i64,
}

pub fn mark_agent_idle_matched(db: &Db, agent_id: &str) -> Result<usize, CcbdError> {
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

    let changes = tx
        .execute(
            "UPDATE agents SET state = 'IDLE', sub_state = 'Matched', state_version = state_version + 1, updated_at = unixepoch() WHERE id = ? AND state IN ('SPAWNING', 'BUSY') AND state_version = ?",
            params![agent_id, state_version],
        )
        .map_err(|err| map_db_error("mark agent idle matched", err))?;

    if changes == 1 {
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

pub fn mark_agent_unknown(
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

pub fn assert_state_to_idle_tx(
    db: &Db,
    agent_id: &str,
    evidence_id: &str,
) -> Result<AssertStateOutcome, CcbdError> {
    let mut conn = db.conn();
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|err| map_db_error("begin assert_state", err))?;

    let evidence_agent_id = tx
        .query_row(
            "SELECT agent_id FROM evidence WHERE id = ?",
            params![evidence_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|err| map_db_error("query assert evidence", err))?
        .ok_or_else(|| CcbdError::DbEvidenceNotFound {
            details: format!("evidence_id={evidence_id}"),
        })?;
    if evidence_agent_id != agent_id {
        return Err(CcbdError::DbEvidenceNotFound {
            details: "agent_id mismatch".into(),
        });
    }

    let current = tx
        .query_row(
            "SELECT state, state_version FROM agents WHERE id = ?",
            params![agent_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
        )
        .optional()
        .map_err(|err| map_db_error("query assert agent", err))?;
    let Some((current_state, state_version)) = current else {
        return Err(CcbdError::AgentNotFound(agent_id.to_string()));
    };
    if current_state != "UNKNOWN" {
        return Err(CcbdError::AgentWrongState { current_state });
    }

    let changes = tx
        .execute(
            "UPDATE agents SET state='IDLE', sub_state='Asserted', state_version=state_version+1, updated_at=unixepoch() WHERE id=? AND state='UNKNOWN' AND state_version=?",
            params![agent_id, state_version],
        )
        .map_err(|err| map_db_error("assert agent state", err))?;
    if changes == 0 {
        let current_state = tx
            .query_row(
                "SELECT state FROM agents WHERE id = ?",
                params![agent_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|err| map_db_error("requery assert agent", err))?
            .unwrap_or_else(|| "MISSING".to_string());
        return Err(CcbdError::AgentWrongState { current_state });
    }

    tx.execute(
        "UPDATE evidence SET status = 'REVIEWED', l3_asserted_state = 'IDLE' WHERE id = ?",
        params![evidence_id],
    )
    .map_err(|err| map_db_error("update assert evidence", err))?;
    let payload = json!({
        "from": "UNKNOWN",
        "to": "IDLE",
        "sub_state": "Asserted",
        "reason": "L3_ASSERTED",
        "evidence_id": evidence_id,
    })
    .to_string();
    tx.execute(
        "INSERT INTO events (agent_id, request_id, event_type, payload) VALUES (?, NULL, 'state_change', ?)",
        params![agent_id, payload],
    )
    .map_err(|err| map_db_error("insert assert state_change", err))?;
    let state_change_seq_id = tx.last_insert_rowid();
    tx.commit()
        .map_err(|err| map_db_error("commit assert_state", err))?;

    Ok(AssertStateOutcome {
        state_change_seq_id,
    })
}

#[cfg(test)]
mod tests {
    use super::{assert_state_to_idle_tx, mark_agent_unknown};
    use crate::db::agents::insert_agent;
    use crate::db::sessions::insert_session;
    use crate::db::{Db, init};

    fn with_test_db_handle<T>(test: impl FnOnce(&Db) -> T) -> T {
        let file = tempfile::NamedTempFile::new().unwrap();
        let db = init(file.path()).unwrap();
        test(&db)
    }

    fn seed_unknown_with_evidence(db: &Db, agent_id: &str) -> String {
        {
            let conn = db.conn();
            insert_session(&conn, "s_assert", "p_assert", "/tmp/assert", 999).unwrap();
            insert_agent(&conn, agent_id, "s_assert", "bash", "BUSY", Some(1)).unwrap();
        }
        mark_agent_unknown(
            db,
            agent_id,
            "PTY_MARKER_TIMEOUT",
            b"pane".to_vec(),
            serde_json::json!(["rule"]),
        )
        .unwrap();
        db.conn()
            .query_row(
                "SELECT id FROM evidence WHERE agent_id = ?",
                [agent_id],
                |row| row.get(0),
            )
            .unwrap()
    }

    #[test]
    fn test_assert_state_to_idle_tx_cases() {
        with_test_db_handle(|db| {
            let evidence_id = seed_unknown_with_evidence(db, "a_assert");
            assert!(matches!(
                assert_state_to_idle_tx(db, "a_assert", "missing").unwrap_err(),
                crate::error::CcbdError::DbEvidenceNotFound { .. }
            ));
            {
                let conn = db.conn();
                insert_agent(&conn, "a_other", "s_assert", "bash", "UNKNOWN", Some(2)).unwrap();
            }
            assert!(matches!(
                assert_state_to_idle_tx(db, "a_other", &evidence_id).unwrap_err(),
                crate::error::CcbdError::DbEvidenceNotFound { .. }
            ));
            db.conn()
                .execute("UPDATE agents SET state='BUSY' WHERE id='a_assert'", [])
                .unwrap();
            assert!(matches!(
                assert_state_to_idle_tx(db, "a_assert", &evidence_id).unwrap_err(),
                crate::error::CcbdError::AgentWrongState { .. }
            ));
            db.conn()
                .execute("UPDATE agents SET state='UNKNOWN' WHERE id='a_assert'", [])
                .unwrap();
            let outcome = assert_state_to_idle_tx(db, "a_assert", &evidence_id).unwrap();
            assert!(outcome.state_change_seq_id > 0);
            let (state, sub_state, evidence_status): (String, Option<String>, String) = db
                .conn()
                .query_row(
                    "SELECT agents.state, agents.sub_state, evidence.status FROM agents JOIN evidence ON evidence.agent_id = agents.id WHERE agents.id='a_assert' AND evidence.id=?",
                    [evidence_id],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .unwrap();
            assert_eq!(state, "IDLE");
            assert_eq!(sub_state.as_deref(), Some("Asserted"));
            assert_eq!(evidence_status, "REVIEWED");
        });
    }
}
