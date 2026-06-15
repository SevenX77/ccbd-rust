use crate::db::Db;
use crate::db::common::map_db_error;
use crate::error::CcbdError;
use rusqlite::{OptionalExtension, TransactionBehavior, params};

pub const MASTER_CUTOVERS_DDL: &str = r#"
CREATE TABLE IF NOT EXISTS master_cutovers (
    id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    state TEXT NOT NULL CHECK(state IN (
        'PREPARING',
        'SPAWNING',
        'VERIFYING',
        'ACTIVE',
        'ROLLED_BACK',
        'FAILED',
        'RELEASED'
    )),
    old_master_pid INTEGER,
    new_master_pid INTEGER,
    new_master_generation INTEGER,
    new_master_pane_id TEXT,
    ah_state_dir TEXT NOT NULL,
    ah_socket_path TEXT NOT NULL,
    handoff_path TEXT NOT NULL,
    created_at INTEGER NOT NULL DEFAULT (unixepoch()),
    updated_at INTEGER NOT NULL DEFAULT (unixepoch()),
    completed_at INTEGER
) STRICT;

CREATE UNIQUE INDEX IF NOT EXISTS idx_master_cutovers_active
ON master_cutovers(session_id)
WHERE state IN ('PREPARING', 'SPAWNING', 'VERIFYING', 'ACTIVE');
"#;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MasterCutoverClaim {
    Claimed,
    AlreadyActive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MasterCutoverUpdate {
    Updated,
    Stale,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MasterCutover {
    pub id: String,
    pub session_id: String,
    pub state: String,
    pub old_master_pid: Option<i64>,
    pub new_master_pid: Option<i64>,
    pub new_master_generation: Option<i64>,
    pub new_master_pane_id: Option<String>,
    pub ah_state_dir: String,
    pub ah_socket_path: String,
    pub handoff_path: String,
}

pub fn claim_master_cutover(
    db: &Db,
    _id: &str,
    _session_id: &str,
    _old_master_pid: Option<i64>,
    _ah_state_dir: &str,
    _ah_socket_path: &str,
    _handoff_path: &str,
) -> Result<MasterCutoverClaim, CcbdError> {
    let mut conn = db.conn();
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|err| map_db_error("begin master cutover claim", err))?;
    let active_exists: bool = tx
        .query_row(
            "SELECT 1 FROM master_cutovers
             WHERE session_id = ?1
               AND state IN ('PREPARING', 'SPAWNING', 'VERIFYING', 'ACTIVE')
             LIMIT 1",
            params![_session_id],
            |_| Ok(true),
        )
        .optional()
        .map(|value| value.unwrap_or(false))
        .map_err(|err| map_db_error("query active master cutover", err))?;
    if active_exists {
        tx.commit()
            .map_err(|err| map_db_error("commit master cutover already active", err))?;
        return Ok(MasterCutoverClaim::AlreadyActive);
    }
    tx.execute(
        "INSERT INTO master_cutovers (
            id,
            session_id,
            state,
            old_master_pid,
            ah_state_dir,
            ah_socket_path,
            handoff_path
         ) VALUES (?1, ?2, 'PREPARING', ?3, ?4, ?5, ?6)",
        params![
            _id,
            _session_id,
            _old_master_pid,
            _ah_state_dir,
            _ah_socket_path,
            _handoff_path
        ],
    )
    .map_err(|err| map_db_error("insert master cutover", err))?;
    tx.commit()
        .map_err(|err| map_db_error("commit master cutover claim", err))?;
    Ok(MasterCutoverClaim::Claimed)
}

pub fn update_master_cutover_state(
    db: &Db,
    _id: &str,
    _expected_state: &str,
    _next_state: &str,
) -> Result<MasterCutoverUpdate, CcbdError> {
    let conn = db.conn();
    let changes = conn
        .execute(
            "UPDATE master_cutovers
             SET state = ?3,
                 updated_at = unixepoch(),
                 completed_at = CASE
                     WHEN ?3 IN ('ROLLED_BACK', 'FAILED', 'RELEASED') THEN unixepoch()
                     ELSE completed_at
                 END
             WHERE id = ?1
               AND state = ?2",
            params![_id, _expected_state, _next_state],
        )
        .map_err(|err| map_db_error("update master cutover state", err))?;
    if changes == 1 {
        Ok(MasterCutoverUpdate::Updated)
    } else {
        Ok(MasterCutoverUpdate::Stale)
    }
}

pub fn release_master_cutover(
    db: &Db,
    id: &str,
    expected_state: &str,
) -> Result<MasterCutoverUpdate, CcbdError> {
    update_master_cutover_state(db, id, expected_state, "RELEASED")
}

