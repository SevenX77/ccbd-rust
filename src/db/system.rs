use crate::db::Db;
use crate::db::agents_lifecycle::mark_agent_killed_sync;
use crate::db::common::{map_db_error, spawn_db};
use crate::error::CcbdError;
use rusqlite::{Connection, params};
use serde_json::{Value, json};
use std::sync::Arc;

pub(crate) fn system_dump_sync(db: &Db) -> Result<Value, CcbdError> {
    let conn = db.conn();
    let projects = {
        let mut stmt = conn
            .prepare("SELECT id, absolute_path, created_at FROM projects ORDER BY created_at ASC")
            .map_err(|err| map_db_error("prepare dump projects", err))?;
        stmt.query_map([], |row| {
            Ok(json!({
                "id": row.get::<_, String>(0)?,
                "absolute_path": row.get::<_, String>(1)?,
                "created_at": row.get::<_, i64>(2)?,
            }))
        })
        .map_err(|err| map_db_error("query dump projects", err))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|err| map_db_error("collect dump projects", err))?
    };
    let sessions = {
        let mut stmt = conn
            .prepare(
                "SELECT id, project_id, master_pid, created_at FROM sessions ORDER BY created_at ASC",
            )
            .map_err(|err| map_db_error("prepare dump sessions", err))?;
        stmt.query_map([], |row| {
            Ok(json!({
                "id": row.get::<_, String>(0)?,
                "project_id": row.get::<_, String>(1)?,
                "master_pid": row.get::<_, i64>(2)?,
                "created_at": row.get::<_, i64>(3)?,
            }))
        })
        .map_err(|err| map_db_error("query dump sessions", err))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|err| map_db_error("collect dump sessions", err))?
    };
    let agents = {
        let mut stmt = conn
            .prepare(
                "SELECT id, session_id, provider, state, sub_state, state_version, pid, exit_code, error_code, created_at, updated_at FROM agents ORDER BY created_at ASC",
            )
            .map_err(|err| map_db_error("prepare dump agents", err))?;
        stmt.query_map([], |row| {
            Ok(json!({
                "id": row.get::<_, String>(0)?,
                "session_id": row.get::<_, String>(1)?,
                "provider": row.get::<_, String>(2)?,
                "state": row.get::<_, String>(3)?,
                "sub_state": row.get::<_, Option<String>>(4)?,
                "state_version": row.get::<_, i64>(5)?,
                "pid": row.get::<_, Option<i64>>(6)?,
                "exit_code": row.get::<_, Option<i64>>(7)?,
                "error_code": row.get::<_, Option<String>>(8)?,
                "created_at": row.get::<_, i64>(9)?,
                "updated_at": row.get::<_, i64>(10)?,
            }))
        })
        .map_err(|err| map_db_error("query dump agents", err))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|err| map_db_error("collect dump agents", err))?
    };
    let evidence_pending = {
        let mut stmt = conn
            .prepare(
                "SELECT id, agent_id, status, created_at FROM evidence WHERE status = 'PENDING' ORDER BY created_at DESC LIMIT 100",
            )
            .map_err(|err| map_db_error("prepare dump evidence", err))?;
        stmt.query_map([], |row| {
            Ok(json!({
                "id": row.get::<_, String>(0)?,
                "agent_id": row.get::<_, String>(1)?,
                "status": row.get::<_, String>(2)?,
                "created_at": row.get::<_, i64>(3)?,
            }))
        })
        .map_err(|err| map_db_error("query dump evidence", err))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|err| map_db_error("collect dump evidence", err))?
    };

    Ok(json!({
        "projects": projects,
        "sessions": sessions,
        "agents": agents,
        "evidence_pending": evidence_pending,
    }))
}

pub(crate) fn cascade_kill_session_agents_sync(
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
        changed += mark_agent_killed_sync(db, &agent_id, reason)?;
        if let Some(handle) = crate::marker::registry::take(&agent_id) {
            let _ = handle.cancel_tx.send(());
        }
        let _ = crate::marker::parser_registry::remove(&agent_id);
    }
    Ok(changed)
}

/// Re-register master pidfds and reconcile active agents during daemon startup.
pub(crate) fn reconcile_startup_sync(db: &Db) -> Result<usize, CcbdError> {
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
                changed +=
                    cascade_kill_session_agents_sync(db, &session_id, "MASTER_ALREADY_DEAD")?;
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

pub(crate) fn reconcile_active_agents_to_crashed(
    conn: &mut Connection,
) -> Result<usize, CcbdError> {
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

pub async fn system_dump(db: Db) -> Result<Value, CcbdError> {
    spawn_db("system::system_dump", move || system_dump_sync(&db)).await
}

pub async fn cascade_kill_session_agents(
    db: Db,
    session_id: String,
    reason: String,
) -> Result<usize, CcbdError> {
    spawn_db("system::cascade_kill_session_agents", move || {
        cascade_kill_session_agents_sync(&db, &session_id, &reason)
    })
    .await
}

pub async fn reconcile_startup(db: Db) -> Result<usize, CcbdError> {
    spawn_db("system::reconcile_startup", move || {
        reconcile_startup_sync(&db)
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::{cascade_kill_session_agents_sync, reconcile_startup_sync};
    use crate::db::agents::insert_agent_sync;
    use crate::db::sessions::insert_session_sync;
    use crate::db::{Db, init};

    fn with_test_db_handle<T>(test: impl FnOnce(&Db) -> T) -> T {
        let file = tempfile::NamedTempFile::new().unwrap();
        let db = init(file.path()).unwrap();
        test(&db)
    }

    #[test]
    fn test_cascade_kill_session_agents_counts_active_only() {
        with_test_db_handle(|db| {
            {
                let conn = db.conn();
                insert_session_sync(&conn, "s1", "p1", "/tmp/foo", 999).unwrap();
                insert_agent_sync(&conn, "a1", "s1", "bash", "IDLE", Some(1)).unwrap();
                insert_agent_sync(&conn, "a2", "s1", "bash", "BUSY", Some(2)).unwrap();
                insert_agent_sync(&conn, "a3", "s1", "bash", "CRASHED", Some(3)).unwrap();
            }
            let count = cascade_kill_session_agents_sync(db, "s1", "MASTER_DEATH").unwrap();
            let killed_count: i64 = db
                .conn()
                .query_row(
                    "SELECT COUNT(*) FROM agents WHERE state = 'KILLED'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(count, 2);
            assert_eq!(killed_count, 2);
        });
    }

    #[test]
    fn test_reconcile_startup_cascades_dead_master_before_crash_fallback() {
        with_test_db_handle(|db| {
            {
                let conn = db.conn();
                insert_session_sync(&conn, "s_dead", "p_dead", "/tmp/dead-master", 999_999_999)
                    .unwrap();
                insert_agent_sync(&conn, "ag_dead", "s_dead", "bash", "IDLE", Some(1)).unwrap();
            }
            let count = reconcile_startup_sync(db).unwrap();
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
