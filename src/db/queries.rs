use crate::db::schema::{Agent, Event};
use crate::error::CcbdError;
use rusqlite::{Connection, Error as SqlError, OptionalExtension, params};

fn is_constraint_error(err: &SqlError) -> bool {
    matches!(
        err,
        SqlError::SqliteFailure(sqlite_err, _)
            if sqlite_err.code == rusqlite::ErrorCode::ConstraintViolation
    )
}

fn is_unique_constraint_error(err: &SqlError) -> bool {
    matches!(
        err,
        SqlError::SqliteFailure(sqlite_err, _)
            if sqlite_err.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE
    )
}

fn map_db_error(context: &str, err: SqlError) -> CcbdError {
    CcbdError::DbConstraintViolation(format!("{context}: {err}"))
}

pub fn insert_session(
    conn: &Connection,
    session_id: &str,
    project_id: &str,
    absolute_path: &str,
    master_pid: i64,
) -> Result<(), CcbdError> {
    conn.execute(
        "INSERT OR IGNORE INTO projects (id, absolute_path) VALUES (?, ?)",
        params![project_id, absolute_path],
    )
    .map_err(|err| map_db_error("insert project", err))?;

    conn.execute(
        "INSERT INTO sessions (id, project_id, master_pid) VALUES (?, ?, ?)",
        params![session_id, project_id, master_pid],
    )
    .map_err(|err| map_db_error("insert session", err))?;

    Ok(())
}

pub fn insert_agent(
    conn: &Connection,
    agent_id: &str,
    session_id: &str,
    provider: &str,
    state: &str,
    pid: Option<i64>,
) -> Result<(), CcbdError> {
    conn.execute(
        "INSERT INTO agents (id, session_id, provider, state, pid) VALUES (?, ?, ?, ?, ?)",
        params![agent_id, session_id, provider, state, pid],
    )
    .map_err(|err| {
        if is_constraint_error(&err) {
            CcbdError::AgentAlreadyExists(agent_id.to_string())
        } else {
            map_db_error("insert agent", err)
        }
    })?;

    Ok(())
}

pub fn update_agent_state(
    conn: &Connection,
    agent_id: &str,
    new_state: &str,
) -> Result<(), CcbdError> {
    conn.execute(
        "UPDATE agents SET state = ?, state_version = state_version + 1, updated_at = unixepoch() WHERE id = ? AND state != 'CRASHED'",
        params![new_state, agent_id],
    )
    .map_err(|err| map_db_error("update agent state", err))?;

    Ok(())
}

pub fn query_event_by_request_id(
    conn: &Connection,
    agent_id: &str,
    request_id: &str,
) -> Result<Option<Event>, CcbdError> {
    conn.query_row(
        "SELECT seq_id, event_type, payload, created_at FROM events WHERE agent_id = ? AND request_id = ? LIMIT 1",
        params![agent_id, request_id],
        |row| {
            Ok(Event {
                seq_id: row.get(0)?,
                agent_id: agent_id.to_string(),
                request_id: Some(request_id.to_string()),
                event_type: row.get(1)?,
                payload: row.get(2)?,
                created_at: row.get(3)?,
            })
        },
    )
    .optional()
    .map_err(|err| map_db_error("query event by request_id", err))
}

pub fn insert_event(
    conn: &Connection,
    agent_id: &str,
    request_id: Option<&str>,
    event_type: &str,
    payload: &str,
) -> Result<i64, CcbdError> {
    let result = conn.execute(
        "INSERT INTO events (agent_id, request_id, event_type, payload) VALUES (?, ?, ?, ?)",
        params![agent_id, request_id, event_type, payload],
    );

    match result {
        Ok(_) => Ok(conn.last_insert_rowid()),
        Err(err) if is_unique_constraint_error(&err) && request_id.is_some() => {
            let existing_seq_id = conn
                .query_row(
                    "SELECT seq_id FROM events WHERE agent_id = ? AND request_id = ? LIMIT 1",
                    params![agent_id, request_id],
                    |row| row.get(0),
                )
                .map_err(|select_err| {
                    map_db_error("query duplicate event by request_id", select_err)
                })?;

            Err(CcbdError::DuplicateRequest { existing_seq_id })
        }
        Err(err) => Err(map_db_error("insert event", err)),
    }
}

