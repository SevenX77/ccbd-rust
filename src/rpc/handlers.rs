use crate::db::queries::{
    insert_agent, insert_event, query_agent, query_event_by_request_id, query_events_since,
};
use crate::error::CcbdError;
use crate::monitor;
use crate::monitor::master_watch::spawn_master_pidfd_watch_task;
use crate::pty::tasks::{spawn_child_wait_task, spawn_pty_reader_task};
use crate::rpc::Ctx;
use rusqlite::{TransactionBehavior, params};
use serde_json::{Value, json};
use std::io::{Error as IoError, ErrorKind, Write};
use std::sync::Arc;
use uuid::Uuid;

pub async fn handle_session_create(params: Value, ctx: &Ctx) -> Result<Value, CcbdError> {
    let project_id = required_str(&params, "project_id")?;
    let absolute_path = required_str(&params, "absolute_path")?;
    let master_pid = required_i64(&params, "master_pid")?;
    let session_id = format!("sess_{}", Uuid::new_v4());

    let master_pidfd = {
        let mut conn = ctx.db.conn();
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|err| {
                CcbdError::DbConstraintViolation(format!("begin session.create: {err}"))
            })?;
        tx.execute(
            "INSERT OR IGNORE INTO projects (id, absolute_path) VALUES (?, ?)",
            params![project_id, absolute_path],
        )
        .map_err(|err| CcbdError::DbConstraintViolation(format!("insert project: {err}")))?;

        let master_pidfd = match monitor::pidfd_open(master_pid as i32) {
            Ok(pidfd) => pidfd,
            Err(CcbdError::AgentUnexpectedExit { .. }) => {
                let _ = tx.rollback();
                return Err(CcbdError::IpcInvalidRequest(format!(
                    "master_pid {master_pid} not alive"
                )));
            }
            Err(err) => {
                let _ = tx.rollback();
                return Err(err);
            }
        };

        tx.execute(
            "INSERT INTO sessions (id, project_id, master_pid) VALUES (?, ?, ?)",
            params![session_id, project_id, master_pid],
        )
        .map_err(|err| CcbdError::DbConstraintViolation(format!("insert session: {err}")))?;
        tx.commit().map_err(|err| {
            CcbdError::DbConstraintViolation(format!("commit session.create: {err}"))
        })?;
        master_pidfd
    };

    let task_fd = master_pidfd
        .try_clone()
        .map_err(|err| CcbdError::EnvironmentNotSupported {
            details: format!("clone master pidfd for session {session_id}: {err}"),
        })?;
    monitor::register(format!("master:{session_id}"), master_pidfd);
    spawn_master_pidfd_watch_task(session_id.clone(), task_fd, Arc::new(ctx.db.clone()));

    Ok(json!({ "session_id": session_id }))
}

pub async fn handle_agent_spawn(params: Value, ctx: &Ctx) -> Result<Value, CcbdError> {
    let session_id = required_str(&params, "session_id")?;
    let agent_id = required_str(&params, "agent_id")?;
    let provider = required_str(&params, "provider")?;

    let mut spawn_result = crate::pty::spawn_agent(agent_id, provider)?;
    let insert_result = {
        let conn = ctx.db.conn();
        insert_agent(
            &conn,
            agent_id,
            session_id,
            provider,
            "IDLE",
            Some(spawn_result.pid as i64),
        )
    };

    if let Err(err) = insert_result {
        let _ = spawn_result.child.kill();
        remove_pty_writer(agent_id);
        return Err(err);
    }

    spawn_pty_reader_task(
        agent_id.to_string(),
        spawn_result.master_reader,
        ctx.db.0.clone(),
    );
    spawn_child_wait_task(agent_id.to_string(), spawn_result.child, ctx.db.0.clone());

    Ok(json!({ "state": "IDLE" }))
}

