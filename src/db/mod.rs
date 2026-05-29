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
pub mod jobs;
pub mod prompt_experience;
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
    migrate_jobs_cancel_requested(&conn)?;
    migrate_sessions_status(&conn)?;
    migrate_sessions_master_pane_id(&conn)?;
    migrate_evidence_record_columns(&conn)?;
    migrate_jobs_evidence_requirements(&conn)?;

    Ok(Db(Arc::new(Mutex::new(conn))))
}

fn migrate_sessions_master_pane_id(conn: &Connection) -> Result<(), CcbdError> {
    match conn.execute("ALTER TABLE sessions ADD COLUMN master_pane_id TEXT", []) {
        Ok(_) => Ok(()),
        Err(rusqlite::Error::SqliteFailure(_, Some(message)))
            if message.contains("duplicate column name") =>
        {
            Ok(())
        }
        Err(err) => Err(CcbdError::DbConstraintViolation(format!(
            "migrate sessions.master_pane_id: {err}"
        ))),
    }
}

fn migrate_evidence_record_columns(conn: &Connection) -> Result<(), CcbdError> {
    add_column_if_missing(
        conn,
        "evidence",
        "job_id",
        "ALTER TABLE evidence ADD COLUMN job_id TEXT REFERENCES jobs(id) ON DELETE CASCADE",
    )?;
    add_column_if_missing(
        conn,
        "evidence",
        "evidence_type",
        "ALTER TABLE evidence ADD COLUMN evidence_type TEXT",
    )?;
    add_column_if_missing(
        conn,
        "evidence",
        "subject_path",
        "ALTER TABLE evidence ADD COLUMN subject_path TEXT",
    )?;
    add_column_if_missing(
        conn,
        "evidence",
        "payload",
        "ALTER TABLE evidence ADD COLUMN payload TEXT",
    )?;
    conn.execute_batch(
        r#"
        CREATE INDEX IF NOT EXISTS idx_evidence_agent_job_type ON evidence(agent_id, job_id, evidence_type);
        CREATE INDEX IF NOT EXISTS idx_evidence_agent_type_path ON evidence(agent_id, evidence_type, subject_path);
        "#,
    )
    .map_err(|err| CcbdError::DbConstraintViolation(format!("migrate evidence indexes: {err}")))
}

fn migrate_jobs_evidence_requirements(conn: &Connection) -> Result<(), CcbdError> {
    add_column_if_missing(
        conn,
        "jobs",
        "requires_physical_evidence",
        "ALTER TABLE jobs ADD COLUMN requires_physical_evidence INTEGER NOT NULL DEFAULT 0",
    )?;
    add_column_if_missing(
        conn,
        "jobs",
        "requires_test_evidence",
        "ALTER TABLE jobs ADD COLUMN requires_test_evidence INTEGER NOT NULL DEFAULT 0",
    )
}

fn add_column_if_missing(
    conn: &Connection,
    table: &str,
    column: &str,
    statement: &str,
) -> Result<(), CcbdError> {
    match conn.execute(statement, []) {
        Ok(_) => Ok(()),
        Err(rusqlite::Error::SqliteFailure(_, Some(message)))
            if message.contains("duplicate column name") =>
        {
            Ok(())
        }
        Err(err) => Err(CcbdError::DbConstraintViolation(format!(
            "migrate {table}.{column}: {err}"
        ))),
    }
}

fn migrate_sessions_status(conn: &Connection) -> Result<(), CcbdError> {
    match conn.execute(
        "ALTER TABLE sessions ADD COLUMN status TEXT NOT NULL DEFAULT 'ACTIVE'",
        [],
    ) {
        Ok(_) => Ok(()),
        Err(rusqlite::Error::SqliteFailure(_, Some(message)))
            if message.contains("duplicate column name") =>
        {
            Ok(())
        }
        Err(err) => Err(CcbdError::DbConstraintViolation(format!(
            "migrate sessions.status: {err}"
        ))),
    }
}