pub fn get_active_master_cutover(
    _db: &Db,
    _session_id: &str,
) -> Result<Option<MasterCutover>, CcbdError> {
    let conn = _db.conn();
    conn.query_row(
        "SELECT id,
                session_id,
                state,
                old_master_pid,
                new_master_pid,
                new_master_generation,
                new_master_pane_id,
                ah_state_dir,
                ah_socket_path,
                handoff_path
         FROM master_cutovers
         WHERE session_id = ?1
           AND state IN ('PREPARING', 'SPAWNING', 'VERIFYING', 'ACTIVE')
         ORDER BY created_at DESC, id DESC
         LIMIT 1",
        params![_session_id],
        |row| {
            Ok(MasterCutover {
                id: row.get(0)?,
                session_id: row.get(1)?,
                state: row.get(2)?,
                old_master_pid: row.get(3)?,
                new_master_pid: row.get(4)?,
                new_master_generation: row.get(5)?,
                new_master_pane_id: row.get(6)?,
                ah_state_dir: row.get(7)?,
                ah_socket_path: row.get(8)?,
                handoff_path: row.get(9)?,
            })
        },
    )
    .optional()
    .map_err(|err| map_db_error("query active master cutover", err))
}

#[cfg(test)]
mod tests {
    use super::{
        MasterCutoverClaim, MasterCutoverUpdate, claim_master_cutover, get_active_master_cutover,
        release_master_cutover, update_master_cutover_state,
    };
    use crate::db::init;

    fn insert_session(db: &crate::db::Db, session_id: &str) {
        let conn = db.conn();
        conn.execute(
            "INSERT INTO projects (id, absolute_path) VALUES (?1, ?2)",
            [format!("project_{session_id}"), format!("/tmp/{session_id}")],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO sessions (id, project_id, master_pid) VALUES (?1, ?2, 123)",
            [session_id.to_string(), format!("project_{session_id}")],
        )
        .unwrap();
    }

    #[test]
    fn schema_has_master_cutovers_table_and_active_unique_index() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let db = init(file.path()).unwrap();
        let conn = db.conn();
        let table_exists: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'master_cutovers')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let index_sql: String = conn
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type = 'index' AND name = 'idx_master_cutovers_active'",
                [],
                |row| row.get(0),
            )
            .unwrap();

        assert!(table_exists);
        assert!(index_sql.contains("UNIQUE INDEX"));
        assert!(index_sql.contains("session_id"));
        assert!(index_sql.contains("'PREPARING'"));
        assert!(index_sql.contains("'SPAWNING'"));
        assert!(index_sql.contains("'VERIFYING'"));
        assert!(index_sql.contains("'ACTIVE'"));
    }

    #[test]
    fn master_cutover_claim_is_single_active_per_session() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let db = init(file.path()).unwrap();
        insert_session(&db, "s_cutover");

        let first = claim_master_cutover(
            &db,
            "cutover_1",
            "s_cutover",
            Some(111),
            "/tmp/ah-state",
            "/tmp/ah-state/ahd.sock",
            "/tmp/ah-state/cutovers/cutover_1/handoff.md",
        )
        .unwrap();
        let second = claim_master_cutover(
            &db,
            "cutover_2",
            "s_cutover",
            Some(111),
            "/tmp/ah-state",
            "/tmp/ah-state/ahd.sock",
            "/tmp/ah-state/cutovers/cutover_2/handoff.md",
        )
        .unwrap();
        let active = get_active_master_cutover(&db, "s_cutover")
            .unwrap()
            .expect("active cutover");

        assert_eq!(first, MasterCutoverClaim::Claimed);
        assert_eq!(second, MasterCutoverClaim::AlreadyActive);
        assert_eq!(active.id, "cutover_1");
        assert_eq!(active.state, "PREPARING");

        assert_eq!(
            release_master_cutover(&db, "cutover_1", "PREPARING").unwrap(),
            MasterCutoverUpdate::Updated
        );
        assert_eq!(
            claim_master_cutover(
                &db,
                "cutover_3",
                "s_cutover",
                Some(111),
                "/tmp/ah-state",
                "/tmp/ah-state/ahd.sock",
                "/tmp/ah-state/cutovers/cutover_3/handoff.md",
            )
            .unwrap(),
            MasterCutoverClaim::Claimed
        );
    }

    #[test]
    fn master_cutover_state_transitions_are_cas_guarded() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let db = init(file.path()).unwrap();
        insert_session(&db, "s_cutover_cas");
        assert_eq!(
            claim_master_cutover(
                &db,
                "cutover_cas",
                "s_cutover_cas",
                Some(222),
                "/tmp/ah-state",
                "/tmp/ah-state/ahd.sock",
                "/tmp/ah-state/cutovers/cutover_cas/handoff.md",
            )
            .unwrap(),
            MasterCutoverClaim::Claimed
        );

        assert_eq!(
            update_master_cutover_state(&db, "cutover_cas", "VERIFYING", "ACTIVE").unwrap(),
            MasterCutoverUpdate::Stale
        );
        let active = get_active_master_cutover(&db, "s_cutover_cas")
            .unwrap()
            .expect("active cutover");
        assert_eq!(active.state, "PREPARING");

        assert_eq!(
            update_master_cutover_state(&db, "cutover_cas", "PREPARING", "SPAWNING").unwrap(),
            MasterCutoverUpdate::Updated
        );
        let active = get_active_master_cutover(&db, "s_cutover_cas")
            .unwrap()
            .expect("active cutover");
        assert_eq!(active.state, "SPAWNING");
    }
}
