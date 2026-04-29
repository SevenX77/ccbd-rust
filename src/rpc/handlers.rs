use crate::db::agents::{agent_exists, delete_agent, insert_agent, query_agent, query_agent_state};
use crate::db::agents_lifecycle::mark_agent_killed;
use crate::db::events::{insert_event, query_event_by_request_id, query_events_since};
use crate::db::events_progress::record_send_progress;
use crate::db::evidence::discard_evidence;
use crate::db::sessions::{create_session, session_exists};
use crate::db::state_machine_assert::assert_state_to_idle;
use crate::db::system::system_dump;
use crate::error::CcbdError;
use crate::marker::{
    MarkerMatcher, MatchResult, TimerKind, parser_registry, registry, spawn_marker_timer_task,
};
use crate::monitor;
use crate::monitor::agent_watch::spawn_agent_pidfd_watch_task;
use crate::monitor::master_watch::spawn_master_pidfd_watch_task;
use crate::rpc::Ctx;
use crate::sandbox::{bwrap, path, systemd};
use crate::tmux::SESSION_NAME;
use nix::sys::stat::Mode;
use serde_json::{Value, json};
use std::fs;
use std::os::unix::fs::OpenOptionsExt;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use uuid::Uuid;

pub async fn handle_session_create(params: Value, ctx: &Ctx) -> Result<Value, CcbdError> {
    let project_id = required_str(&params, "project_id")?;
    let absolute_path = required_str(&params, "absolute_path")?;
    let master_pid = required_i64(&params, "master_pid")?;
    let session_id = format!("sess_{}", Uuid::new_v4());

    let master_pidfd = create_session(
        ctx.db.clone(),
        session_id.clone(),
        project_id.to_string(),
        absolute_path.to_string(),
        master_pid as i32,
    )
    .await?;

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
    let overrides = match params.get("sandbox_overrides") {
        Some(value) => serde_json::from_value(value.clone()).map_err(|err| {
            CcbdError::IpcInvalidRequest(format!("invalid sandbox_overrides: {err}"))
        })?,
        None => bwrap::SandboxOverrides::default(),
    };

    if agent_exists(ctx.db.clone(), agent_id.to_string()).await? {
        return Err(CcbdError::AgentAlreadyExists(agent_id.to_string()));
    }

    if !session_exists(ctx.db.clone(), session_id.to_string()).await? {
        return Err(CcbdError::DbConstraintViolation(format!(
            "session not found: {session_id}"
        )));
    }

    let sandbox_dir = if ctx.env_state.unsafe_no_sandbox {
        None
    } else {
        Some(path::resolve_sandbox_dir(&ctx.state_dir, agent_id)?)
    };
    let bwrap_args = match &sandbox_dir {
        Some(dir) => match bwrap::build_args(dir, &overrides) {
            Ok(args) => args,
            Err(err) => {
                cleanup_sandbox_dir(sandbox_dir.as_deref());
                return Err(err);
            }
        },
        None => Vec::new(),
    };
    let cmd = systemd::wrap_command(agent_id, &ctx.env_state, &bwrap_args, provider);
    let session_dir = sandbox_dir.clone().unwrap_or_else(|| ctx.state_dir.clone());
    let fifo_dir = ctx.state_dir.join("pipes");
    if let Err(err) = fs::create_dir_all(&fifo_dir) {
        cleanup_sandbox_dir(sandbox_dir.as_deref());
        return Err(CcbdError::EnvironmentNotSupported {
            details: format!("create fifo dir {}: {err}", fifo_dir.display()),
        });
    }
    let fifo_path = fifo_dir.join(format!("{agent_id}.fifo"));
    let _ = fs::remove_file(&fifo_path);
    crate::db::common::spawn_db("agent_io::mkfifo", {
        let fifo_path = fifo_path.clone();
        move || {
            nix::unistd::mkfifo(&fifo_path, Mode::S_IRUSR | Mode::S_IWUSR).map_err(|err| {
                CcbdError::TmuxCommandFailed {
                    cmd: format!("mkfifo {}", fifo_path.display()),
                    stderr: err.to_string(),
                    exit: -1,
                }
            })
        }
    })
    .await?;
    let fifo_file = match fs::OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(&fifo_path)
    {
        Ok(file) => file,
        Err(err) => {
            cleanup_sandbox_dir(sandbox_dir.as_deref());
            let _ = fs::remove_file(&fifo_path);
            return Err(CcbdError::EnvironmentNotSupported {
                details: format!("open fifo {}: {err}", fifo_path.display()),
            });
        }
    };

    let tmux = ctx.tmux_server.clone();
    if let Err(err) = tmux
        .ensure_session(SESSION_NAME.to_string(), session_dir.clone())
        .await
    {
        cleanup_sandbox_dir(sandbox_dir.as_deref());
        drop(fifo_file);
        let _ = fs::remove_file(&fifo_path);
        return Err(err);
    }
    let pane_id = match tmux
        .spawn_window(
            SESSION_NAME.to_string(),
            agent_id.to_string(),
            session_dir,
            cmd,
        )
        .await
    {
        Ok(pane_id) => pane_id,
        Err(err) => {
            cleanup_sandbox_dir(sandbox_dir.as_deref());
            drop(fifo_file);
            let _ = fs::remove_file(&fifo_path);
            return Err(err);
        }
    };
    let pid = match tmux.get_pane_pid(pane_id.clone()).await {
        Ok(pid) => pid,
        Err(err) => {
            cleanup_spawn_resources(&tmux, Some(pane_id.clone()), Some(fifo_file), &fifo_path).await;
            cleanup_sandbox_dir(sandbox_dir.as_deref());
            return Err(err);
        }
    };
    if let Err(err) = tmux
        .pipe_pane_to_fifo(pane_id.clone(), fifo_path.clone())
        .await
    {
        cleanup_spawn_resources(&tmux, Some(pane_id.clone()), Some(fifo_file), &fifo_path).await;
        cleanup_sandbox_dir(sandbox_dir.as_deref());
        return Err(err);
    }

    let pidfd = match monitor::pidfd_open(pid) {
        Ok(fd) => fd,
        Err(err) => {
            cleanup_spawn_resources(&tmux, Some(pane_id.clone()), Some(fifo_file), &fifo_path).await;
            cleanup_sandbox_dir(sandbox_dir.as_deref());
            return Err(err);
        }
    };

    if let Err(err) = insert_agent(
        ctx.db.clone(),
        agent_id.to_string(),
        session_id.to_string(),
        provider.to_string(),
        "SPAWNING".to_string(),
        Some(pid as i64),
    )
    .await
    {
        cleanup_spawn_resources(&tmux, Some(pane_id.clone()), Some(fifo_file), &fifo_path).await;
        cleanup_sandbox_dir(sandbox_dir.as_deref());
        return Err(err);
    }

    let pidfd_for_task = match pidfd.try_clone() {
        Ok(fd) => fd,
        Err(err) => {
            cleanup_spawn_resources(&tmux, Some(pane_id.clone()), Some(fifo_file), &fifo_path).await;
            cleanup_sandbox_dir(sandbox_dir.as_deref());
            let _ = delete_agent(ctx.db.clone(), agent_id.to_string()).await;
            return Err(CcbdError::EnvironmentNotSupported {
                details: format!("clone agent pidfd for {agent_id}: {err}"),
            });
        }
    };

    monitor::register(agent_id.to_string(), pidfd);
    let parser_handle = Arc::new(Mutex::new(vt100::Parser::new(200, 200, 0)));
    parser_registry::register(agent_id.to_string(), parser_handle.clone());
    let marker_handle = spawn_marker_timer_task(
        agent_id.to_string(),
        TimerKind::Startup,
        Arc::new(ctx.db.clone()),
        parser_handle.clone(),
    );
    registry::register(agent_id.to_string(), marker_handle);
    seed_parser_from_tmux_capture(ctx, agent_id, &pane_id, parser_handle.clone()).await;
    let reader_handle = crate::agent_io::spawn_agent_io_reader_task(
        agent_id.to_string(),
        fifo_file,
        ctx.db.clone(),
        parser_handle,
    );
    crate::agent_io::register(
        agent_id.to_string(),
        crate::agent_io::AgentIoEntry {
            pane_id,
            reader_handle,
            fifo_path,
        },
    );
    spawn_agent_pidfd_watch_task(
        agent_id.to_string(),
        pidfd_for_task,
        Arc::new(ctx.db.clone()),
    );

    Ok(json!({ "state": "SPAWNING", "pid": pid }))
}

