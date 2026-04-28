use crate::db::{
    Db,
    schema::{Agent, Event},
};
use crate::error::CcbdError;
use rusqlite::{Connection, Error as SqlError, OptionalExtension, params};
use std::sync::Arc;

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

pub fn mark_agent_unknown(db: &Db, agent_id: &str, reason: &str) -> Result<usize, CcbdError> {
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
        return Ok(0);
    };
    if !matches!(previous_state.as_str(), "SPAWNING" | "BUSY") {
        tracing::trace!(agent_id, state = %previous_state, "marker timeout swallowed: agent not waiting for marker");
        return Ok(0);
    }

    let changes = tx
        .execute(
            "UPDATE agents SET state = 'UNKNOWN', error_code = ?, state_version = state_version + 1, updated_at = unixepoch() WHERE id = ? AND state IN ('SPAWNING', 'BUSY') AND state_version = ?",
            params![reason, agent_id, state_version],
        )
        .map_err(|err| map_db_error("mark agent unknown", err))?;

    if changes == 1 {
        let payload = serde_json::json!({
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
    } else {
        tracing::trace!(
            agent_id,
            "marker timeout swallowed: state_version CAS failed"
        );
    }

    tx.commit()
        .map_err(|err| map_db_error("commit mark agent unknown", err))?;
    Ok(changes)
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

/// Delete one agent row by id.
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

/// Mark all active agents in a session as KILLED.
pub fn cascade_kill_session_agents(
    db: &Db,
    session_id: &str,
    reason: &str,
) -> Result<usize, CcbdError> {
    let agent_ids = {
        let conn = db.conn();
        let mut stmt = conn
            .prepare(
                "SELECT id FROM agents WHERE session_id = ? AND state NOT IN ('CRASHED', 'KILLED')",
            )
            .map_err(|err| map_db_error("prepare cascade agent query", err))?;
        let rows = stmt
            .query_map(params![session_id], |row| row.get::<_, String>(0))
            .map_err(|err| map_db_error("query cascade agents", err))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|err| map_db_error("collect cascade agents", err))?
    };

    let mut changed = 0;
    for agent_id in agent_ids {
        match crate::monitor::with_borrowed(&agent_id, crate::monitor::pidfd_send_sigkill) {
            Some(Ok(())) => {}
            Some(Err(err)) => {
                tracing::warn!(agent_id = %agent_id, error = %err, "failed to SIGKILL agent during cascade")
            }
            None => tracing::debug!(agent_id = %agent_id, "no pidfd registered during cascade"),
        }
        changed += mark_agent_killed(db, &agent_id, reason)?;
        if let Some(handle) = crate::marker::registry::take(&agent_id) {
            let _ = handle.cancel_tx.send(());
        }
    }
    Ok(changed)
}

/// Re-register master pidfds and reconcile active agents during daemon startup.
pub fn reconcile_startup(db: &Db) -> Result<usize, CcbdError> {
    let sessions = {
        let conn = db.conn();
        let mut stmt = conn
            .prepare(
                "SELECT DISTINCT sessions.id, sessions.master_pid \
                 FROM sessions \
                 JOIN agents ON agents.session_id = sessions.id \
                 WHERE agents.state NOT IN ('CRASHED', 'KILLED')",
            )
            .map_err(|err| map_db_error("prepare startup master reconcile", err))?;
        let rows = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })
            .map_err(|err| map_db_error("query startup master reconcile", err))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|err| map_db_error("collect startup master reconcile", err))?
    };

    let mut changed = 0;
    for (session_id, master_pid) in sessions {
        match crate::monitor::pidfd_open(master_pid as i32) {
            Ok(pidfd) => {
                let task_fd =
                    pidfd
                        .try_clone()
                        .map_err(|err| CcbdError::EnvironmentNotSupported {
                            details: format!("clone master pidfd for session {session_id}: {err}"),
                        })?;
                crate::monitor::register(format!("master:{session_id}"), pidfd);
                crate::monitor::master_watch::spawn_master_pidfd_watch_task(
                    session_id.clone(),
                    task_fd,
                    Arc::new(db.clone()),
                );
            }
            Err(CcbdError::AgentUnexpectedExit { .. }) => {
                changed += cascade_kill_session_agents(db, &session_id, "MASTER_ALREADY_DEAD")?;
            }
            Err(err) => return Err(err),
        }
    }

    let active_reconciled = {
        let mut conn = db.conn();
        reconcile_active_agents_to_crashed(&mut conn)?
    };

    Ok(changed + active_reconciled)
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
        cascade_kill_session_agents, delete_agent, insert_agent, insert_event, insert_session,
        mark_agent_crashed_with_exit, mark_agent_idle_matched, mark_agent_killed,
        mark_agent_unknown, query_agent, query_agent_state, query_event_by_request_id,
        reconcile_active_agents_to_crashed, reconcile_startup, update_agent_state,
    };
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
            assert_eq!(payload["from"], "IDLE");
            assert_eq!(payload["to"], "KILLED");
            assert_eq!(payload["reason"], "SIGKILL_BY_DAEMON");
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

            let (state, exit_code, error_code, payload): (
                String,
                Option<i64>,
                Option<String>,
                String,
            ) = db
                .conn()
                .query_row(
                    "SELECT agents.state, agents.exit_code, agents.error_code, events.payload FROM agents JOIN events ON events.agent_id = agents.id WHERE agents.id = 'a1'",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
                )
                .unwrap();
            let payload: serde_json::Value = serde_json::from_str(&payload).unwrap();

            assert_eq!(changes, 1);
            assert_eq!(state, "CRASHED");
            assert_eq!(exit_code, Some(137));
            assert_eq!(error_code.as_deref(), Some("AGENT_UNEXPECTED_EXIT"));
            assert_eq!(payload["to"], "CRASHED");
            assert_eq!(payload["reason"], "AGENT_UNEXPECTED_EXIT");
            assert_eq!(payload["exit_code"], 137);
        });
    }

    #[test]
    fn test_cascade_kill_session_agents_counts_active_only() {
        with_test_db_handle(|db| {
            {
                let conn = db.conn();
                insert_session(&conn, "s1", "p1", "/tmp/foo", 999).unwrap();
                insert_agent(&conn, "a1", "s1", "bash", "IDLE", Some(1)).unwrap();
                insert_agent(&conn, "a2", "s1", "bash", "BUSY", Some(2)).unwrap();
                insert_agent(&conn, "a3", "s1", "bash", "CRASHED", Some(3)).unwrap();
            }

            let count = cascade_kill_session_agents(db, "s1", "MASTER_DEATH").unwrap();

            let killed_count: i64 = db
                .conn()
                .query_row(
                    "SELECT COUNT(*) FROM agents WHERE state = 'KILLED'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            let event_count: i64 = db
                .conn()
                .query_row(
                    "SELECT COUNT(*) FROM events WHERE event_type = 'state_change'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();

            assert_eq!(count, 2);
            assert_eq!(killed_count, 2);
            assert_eq!(event_count, 2);
        });
    }

    #[test]
    fn test_query_agent_state_returns_state_and_version() {
        with_test_db_handle(|db| {
            {
                let conn = db.conn();
                seed_agent(&conn);
            }

            let state = query_agent_state(db, "a1").unwrap().unwrap();

            assert_eq!(state, ("IDLE".to_string(), 1));
        });
    }

    #[test]
    fn test_mark_agent_idle_matched_from_spawning() {
        with_test_db_handle(|db| {
            {
                let conn = db.conn();
                insert_session(&conn, "s1", "p1", "/tmp/foo", 999).unwrap();
                insert_agent(&conn, "a1", "s1", "bash", "SPAWNING", Some(1)).unwrap();
            }

            let changes = mark_agent_idle_matched(db, "a1").unwrap();

            let (state, sub_state, state_version, payload): (
                String,
                Option<String>,
                i64,
                String,
            ) = db
                .conn()
                .query_row(
                    "SELECT agents.state, agents.sub_state, agents.state_version, events.payload FROM agents JOIN events ON events.agent_id = agents.id WHERE agents.id = 'a1'",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
                )
                .unwrap();
            let payload: serde_json::Value = serde_json::from_str(&payload).unwrap();

            assert_eq!(changes, 1);
            assert_eq!(state, "IDLE");
            assert_eq!(sub_state.as_deref(), Some("Matched"));
            assert_eq!(state_version, 2);
            assert_eq!(payload["to"], "IDLE");
            assert_eq!(payload["sub_state"], "Matched");
            assert_eq!(payload["reason"], "MARKER_MATCHED");
        });
    }

    #[test]
    fn test_mark_agent_idle_matched_from_busy() {
        with_test_db_handle(|db| {
            {
                let conn = db.conn();
                insert_session(&conn, "s1", "p1", "/tmp/foo", 999).unwrap();
                insert_agent(&conn, "a1", "s1", "bash", "BUSY", Some(1)).unwrap();
            }

            let changes = mark_agent_idle_matched(db, "a1").unwrap();
            let state: String = db
                .conn()
                .query_row("SELECT state FROM agents WHERE id = 'a1'", [], |row| {
                    row.get(0)
                })
                .unwrap();

            assert_eq!(changes, 1);
            assert_eq!(state, "IDLE");
        });
    }

    #[test]
    fn test_mark_agent_unknown_from_busy() {
        with_test_db_handle(|db| {
            {
                let conn = db.conn();
                insert_session(&conn, "s1", "p1", "/tmp/foo", 999).unwrap();
                insert_agent(&conn, "a1", "s1", "bash", "BUSY", Some(1)).unwrap();
            }

            let changes = mark_agent_unknown(db, "a1", "PTY_MARKER_TIMEOUT").unwrap();

            let (state, error_code, payload): (String, Option<String>, String) = db
                .conn()
                .query_row(
                    "SELECT agents.state, agents.error_code, events.payload FROM agents JOIN events ON events.agent_id = agents.id WHERE agents.id = 'a1'",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .unwrap();
            let payload: serde_json::Value = serde_json::from_str(&payload).unwrap();

            assert_eq!(changes, 1);
            assert_eq!(state, "UNKNOWN");
            assert_eq!(error_code.as_deref(), Some("PTY_MARKER_TIMEOUT"));
            assert_eq!(payload["to"], "UNKNOWN");
            assert_eq!(payload["reason"], "PTY_MARKER_TIMEOUT");
        });
    }

    #[test]
    fn test_marker_helpers_do_not_overwrite_terminal_states() {
        with_test_db_handle(|db| {
            {
                let conn = db.conn();
                insert_session(&conn, "s1", "p1", "/tmp/foo", 999).unwrap();
                insert_agent(&conn, "a1", "s1", "bash", "KILLED", Some(1)).unwrap();
            }

            let matched = mark_agent_idle_matched(db, "a1").unwrap();
            let unknown = mark_agent_unknown(db, "a1", "PTY_MARKER_TIMEOUT").unwrap();
            let event_count: i64 = db
                .conn()
                .query_row("SELECT COUNT(*) FROM events", [], |row| row.get(0))
                .unwrap();

            assert_eq!(matched, 0);
            assert_eq!(unknown, 0);
            assert_eq!(event_count, 0);
        });
    }

    #[test]
    fn test_reconcile_startup_cascades_dead_master_before_crash_fallback() {
        with_test_db_handle(|db| {
            {
                let conn = db.conn();
                insert_session(&conn, "s_dead", "p_dead", "/tmp/dead-master", 999_999_999).unwrap();
                insert_agent(&conn, "ag_dead", "s_dead", "bash", "IDLE", Some(1)).unwrap();
            }

            let count = reconcile_startup(db).unwrap();

            let (state, payload): (String, String) = db
                .conn()
                .query_row(
                    "SELECT agents.state, events.payload FROM agents JOIN events ON events.agent_id = agents.id WHERE agents.id = 'ag_dead'",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .unwrap();
            let payload: serde_json::Value = serde_json::from_str(&payload).unwrap();

            assert_eq!(count, 1);
            assert_eq!(state, "KILLED");
            assert_eq!(payload["reason"], "MASTER_ALREADY_DEAD");
        });
    }
}