pub async fn handle_agent_send(params: Value, ctx: &Ctx) -> Result<Value, CcbdError> {
    let agent_id = required_str(&params, "agent_id")?;
    let text = required_str(&params, "text")?;
    let request_id = params.get("request_id").and_then(Value::as_str);

    let agent = {
        let conn = ctx.db.conn();
        query_agent(&conn, agent_id)?
    };
    let Some(agent) = agent else {
        return Err(CcbdError::AgentNotFound(agent_id.to_string()));
    };
    if agent.state == "CRASHED" {
        return Err(CcbdError::AgentNotFound(agent_id.to_string()));
    }

    let writer_exists = crate::pty::PTY_MAP
        .lock()
        .map_err(|_| CcbdError::PtyIoError("PTY_MAP mutex poisoned".into()))?
        .contains_key(agent_id);
    if !writer_exists {
        return Err(CcbdError::PtyIoError(format!(
            "writer not in PTY_MAP for {agent_id}"
        )));
    }

    let pending_payload = json!({ "cmd": text, "status": "PENDING" }).to_string();
    let seq_id = {
        let conn = ctx.db.conn();
        match insert_event(
            &conn,
            agent_id,
            request_id,
            "command_received",
            &pending_payload,
        ) {
            Ok(seq_id) => seq_id,
            Err(CcbdError::DuplicateRequest { existing_seq_id }) => {
                let request_id = request_id.ok_or_else(|| {
                    CcbdError::IpcInvalidRequest("duplicate request without request_id".into())
                })?;
                let existing =
                    query_event_by_request_id(&conn, agent_id, request_id)?.ok_or_else(|| {
                        CcbdError::DbConstraintViolation(format!(
                            "duplicate event seq_id={existing_seq_id} not found"
                        ))
                    })?;
                let payload: Value = serde_json::from_str(&existing.payload).map_err(|err| {
                    CcbdError::IpcInvalidRequest(format!(
                        "invalid command_received payload for seq_id={existing_seq_id}: {err}"
                    ))
                })?;
                let status = payload
                    .get("status")
                    .and_then(Value::as_str)
                    .unwrap_or("PENDING");
                return match status {
                    "SENT" | "PENDING" => Ok(json!({
                        "state": agent.state,
                        "seq_id": existing.seq_id,
                    })),
                    "FAILED" => Err(CcbdError::PtyIoError(format!(
                        "previous attempt seq_id={existing_seq_id} failed; retry with new request_id"
                    ))),
                    other => Err(CcbdError::IpcInvalidRequest(format!(
                        "unknown command_received status: {other}"
                    ))),
                };
            }
            Err(err) => return Err(err),
        }
    };

    let write_result = {
        let mut pty_map = crate::pty::PTY_MAP
            .lock()
            .map_err(|_| CcbdError::PtyIoError("PTY_MAP mutex poisoned".into()))?;
        match pty_map.get_mut(agent_id) {
            Some(writer) => writer
                .write_all(text.as_bytes())
                .and_then(|_| writer.flush())
                .map_err(|err| IoError::new(err.kind(), err.to_string())),
            None => Err(IoError::new(
                ErrorKind::BrokenPipe,
                "PTY writer disappeared between pre-check and write",
            )),
        }
    };

    let write_error = write_result
        .as_ref()
        .err()
        .map(|err| format!("{}: {err}", err.kind()));
    let final_status = if write_result.is_ok() {
        "SENT"
    } else {
        "FAILED"
    };
    let final_payload = json!({ "cmd": text, "status": final_status }).to_string();

    {
        let mut conn = ctx.db.conn();
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|err| CcbdError::DbConstraintViolation(format!("begin send update: {err}")))?;
        tx.execute(
            "UPDATE events SET payload = ? WHERE seq_id = ?",
            params![final_payload, seq_id],
        )
        .map_err(|err| CcbdError::DbConstraintViolation(format!("update send event: {err}")))?;
        if write_result.is_ok() {
            tx.execute(
                "UPDATE agents SET state = 'BUSY', state_version = state_version + 1, updated_at = unixepoch() WHERE id = ? AND state != 'CRASHED'",
                params![agent_id],
            )
            .map_err(|err| CcbdError::DbConstraintViolation(format!("update agent busy: {err}")))?;
        }
        tx.commit().map_err(|err| {
            CcbdError::DbConstraintViolation(format!("commit send update: {err}"))
        })?;
    }

    if let Some(write_error) = write_error {
        return Err(CcbdError::PtyIoError(write_error));
    }

    Ok(json!({ "state": "BUSY", "seq_id": seq_id }))
}

