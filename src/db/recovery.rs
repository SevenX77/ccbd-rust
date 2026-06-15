use crate::db::Db;
use crate::error::CcbdError;
use crate::provider::extensions::HookGroup;
use rusqlite::{Connection, OptionalExtension, params};
use std::collections::HashMap;

pub const AGENT_SPAWN_SPECS_DDL: &str = r#"
CREATE TABLE IF NOT EXISTS agent_spawn_specs (
  agent_id TEXT PRIMARY KEY REFERENCES agents(id) ON DELETE CASCADE,
  spec_version INTEGER NOT NULL DEFAULT 1,
  provider TEXT NOT NULL,
  config_hash TEXT NOT NULL,
  spec_json TEXT NOT NULL,
  updated_at INTEGER NOT NULL DEFAULT (unixepoch())
) STRICT;
"#;

pub const AGENTS_BACKOFF_COLUMNS_DDL: &[&str] = &[
    "ALTER TABLE agents ADD COLUMN retry_count INTEGER NOT NULL DEFAULT 0",
    "ALTER TABLE agents ADD COLUMN next_retry_at INTEGER",
    "ALTER TABLE agents ADD COLUMN retry_exhausted INTEGER NOT NULL DEFAULT 0",
];

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AgentSpawnSpec {
    pub agent_id: String,
    pub provider: String,
    #[serde(default)]
    pub env: HashMap<String, String>,
    #[serde(default)]
    pub hooks: HashMap<String, Vec<HookGroup>>,
    #[serde(default)]
    pub plugins: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredAgentSpawnSpec {
    pub spec: AgentSpawnSpec,
    pub config_hash: String,
    pub spec_version: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveryBackoff {
    pub retry_count: i64,
    pub next_retry_at: Option<i64>,
    pub retry_exhausted: bool,
}

pub(crate) fn persist_agent_spawn_spec_sync(
    conn: &Connection,
    spec: &AgentSpawnSpec,
    config_hash: &str,
) -> Result<(), CcbdError> {
    let spec_json = serde_json::to_string(spec).map_err(|err| {
        CcbdError::DbConstraintViolation(format!("serialize agent spawn spec: {err}"))
    })?;
    conn.execute(
        "INSERT INTO agent_spawn_specs (agent_id, spec_version, provider, config_hash, spec_json, updated_at) \
         VALUES (?, 1, ?, ?, ?, unixepoch()) \
         ON CONFLICT(agent_id) DO UPDATE SET \
             spec_version = agent_spawn_specs.spec_version + 1, \
             provider = excluded.provider, \
             config_hash = excluded.config_hash, \
             spec_json = excluded.spec_json, \
             updated_at = unixepoch()",
        params![spec.agent_id, spec.provider, config_hash, spec_json],
    )
    .map_err(|err| CcbdError::DbConstraintViolation(format!("persist agent spawn spec: {err}")))?;
    Ok(())
}

pub(crate) fn query_agent_spawn_spec_sync(
    conn: &Connection,
    agent_id: &str,
) -> Result<Option<StoredAgentSpawnSpec>, CcbdError> {
    let row = conn
        .query_row(
            "SELECT spec_json, config_hash, spec_version, updated_at FROM agent_spawn_specs WHERE agent_id = ?",
            params![agent_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            },
        )
        .optional()
        .map_err(|err| CcbdError::DbConstraintViolation(format!("query agent spawn spec: {err}")))?;
    row.map(|(spec_json, config_hash, spec_version, updated_at)| {
        let spec = serde_json::from_str(&spec_json).map_err(|err| {
            CcbdError::DbConstraintViolation(format!("deserialize agent spawn spec: {err}"))
        })?;
        Ok(StoredAgentSpawnSpec {
            spec,
            config_hash,
            spec_version,
            updated_at,
        })
    })
        .transpose()
}

pub(crate) fn query_agent_spawn_specs_for_session_sync(
    conn: &Connection,
    session_id: &str,
) -> Result<Vec<StoredAgentSpawnSpec>, CcbdError> {
    let mut stmt = conn
        .prepare(
            "SELECT s.spec_json, s.config_hash, s.spec_version, s.updated_at \
             FROM agent_spawn_specs s \
             JOIN agents a ON a.id = s.agent_id \
             WHERE a.session_id = ? \
             ORDER BY a.created_at ASC, a.id ASC",
        )
        .map_err(|err| {
            CcbdError::DbConstraintViolation(format!(
                "prepare query session agent spawn specs: {err}"
            ))
        })?;
    let rows = stmt
        .query_map(params![session_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
            ))
        })
        .map_err(|err| {
            CcbdError::DbConstraintViolation(format!("query session agent spawn specs: {err}"))
        })?;
    rows.map(|row| {
        let (spec_json, config_hash, spec_version, updated_at) = row.map_err(|err| {
            CcbdError::DbConstraintViolation(format!("collect session agent spawn specs: {err}"))
        })?;
        let spec = serde_json::from_str(&spec_json).map_err(|err| {
            CcbdError::DbConstraintViolation(format!(
                "deserialize session agent spawn spec: {err}"
            ))
        })?;
        Ok(StoredAgentSpawnSpec {
            spec,
            config_hash,
            spec_version,
            updated_at,
        })
    })
    .collect()
}

