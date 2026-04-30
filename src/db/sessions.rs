use crate::db::Db;
use crate::db::common::{map_db_error, spawn_db};
use crate::db::schema::Session;
use crate::error::CcbdError;
use crate::monitor;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use std::os::fd::OwnedFd;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionSummary {
    pub id: String,
    pub project_id: String,
    pub absolute_path: String,
    pub master_pid: i64,
    pub active_agents: i64,
    pub created_at: i64,
}

pub(crate) fn insert_session_sync(
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

pub(crate) fn create_session_sync(
    db: &Db,
    session_id: &str,
    project_id: &str,
    absolute_path: &str,
    master_pid: i32,
) -> Result<OwnedFd, CcbdError> {
    let mut conn = db.conn();
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|err| map_db_error("begin session.create", err))?;
    tx.execute(
        "INSERT OR IGNORE INTO projects (id, absolute_path) VALUES (?, ?)",
        params![project_id, absolute_path],
    )
    .map_err(|err| map_db_error("insert project", err))?;

    let master_pidfd = match monitor::pidfd_open(master_pid) {
        Ok(fd) => fd,
        Err(CcbdError::AgentUnexpectedExit { .. }) => {
            return Err(CcbdError::IpcInvalidRequest(format!(
                "master_pid {master_pid} not alive"
            )));
        }
        Err(err) => return Err(err),
    };

    tx.execute(
        "INSERT INTO sessions (id, project_id, master_pid) VALUES (?, ?, ?)",
        params![session_id, project_id, master_pid],
    )
    .map_err(|err| map_db_error("insert session", err))?;
    tx.commit()
        .map_err(|err| map_db_error("commit session.create", err))?;

    Ok(master_pidfd)
}

pub(crate) fn session_exists_sync(conn: &Connection, session_id: &str) -> Result<bool, CcbdError> {
    conn.query_row(
        "SELECT 1 FROM sessions WHERE id = ? LIMIT 1",
        params![session_id],
        |row| row.get::<_, i64>(0),
    )
    .optional()
    .map(|value| value.is_some())
    .map_err(|err| map_db_error("check session exists", err))
}

pub(crate) fn query_session_by_id_sync(
    conn: &Connection,
    session_id: &str,
) -> Result<Option<Session>, CcbdError> {
    conn.query_row(
        "SELECT id, project_id, master_pid, created_at FROM sessions WHERE id = ?",
        params![session_id],
        |row| {
            Ok(Session {
                id: row.get(0)?,
                project_id: row.get(1)?,
                master_pid: row.get(2)?,
                created_at: row.get(3)?,
            })
        },
    )
    .optional()
    .map_err(|err| map_db_error("query session by id", err))
}

pub(crate) fn query_active_sessions_sync(conn: &Connection) -> Result<Vec<Session>, CcbdError> {
    let mut stmt = conn
        .prepare(
            "SELECT DISTINCT sessions.id, sessions.project_id, sessions.master_pid, sessions.created_at \
             FROM sessions \
             JOIN agents ON agents.session_id = sessions.id \
             WHERE agents.state NOT IN ('CRASHED', 'KILLED') \
             ORDER BY sessions.created_at ASC, sessions.id ASC",
        )
        .map_err(|err| map_db_error("prepare active sessions query", err))?;
    let rows = stmt
        .query_map([], |row| {
            Ok(Session {
                id: row.get(0)?,
                project_id: row.get(1)?,
                master_pid: row.get(2)?,
                created_at: row.get(3)?,
            })
        })
        .map_err(|err| map_db_error("query active sessions", err))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|err| map_db_error("collect active sessions", err))
}

pub(crate) fn query_session_by_cwd_sync(
    conn: &Connection,
    absolute_path: &str,
) -> Result<Option<Session>, CcbdError> {
    conn.query_row(
        "SELECT sessions.id, sessions.project_id, sessions.master_pid, sessions.created_at \
         FROM sessions \
         JOIN projects ON projects.id = sessions.project_id \
         WHERE projects.absolute_path = ? \
         ORDER BY sessions.created_at DESC \
         LIMIT 1",
        params![absolute_path],
        |row| {
            Ok(Session {
                id: row.get(0)?,
                project_id: row.get(1)?,
                master_pid: row.get(2)?,
                created_at: row.get(3)?,
            })
        },
    )
    .optional()
    .map_err(|err| map_db_error("query session by cwd", err))
}

pub(crate) fn list_session_summaries_sync(
    conn: &Connection,
) -> Result<Vec<SessionSummary>, CcbdError> {
    let mut stmt = conn
        .prepare(
            "SELECT sessions.id, sessions.project_id, projects.absolute_path, sessions.master_pid, \
                    COALESCE(SUM(CASE WHEN agents.state NOT IN ('CRASHED', 'KILLED') THEN 1 ELSE 0 END), 0) AS active_agents, \
                    sessions.created_at \
             FROM sessions \
             JOIN projects ON projects.id = sessions.project_id \
             LEFT JOIN agents ON agents.session_id = sessions.id \
             GROUP BY sessions.id, sessions.project_id, projects.absolute_path, sessions.master_pid, sessions.created_at \
             ORDER BY sessions.created_at ASC, sessions.id ASC",
        )
        .map_err(|err| map_db_error("prepare session summaries query", err))?;
    let rows = stmt
        .query_map([], |row| {
            Ok(SessionSummary {
                id: row.get(0)?,
                project_id: row.get(1)?,
                absolute_path: row.get(2)?,
                master_pid: row.get(3)?,
                active_agents: row.get(4)?,
                created_at: row.get(5)?,
            })
        })
        .map_err(|err| map_db_error("query session summaries", err))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|err| map_db_error("collect session summaries", err))
}