pub async fn handle_agent_read(params: Value, ctx: &Ctx) -> Result<Value, CcbdError> {
    let agent_id = required_str(&params, "agent_id")?;
    let since_event_id = required_i64(&params, "since_event_id")?;

    {
        let conn = ctx.db.conn();
        if query_agent(&conn, agent_id)?.is_none() {
            return Err(CcbdError::AgentNotFound(agent_id.to_string()));
        }
    }

    let events = {
        let conn = ctx.db.conn();
        query_events_since(&conn, agent_id, since_event_id)?
    };
    let events: Vec<Value> = events
        .into_iter()
        .map(|event| {
            json!({
                "seq_id": event.seq_id,
                "event_type": event.event_type,
                "payload": event.payload,
                "created_at": event.created_at,
            })
        })
        .collect();

    Ok(json!({ "events": events, "is_truncated": false }))
}

fn required_str<'a>(params: &'a Value, field: &str) -> Result<&'a str, CcbdError> {
    params
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| CcbdError::IpcInvalidRequest(format!("missing or invalid field '{field}'")))
}

fn required_i64(params: &Value, field: &str) -> Result<i64, CcbdError> {
    params
        .get(field)
        .and_then(Value::as_i64)
        .ok_or_else(|| CcbdError::IpcInvalidRequest(format!("missing or invalid field '{field}'")))
}

fn remove_pty_writer(agent_id: &str) {
    match crate::pty::PTY_MAP.lock() {
        Ok(mut pty_map) => {
            let _ = pty_map.remove(agent_id);
        }
        Err(err) => tracing::warn!(error = %err, "PTY_MAP mutex poisoned during cleanup"),
    }
}

#[cfg(test)]
mod tests {
    use super::{handle_agent_read, handle_agent_send, handle_agent_spawn, handle_session_create};
    use crate::db;
    use crate::db::queries::{insert_agent, insert_session};
    use crate::rpc::Ctx;
    use serde_json::json;
    use std::time::Duration;
    use uuid::Uuid;

    fn test_ctx() -> Ctx {
        let file = tempfile::NamedTempFile::new().unwrap();
        Ctx {
            db: db::init(file.path()).unwrap(),
        }
    }

    async fn sleep_ms(ms: u64) {
        tokio::task::spawn_blocking(move || std::thread::sleep(Duration::from_millis(ms)))
            .await
            .unwrap();
    }

    fn command_count(ctx: &Ctx, agent_id: &str) -> i64 {
        let conn = ctx.db.conn();
        conn.query_row(
            "SELECT COUNT(*) FROM events WHERE agent_id = ? AND event_type = 'command_received'",
            [agent_id],
            |row| row.get(0),
        )
        .unwrap()
    }

