use crate::db::Db;
use crate::db::agents::{agent_exists, delete_agent, insert_agent, query_agent, query_agent_state};
use crate::db::agents_lifecycle::mark_agent_killed;
use crate::db::events::{insert_event, query_event_by_request_id, query_events_since};
use crate::db::events_progress::record_send_progress;
use crate::db::evidence::discard_evidence;
use crate::db::jobs::{
    insert_job, mark_dispatched_job_cancelled_if_agent_idle, mark_queued_job_cancelled, query_job,
    request_dispatched_job_cancel,
};
use crate::db::sessions::list_session_summaries;
use crate::db::sessions::{create_session, query_session_by_id};
use crate::db::state_machine_assert::assert_state_to_idle;
use crate::db::system::{cascade_kill_session_agents, session_agent_ids, system_dump};
use crate::error::CcbdError;
use crate::marker::{
    MarkerMatcher, MatchResult, TimerKind, parser_registry, registry, spawn_marker_timer_task,
};
use crate::monitor;
use crate::monitor::agent_watch::spawn_agent_pidfd_watch_task;
use crate::rpc::Ctx;
use crate::sandbox::{bwrap, path, systemd};
use crate::tmux::SESSION_NAME;
use nix::sys::stat::Mode;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::fs;
use std::os::unix::fs::OpenOptionsExt;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;
use tokio::sync::Mutex as AsyncMutex;
use uuid::Uuid;

static SESSION_WINDOW_LOCKS: LazyLock<Mutex<HashMap<String, Arc<AsyncMutex<()>>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn session_window_lock(session_id: &str) -> Arc<AsyncMutex<()>> {
    let mut locks = SESSION_WINDOW_LOCKS
        .lock()
        .expect("session window locks mutex poisoned");
    locks
        .entry(session_id.to_string())
        .or_insert_with(|| Arc::new(AsyncMutex::new(())))
        .clone()
}

fn session_window_target(project_id: &str) -> String {
    format!("{}:{project_id}", SESSION_NAME)
}

pub async fn handle_session_create(params: Value, ctx: &Ctx) -> Result<Value, CcbdError> {
    let project_id = required_str(&params, "project_id")?;
    let absolute_path = required_str(&params, "absolute_path")?;
    let session_id = format!("sess_{}", Uuid::new_v4());

    create_session(
        ctx.db.clone(),
        session_id.clone(),
        project_id.to_string(),
        absolute_path.to_string(),
    )
    .await?;

    Ok(json!({ "session_id": session_id }))
}

pub async fn handle_session_kill(params: Value, ctx: &Ctx) -> Result<Value, CcbdError> {
    let session_id = required_str(&params, "session_id")?;
    let force = params
        .get("force")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let session = query_session_by_id(ctx.db.clone(), session_id.to_string())
        .await?
        .ok_or_else(|| CcbdError::IpcInvalidRequest(format!("session not found: {session_id}")))?;
    let agent_ids = session_agent_ids(ctx.db.clone(), session_id.to_string()).await?;

    let killed = cascade_kill_session_agents(
        ctx.db.clone(),
        session_id.to_string(),
        "SESSION_KILL".to_string(),
    )
    .await?;

    if force {
        for agent_id in &agent_ids {
            if let Some(pane_id) = crate::agent_io::pane_id(agent_id) {
                let _ = ctx.tmux_server.kill_pane(pane_id).await;
            }
            let _ = ctx
                .tmux_server
                .kill_window(SESSION_NAME.to_string(), agent_id.clone())
                .await;
        }
        let _ = ctx
            .tmux_server
            .kill_window(SESSION_NAME.to_string(), session.project_id.clone())
            .await;
        let _ = ctx
            .tmux_server
            .kill_session_window(session.id.clone())
            .await;
    }
    Ok(json!({
        "session_id": session_id,
        "state": "KILLED",
        "killed_agents": killed,
    }))
}

pub async fn handle_session_apply_layout(params: Value, ctx: &Ctx) -> Result<Value, CcbdError> {
    let session_id = required_str(&params, "session_id")?;
    let layout = required_str(&params, "layout")?;
    let layout = crate::tmux::LayoutKind::parse(layout)?;
    let session = query_session_by_id(ctx.db.clone(), session_id.to_string())
        .await?
        .ok_or_else(|| CcbdError::IpcInvalidRequest(format!("session not found: {session_id}")))?;
    let target = session_window_target(&session.id);
    crate::tmux::layout::apply_layout(&ctx.tmux_server, target, layout).await?;
    Ok(json!({
        "status": "ok",
        "layout": layout.as_str(),
    }))
}