pub async fn insert_session(
    db: Db,
    session_id: String,
    project_id: String,
    absolute_path: String,
    master_pid: i64,
) -> Result<(), CcbdError> {
    spawn_db("sessions::insert_session", move || {
        let conn = db.conn();
        insert_session_sync(&conn, &session_id, &project_id, &absolute_path, master_pid)
    })
    .await
}

pub async fn create_session(
    db: Db,
    session_id: String,
    project_id: String,
    absolute_path: String,
    master_pid: i32,
) -> Result<OwnedFd, CcbdError> {
    spawn_db("sessions::create_session", move || {
        create_session_sync(&db, &session_id, &project_id, &absolute_path, master_pid)
    })
    .await
}

pub async fn session_exists(db: Db, session_id: String) -> Result<bool, CcbdError> {
    spawn_db("sessions::session_exists", move || {
        let conn = db.conn();
        session_exists_sync(&conn, &session_id)
    })
    .await
}

pub async fn query_session_by_id(db: Db, session_id: String) -> Result<Option<Session>, CcbdError> {
    spawn_db("sessions::query_session_by_id", move || {
        let conn = db.conn();
        query_session_by_id_sync(&conn, &session_id)
    })
    .await
}

pub async fn query_active_sessions(db: Db) -> Result<Vec<Session>, CcbdError> {
    spawn_db("sessions::query_active_sessions", move || {
        let conn = db.conn();
        query_active_sessions_sync(&conn)
    })
    .await
}

pub async fn query_session_by_cwd(
    db: Db,
    absolute_path: String,
) -> Result<Option<Session>, CcbdError> {
    spawn_db("sessions::query_session_by_cwd", move || {
        let conn = db.conn();
        query_session_by_cwd_sync(&conn, &absolute_path)
    })
    .await
}

pub async fn list_session_summaries(db: Db) -> Result<Vec<SessionSummary>, CcbdError> {
    spawn_db("sessions::list_session_summaries", move || {
        let conn = db.conn();
        list_session_summaries_sync(&conn)
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::{
        create_session_sync, insert_session_sync, list_session_summaries_sync,
        query_active_sessions_sync, query_session_by_cwd_sync, query_session_by_id_sync,
        session_exists_sync,
    };
    use crate::db::agents::insert_agent_sync;
    use crate::db::{Db, init};

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

    #[test]
    fn test_insert_session_then_agent() {
        with_test_db(|conn| {
            insert_session_sync(conn, "s1", "p1", "/tmp/foo", 999).unwrap();
            insert_agent_sync(conn, "a1", "s1", "bash", "IDLE", Some(123)).unwrap();

            let count: i64 = conn
                .query_row("SELECT COUNT(*) FROM agents", [], |row| row.get(0))
                .unwrap();

            assert_eq!(count, 1);
        });
    }

    #[test]
    fn test_session_exists() {
        with_test_db(|conn| {
            insert_session_sync(conn, "s1", "p1", "/tmp/foo", std::process::id() as i64).unwrap();
            assert!(session_exists_sync(conn, "s1").unwrap());
            assert!(!session_exists_sync(conn, "missing").unwrap());
        });
    }

    #[test]
    fn test_create_session_tx_success_and_existing_project() {
        with_test_db_handle(|db| {
            create_session_sync(db, "s1", "p1", "/tmp/foo", std::process::id() as i32).unwrap();
            create_session_sync(db, "s2", "p1", "/tmp/foo", std::process::id() as i32).unwrap();
            let count: i64 = db
                .conn()
                .query_row("SELECT COUNT(*) FROM sessions", [], |row| row.get(0))
                .unwrap();
            assert_eq!(count, 2);
        });
    }

    #[test]
    fn test_create_session_tx_dead_master_rolls_back_project() {
        with_test_db_handle(|db| {
            let err = create_session_sync(db, "s1", "p_dead", "/tmp/dead", i32::MAX).unwrap_err();
            assert!(matches!(err, crate::error::CcbdError::IpcInvalidRequest(_)));
            let count: i64 = db
                .conn()
                .query_row("SELECT COUNT(*) FROM projects", [], |row| row.get(0))
                .unwrap();
            assert_eq!(count, 0);
        });
    }

    #[test]
    fn test_query_session_helpers() {
        with_test_db(|conn| {
            insert_session_sync(conn, "s1", "p1", "/tmp/foo", std::process::id() as i64).unwrap();
            insert_session_sync(conn, "s2", "p2", "/tmp/bar", std::process::id() as i64).unwrap();
            insert_agent_sync(conn, "a1", "s1", "bash", "IDLE", Some(123)).unwrap();
            insert_agent_sync(conn, "a2", "s1", "bash", "KILLED", Some(456)).unwrap();

            assert_eq!(
                query_session_by_id_sync(conn, "s1").unwrap().unwrap().id,
                "s1"
            );
            assert!(query_session_by_id_sync(conn, "missing").unwrap().is_none());
            assert_eq!(
                query_session_by_cwd_sync(conn, "/tmp/bar")
                    .unwrap()
                    .unwrap()
                    .id,
                "s2"
            );
            assert_eq!(query_active_sessions_sync(conn).unwrap().len(), 1);

            let summaries = list_session_summaries_sync(conn).unwrap();
            let s1 = summaries.iter().find(|summary| summary.id == "s1").unwrap();
            let s2 = summaries.iter().find(|summary| summary.id == "s2").unwrap();
            assert_eq!(s1.active_agents, 1);
            assert_eq!(s2.active_agents, 0);
        });
    }
}