pub fn query_agent(conn: &Connection, agent_id: &str) -> Result<Option<Agent>, CcbdError> {
    conn.query_row(
        "SELECT id, session_id, provider, state, state_version, pid, exit_code, error_code, created_at, updated_at FROM agents WHERE id = ?",
        params![agent_id],
        |row| {
            Ok(Agent {
                id: row.get(0)?,
                session_id: row.get(1)?,
                provider: row.get(2)?,
                state: row.get(3)?,
                state_version: row.get(4)?,
                pid: row.get(5)?,
                exit_code: row.get(6)?,
                error_code: row.get(7)?,
                created_at: row.get(8)?,
                updated_at: row.get(9)?,
            })
        },
    )
    .optional()
    .map_err(|err| map_db_error("query agent", err))
}

pub fn query_events_since(
    conn: &Connection,
    agent_id: &str,
    since_seq_id: i64,
) -> Result<Vec<Event>, CcbdError> {
    let mut stmt = conn
        .prepare(
            "SELECT seq_id, request_id, event_type, payload, created_at FROM events WHERE agent_id = ? AND seq_id > ? ORDER BY seq_id ASC",
        )
        .map_err(|err| map_db_error("prepare query events since", err))?;
    let rows = stmt
        .query_map(params![agent_id, since_seq_id], |row| {
            Ok(Event {
                seq_id: row.get(0)?,
                agent_id: agent_id.to_string(),
                request_id: row.get(1)?,
                event_type: row.get(2)?,
                payload: row.get(3)?,
                created_at: row.get(4)?,
            })
        })
        .map_err(|err| map_db_error("query events since", err))?;

    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|err| map_db_error("collect events since", err))
}

pub fn reconcile_active_agents_to_crashed(conn: &mut Connection) -> Result<usize, CcbdError> {
    let tx = conn
        .transaction()
        .map_err(|err| map_db_error("begin startup reconcile", err))?;

    let active_agents = {
        let mut stmt = tx
            .prepare("SELECT id, state FROM agents WHERE state NOT IN ('CRASHED', 'KILLED')")
            .map_err(|err| map_db_error("prepare active agents query", err))?;
        let rows = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|err| map_db_error("query active agents", err))?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|err| map_db_error("collect active agents", err))?
    };

    for (agent_id, prev_state) in &active_agents {
        let payload = serde_json::json!({
            "from": prev_state,
            "to": "CRASHED",
            "reason": "STARTUP_RECONCILE",
        })
        .to_string();

        tx.execute(
            "INSERT INTO events (agent_id, request_id, event_type, payload) VALUES (?, NULL, 'state_change', ?)",
            params![agent_id, payload],
        )
        .map_err(|err| map_db_error("insert startup reconcile state_change", err))?;
    }

    tx.execute(
        "UPDATE agents SET state = 'CRASHED', state_version = state_version + 1, updated_at = unixepoch() WHERE state NOT IN ('CRASHED', 'KILLED')",
        [],
    )
    .map_err(|err| map_db_error("update active agents to crashed", err))?;

    tx.commit()
        .map_err(|err| map_db_error("commit startup reconcile", err))?;

    Ok(active_agents.len())
}

#[cfg(test)]
mod tests {
    use super::{
        insert_agent, insert_event, insert_session, query_event_by_request_id,
        reconcile_active_agents_to_crashed, update_agent_state,
    };
    use crate::db::init;
    use crate::error::CcbdError;

    fn with_test_db<T>(test: impl FnOnce(&mut rusqlite::Connection) -> T) -> T {
        let file = tempfile::NamedTempFile::new().unwrap();
        let db = init(file.path()).unwrap();
        let mut conn = db.conn();
        test(&mut conn)
    }

    fn seed_agent(conn: &rusqlite::Connection) {
        insert_session(conn, "s1", "p1", "/tmp/foo", 999).unwrap();
        insert_agent(conn, "a1", "s1", "bash", "IDLE", Some(123)).unwrap();
    }

    #[test]
    fn test_insert_session_then_agent() {
        with_test_db(|conn| {
            seed_agent(conn);

            let count: i64 = conn
                .query_row("SELECT COUNT(*) FROM agents", [], |row| row.get(0))
                .unwrap();

            assert_eq!(count, 1);
        });
    }

    #[test]
    fn test_insert_agent_duplicate_id() {
        with_test_db(|conn| {
            seed_agent(conn);

            let err = insert_agent(conn, "a1", "s1", "bash", "IDLE", Some(456)).unwrap_err();

            assert!(matches!(err, CcbdError::AgentAlreadyExists(agent_id) if agent_id == "a1"));
        });
    }

    #[test]
    fn test_insert_event_idempotent() {
        with_test_db(|conn| {
            seed_agent(conn);

            let seq_id = insert_event(
                conn,
                "a1",
                Some("req-1"),
                "command_received",
                r#"{"cmd":"echo 1\n"}"#,
            )
            .unwrap();

            let err = insert_event(
                conn,
                "a1",
                Some("req-1"),
                "command_received",
                r#"{"cmd":"echo 2\n"}"#,
            )
            .unwrap_err();

            assert!(
                matches!(err, CcbdError::DuplicateRequest { existing_seq_id } if existing_seq_id == seq_id)
            );
        });
    }