pub async fn handle_session_list(_params: Value, ctx: &Ctx) -> Result<Value, CcbdError> {
    let sessions = list_session_summaries(ctx.db.clone()).await?;
    let sessions = sessions
        .into_iter()
        .map(|session| {
            json!({
                "id": session.id,
                "project_id": session.project_id,
                "absolute_path": session.absolute_path,
                "active_agents": session.active_agents,
                "created_at": session.created_at,
            })
        })
        .collect::<Vec<_>>();
    Ok(json!({ "sessions": sessions }))
}

pub async fn handle_agent_spawn(params: Value, ctx: &Ctx) -> Result<Value, CcbdError> {
    let session_id = required_str(&params, "session_id")?;
    let agent_id = required_str(&params, "agent_id")?;
    let provider = required_str(&params, "provider")?;
    let manifest = crate::provider::manifest::get_manifest(provider);
    let extra_env_vars = match params.get("extra_env_vars") {
        Some(value) => {
            serde_json::from_value::<HashMap<String, String>>(value.clone()).map_err(|err| {
                CcbdError::IpcInvalidRequest(format!("invalid extra_env_vars: {err}"))
            })?
        }
        None => HashMap::new(),
    };
    let overrides = match params.get("sandbox_overrides") {
        Some(value) => serde_json::from_value(value.clone()).map_err(|err| {
            CcbdError::IpcInvalidRequest(format!("invalid sandbox_overrides: {err}"))
        })?,
        None => bwrap::SandboxOverrides::default(),
    };

    if agent_exists(ctx.db.clone(), agent_id.to_string()).await? {
        return Err(CcbdError::AgentAlreadyExists(agent_id.to_string()));
    }

    let session = query_session_by_id(ctx.db.clone(), session_id.to_string())
        .await?
        .ok_or_else(|| {
            CcbdError::DbConstraintViolation(format!("session not found: {session_id}"))
        })?;

    let sandbox_dir = if ctx.env_state.unsafe_no_sandbox {
        None
    } else {
        Some(path::resolve_sandbox_dir(&ctx.state_dir, agent_id)?)
    };
    let bwrap_args = match &sandbox_dir {
        Some(dir) => match bwrap::build_args(dir, &overrides, Some(&manifest)) {
            Ok(args) => args,
            Err(err) => {
                cleanup_sandbox_dir(sandbox_dir.as_deref());
                return Err(err);
            }
        },
        None => Vec::new(),
    };
    let cmd = systemd::wrap_command(
        agent_id,
        &ctx.env_state,
        &bwrap_args,
        &manifest,
        &extra_env_vars,
    );
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
    let window_name = session.id.clone();
    let window_target = session_window_target(&window_name);
    let lock = session_window_lock(session_id);
    let pane_id = {
        let _guard = lock.lock().await;
        if let Err(err) = tmux
            .ensure_session(SESSION_NAME.to_string(), session_dir.clone())
            .await
        {
            cleanup_sandbox_dir(sandbox_dir.as_deref());
            drop(fifo_file);
            let _ = fs::remove_file(&fifo_path);
            return Err(err);
        }
        let window_exists = match tmux
            .window_exists(SESSION_NAME.to_string(), window_name.clone())
            .await
        {
            Ok(exists) => exists,
            Err(err) => {
                cleanup_sandbox_dir(sandbox_dir.as_deref());
                drop(fifo_file);
                let _ = fs::remove_file(&fifo_path);
                return Err(err);
            }
        };
        let spawn_result = if window_exists {
            tmux.split_window(window_target.clone(), session_dir, cmd)
                .await
        } else {
            tmux.spawn_window(SESSION_NAME.to_string(), window_name, session_dir, cmd)
                .await
        };
        match spawn_result {
            Ok(pane_id) => pane_id,
            Err(err) => {
                cleanup_sandbox_dir(sandbox_dir.as_deref());
                drop(fifo_file);
                let _ = fs::remove_file(&fifo_path);
                return Err(err);
            }
        }
    };
    let pid = match tmux.get_pane_pid(pane_id.clone()).await {
        Ok(pid) => pid,
        Err(err) => {
            cleanup_spawn_resources(&tmux, Some(pane_id.clone()), Some(fifo_file), &fifo_path)
                .await;
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
            cleanup_spawn_resources(&tmux, Some(pane_id.clone()), Some(fifo_file), &fifo_path)
                .await;
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
            cleanup_spawn_resources(&tmux, Some(pane_id.clone()), Some(fifo_file), &fifo_path)
                .await;
            cleanup_sandbox_dir(sandbox_dir.as_deref());
            let _ = delete_agent(ctx.db.clone(), agent_id.to_string()).await;
            return Err(CcbdError::EnvironmentNotSupported {
                details: format!("clone agent pidfd for {agent_id}: {err}"),
            });
        }
    };

    monitor::register(agent_id.to_string(), pidfd);
    let parser_handle = Arc::new(Mutex::new(vt100::Parser::new(200, 200, 0)));
    let matcher = Arc::new(MarkerMatcher::from_manifest(&manifest));
    parser_registry::register(agent_id.to_string(), parser_handle.clone());
    let marker_handle = spawn_marker_timer_task(
        agent_id.to_string(),
        TimerKind::Startup,
        Arc::new(ctx.db.clone()),
        parser_handle.clone(),
    );
    registry::register(agent_id.to_string(), marker_handle);
    seed_parser_from_tmux_capture(
        ctx,
        agent_id,
        &pane_id,
        parser_handle.clone(),
        matcher.clone(),
    )
    .await;
    let reader_handle = crate::agent_io::spawn_agent_io_reader_task_with_config(
        agent_id.to_string(),
        fifo_file,
        ctx.db.clone(),
        parser_handle,
        crate::agent_io::ReaderMarkerConfig {
            matcher,
            stability_ms: manifest.stability_ms,
        },
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

pub async fn handle_job_submit(params: Value, ctx: &Ctx) -> Result<Value, CcbdError> {
    let agent_id = required_str(&params, "agent_id")?;
    let prompt_text = required_str(&params, "text")?;
    let request_id = params
        .get("request_id")
        .and_then(Value::as_str)
        .map(str::to_string);

    let agent = query_agent(ctx.db.clone(), agent_id.to_string())
        .await?
        .ok_or_else(|| CcbdError::AgentNotFound(agent_id.to_string()))?;
    if matches!(agent.state.as_str(), "CRASHED" | "KILLED") {
        return Err(CcbdError::AgentWrongState {
            current_state: agent.state,
        });
    }

    let job_id = format!("job_{}", Uuid::new_v4());
    let returned_job_id = insert_job(
        ctx.db.clone(),
        job_id,
        agent_id.to_string(),
        request_id,
        prompt_text.to_string(),
    )
    .await?;
    crate::orchestrator::wake_up();

    Ok(json!({
        "job_id": returned_job_id,
        "status": "QUEUED",
    }))
}

pub async fn handle_job_wait(params: Value, ctx: &Ctx) -> Result<Value, CcbdError> {
    let job_id = required_str(&params, "job_id")?.to_string();
    let timeout_secs = params.get("timeout").and_then(Value::as_u64).unwrap_or(30);
    let mut rx = crate::orchestrator::pubsub::subscribe_job_updates();

    if let Some(result) = terminal_job_response(ctx, &job_id).await? {
        return Ok(result);
    }

    let wait_future = async {
        loop {
            match rx.recv().await {
                Ok(updated_job_id) if updated_job_id == job_id => {
                    if let Some(result) = terminal_job_response(ctx, &job_id).await? {
                        return Ok(result);
                    }
                }
                Ok(_) => {}
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                    if let Some(result) = terminal_job_response(ctx, &job_id).await? {
                        return Ok(result);
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    return Err(CcbdError::IpcInvalidRequest(
                        "job update subscription closed".into(),
                    ));
                }
            }
        }
    };

    match tokio::time::timeout(Duration::from_secs(timeout_secs), wait_future).await {
        Ok(result) => result,
        Err(_) => Err(CcbdError::PtyIoError(
            "Timeout waiting for job completion".into(),
        )),
    }
}

pub async fn handle_job_cancel(params: Value, ctx: &Ctx) -> Result<Value, CcbdError> {
    let job_id = required_str(&params, "job_id")?.to_string();
    let job = query_job(ctx.db.clone(), job_id.clone())
        .await?
        .ok_or_else(|| CcbdError::IpcInvalidRequest(format!("job_id not found: {job_id}")))?;

    match job.status.as_str() {
        "QUEUED" => {
            let _ = mark_queued_job_cancelled(ctx.db.clone(), job_id.clone()).await?;
            Ok(json!({ "job_id": job_id, "status": "CANCELLED" }))
        }
        "DISPATCHED" => {
            let _ = request_dispatched_job_cancel(ctx.db.clone(), job_id.clone()).await?;
            if mark_dispatched_job_cancelled_if_agent_idle(ctx.db.clone(), job_id.clone()).await?
                > 0
            {
                return Ok(json!({ "job_id": job_id, "status": "CANCELLED" }));
            }
            let pane_id = crate::agent_io::pane_id(&job.agent_id).ok_or_else(|| {
                CcbdError::PtyIoError(format!("tmux pane not registered for {}", job.agent_id))
            })?;
            ctx.tmux_server.send_ctrl_c(pane_id).await?;
            let _ =
                mark_dispatched_job_cancelled_if_agent_idle(ctx.db.clone(), job_id.clone()).await?;
            spawn_cancel_settlement_watch(ctx.db.clone(), job_id.clone());
            Ok(json!({ "job_id": job_id, "status": "CANCEL_REQUESTED" }))
        }
        "COMPLETED" | "FAILED" | "CANCELLED" => Ok(json!({
            "job_id": job_id,
            "status": job.status,
        })),
        other => Err(CcbdError::IpcInvalidRequest(format!(
            "job {job_id} is in unknown status {other}"
        ))),
    }
}

fn spawn_cancel_settlement_watch(db: Db, job_id: String) {
    tokio::spawn(async move {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(25);
        while tokio::time::Instant::now() < deadline {
            match mark_dispatched_job_cancelled_if_agent_idle(db.clone(), job_id.clone()).await {
                Ok(changes) if changes > 0 => return,
                Ok(_) => {}
                Err(err) => {
                    tracing::warn!(job_id = %job_id, error = %err, "cancel settlement watch failed");
                    return;
                }
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    });
}

pub async fn handle_agent_kill(params: Value, ctx: &Ctx) -> Result<Value, CcbdError> {
    let agent_id = required_str(&params, "agent_id")?;
    let agent = query_agent(ctx.db.clone(), agent_id.to_string())
        .await?
        .ok_or_else(|| CcbdError::AgentNotFound(agent_id.to_string()))?;

    let changes = mark_agent_killed(
        ctx.db.clone(),
        agent_id.to_string(),
        "SIGKILL_BY_DAEMON".to_string(),
    )
    .await?;
    if changes == 0 {
        return Err(CcbdError::AgentNotFound(agent_id.to_string()));
    }

    if let Some(pid) = agent.pid {
        // SAFETY: pid comes from the agents table and SIGKILL is a constant.
        let rc = unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) };
        if rc != 0 {
            let err = std::io::Error::last_os_error();
            tracing::warn!(agent_id, pid, error = %err, "failed to SIGKILL agent pid");
        }
    } else {
        tracing::warn!(agent_id, "agent.kill found no pid in database");
    }

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

    let agent = query_agent(ctx.db.clone(), agent_id.to_string())
        .await?
        .ok_or_else(|| CcbdError::AgentNotFound(agent_id.to_string()))?;
    let state = agent.state.clone();
    if state != "IDLE" && state != "UNKNOWN" {
        return Err(CcbdError::AgentWrongState {
            current_state: state,
        });
    }
    let manifest = crate::provider::manifest::get_manifest(&agent.provider);
    let matcher = Arc::new(MarkerMatcher::from_manifest(&manifest));

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

    let capture_baseline = ctx.tmux_server.capture_pane(pane_id.clone()).await.ok();
    let write_result = crate::agent_io::send_text_to_pane(
        ctx.tmux_server.clone(),
        agent_id,
        pane_id,
        text.to_string(),
    )
    .await;

    let write_error = write_result.as_ref().err().map(|err| err.to_string());
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
    if let Some(capture_baseline) = capture_baseline {
        spawn_new_capture_seed(
            ctx.db.clone(),
            ctx.tmux_server.clone(),
            agent_id.to_string(),
            capture_baseline,
            matcher,
        );
    }

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

    Ok(json!({ "events": format_events(events), "is_truncated": false }))
}

pub async fn handle_agent_watch(params: Value, ctx: &Ctx) -> Result<Value, CcbdError> {
    let agent_id = required_str(&params, "agent_id")?.to_string();
    let since_event_id = required_i64(&params, "since_event_id")?;
    let timeout_secs = params.get("timeout").and_then(Value::as_u64).unwrap_or(30);
    let mut rx = crate::orchestrator::pubsub::subscribe_agent_output();

    if query_agent(ctx.db.clone(), agent_id.clone())
        .await?
        .is_none()
    {
        return Err(CcbdError::AgentNotFound(agent_id));
    }

    let events = query_events_since(ctx.db.clone(), agent_id.clone(), since_event_id).await?;
    if !events.is_empty() {
        return Ok(json!({ "events": format_events(events), "is_truncated": false }));
    }

    let wait_future = async {
        loop {
            match rx.recv().await {
                Ok(updated_agent_id) if updated_agent_id == agent_id => {
                    let events =
                        query_events_since(ctx.db.clone(), agent_id.clone(), since_event_id)
                            .await?;
                    if !events.is_empty() {
                        return Ok(json!({
                            "events": format_events(events),
                            "is_truncated": false,
                        }));
                    }
                }
                Ok(_) => {}
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                    let events =
                        query_events_since(ctx.db.clone(), agent_id.clone(), since_event_id)
                            .await?;
                    if !events.is_empty() {
                        return Ok(json!({
                            "events": format_events(events),
                            "is_truncated": false,
                        }));
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    return Err(CcbdError::IpcInvalidRequest(
                        "agent output subscription closed".into(),
                    ));
                }
            }
        }
    };

    match tokio::time::timeout(Duration::from_secs(timeout_secs), wait_future).await {
        Ok(result) => result,
        Err(_) => Ok(json!({ "events": [], "is_truncated": false })),
    }
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

async fn terminal_job_response(ctx: &Ctx, job_id: &str) -> Result<Option<Value>, CcbdError> {
    let job = query_job(ctx.db.clone(), job_id.to_string())
        .await?
        .ok_or_else(|| CcbdError::IpcInvalidRequest(format!("job_id not found: {job_id}")))?;
    if matches!(job.status.as_str(), "COMPLETED" | "FAILED" | "CANCELLED") {
        Ok(Some(json!({
            "job_id": job.id,
            "status": job.status,
            "reply_text": job.reply_text,
            "error_reason": job.error_reason,
        })))
    } else {
        Ok(None)
    }
}

fn format_events(events: Vec<crate::db::schema::Event>) -> Vec<Value> {
    events
        .into_iter()
        .map(|event| {
            json!({
                "seq_id": event.seq_id,
                "event_type": event.event_type,
                "payload": event.payload,
                "created_at": event.created_at,
            })
        })
        .collect()
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
    matcher: Arc<MarkerMatcher>,
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
                matcher.scan(&parser)
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

fn spawn_new_capture_seed(
    db: crate::db::Db,
    tmux: Arc<crate::tmux::TmuxServer>,
    agent_id: String,
    baseline: String,
    matcher: Arc<MarkerMatcher>,
) {
    tokio::spawn(async move {
        let allow_direct_idle =
            matcher.mode() != crate::provider::manifest::IdleDetectionMode::ObservedStability;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        let mut processed_len = 0_usize;
        while tokio::time::Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(100)).await;
            let Some(pane_id) = crate::agent_io::pane_id(&agent_id) else {
                return;
            };
            let Some(parser_handle) = parser_registry::get(&agent_id) else {
                return;
            };
            let Ok(capture) = tmux.capture_pane(pane_id).await else {
                return;
            };
            if !capture.starts_with(&baseline) && !capture.contains(&baseline) {
                let mut parser = vt100::Parser::new(200, 200, 0);
                parser.process(capture.as_bytes());
                if allow_direct_idle && capture_seed_matches(&parser, &matcher) {
                    if let Err(err) =
                        crate::db::state_machine::mark_agent_idle_matched(db, agent_id.clone())
                            .await
                    {
                        tracing::warn!(agent_id = %agent_id, error = %err, "failed to mark agent IDLE from replacement tmux capture seed");
                    }
                    if let Some(handle) = registry::take(&agent_id) {
                        let _ = handle.cancel_tx.send(());
                    }
                    return;
                }
                continue;
            }
            let Some(suffix) = capture.strip_prefix(&baseline) else {
                return;
            };
            if suffix.len() <= processed_len {
                continue;
            }
            let delta = &suffix[processed_len..];
            processed_len = suffix.len();
            let matched = {
                match parser_handle.lock() {
                    Ok(mut parser) => {
                        parser.process(delta.as_bytes());
                        if allow_direct_idle {
                            matcher.scan(&parser)
                        } else {
                            MatchResult::NoMatch
                        }
                    }
                    Err(err) => {
                        tracing::warn!(agent_id = %agent_id, error = %err, "parser mutex poisoned during new tmux capture seed");
                        MatchResult::NoMatch
                    }
                }
            };
            if matched == MatchResult::Matched {
                if let Err(err) =
                    crate::db::state_machine::mark_agent_idle_matched(db, agent_id.clone()).await
                {
                    tracing::warn!(agent_id = %agent_id, error = %err, "failed to mark agent IDLE from new tmux capture seed");
                }
                if let Some(handle) = registry::take(&agent_id) {
                    let _ = handle.cancel_tx.send(());
                }
                return;
            }
        }
    });
}

fn capture_seed_matches(parser: &vt100::Parser, matcher: &MarkerMatcher) -> bool {
    if matcher.mode() == crate::provider::manifest::IdleDetectionMode::ObservedStability {
        return false;
    }
    parser
        .screen()
        .contents()
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .map(str::trim_end)
        .is_some_and(|line| {
            line.ends_with('$') || line.ends_with('#') || line.ends_with('>') || line.ends_with('✦')
        })
}

#[cfg(test)]
mod tests {
    use super::{
        capture_seed_matches, handle_agent_assert_state, handle_agent_discard_evidence,
        handle_agent_kill, handle_agent_read, handle_agent_send, handle_agent_spawn,
        handle_agent_watch, handle_job_submit, handle_job_wait, handle_session_create,
        handle_system_dump,
    };
    use crate::db;
    use crate::db::agents::{insert_agent_sync, query_agent_state_sync, update_agent_state_sync};
    use crate::db::jobs::{insert_job_sync, query_job_sync};
    use crate::db::sessions::insert_session_sync;
    use crate::db::state_machine::mark_agent_unknown_sync;
    use crate::error::CcbdError;
    use crate::marker::MarkerMatcher;
    use crate::provider::manifest::{IdleDetectionMode, ProviderManifest};
    use crate::rpc::Ctx;
    use crate::sandbox::EnvState;
    use crate::tmux::TmuxServer;
    use serde_json::json;
    use std::sync::Arc;
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

    #[test]
    fn test_observed_stability_capture_seed_never_marks_direct_idle() {
        let manifest = ProviderManifest {
            provider_name: "fake-observed",
            auth_mount_paths: vec![],
            idle_detection_mode: IdleDetectionMode::ObservedStability,
            marker_pattern: r"READY_PROMPT",
            stability_ms: 300,
            command: &["fake"],
            env_vars: &[],
        };
        let matcher = MarkerMatcher::from_manifest(&manifest);
        let mut parser = vt100::Parser::new(200, 200, 0);
        parser.process(b"old screen\nREADY_PROMPT\nnew output\n");

        assert!(!capture_seed_matches(&parser, &matcher));
    }

    async fn wait_for_state(ctx: &Ctx, agent_id: &str, expected: &str) {
        let timeout = Duration::from_secs(15);
        let deadline = std::time::Instant::now() + timeout;
        while std::time::Instant::now() < deadline {
            let state = query_agent_state_sync(&ctx.db, agent_id)
                .unwrap()
                .map(|(state, _)| state);
            if state.as_deref() == Some(expected) {
                return;
            }
            sleep_ms(100).await;
        }
        panic!("agent {agent_id} did not reach {expected} within {timeout:?}");
    }

    async fn cleanup_agent(ctx: &Ctx, agent_id: &str) {
        let _ = handle_agent_kill(json!({ "agent_id": agent_id }), ctx).await;
        sleep_ms(300).await;
    }

    fn seed_unknown_with_evidence(ctx: &Ctx, agent_id: &str) -> String {
        {
            let conn = ctx.db.conn();
            insert_session_sync(&conn, "s_evidence", "p_evidence", "/tmp/t4-evidence").unwrap();
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
            insert_session_sync(&conn, "s1", "p1", "/tmp/t4-spawn").unwrap();
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
    async fn test_handle_job_submit_queues_job() {
        let ctx = test_ctx();
        {
            let conn = ctx.db.conn();
            insert_session_sync(&conn, "s_job", "p_job", "/tmp/t8-job").unwrap();
            insert_agent_sync(&conn, "ag_job", "s_job", "bash", "IDLE", Some(123)).unwrap();
        }

        let result = handle_job_submit(
            json!({
                "agent_id": "ag_job",
                "text": "echo from job\n",
                "request_id": "job-req-1",
            }),
            &ctx,
        )
        .await
        .unwrap();

        let job_id = result["job_id"].as_str().unwrap();
        assert!(job_id.starts_with("job_"));
        assert_eq!(result["status"], "QUEUED");
        let job = query_job_sync(&ctx.db.conn(), job_id).unwrap().unwrap();
        assert_eq!(job.agent_id, "ag_job");
        assert_eq!(job.prompt_text, "echo from job\n");
        assert_eq!(job.request_id.as_deref(), Some("job-req-1"));
        assert_eq!(job.status, "QUEUED");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_handle_job_submit_is_idempotent_by_request_id() {
        let ctx = test_ctx();
        {
            let conn = ctx.db.conn();
            insert_session_sync(&conn, "s_job", "p_job", "/tmp/t8-job").unwrap();
            insert_agent_sync(&conn, "ag_job", "s_job", "bash", "BUSY", Some(123)).unwrap();
        }

        let first = handle_job_submit(
            json!({
                "agent_id": "ag_job",
                "text": "echo first\n",
                "request_id": "same-req",
            }),
            &ctx,
        )
        .await
        .unwrap();
        let second = handle_job_submit(
            json!({
                "agent_id": "ag_job",
                "text": "echo second\n",
                "request_id": "same-req",
            }),
            &ctx,
        )
        .await
        .unwrap();

        assert_eq!(first["job_id"], second["job_id"]);
        let job = query_job_sync(&ctx.db.conn(), first["job_id"].as_str().unwrap())
            .unwrap()
            .unwrap();
        assert_eq!(job.prompt_text, "echo first\n");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_handle_job_submit_rejects_missing_or_terminal_agent() {
        let ctx = test_ctx();
        let missing = handle_job_submit(
            json!({
                "agent_id": "missing",
                "text": "echo nope\n",
            }),
            &ctx,
        )
        .await
        .unwrap_err();
        assert!(matches!(missing, CcbdError::AgentNotFound(agent) if agent == "missing"));

        {
            let conn = ctx.db.conn();
            insert_session_sync(&conn, "s_job", "p_job", "/tmp/t8-job").unwrap();
            insert_agent_sync(&conn, "ag_dead", "s_job", "bash", "CRASHED", Some(123)).unwrap();
        }
        let terminal = handle_job_submit(
            json!({
                "agent_id": "ag_dead",
                "text": "echo nope\n",
            }),
            &ctx,
        )
        .await
        .unwrap_err();
        assert!(
            matches!(terminal, CcbdError::AgentWrongState { current_state } if current_state == "CRASHED")
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_handle_job_wait_fast_path_completed() {
        let ctx = test_ctx();
        {
            let conn = ctx.db.conn();
            insert_session_sync(&conn, "s_job_wait", "p_job_wait", "/tmp/t8-wait").unwrap();
            insert_agent_sync(&conn, "ag_wait", "s_job_wait", "bash", "IDLE", Some(123)).unwrap();
            insert_job_sync(&conn, "job_wait_done", "ag_wait", None, "echo done\n").unwrap();
            conn.execute(
                "UPDATE jobs SET status='COMPLETED', reply_text='done\n', completed_at=unixepoch() WHERE id='job_wait_done'",
                [],
            )
            .unwrap();
        }

        let result = handle_job_wait(
            json!({
                "job_id": "job_wait_done",
                "timeout": 1,
            }),
            &ctx,
        )
        .await
        .unwrap();

        assert_eq!(result["status"], "COMPLETED");
        assert_eq!(result["reply_text"], "done\n");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_handle_job_wait_rejects_missing_job() {
        let ctx = test_ctx();
        let err = handle_job_wait(
            json!({
                "job_id": "missing",
                "timeout": 1,
            }),
            &ctx,
        )
        .await
        .unwrap_err();

        assert!(
            matches!(err, CcbdError::IpcInvalidRequest(message) if message.contains("job_id not found"))
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_handle_job_wait_times_out_for_queued_job() {
        let ctx = test_ctx();
        {
            let conn = ctx.db.conn();
            insert_session_sync(
                &conn,
                "s_job_wait_timeout",
                "p_job_wait_timeout",
                "/tmp/t8-wait-timeout",
            )
            .unwrap();
            insert_agent_sync(
                &conn,
                "ag_wait_timeout",
                "s_job_wait_timeout",
                "bash",
                "IDLE",
                Some(123),
            )
            .unwrap();
            insert_job_sync(
                &conn,
                "job_wait_timeout",
                "ag_wait_timeout",
                None,
                "echo later\n",
            )
            .unwrap();
        }

        let err = handle_job_wait(
            json!({
                "job_id": "job_wait_timeout",
                "timeout": 0,
            }),
            &ctx,
        )
        .await
        .unwrap_err();

        assert!(matches!(err, CcbdError::PtyIoError(message) if message.contains("Timeout")));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_handle_agent_send_idempotent() {
        let ctx = test_ctx();
        {
            let conn = ctx.db.conn();
            insert_session_sync(&conn, "s1", "p1", "/tmp/t4-send").unwrap();
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
            insert_session_sync(&conn, "s1", "p1", "/tmp/t4-read").unwrap();
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
            insert_session_sync(&conn, "s1", "p1", "/tmp/t4-crashed-read").unwrap();
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
    async fn test_handle_agent_watch_fast_path_returns_existing_events() {
        let ctx = test_ctx();
        {
            let conn = ctx.db.conn();
            insert_session_sync(&conn, "s_watch", "p_watch", "/tmp/t8-watch").unwrap();
            insert_agent_sync(&conn, "ag_watch", "s_watch", "bash", "IDLE", Some(123)).unwrap();
            conn.execute(
                "INSERT INTO events (agent_id, request_id, event_type, payload) VALUES ('ag_watch', NULL, 'output_chunk', '{\"text\":\"watch-ok\"}')",
                [],
            )
            .unwrap();
        }

        let result = handle_agent_watch(
            json!({
                "agent_id": "ag_watch",
                "since_event_id": 0,
                "timeout": 1,
            }),
            &ctx,
        )
        .await
        .unwrap();

        let events = result["events"].as_array().unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["event_type"], "output_chunk");
        assert!(events[0]["payload"].as_str().unwrap().contains("watch-ok"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_handle_agent_watch_times_out_empty() {
        let ctx = test_ctx();
        {
            let conn = ctx.db.conn();
            insert_session_sync(
                &conn,
                "s_watch_empty",
                "p_watch_empty",
                "/tmp/t8-watch-empty",
            )
            .unwrap();
            insert_agent_sync(
                &conn,
                "ag_watch_empty",
                "s_watch_empty",
                "bash",
                "IDLE",
                Some(123),
            )
            .unwrap();
        }

        let result = handle_agent_watch(
            json!({
                "agent_id": "ag_watch_empty",
                "since_event_id": 0,
                "timeout": 0,
            }),
            &ctx,
        )
        .await
        .unwrap();

        assert!(result["events"].as_array().unwrap().is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_handle_agent_kill_marks_killed_and_rejects_repeat() {
        let ctx = test_ctx();
        {
            let conn = ctx.db.conn();
            insert_session_sync(&conn, "s1", "p1", "/tmp/t4-kill").unwrap();
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
            insert_session_sync(&conn, "s_unknown", "p_unknown", "/tmp/t4-unknown").unwrap();
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