    async fn cleanup_agent(ctx: &Ctx, agent_id: &str) {
        let _ = handle_agent_send(
            json!({
                "agent_id": agent_id,
                "text": "exit\n",
            }),
            ctx,
        )
        .await;
        sleep_ms(300).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_handle_session_create_returns_session_id() {
        let ctx = test_ctx();
        let result = handle_session_create(
            json!({
                "project_id": "p1",
                "absolute_path": "/tmp/t4-session",
                "master_pid": std::process::id(),
            }),
            &ctx,
        )
        .await
        .unwrap();

        assert!(result["session_id"].as_str().unwrap().starts_with("sess_"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_handle_agent_spawn_returns_idle_and_inserts_pid() {
        let ctx = test_ctx();
        {
            let conn = ctx.db.conn();
            insert_session(&conn, "s1", "p1", "/tmp/t4-spawn", 999).unwrap();
        }
        let agent_id = format!("ag_spawn_{}", Uuid::new_v4());
        let result = handle_agent_spawn(
            json!({
                "session_id": "s1",
                "agent_id": agent_id,
                "provider": "bash",
            }),
            &ctx,
        )
        .await
        .unwrap();

        let pid: Option<i64> = {
            let conn = ctx.db.conn();
            conn.query_row(
                "SELECT pid FROM agents WHERE id = ?",
                [agent_id.as_str()],
                |row| row.get(0),
            )
            .unwrap()
        };

        assert_eq!(result["state"], "IDLE");
        assert!(pid.is_some());
        cleanup_agent(&ctx, &agent_id).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_handle_agent_send_idempotent() {
        let ctx = test_ctx();
        {
            let conn = ctx.db.conn();
            insert_session(&conn, "s1", "p1", "/tmp/t4-send", 999).unwrap();
        }
        let agent_id = format!("ag_send_{}", Uuid::new_v4());
        handle_agent_spawn(
            json!({
                "session_id": "s1",
                "agent_id": agent_id,
                "provider": "bash",
            }),
            &ctx,
        )
        .await
        .unwrap();

        let first = handle_agent_send(
            json!({
                "agent_id": agent_id,
                "text": "echo first\n",
                "request_id": "A",
            }),
            &ctx,
        )
        .await
        .unwrap();
        let second = handle_agent_send(
            json!({
                "agent_id": agent_id,
                "text": "echo second\n",
                "request_id": "A",
            }),
            &ctx,
        )
        .await
        .unwrap();

        assert_eq!(first["seq_id"], second["seq_id"]);
        assert_eq!(command_count(&ctx, &agent_id), 1);
        cleanup_agent(&ctx, &agent_id).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_handle_agent_read_streams_output_chunk() {
        let ctx = test_ctx();
        {
            let conn = ctx.db.conn();
            insert_session(&conn, "s1", "p1", "/tmp/t4-read", 999).unwrap();
        }
        let agent_id = format!("ag_read_{}", Uuid::new_v4());
        handle_agent_spawn(
            json!({
                "session_id": "s1",
                "agent_id": agent_id,
                "provider": "bash",
            }),
            &ctx,
        )
        .await
        .unwrap();
        handle_agent_send(
            json!({
                "agent_id": agent_id,
                "text": "echo handler_read\n",
                "request_id": "read-1",
            }),
            &ctx,
        )
        .await
        .unwrap();
        sleep_ms(500).await;

        let result = handle_agent_read(
            json!({
                "agent_id": agent_id,
                "since_event_id": 0,
            }),
            &ctx,
        )
        .await
        .unwrap();
        let events = result["events"].as_array().unwrap();

        assert!(
            events.iter().any(|event| event["payload"]
                .as_str()
                .unwrap_or("")
                .contains("handler_read")),
            "events={events:?}"
        );
        cleanup_agent(&ctx, &agent_id).await;
    }

    #[test]
    fn test_agent_read_allows_crashed_state() {
        let ctx = test_ctx();
        {
            let conn = ctx.db.conn();
            insert_session(&conn, "s1", "p1", "/tmp/t4-crashed-read", 999).unwrap();
            insert_agent(&conn, "ag_crashed", "s1", "bash", "CRASHED", None).unwrap();
        }

        let result = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(handle_agent_read(
                json!({
                    "agent_id": "ag_crashed",
                    "since_event_id": 0,
                }),
                &ctx,
            ))
            .unwrap();

        assert_eq!(result["is_truncated"], false);
        assert_eq!(result["events"].as_array().unwrap().len(), 0);
    }
}