    #[test]
    fn test_insert_event_no_request_id_no_unique() {
        with_test_db(|conn| {
            seed_agent(conn);

            let seq_1 =
                insert_event(conn, "a1", None, "output_chunk", r#"{"text":"one"}"#).unwrap();
            let seq_2 =
                insert_event(conn, "a1", None, "output_chunk", r#"{"text":"two"}"#).unwrap();

            assert_ne!(seq_1, seq_2);
        });
    }

    #[test]
    fn test_query_event_by_request_id_found_and_missing() {
        with_test_db(|conn| {
            seed_agent(conn);

            let seq_id = insert_event(
                conn,
                "a1",
                Some("req-1"),
                "command_received",
                r#"{"cmd":"echo 1\n"}"#,
            )
            .unwrap();

            let found = query_event_by_request_id(conn, "a1", "req-1")
                .unwrap()
                .unwrap();
            let missing = query_event_by_request_id(conn, "a1", "req-2").unwrap();

            assert_eq!(found.seq_id, seq_id);
            assert_eq!(found.agent_id, "a1");
            assert_eq!(found.request_id.as_deref(), Some("req-1"));
            assert_eq!(found.event_type, "command_received");
            assert!(found.payload.contains("echo 1"));
            assert!(missing.is_none());
        });
    }

    #[test]
    fn test_update_agent_state_does_not_overwrite_crashed() {
        with_test_db(|conn| {
            seed_agent(conn);

            update_agent_state(conn, "a1", "BUSY").unwrap();
            let state: String = conn
                .query_row("SELECT state FROM agents WHERE id = 'a1'", [], |row| {
                    row.get(0)
                })
                .unwrap();
            assert_eq!(state, "BUSY");

            conn.execute("UPDATE agents SET state = 'CRASHED' WHERE id = 'a1'", [])
                .unwrap();
            update_agent_state(conn, "a1", "BUSY").unwrap();

            let state: String = conn
                .query_row("SELECT state FROM agents WHERE id = 'a1'", [], |row| {
                    row.get(0)
                })
                .unwrap();
            assert_eq!(state, "CRASHED");
        });
    }

    #[test]
    fn test_reconcile_idle_to_crashed() {
        with_test_db(|conn| {
            insert_session(conn, "s1", "p1", "/tmp/foo", 999).unwrap();
            insert_agent(conn, "ag_1", "s1", "bash", "IDLE", Some(123)).unwrap();

            let count = reconcile_active_agents_to_crashed(conn).unwrap();

            let state: String = conn
                .query_row("SELECT state FROM agents WHERE id = 'ag_1'", [], |row| {
                    row.get(0)
                })
                .unwrap();
            let (event_type, payload): (String, String) = conn
                .query_row(
                    "SELECT event_type, payload FROM events WHERE agent_id = 'ag_1'",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .unwrap();
            let payload: serde_json::Value = serde_json::from_str(&payload).unwrap();

            assert_eq!(count, 1);
            assert_eq!(state, "CRASHED");
            assert_eq!(event_type, "state_change");
            assert_eq!(payload["from"], "IDLE");
            assert_eq!(payload["to"], "CRASHED");
            assert_eq!(payload["reason"], "STARTUP_RECONCILE");
        });
    }

    #[test]
    fn test_reconcile_skips_already_crashed() {
        with_test_db(|conn| {
            insert_session(conn, "s1", "p1", "/tmp/foo", 999).unwrap();
            insert_agent(conn, "ag_1", "s1", "bash", "CRASHED", Some(123)).unwrap();

            let count = reconcile_active_agents_to_crashed(conn).unwrap();
            let event_count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM events WHERE agent_id = 'ag_1'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();

            assert_eq!(count, 0);
            assert_eq!(event_count, 0);
        });
    }

    #[test]
    fn test_reconcile_state_version_bumps() {
        with_test_db(|conn| {
            insert_session(conn, "s1", "p1", "/tmp/foo", 999).unwrap();
            insert_agent(conn, "ag_1", "s1", "bash", "IDLE", Some(123)).unwrap();

            reconcile_active_agents_to_crashed(conn).unwrap();

            let state_version: i64 = conn
                .query_row(
                    "SELECT state_version FROM agents WHERE id = 'ag_1'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();

            assert_eq!(state_version, 2);
        });
    }
}