pub(crate) fn try_claim_agent_recovery_sync(
    conn: &Connection,
    agent_id: &str,
    expected_state_version: i64,
    now: i64,
) -> Result<bool, CcbdError> {
    let changed = conn
        .execute(
            "UPDATE agents \
             SET state_version = state_version + 1, updated_at = unixepoch() \
             WHERE id = ? \
               AND state = 'CRASHED' \
               AND state_version = ? \
               AND (retry_exhausted = 0 OR retry_exhausted IS NULL) \
               AND (next_retry_at IS NULL OR next_retry_at <= ?)",
            params![agent_id, expected_state_version, now],
        )
        .map_err(|err| CcbdError::DbConstraintViolation(format!("claim agent recovery: {err}")))?;
    Ok(changed == 1)
}

pub(crate) fn record_recovery_failure_backoff_sync(
    conn: &Connection,
    agent_id: &str,
    now: i64,
) -> Result<RecoveryBackoff, CcbdError> {
    let current = conn
        .query_row(
            "SELECT retry_count FROM agents WHERE id = ?",
            params![agent_id],
            |row| row.get::<_, i64>(0),
        )
        .optional()
        .map_err(|err| {
            CcbdError::DbConstraintViolation(format!("query recovery backoff: {err}"))
        })?;
    let Some(current_retry_count) = current else {
        return Err(CcbdError::AgentNotFound(agent_id.to_string()));
    };
    let retry_count = (current_retry_count + 1).min(5);
    let retry_exhausted = retry_count >= 5;
    let next_retry_at = if retry_exhausted {
        None
    } else {
        let delay = match retry_count {
            1 => 1,
            2 => 2,
            _ => 4,
        };
        Some(now + delay)
    };
    conn.execute(
        "UPDATE agents \
         SET retry_count = ?, next_retry_at = ?, retry_exhausted = ?, updated_at = unixepoch() \
         WHERE id = ?",
        params![
            retry_count,
            next_retry_at,
            if retry_exhausted { 1 } else { 0 },
            agent_id
        ],
    )
    .map_err(|err| CcbdError::DbConstraintViolation(format!("record recovery backoff: {err}")))?;
    Ok(RecoveryBackoff {
        retry_count,
        next_retry_at,
        retry_exhausted,
    })
}