pub async fn handle_agent_kill(params: Value, ctx: &Ctx) -> Result<Value, CcbdError> {
    let agent_id = required_str(&params, "agent_id")?;
    let changes = mark_agent_killed(
        ctx.db.clone(),
        agent_id.to_string(),
        "SIGKILL_BY_DAEMON".to_string(),
    )
    .await?;
    if changes == 0 {
        return Err(CcbdError::AgentNotFound(agent_id.to_string()));
    }

    if let Some(handle) = registry::take(agent_id) {
        let _ = handle.cancel_tx.send(());
    }
    let _ = parser_registry::remove(agent_id);

    match monitor::with_borrowed(agent_id, monitor::pidfd_send_sigkill) {
        Some(Ok(())) => {}
        Some(Err(err)) => {
            tracing::error!(agent_id, error = %err, "failed to SIGKILL agent via pidfd")
        }
        None => tracing::warn!(agent_id, "agent.kill found no pidfd in registry"),
    }
    let _ = crate::agent_io::shutdown_reader(agent_id).await;
    let _ = monitor::remove(agent_id);

    Ok(json!({ "state": "KILLED" }))
}

pub async fn handle_agent_send(params: Value, ctx: &Ctx) -> Result<Value, CcbdError> {
    let agent_id = required_str(&params, "agent_id")?;
    let text = required_str(&params, "text")?;
    let request_id = params.get("request_id").and_then(Value::as_str);

    if let Some(request_id) = request_id {
        let existing =
            query_event_by_request_id(ctx.db.clone(), agent_id.to_string(), request_id.to_string())
                .await?;
        if let Some(existing) = existing {
            let payload: Value = serde_json::from_str(&existing.payload).map_err(|err| {
                CcbdError::IpcInvalidRequest(format!(
                    "invalid command_received payload for seq_id={}: {err}",
                    existing.seq_id
                ))
            })?;
            let status = payload
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or("PENDING");
            let state = query_agent_state(ctx.db.clone(), agent_id.to_string())
                .await?
                .map(|(state, _)| state)
                .ok_or_else(|| CcbdError::AgentNotFound(agent_id.to_string()))?;
            return match status {
                "SENT" | "PENDING" => Ok(json!({
                    "state": state,
                    "seq_id": existing.seq_id,
                })),
                "FAILED" => Err(CcbdError::PtyIoError(format!(
                    "previous attempt seq_id={} failed; retry with new request_id",
                    existing.seq_id
                ))),
                other => Err(CcbdError::IpcInvalidRequest(format!(
                    "unknown command_received status: {other}"
                ))),
            };
        }
    }

    let (state, _) = query_agent_state(ctx.db.clone(), agent_id.to_string())
        .await?
        .ok_or_else(|| CcbdError::AgentNotFound(agent_id.to_string()))?;
    if state != "IDLE" && state != "UNKNOWN" {
        return Err(CcbdError::AgentWrongState {
            current_state: state,
        });
    }

    let pane_id = if let Some(pane_id) = crate::agent_io::pane_id(agent_id) {
        pane_id
    } else {
        return Err(CcbdError::PtyIoError(format!(
            "tmux pane not registered for {agent_id}"
        )));
    };

    let pending_payload = json!({ "cmd": text, "status": "PENDING" }).to_string();
    let seq_id = {
        match insert_event(
            ctx.db.clone(),
            agent_id.to_string(),
            request_id.map(str::to_string),
            "command_received".to_string(),
            pending_payload.clone(),
        )
        .await
        {
            Ok(seq_id) => seq_id,
            Err(CcbdError::DuplicateRequest { existing_seq_id }) => {
                let request_id = request_id.ok_or_else(|| {
                    CcbdError::IpcInvalidRequest("duplicate request without request_id".into())
                })?;
                let existing = query_event_by_request_id(
                    ctx.db.clone(),
                    agent_id.to_string(),
                    request_id.to_string(),
                )
                .await?
                .ok_or_else(|| {
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
                        "state": state,
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

    let write_result =
        crate::agent_io::send_text_to_pane(ctx.tmux_server.clone(), pane_id, text.to_string())
            .await;

    let write_error = write_result
        .as_ref()
        .err()
        .map(|err| err.to_string());
    let final_status = if write_result.is_ok() {
        "SENT"
    } else {
        "FAILED"
    };
    let final_payload = json!({ "cmd": text, "status": final_status });
    record_send_progress(
        ctx.db.clone(),
        seq_id,
        final_payload,
        agent_id.to_string(),
        write_result.is_ok(),
    )
    .await?;

    if let Some(write_error) = write_error {
        return Err(CcbdError::PtyIoError(write_error));
    }

    let parser_handle = parser_registry::get(agent_id).ok_or_else(|| {
        CcbdError::PtyIoError(format!("parser not in PARSER_REGISTRY for {agent_id}"))
    })?;
    let marker_handle = spawn_marker_timer_task(
        agent_id.to_string(),
        TimerKind::Busy,
        Arc::new(ctx.db.clone()),
        parser_handle,
    );
    registry::register(agent_id.to_string(), marker_handle);
    spawn_delayed_capture_seed(
        ctx.db.clone(),
        ctx.tmux_server.clone(),
        agent_id.to_string(),
        Duration::from_millis(100),
    );

    Ok(json!({ "state": "BUSY", "seq_id": seq_id }))
}

pub async fn handle_agent_read(params: Value, ctx: &Ctx) -> Result<Value, CcbdError> {
    let agent_id = required_str(&params, "agent_id")?;
    let since_event_id = required_i64(&params, "since_event_id")?;

    if query_agent(ctx.db.clone(), agent_id.to_string())
        .await?
        .is_none()
    {
        return Err(CcbdError::AgentNotFound(agent_id.to_string()));
    }

    let events = query_events_since(ctx.db.clone(), agent_id.to_string(), since_event_id).await?;
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

pub async fn handle_agent_assert_state(params: Value, ctx: &Ctx) -> Result<Value, CcbdError> {
    let agent_id = required_str(&params, "agent_id")?;
    let state = required_str(&params, "state")?;
    let evidence_id = required_str(&params, "evidence_id")?;
    if state != "IDLE" {
        return Err(CcbdError::IpcInvalidRequest(
            "assert_state only accepts state=IDLE".into(),
        ));
    }

    let _outcome = assert_state_to_idle(
        ctx.db.clone(),
        agent_id.to_string(),
        evidence_id.to_string(),
    )
    .await?;

    Ok(json!({ "state": "IDLE", "sub_state": "Asserted" }))
}

pub async fn handle_agent_discard_evidence(params: Value, ctx: &Ctx) -> Result<Value, CcbdError> {
    let evidence_id = required_str(&params, "evidence_id")?;
    discard_evidence(ctx.db.clone(), evidence_id.to_string()).await?;

    Ok(json!({ "status": "DISCARDED" }))
}

pub async fn handle_system_dump(_params: Value, ctx: &Ctx) -> Result<Value, CcbdError> {
    system_dump(ctx.db.clone()).await
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

fn cleanup_sandbox_dir(sandbox_dir: Option<&std::path::Path>) {
    if let Some(sandbox_dir) = sandbox_dir
        && let Err(err) = fs::remove_dir_all(sandbox_dir)
        && err.kind() != std::io::ErrorKind::NotFound
    {
        tracing::error!(?sandbox_dir, error = %err, "failed to remove sandbox dir during rollback");
    }
}

async fn cleanup_spawn_resources(
    tmux: &crate::tmux::TmuxServer,
    pane_id: Option<crate::tmux::TmuxPaneId>,
    fifo_file: Option<std::fs::File>,
    fifo_path: &std::path::Path,
) {
    if let Some(pane_id) = pane_id {
        let _ = tmux.kill_pane(pane_id).await;
    }
    drop(fifo_file);
    let _ = fs::remove_file(fifo_path);
}

async fn seed_parser_from_tmux_capture(
    ctx: &Ctx,
    agent_id: &str,
    pane_id: &crate::tmux::TmuxPaneId,
    parser_handle: Arc<Mutex<vt100::Parser>>,
) {
    let Ok(capture) = ctx.tmux_server.capture_pane(pane_id.clone()).await else {
        return;
    };
    if capture.is_empty() {
        return;
    }
    let matched = {
        match parser_handle.lock() {
            Ok(mut parser) => {
                parser.process(capture.as_bytes());
                MarkerMatcher::default().scan(&parser)
            }
            Err(err) => {
                tracing::warn!(agent_id, error = %err, "parser mutex poisoned during tmux capture seed");
                MatchResult::NoMatch
            }
        }
    };
    if matched == MatchResult::Matched {
        if let Err(err) =
            crate::db::state_machine::mark_agent_idle_matched(ctx.db.clone(), agent_id.to_string())
                .await
        {
            tracing::warn!(agent_id, error = %err, "failed to mark agent IDLE from tmux capture seed");
        }
        if let Some(handle) = registry::take(agent_id) {
            let _ = handle.cancel_tx.send(());
        }
    }
}

fn spawn_delayed_capture_seed(
    db: crate::db::Db,
    tmux: Arc<crate::tmux::TmuxServer>,
    agent_id: String,
    delay: Duration,
) {
    tokio::spawn(async move {
        tokio::time::sleep(delay).await;
        let Some(pane_id) = crate::agent_io::pane_id(&agent_id) else {
            return;
        };
        let Some(parser_handle) = parser_registry::get(&agent_id) else {
            return;
        };
        let Ok(capture) = tmux.capture_pane(pane_id).await else {
            return;
        };
        let matched = {
            match parser_handle.lock() {
                Ok(mut parser) => {
                    parser.process(capture.as_bytes());
                    MarkerMatcher::default().scan(&parser)
                }
                Err(err) => {
                    tracing::warn!(agent_id = %agent_id, error = %err, "parser mutex poisoned during delayed capture seed");
                    MatchResult::NoMatch
                }
            }
        };
        if matched == MatchResult::Matched {
            if let Err(err) =
                crate::db::state_machine::mark_agent_idle_matched(db, agent_id.clone()).await
            {
                tracing::warn!(agent_id = %agent_id, error = %err, "failed to mark agent IDLE from delayed tmux capture seed");
            }
            if let Some(handle) = registry::take(&agent_id) {
                let _ = handle.cancel_tx.send(());
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::{
        handle_agent_assert_state, handle_agent_discard_evidence, handle_agent_kill,
        handle_agent_read, handle_agent_send, handle_agent_spawn, handle_session_create,
        handle_system_dump,
    };
    use crate::db;
    use crate::db::agents::{insert_agent_sync, query_agent_state_sync, update_agent_state_sync};
    use crate::db::sessions::insert_session_sync;
    use crate::db::state_machine::mark_agent_unknown_sync;
    use crate::error::CcbdError;
    use crate::marker::parser_registry;
    use crate::rpc::Ctx;
    use crate::sandbox::EnvState;
    use crate::tmux::TmuxServer;
    use serde_json::json;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;
    use uuid::Uuid;

    fn test_ctx() -> Ctx {
        let file = tempfile::NamedTempFile::new().unwrap();
        let state_dir = tempfile::TempDir::new().unwrap().keep();
        Ctx {
            db: db::init(file.path()).unwrap(),
            state_dir: state_dir.clone(),
            env_state: EnvState {
                bwrap_available: false,
                systemd_run_available: false,
                unsafe_no_sandbox: true,
            },
            tmux_server: Arc::new(TmuxServer::new(&state_dir)),
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

    async fn wait_for_state(ctx: &Ctx, agent_id: &str, expected: &str) {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while std::time::Instant::now() < deadline {
            let state = query_agent_state_sync(&ctx.db, agent_id)
                .unwrap()
                .map(|(state, _)| state);
            if state.as_deref() == Some(expected) {
                return;
            }
            sleep_ms(50).await;
        }
        panic!("agent {agent_id} did not reach {expected} within 5s");
    }

    async fn cleanup_agent(ctx: &Ctx, agent_id: &str) {
        let _ = handle_agent_kill(json!({ "agent_id": agent_id }), ctx).await;
        sleep_ms(300).await;
    }

    fn seed_unknown_with_evidence(ctx: &Ctx, agent_id: &str) -> String {
        {
            let conn = ctx.db.conn();
            insert_session_sync(&conn, "s_evidence", "p_evidence", "/tmp/t4-evidence", 999)
                .unwrap();
            insert_agent_sync(&conn, agent_id, "s_evidence", "bash", "BUSY", Some(1)).unwrap();
        }
        mark_agent_unknown_sync(
            &ctx.db,
            agent_id,
            "PTY_MARKER_TIMEOUT",
            b"pane".to_vec(),
            serde_json::json!(["rule"]),
        )
        .unwrap();
        ctx.db
            .conn()
            .query_row(
                "SELECT id FROM evidence WHERE agent_id = ?",
                [agent_id],
                |row| row.get(0),
            )
            .unwrap()
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
            insert_session_sync(&conn, "s1", "p1", "/tmp/t4-spawn", 999).unwrap();
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

        assert_eq!(result["state"], "SPAWNING");
        assert!(result["pid"].as_i64().unwrap() > 0);
        assert!(pid.is_some());
        wait_for_state(&ctx, &agent_id, "IDLE").await;
        cleanup_agent(&ctx, &agent_id).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_handle_agent_send_idempotent() {
        let ctx = test_ctx();
        {
            let conn = ctx.db.conn();
            insert_session_sync(&conn, "s1", "p1", "/tmp/t4-send", 999).unwrap();
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
        wait_for_state(&ctx, &agent_id, "IDLE").await;

        let first = handle_agent_send(
            json!({
                "agent_id": agent_id,
                "text": "sleep 1; echo first\n",
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
            insert_session_sync(&conn, "s1", "p1", "/tmp/t4-read", 999).unwrap();
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
        wait_for_state(&ctx, &agent_id, "IDLE").await;
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
            insert_session_sync(&conn, "s1", "p1", "/tmp/t4-crashed-read", 999).unwrap();
            insert_agent_sync(&conn, "ag_crashed", "s1", "bash", "CRASHED", None).unwrap();
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

    #[tokio::test(flavor = "multi_thread")]
    async fn test_handle_agent_kill_marks_killed_and_rejects_repeat() {
        let ctx = test_ctx();
        {
            let conn = ctx.db.conn();
            insert_session_sync(&conn, "s1", "p1", "/tmp/t4-kill", 999).unwrap();
        }
        let agent_id = format!("ag_kill_{}", Uuid::new_v4());
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
        wait_for_state(&ctx, &agent_id, "IDLE").await;

        let result = handle_agent_kill(json!({ "agent_id": agent_id }), &ctx)
            .await
            .unwrap();
        let repeat = handle_agent_kill(json!({ "agent_id": agent_id }), &ctx)
            .await
            .unwrap_err();

        let (state, payload): (String, String) = ctx
            .db
            .conn()
            .query_row(
                "SELECT agents.state, events.payload FROM agents JOIN events ON events.agent_id = agents.id WHERE agents.id = ? AND events.event_type = 'state_change' AND events.payload LIKE '%SIGKILL_BY_DAEMON%'",
                [agent_id.as_str()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        let payload: serde_json::Value = serde_json::from_str(&payload).unwrap();

        assert_eq!(result["state"], "KILLED");
        assert!(matches!(repeat, crate::error::CcbdError::AgentNotFound(_)));
        assert_eq!(state, "KILLED");
        assert_eq!(payload["to"], "KILLED");
        assert_eq!(payload["reason"], "SIGKILL_BY_DAEMON");
        assert!(!crate::monitor::contains(&agent_id));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_handle_agent_assert_state_reviews_evidence() {
        let ctx = test_ctx();
        let agent_id = format!("ag_assert_{}", Uuid::new_v4());
        let evidence_id = seed_unknown_with_evidence(&ctx, &agent_id);

        let result = handle_agent_assert_state(
            json!({
                "agent_id": agent_id,
                "state": "IDLE",
                "evidence_id": evidence_id,
            }),
            &ctx,
        )
        .await
        .unwrap();
        let (state, sub_state, evidence_status, asserted): (
            String,
            Option<String>,
            String,
            Option<String>,
        ) = ctx
            .db
            .conn()
            .query_row(
                "SELECT agents.state, agents.sub_state, evidence.status, evidence.l3_asserted_state FROM agents JOIN evidence ON evidence.agent_id = agents.id WHERE agents.id = ?",
                [agent_id.as_str()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();

        assert_eq!(result["state"], "IDLE");
        assert_eq!(result["sub_state"], "Asserted");
        assert_eq!(state, "IDLE");
        assert_eq!(sub_state.as_deref(), Some("Asserted"));
        assert_eq!(evidence_status, "REVIEWED");
        assert_eq!(asserted.as_deref(), Some("IDLE"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_handle_agent_assert_state_rejects_wrong_agent() {
        let ctx = test_ctx();
        let evidence_id = seed_unknown_with_evidence(&ctx, "ag_owner");
        {
            let conn = ctx.db.conn();
            insert_agent_sync(&conn, "ag_other", "s_evidence", "bash", "UNKNOWN", Some(2)).unwrap();
        }

        let err = handle_agent_assert_state(
            json!({
                "agent_id": "ag_other",
                "state": "IDLE",
                "evidence_id": evidence_id,
            }),
            &ctx,
        )
        .await
        .unwrap_err();

        assert!(matches!(err, CcbdError::DbEvidenceNotFound { .. }));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_handle_agent_discard_evidence_is_idempotent() {
        let ctx = test_ctx();
        let evidence_id = seed_unknown_with_evidence(&ctx, "ag_discard");

        let first = handle_agent_discard_evidence(json!({ "evidence_id": evidence_id }), &ctx)
            .await
            .unwrap();
        let second = handle_agent_discard_evidence(json!({ "evidence_id": evidence_id }), &ctx)
            .await
            .unwrap();

        assert_eq!(first["status"], "DISCARDED");
        assert_eq!(second["status"], "DISCARDED");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_handle_system_dump_excludes_pane_bytes() {
        let ctx = test_ctx();
        seed_unknown_with_evidence(&ctx, "ag_dump");

        let dump = handle_system_dump(json!({}), &ctx).await.unwrap();

        assert_eq!(dump["projects"].as_array().unwrap().len(), 1);
        assert_eq!(dump["sessions"].as_array().unwrap().len(), 1);
        assert_eq!(dump["agents"].as_array().unwrap().len(), 1);
        assert_eq!(dump["evidence_pending"].as_array().unwrap().len(), 1);
        assert!(!dump.to_string().contains("pane_bytes"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_handle_agent_send_allows_unknown_with_new_request() {
        let ctx = test_ctx();
        let agent_id = format!("ag_unknown_send_{}", Uuid::new_v4());
        {
            let conn = ctx.db.conn();
            insert_session_sync(&conn, "s_unknown", "p_unknown", "/tmp/t4-unknown", 999).unwrap();
        }
        let _ = handle_agent_spawn(
            json!({
                "session_id": "s_unknown",
                "agent_id": agent_id,
                "provider": "bash",
            }),
            &ctx,
        )
        .await
        .unwrap();
        wait_for_state(&ctx, &agent_id, "IDLE").await;
        {
            let conn = ctx.db.conn();
            update_agent_state_sync(&conn, &agent_id, "UNKNOWN").unwrap();
        }

        let result = handle_agent_send(
            json!({
                "agent_id": agent_id,
                "text": "echo recover\n",
                "request_id": "recover-1",
            }),
            &ctx,
        )
        .await
        .unwrap();

        assert_eq!(result["state"], "BUSY");
        assert_eq!(command_count(&ctx, &agent_id), 1);
        let _ = handle_agent_kill(json!({ "agent_id": agent_id }), &ctx).await;
        let _ = crate::marker::registry::take(&agent_id);
    }
}
