use crate::error::CcbdError;
use rusqlite::Connection;
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};

pub mod queries;
pub mod schema;

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

    Ok(Db(Arc::new(Mutex::new(conn))))
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
}
