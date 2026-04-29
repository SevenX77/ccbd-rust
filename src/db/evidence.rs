use crate::db::Db;
use crate::db::common::map_db_error;
use crate::db::schema::Evidence;
use crate::error::CcbdError;
use rusqlite::{Connection, OptionalExtension, params};

pub fn query_evidence_by_id(
    conn: &Connection,
    evidence_id: &str,
) -> Result<Option<Evidence>, CcbdError> {
    conn.query_row(
        "SELECT id, agent_id, event_seq_id, status, created_at FROM evidence WHERE id = ?",
        params![evidence_id],
        |row| {
            Ok(Evidence {
                id: row.get(0)?,
                agent_id: row.get(1)?,
                event_seq_id: row.get(2)?,
                status: row.get(3)?,
                created_at: row.get(4)?,
            })
        },
    )
    .optional()
    .map_err(|err| map_db_error("query evidence by id", err))
}

pub fn update_evidence_status(
    conn: &Connection,
    evidence_id: &str,
    status: &str,
    l3_asserted_state: Option<&str>,
) -> Result<usize, CcbdError> {
    conn.execute(
        "UPDATE evidence SET status = ?, l3_asserted_state = ? WHERE id = ?",
        params![status, l3_asserted_state, evidence_id],
    )
    .map_err(|err| map_db_error("update evidence status", err))
}

pub fn discard_evidence_tx(db: &Db, evidence_id: &str) -> Result<(), CcbdError> {
    let conn = db.conn();
    if query_evidence_by_id(&conn, evidence_id)?.is_none() {
        return Err(CcbdError::DbEvidenceNotFound {
            details: format!("evidence_id={evidence_id}"),
        });
    }
    update_evidence_status(&conn, evidence_id, "DISCARDED", None)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{discard_evidence_tx, query_evidence_by_id, update_evidence_status};
    use crate::db::agents::insert_agent;
    use crate::db::sessions::insert_session;
    use crate::db::state_machine::mark_agent_unknown;
    use crate::db::{Db, init};

    fn with_test_db_handle<T>(test: impl FnOnce(&Db) -> T) -> T {
        let file = tempfile::NamedTempFile::new().unwrap();
        let db = init(file.path()).unwrap();
        test(&db)
    }

    #[test]
    fn test_evidence_helpers() {
        with_test_db_handle(|db| {
            {
                let conn = db.conn();
                insert_session(&conn, "s1", "p1", "/tmp/foo", 999).unwrap();
                insert_agent(&conn, "a1", "s1", "bash", "BUSY", Some(1)).unwrap();
            }
            mark_agent_unknown(
                db,
                "a1",
                "PTY_MARKER_TIMEOUT",
                b"pane".to_vec(),
                serde_json::json!(["rule"]),
            )
            .unwrap();
            let evidence_id: String = db
                .conn()
                .query_row("SELECT id FROM evidence WHERE agent_id = 'a1'", [], |row| {
                    row.get(0)
                })
                .unwrap();
            let evidence = query_evidence_by_id(&db.conn(), &evidence_id)
                .unwrap()
                .unwrap();
            assert_eq!(evidence.agent_id, "a1");
            assert_eq!(evidence.status, "PENDING");
            let changes =
                update_evidence_status(&db.conn(), &evidence_id, "REVIEWED", Some("IDLE")).unwrap();
            assert_eq!(changes, 1);
            discard_evidence_tx(db, &evidence_id).unwrap();
        });
    }
}
