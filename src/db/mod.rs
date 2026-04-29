use crate::error::CcbdError;
use rusqlite::Connection;
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};

pub mod agents;
pub mod agents_lifecycle;
pub mod common;
pub mod events;
pub mod events_progress;
pub mod evidence;
pub mod schema;
pub mod sessions;
pub mod state_machine;
pub mod state_machine_assert;
pub mod system;

#[derive(Clone)]
pub struct Db(pub Arc<Mutex<Connection>>);

impl Db {
    pub fn conn(&self) -> MutexGuard<'_, Connection> {
        self.0.lock().expect("database connection mutex poisoned")
    }
}

pub fn init(db_path: &Path) -> Result<Db, CcbdError> {
    let conn = Connection::open(db_path).map_err(|err| {
        CcbdError::DbConstraintViolation(format!("open {}: {err}", db_path.display()))
    })?;

    conn.execute_batch(
        r#"
        PRAGMA foreign_keys = ON;
        PRAGMA journal_mode = WAL;
        PRAGMA synchronous = NORMAL;
        PRAGMA busy_timeout = 5000;
        "#,
    )
    .map_err(|err| CcbdError::DbConstraintViolation(format!("initialize pragmas: {err}")))?;

    conn.execute_batch(schema::SCHEMA_DDL)
        .map_err(|err| CcbdError::DbConstraintViolation(format!("initialize schema: {err}")))?;
    migrate_sub_state(&conn)?;

    Ok(Db(Arc::new(Mutex::new(conn))))
}

fn migrate_sub_state(conn: &Connection) -> Result<(), CcbdError> {
    match conn.execute("ALTER TABLE agents ADD COLUMN sub_state TEXT", []) {
        Ok(_) => Ok(()),
        Err(rusqlite::Error::SqliteFailure(_, Some(message)))
            if message.contains("duplicate column name") =>
        {
            Ok(())
        }
        Err(err) => Err(CcbdError::DbConstraintViolation(format!(
            "migrate agents.sub_state: {err}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::init;

    #[test]
    fn test_init_creates_wal_db() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let db = init(file.path()).unwrap();
        let conn = db.conn();

        let journal_mode: String = conn
            .pragma_query_value(None, "journal_mode", |row| row.get(0))
            .unwrap();

        assert_eq!(journal_mode, "wal");
    }

    #[test]
    fn test_init_busy_timeout_5000() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let db = init(file.path()).unwrap();
        let conn = db.conn();

        let busy_timeout: i64 = conn
            .pragma_query_value(None, "busy_timeout", |row| row.get(0))
            .unwrap();

        assert_eq!(busy_timeout, 5000);
    }

    #[test]
    fn test_init_foreign_keys_on() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let db = init(file.path()).unwrap();
        let conn = db.conn();

        let foreign_keys: i64 = conn
            .pragma_query_value(None, "foreign_keys", |row| row.get(0))
            .unwrap();

        assert_eq!(foreign_keys, 1);
    }

    #[test]
    fn test_init_schema_has_sub_state() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let db = init(file.path()).unwrap();
        let conn = db.conn();
        let has_sub_state: bool = conn
            .prepare("PRAGMA table_info(agents)")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
            .iter()
            .any(|name| name == "sub_state");

        assert!(has_sub_state);
    }

    #[test]
    fn test_init_schema_has_evidence_table_and_indexes() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let db = init(file.path()).unwrap();
        let conn = db.conn();
        let objects = conn
            .prepare("SELECT name FROM sqlite_master WHERE type IN ('table', 'index')")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();

        assert!(objects.iter().any(|name| name == "evidence"));
        assert!(objects.iter().any(|name| name == "idx_evidence_pending"));
        assert!(objects.iter().any(|name| name == "idx_evidence_agent"));
    }

    #[test]
    fn test_init_migrates_old_agents_schema() {
        let file = tempfile::NamedTempFile::new().unwrap();
        {
            let conn = rusqlite::Connection::open(file.path()).unwrap();
            conn.execute_batch(
                r#"
                CREATE TABLE projects (
                    id TEXT PRIMARY KEY,
                    absolute_path TEXT NOT NULL UNIQUE,
                    created_at INTEGER NOT NULL DEFAULT (unixepoch())
                ) STRICT;
                CREATE TABLE sessions (
                    id TEXT PRIMARY KEY,
                    project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
                    master_pid INTEGER NOT NULL,
                    created_at INTEGER NOT NULL DEFAULT (unixepoch())
                ) STRICT;
                CREATE TABLE agents (
                    id TEXT PRIMARY KEY,
                    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
                    provider TEXT NOT NULL,
                    state TEXT NOT NULL,
                    state_version INTEGER NOT NULL DEFAULT 1,
                    pid INTEGER,
                    exit_code INTEGER,
                    error_code TEXT,
                    created_at INTEGER NOT NULL DEFAULT (unixepoch()),
                    updated_at INTEGER NOT NULL DEFAULT (unixepoch())
                ) STRICT;
                "#,
            )
            .unwrap();
        }

        let db = init(file.path()).unwrap();
        let conn = db.conn();
        let has_sub_state: bool = conn
            .prepare("PRAGMA table_info(agents)")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
            .iter()
            .any(|name| name == "sub_state");

        assert!(has_sub_state);

        let evidence_exists: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='evidence')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(evidence_exists);
    }
}
