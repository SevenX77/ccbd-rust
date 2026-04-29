use crate::db::Db;
use crate::db::common::{is_constraint_error, map_db_error};
use crate::db::schema::Agent;
use crate::error::CcbdError;
use rusqlite::{Connection, OptionalExtension, params};

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

pub fn query_agent(conn: &Connection, agent_id: &str) -> Result<Option<Agent>, CcbdError> {
    conn.query_row(
        "SELECT id, session_id, provider, state, state_version, pid, exit_code, error_code, sub_state, created_at, updated_at FROM agents WHERE id = ?",
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
                sub_state: row.get(8)?,
                created_at: row.get(9)?,
                updated_at: row.get(10)?,
            })
        },
    )
    .optional()
    .map_err(|err| map_db_error("query agent", err))
}

pub fn agent_exists(conn: &Connection, agent_id: &str) -> Result<bool, CcbdError> {
    conn.query_row(
        "SELECT 1 FROM agents WHERE id = ? LIMIT 1",
        params![agent_id],
        |row| row.get::<_, i64>(0),
    )
    .optional()
    .map(|value| value.is_some())
    .map_err(|err| map_db_error("check agent uniqueness", err))
}

pub fn query_agent_state(db: &Db, agent_id: &str) -> Result<Option<(String, i64)>, CcbdError> {
    let conn = db.conn();
    conn.query_row(
        "SELECT state, state_version FROM agents WHERE id = ?",
        params![agent_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .optional()
    .map_err(|err| map_db_error("query agent state", err))
}

pub fn delete_agent(db: &Db, agent_id: &str) -> Result<(), CcbdError> {
    let conn = db.conn();
    conn.execute("DELETE FROM agents WHERE id = ?", params![agent_id])
        .map_err(|err| map_db_error("delete agent", err))?;
    Ok(())
}

/// Mark one active agent as KILLED and emit a state_change event in the same transaction.
pub fn mark_agent_killed(db: &Db, agent_id: &str, reason: &str) -> Result<usize, CcbdError> {
    let mut conn = db.conn();
    let tx = conn
        .transaction()
        .map_err(|err| map_db_error("begin mark agent killed", err))?;
    let previous_state = tx
        .query_row(
            "SELECT state FROM agents WHERE id = ?",
            params![agent_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|err| map_db_error("query agent state for killed", err))?;
    let changes = tx
        .execute(
            "UPDATE agents SET state = 'KILLED', state_version = state_version + 1, updated_at = unixepoch() WHERE id = ? AND state NOT IN ('CRASHED', 'KILLED')",
            params![agent_id],
        )
        .map_err(|err| map_db_error("mark agent killed", err))?;

    if changes == 1 {
        let payload = serde_json::json!({
            "from": previous_state.as_deref().unwrap_or("UNKNOWN"),
            "to": "KILLED",
            "reason": reason,
        })
        .to_string();
        tx.execute(
            "INSERT INTO events (agent_id, request_id, event_type, payload) VALUES (?, NULL, 'state_change', ?)",
            params![agent_id, payload],
        )
        .map_err(|err| map_db_error("insert killed state_change", err))?;
    }

    tx.commit()
        .map_err(|err| map_db_error("commit mark agent killed", err))?;
    Ok(changes)
}

/// Mark one active agent as CRASHED with exit metadata and emit state_change atomically.
pub fn mark_agent_crashed_with_exit(
    db: &Db,
    agent_id: &str,
    exit_code: i32,
) -> Result<usize, CcbdError> {
    let mut conn = db.conn();
    let tx = conn
        .transaction()
        .map_err(|err| map_db_error("begin mark agent crashed", err))?;
    let previous_state = tx
        .query_row(
            "SELECT state FROM agents WHERE id = ?",
            params![agent_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|err| map_db_error("query agent state for crashed", err))?;
    let changes = tx
        .execute(
            "UPDATE agents SET state = 'CRASHED', exit_code = ?, error_code = 'AGENT_UNEXPECTED_EXIT', state_version = state_version + 1, updated_at = unixepoch() WHERE id = ? AND state NOT IN ('CRASHED', 'KILLED')",
            params![exit_code, agent_id],
        )
        .map_err(|err| map_db_error("mark agent crashed", err))?;

    if changes == 1 {
        let payload = serde_json::json!({
            "from": previous_state.as_deref().unwrap_or("UNKNOWN"),
            "to": "CRASHED",
            "reason": "AGENT_UNEXPECTED_EXIT",
            "exit_code": exit_code,
        })
        .to_string();
        tx.execute(
            "INSERT INTO events (agent_id, request_id, event_type, payload) VALUES (?, NULL, 'state_change', ?)",
            params![agent_id, payload],
        )
        .map_err(|err| map_db_error("insert crashed state_change", err))?;
    }

    tx.commit()
        .map_err(|err| map_db_error("commit mark agent crashed", err))?;
    Ok(changes)
}

#[cfg(test)]
mod tests {
    use super::{
        delete_agent, insert_agent, mark_agent_crashed_with_exit, mark_agent_killed, query_agent,
        update_agent_state,
    };
    use crate::db::sessions::insert_session;
    use crate::db::{Db, init};
    use crate::error::CcbdError;

    fn with_test_db<T>(test: impl FnOnce(&mut rusqlite::Connection) -> T) -> T {
        let file = tempfile::NamedTempFile::new().unwrap();
        let db = init(file.path()).unwrap();
        let mut conn = db.conn();
        test(&mut conn)
    }

    fn with_test_db_handle<T>(test: impl FnOnce(&Db) -> T) -> T {
        let file = tempfile::NamedTempFile::new().unwrap();
        let db = init(file.path()).unwrap();
        test(&db)
    }

    fn seed_agent(conn: &rusqlite::Connection) {
        insert_session(conn, "s1", "p1", "/tmp/foo", 999).unwrap();
        insert_agent(conn, "a1", "s1", "bash", "IDLE", Some(123)).unwrap();
    }

    #[test]
    fn test_query_agent_sub_state_defaults_none() {
        with_test_db(|conn| {
            seed_agent(conn);
            let agent = query_agent(conn, "a1").unwrap().unwrap();
            assert_eq!(agent.sub_state, None);
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
    fn test_delete_agent_removes_row() {
        with_test_db_handle(|db| {
            {
                let conn = db.conn();
                seed_agent(&conn);
            }
            delete_agent(db, "a1").unwrap();
            let count: i64 = db
                .conn()
                .query_row("SELECT COUNT(*) FROM agents WHERE id = 'a1'", [], |row| {
                    row.get(0)
                })
                .unwrap();
            assert_eq!(count, 0);
        });
    }

    #[test]
    fn test_mark_agent_killed_is_terminal_and_idempotent() {
        with_test_db_handle(|db| {
            {
                let conn = db.conn();
                seed_agent(&conn);
            }
            let first = mark_agent_killed(db, "a1", "SIGKILL_BY_DAEMON").unwrap();
            let second = mark_agent_killed(db, "a1", "SIGKILL_BY_DAEMON").unwrap();
            let (state, state_version, payload): (String, i64, String) = db
                .conn()
                .query_row(
                    "SELECT agents.state, agents.state_version, events.payload FROM agents JOIN events ON events.agent_id = agents.id WHERE agents.id = 'a1'",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .unwrap();
            let payload: serde_json::Value = serde_json::from_str(&payload).unwrap();
            assert_eq!(first, 1);
            assert_eq!(second, 0);
            assert_eq!(state, "KILLED");
            assert_eq!(state_version, 2);
            assert_eq!(payload["to"], "KILLED");
        });
    }

    #[test]
    fn test_mark_agent_crashed_with_exit_sets_metadata() {
        with_test_db_handle(|db| {
            {
                let conn = db.conn();
                seed_agent(&conn);
            }
            let changes = mark_agent_crashed_with_exit(db, "a1", 137).unwrap();
            let (state, exit_code, error_code): (String, Option<i64>, Option<String>) = db
                .conn()
                .query_row(
                    "SELECT state, exit_code, error_code FROM agents WHERE id = 'a1'",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .unwrap();
            assert_eq!(changes, 1);
            assert_eq!(state, "CRASHED");
            assert_eq!(exit_code, Some(137));
            assert_eq!(error_code.as_deref(), Some("AGENT_UNEXPECTED_EXIT"));
        });
    }
}