pub(crate) fn clear_recovery_backoff_sync(
    conn: &Connection,
    agent_id: &str,
) -> Result<(), CcbdError> {
    conn.execute(
        "UPDATE agents \
         SET retry_count = 0, next_retry_at = NULL, retry_exhausted = 0, updated_at = unixepoch() \
         WHERE id = ?",
        params![agent_id],
    )
    .map_err(|err| CcbdError::DbConstraintViolation(format!("clear recovery backoff: {err}")))?;
    Ok(())
}

#[allow(dead_code)] // wired into orchestrator run_once in R-A.2
pub(crate) fn try_claim_agent_recovery(
    db: Db,
    agent_id: String,
    expected_state_version: i64,
    now: i64,
) -> Result<bool, CcbdError> {
    let conn = db.conn();
    try_claim_agent_recovery_sync(&conn, &agent_id, expected_state_version, now)
}

#[cfg(test)]
mod tests {
    use super::{
        AgentSpawnSpec, clear_recovery_backoff_sync, persist_agent_spawn_spec_sync,
        query_agent_spawn_spec_sync, record_recovery_failure_backoff_sync,
        try_claim_agent_recovery_sync,
    };
    use crate::db::agents::insert_agent_sync;
    use crate::db::sessions::insert_session_sync;
    use crate::db::{Db, init};
    use crate::provider::extensions::{HookGroup, HookItem};
    use rusqlite::Connection;
    use std::collections::HashMap;
    use std::sync::{Arc, Barrier};
    use std::thread;

    fn with_db<T>(test: impl FnOnce(&Db) -> T) -> T {
        let file = tempfile::NamedTempFile::new().unwrap();
        let db = init(file.path()).unwrap();
        test(&db)
    }

    fn seed_crashed_agent(conn: &Connection) {
        insert_session_sync(conn, "s1", "p1", "/tmp/ra1").unwrap();
        insert_agent_sync(conn, "a1", "s1", "codex", "CRASHED", None).unwrap();
        conn.execute(
            "UPDATE agents SET config_hash = 'hash1', state_version = 7 WHERE id = 'a1'",
            [],
        )
        .unwrap();
    }

    fn sample_spec(agent_id: &str, provider: &str) -> AgentSpawnSpec {
        let mut env = HashMap::new();
        env.insert("FOO".to_string(), "bar".to_string());
        let hooks = HashMap::from([(
            "PreToolUse".to_string(),
            vec![HookGroup {
                matcher: "*".to_string(),
                hooks: vec![HookItem {
                    hook_type: "command".to_string(),
                    command: "echo checked".to_string(),
                    timeout: Some(3),
                }],
            }],
        )]);
        AgentSpawnSpec {
            agent_id: agent_id.to_string(),
            provider: provider.to_string(),
            env,
            hooks,
            plugins: vec!["github@openai-curated".to_string()],
        }
    }

    fn table_columns(conn: &Connection, table: &str) -> Vec<String> {
        conn.prepare(&format!("PRAGMA table_info({table})"))
            .unwrap()
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    }

    #[test]
    fn ra1_schema_new_db_has_agent_spawn_specs_and_backoff_columns() {
        with_db(|db| {
            let conn = db.conn();
            let create_sql: String = conn
                .query_row(
                    "SELECT sql FROM sqlite_master WHERE type='table' AND name='agent_spawn_specs'",
                    [],
                    |row| row.get(0),
                )
                .expect("agent_spawn_specs table should exist");
            assert!(create_sql.contains("agent_id TEXT PRIMARY KEY"));
            assert!(create_sql.contains("spec_version INTEGER NOT NULL DEFAULT 1"));
            assert!(create_sql.contains("provider TEXT NOT NULL"));
            assert!(create_sql.contains("config_hash TEXT NOT NULL"));
            assert!(create_sql.contains("spec_json TEXT NOT NULL"));
            assert!(create_sql.contains("updated_at INTEGER NOT NULL DEFAULT (unixepoch())"));
            assert!(create_sql.contains("STRICT"));

            let columns = table_columns(&conn, "agents");
            for column in ["retry_count", "next_retry_at", "retry_exhausted"] {
                assert!(
                    columns.iter().any(|name| name == column),
                    "missing {column}"
                );
            }
        });
    }