fn migrate_jobs_cancel_requested(conn: &Connection) -> Result<(), CcbdError> {
    match conn.execute(
        "ALTER TABLE jobs ADD COLUMN cancel_requested INTEGER NOT NULL DEFAULT 0",
        [],
    ) {
        Ok(_) => Ok(()),
        Err(rusqlite::Error::SqliteFailure(_, Some(message)))
            if message.contains("duplicate column name") =>
        {
            Ok(())
        }
        Err(err) => Err(CcbdError::DbConstraintViolation(format!(
            "migrate jobs.cancel_requested: {err}"
        ))),
    }
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
    fn test_init_schema_has_session_status() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let db = init(file.path()).unwrap();
        let conn = db.conn();
        let has_status: bool = conn
            .prepare("PRAGMA table_info(sessions)")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
            .iter()
            .any(|name| name == "status");

        assert!(has_status);
    }

    #[test]
    fn test_init_schema_has_session_master_pane_id() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let db = init(file.path()).unwrap();
        let conn = db.conn();
        let has_master_pane_id: bool = conn
            .prepare("PRAGMA table_info(sessions)")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
            .iter()
            .any(|name| name == "master_pane_id");

        assert!(has_master_pane_id);
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
        assert!(
            objects
                .iter()
                .any(|name| name == "idx_evidence_agent_job_type")
        );
        assert!(
            objects
                .iter()
                .any(|name| name == "idx_evidence_agent_type_path")
        );
    }

    #[test]
    fn test_init_migrates_old_evidence_schema() {
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
                    status TEXT NOT NULL DEFAULT 'ACTIVE',
                    master_pane_id TEXT,
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
                    sub_state TEXT,
                    updated_at INTEGER NOT NULL DEFAULT (unixepoch())
                ) STRICT;
                CREATE TABLE events (
                    seq_id INTEGER PRIMARY KEY AUTOINCREMENT,
                    agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                    request_id TEXT,
                    event_type TEXT NOT NULL,
                    payload TEXT NOT NULL,
                    created_at INTEGER NOT NULL DEFAULT (unixepoch())
                ) STRICT;
                CREATE TABLE evidence (
                    id TEXT PRIMARY KEY,
                    agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                    event_seq_id INTEGER NOT NULL REFERENCES events(seq_id),
                    pane_bytes BLOB NOT NULL,
                    failed_rules TEXT NOT NULL,
                    status TEXT NOT NULL DEFAULT 'PENDING',
                    l3_asserted_state TEXT,
                    created_at INTEGER NOT NULL DEFAULT (unixepoch())
                ) STRICT;
                CREATE TABLE jobs (
                    id TEXT PRIMARY KEY,
                    agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                    request_id TEXT,
                    prompt_text TEXT NOT NULL,
                    reply_text TEXT,
                    status TEXT NOT NULL DEFAULT 'QUEUED',
                    error_reason TEXT,
                    created_at INTEGER NOT NULL DEFAULT (unixepoch()),
                    dispatched_at INTEGER,
                    dispatched_at_seq_id INTEGER,
                    completed_at INTEGER,
                    cancel_requested INTEGER NOT NULL DEFAULT 0
                ) STRICT;
                INSERT INTO projects (id, absolute_path) VALUES ('p1', '/tmp/pr1a-old');
                INSERT INTO sessions (id, project_id, master_pid) VALUES ('s1', 'p1', 1);
                INSERT INTO agents (id, session_id, provider, state) VALUES ('a1', 's1', 'bash', 'UNKNOWN');
                INSERT INTO events (agent_id, event_type, payload) VALUES ('a1', 'state_change', '{}');
                INSERT INTO evidence (id, agent_id, event_seq_id, pane_bytes, failed_rules)
                    VALUES ('evi_old', 'a1', 1, X'01', '[]');
                "#,
            )
            .unwrap();
        }

        let db = init(file.path()).unwrap();
        let conn = db.conn();
        let columns = conn
            .prepare("PRAGMA table_info(evidence)")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        for name in ["job_id", "evidence_type", "subject_path", "payload"] {
            assert!(columns.iter().any(|column| column == name));
        }
        let migrated: (String, Option<String>, Option<String>, Option<String>, Option<String>) =
            conn.query_row(
                "SELECT status, job_id, evidence_type, subject_path, payload FROM evidence WHERE id = 'evi_old'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
            )
            .unwrap();
        assert_eq!(migrated, ("PENDING".to_string(), None, None, None, None));

        let job_columns = conn
            .prepare("PRAGMA table_info(jobs)")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert!(
            job_columns
                .iter()
                .any(|name| name == "requires_physical_evidence")
        );
        assert!(
            job_columns
                .iter()
                .any(|name| name == "requires_test_evidence")
        );
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
