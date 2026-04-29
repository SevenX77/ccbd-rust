use crate::db::Db;
use crate::db::common::{map_db_error, spawn_db};
use crate::error::CcbdError;
use rusqlite::{OptionalExtension, params};

/// Mark one active agent as KILLED and emit a state_change event in the same transaction.
pub(crate) fn mark_agent_killed_sync(
    db: &Db,
    agent_id: &str,
    reason: &str,
) -> Result<usize, CcbdError> {
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
pub(crate) fn mark_agent_crashed_with_exit_sync(
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

pub async fn mark_agent_killed(
    db: Db,
    agent_id: String,
    reason: String,
) -> Result<usize, CcbdError> {
    spawn_db("agents_lifecycle::mark_agent_killed", move || {
        mark_agent_killed_sync(&db, &agent_id, &reason)
    })
    .await
}

pub async fn mark_agent_crashed_with_exit(
    db: Db,
    agent_id: String,
    exit_code: i32,
) -> Result<usize, CcbdError> {
    spawn_db(
        "agents_lifecycle::mark_agent_crashed_with_exit",
        move || mark_agent_crashed_with_exit_sync(&db, &agent_id, exit_code),
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::{mark_agent_crashed_with_exit_sync, mark_agent_killed_sync};
    use crate::db::agents::insert_agent_sync;
    use crate::db::sessions::insert_session_sync;
    use crate::db::{Db, init};

    fn with_test_db_handle<T>(test: impl FnOnce(&Db) -> T) -> T {
        let file = tempfile::NamedTempFile::new().unwrap();
        let db = init(file.path()).unwrap();
        test(&db)
    }

    fn seed_agent(conn: &rusqlite::Connection) {
        insert_session_sync(conn, "s1", "p1", "/tmp/foo", 999).unwrap();
        insert_agent_sync(conn, "a1", "s1", "bash", "IDLE", Some(123)).unwrap();
    }

    #[test]
    fn test_mark_agent_killed_is_terminal_and_idempotent() {
        with_test_db_handle(|db| {
            {
                let conn = db.conn();
                seed_agent(&conn);
            }
            let first = mark_agent_killed_sync(db, "a1", "SIGKILL_BY_DAEMON").unwrap();
            let second = mark_agent_killed_sync(db, "a1", "SIGKILL_BY_DAEMON").unwrap();
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
            let changes = mark_agent_crashed_with_exit_sync(db, "a1", 137).unwrap();
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
