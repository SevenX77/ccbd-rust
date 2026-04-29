use crate::db::Db;
use crate::db::common::{map_db_error, spawn_db};
use crate::error::CcbdError;
use crate::monitor;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use std::os::fd::OwnedFd;

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

#[cfg(test)]
mod tests {
    use super::{create_session_sync, insert_session_sync, session_exists_sync};
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
}