    #[test]
    fn ra1_migration_old_db_adds_specs_backoff_and_is_idempotent() {
        let file = tempfile::NamedTempFile::new().unwrap();
        {
            let conn = Connection::open(file.path()).unwrap();
            conn.execute_batch(
                r#"
                PRAGMA foreign_keys = ON;
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

        init(file.path()).unwrap();
        let db = init(file.path()).expect("R-A.1 migrations must be idempotent");
        let conn = db.conn();
        let specs_exists: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='agent_spawn_specs')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(specs_exists);
        let columns = table_columns(&conn, "agents");
        for column in ["retry_count", "next_retry_at", "retry_exhausted"] {
            assert!(
                columns.iter().any(|name| name == column),
                "missing {column}"
            );
        }
    }

    #[test]
    fn ra1_agent_spawn_specs_cascade_delete_with_agent() {
        with_db(|db| {
            let conn = db.conn();
            seed_crashed_agent(&conn);
            persist_agent_spawn_spec_sync(&conn, &sample_spec("a1", "codex"), "hash1").unwrap();
            conn.execute("DELETE FROM agents WHERE id = 'a1'", [])
                .unwrap();
            let count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM agent_spawn_specs WHERE agent_id = 'a1'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(count, 0);
        });
    }

    #[test]
    fn ra1_snapshot_round_trips_overwrites_and_rebuilds_realign_params_fields() {
        with_db(|db| {
            let conn = db.conn();
            seed_crashed_agent(&conn);
            let first = sample_spec("a1", "codex");
            persist_agent_spawn_spec_sync(&conn, &first, "hash1").unwrap();
            let stored = query_agent_spawn_spec_sync(&conn, "a1").unwrap().unwrap();
            assert_eq!(stored.spec, first);
            assert_eq!(stored.config_hash, "hash1");

            let mut second = sample_spec("a1", "claude");
            second.env.insert("NEW".to_string(), "1".to_string());
            second.plugins.push("extra@local".to_string());
            persist_agent_spawn_spec_sync(&conn, &second, "hash2").unwrap();
            let overwritten = query_agent_spawn_spec_sync(&conn, "a1").unwrap().unwrap();
            assert_eq!(overwritten.spec.agent_id, "a1");
            assert_eq!(overwritten.spec.provider, "claude");
            assert_eq!(overwritten.spec.env["NEW"], "1");
            assert!(overwritten.spec.hooks.contains_key("PreToolUse"));
            assert!(
                overwritten
                    .spec
                    .plugins
                    .contains(&"extra@local".to_string())
            );
            assert_eq!(overwritten.config_hash, "hash2");
            assert!(overwritten.spec_version >= stored.spec_version);
            assert!(overwritten.updated_at >= stored.updated_at);
        });
    }

    #[test]
    fn ra1_missing_snapshot_is_valid_none() {
        with_db(|db| {
            let conn = db.conn();
            seed_crashed_agent(&conn);
            let missing = query_agent_spawn_spec_sync(&conn, "a1").unwrap();
            assert!(missing.is_none());
        });
    }

    #[test]
    fn ra1_recovery_claim_cas_bumps_version_but_keeps_crashed() {
        with_db(|db| {
            let conn = db.conn();
            seed_crashed_agent(&conn);
            assert!(try_claim_agent_recovery_sync(&conn, "a1", 7, 1_000).unwrap());
            let row: (String, i64) = conn
                .query_row(
                    "SELECT state, state_version FROM agents WHERE id = 'a1'",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .unwrap();
            assert_eq!(row, ("CRASHED".to_string(), 8));
            assert!(!try_claim_agent_recovery_sync(&conn, "a1", 7, 1_000).unwrap());
        });
    }

    #[test]
    fn ra1_recovery_claim_cas_allows_only_one_concurrent_winner() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("ra1.db");
        {
            let db = init(&db_path).unwrap();
            let conn = db.conn();
            seed_crashed_agent(&conn);
        }
        let barrier = Arc::new(Barrier::new(2));
        let handles = (0..2)
            .map(|_| {
                let db_path = db_path.clone();
                let barrier = Arc::clone(&barrier);
                thread::spawn(move || {
                    let conn = Connection::open(db_path).unwrap();
                    conn.pragma_update(None, "foreign_keys", "ON").unwrap();
                    barrier.wait();
                    try_claim_agent_recovery_sync(&conn, "a1", 7, 1_000).unwrap()
                })
            })
            .collect::<Vec<_>>();
        let winners = handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .filter(|won| *won)
            .count();
        assert_eq!(winners, 1);
    }

    #[test]
    fn ra1_backoff_failure_sequence_caps_and_exhausts_without_leaving_crashed() {
        with_db(|db| {
            let conn = db.conn();
            seed_crashed_agent(&conn);
            let expected = [
                (1, Some(1_001), false),
                (2, Some(1_002), false),
                (3, Some(1_004), false),
                (4, Some(1_004), false),
                (5, None, true),
            ];
            for (retry_count, next_retry_at, retry_exhausted) in expected {
                let backoff = record_recovery_failure_backoff_sync(&conn, "a1", 1_000).unwrap();
                assert_eq!(backoff.retry_count, retry_count);
                assert_eq!(backoff.next_retry_at, next_retry_at);
                assert_eq!(backoff.retry_exhausted, retry_exhausted);
                let state: String = conn
                    .query_row("SELECT state FROM agents WHERE id = 'a1'", [], |row| {
                        row.get(0)
                    })
                    .unwrap();
                assert_eq!(state, "CRASHED");
            }
        });
    }

    #[test]
    fn ra1_clear_recovery_backoff_resets_retry_fields() {
        with_db(|db| {
            let conn = db.conn();
            seed_crashed_agent(&conn);
            for _ in 0..5 {
                let _ = record_recovery_failure_backoff_sync(&conn, "a1", 1_000).unwrap();
            }
            clear_recovery_backoff_sync(&conn, "a1").unwrap();
            let row: (i64, Option<i64>, i64) = conn
                .query_row(
                    "SELECT retry_count, next_retry_at, retry_exhausted FROM agents WHERE id = 'a1'",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .unwrap();
            assert_eq!(row, (0, None, 0));
        });
    }

    #[test]
    fn ra2_fresh_db_and_migrated_db_recovery_schema_are_equivalent() {
        let fresh_file = tempfile::NamedTempFile::new().unwrap();
        let fresh = init(fresh_file.path()).unwrap();
        let fresh_conn = fresh.conn();
        let fresh_specs_sql: String = fresh_conn
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type='table' AND name='agent_spawn_specs'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let fresh_agent_columns = table_columns(&fresh_conn, "agents");

        let migrated_file = tempfile::NamedTempFile::new().unwrap();
        {
            let conn = Connection::open(migrated_file.path()).unwrap();
            conn.execute_batch(
                r#"
                PRAGMA foreign_keys = ON;
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
        let migrated = init(migrated_file.path()).unwrap();
        let migrated_conn = migrated.conn();
        let migrated_specs_sql: String = migrated_conn
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type='table' AND name='agent_spawn_specs'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let migrated_agent_columns = table_columns(&migrated_conn, "agents");

        assert_eq!(fresh_specs_sql, migrated_specs_sql);
        for column in ["retry_count", "next_retry_at", "retry_exhausted"] {
            assert_eq!(
                fresh_agent_columns.iter().any(|name| name == column),
                migrated_agent_columns.iter().any(|name| name == column),
                "fresh/migrated mismatch for {column}"
            );
        }
    }
}
