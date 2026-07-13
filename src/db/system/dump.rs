use crate::db::Db;
use crate::db::common::{map_db_error, spawn_db};
use crate::error::CcbdError;
use serde_json::{Value, json};

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
                "SELECT id, project_id, status, created_at FROM sessions ORDER BY created_at ASC",
            )
            .map_err(|err| map_db_error("prepare dump sessions", err))?;
        stmt.query_map([], |row| {
            Ok(json!({
                "id": row.get::<_, String>(0)?,
                "project_id": row.get::<_, String>(1)?,
                "status": row.get::<_, String>(2)?,
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
        "monitors": crate::monitor::list_keys(),
    }))
}

pub async fn system_dump(db: Db) -> Result<Value, CcbdError> {
    spawn_db("system::system_dump", move || system_dump_sync(&db)).await
}
