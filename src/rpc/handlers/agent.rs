use super::params::{
    extension_config_from_params, required_i64, required_str, should_press_enter_after_paste,
};
use super::sessions::session_window_lock;
use crate::db::agents::{insert_agent, query_agent, query_agent_state, update_agent_config_hash};
use crate::db::agents_lifecycle::mark_agent_killed;
use crate::db::events::{insert_event, query_event_by_request_id, query_events_since};
use crate::db::events_progress::record_send_progress;
use crate::db::sessions::query_session_by_id;
use crate::db::state_machine::{mark_agent_idle_hook_event, mark_agent_waiting_for_ack};
use crate::db::system::remove_agent_sandbox_dir_sync;
use crate::error::CcbdError;
use crate::marker::{
    MarkerMatcher, PromptTimerScanContext, TimerKind, parser_registry, registry,
    spawn_marker_timer_task_with_prompt,
};
use crate::monitor;
use crate::monitor::agent_watch::spawn_agent_pidfd_watch_task;
use crate::provider::fingerprint::{ConfigFingerprintInput, ConfigRole, compute_config_hash};
use crate::provider::home_layout::{
    HomeLayoutRole, HookPushContext, prepare_home_layout_with_extensions_for_slot,
};
use crate::provider::manifest::compute_recovery_args;
use crate::rpc::Ctx;
use crate::sandbox::{SandboxOverrides, path, systemd};
use crate::tmux::agent_session_name;
use nix::sys::stat::Mode;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::fs;
use std::os::unix::fs::OpenOptionsExt;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub async fn handle_agent_spawn(params: Value, ctx: &Ctx) -> Result<Value, CcbdError> {
    handle_agent_spawn_with_recovery(params, ctx, false).await
}

pub(crate) enum AgentSpawnDbAction {
    InsertDefault,
    ReplaceKilledAndRequeue {
        expected_hash: String,
        captured_intent: Option<crate::db::recovery::AgentRecoveryIntent>,
    },
}

fn persist_agent_spawn_snapshot_after_success(
    db: &crate::db::Db,
    spec: crate::db::recovery::AgentSpawnSpec,
    config_hash: &str,
) -> Result<bool, CcbdError> {
    let conn = db.conn();
    match crate::db::recovery::persist_agent_spawn_spec_sync(&conn, &spec, config_hash) {
        Ok(()) => Ok(true),
        Err(err) => {
            tracing::error!(
                agent_id = %spec.agent_id,
                error = %err,
                "failed to persist agent spawn recovery snapshot after successful spawn"
            );
            Ok(false)
        }
    }
}

pub(super) async fn handle_agent_spawn_with_recovery(
    params: Value,
    ctx: &Ctx,
    is_recovery: bool,
) -> Result<Value, CcbdError> {
    handle_agent_spawn_with_db_action(params, ctx, is_recovery, AgentSpawnDbAction::InsertDefault)
        .await
}

pub(crate) async fn handle_agent_spawn_with_db_action(
    params: Value,
    ctx: &Ctx,
    is_recovery: bool,
    db_action: AgentSpawnDbAction,
) -> Result<Value, CcbdError> {
    let session_id = required_str(&params, "session_id")?;
    let agent_id = required_str(&params, "agent_id")?;
    let provider = required_str(&params, "provider")?;
    let manifest = crate::provider::manifest::try_get_manifest(provider)?;
    let extensions = extension_config_from_params(&params)?;
    let extra_env_vars = match params.get("extra_env_vars") {
        Some(value) => {
            serde_json::from_value::<HashMap<String, String>>(value.clone()).map_err(|err| {
                CcbdError::IpcInvalidRequest(format!("invalid extra_env_vars: {err}"))
            })?
        }
        None => HashMap::new(),
    };
    let sandbox_overrides = match params.get("sandbox_overrides") {
        Some(value) => {
            serde_json::from_value::<SandboxOverrides>(value.clone()).map_err(|err| {
                CcbdError::IpcInvalidRequest(format!("invalid sandbox_overrides: {err}"))
            })?
        }
        None => SandboxOverrides::default(),
    };
    if let Some(existing) = query_agent(ctx.db.clone(), agent_id.to_string()).await?
        && existing.state != "KILLED"
    {
        return Err(CcbdError::AgentAlreadyExists(agent_id.to_string()));
    }

    let session = query_session_by_id(ctx.db.clone(), session_id.to_string())
        .await?
        .ok_or_else(|| {
            CcbdError::DbConstraintViolation(format!("session not found: {session_id}"))
        })?;

    let sandbox_guard = if ctx.env_state.unsafe_no_sandbox {
        None
    } else {
        Some(path::SandboxDirGuard::new(path::resolve_sandbox_dir(
            &ctx.state_dir,
            session_id,
            agent_id,
        )?))
    };
    let agent_cwd: std::path::PathBuf = session.absolute_path.clone().into();
    let mut spawn_env_vars =
        build_agent_spawn_env_vars_for_hook_push(&ctx.state_dir, extra_env_vars);
    let hook_push_enabled = hook_push_enabled_from_spawn_params(&params);
    let mut recovery_args = Vec::new();
    if let Some(dir) = sandbox_guard.as_ref().and_then(|guard| guard.path())
        && manifest.requires_home_materialization
    {
        let hook_push_ctx = HookPushContext {
            agent_id: agent_id.to_string(),
            provider: manifest.provider_name.to_string(),
            ahd_socket_path: ctx.state_dir.join("ahd.sock"),
            enabled: hook_push_enabled,
        };
        let home_overrides = prepare_home_layout_with_extensions_for_slot(
            manifest.provider_name,
            dir,
            &agent_cwd,
            HomeLayoutRole::Worker,
            agent_id,
            &extensions,
            Some(&hook_push_ctx),
        )?;
        if is_recovery {
            recovery_args =
                compute_recovery_args(manifest.provider_name, &home_overrides.home_root);
        }
        spawn_env_vars.extend(home_overrides.extra_env);
    } else if is_recovery {
        recovery_args = compute_recovery_args(manifest.provider_name, &agent_cwd);
    }
    let cmd = systemd::wrap_command_with_recovery_and_sandbox_overrides(
        agent_id,
        &session.project_id,
        ctx.tmux_server.socket_name(),
        &ctx.env_state,
        systemd::RecoverySpawn {
            is_recovery,
            args: recovery_args,
        },
        ctx.daemon_unit.as_deref(),
        &manifest,
        &spawn_env_vars,
        &sandbox_overrides,
    );
    tracing::debug!(agent_id, provider = %manifest.provider_name, cmd_len = cmd.len(), "spawn cmd built");
    tracing::info!(agent_id = %agent_id, "spawn cmd: {}", cmd.join(" "));
    let fifo_dir = ctx.state_dir.join("pipes");
    if let Err(err) = fs::create_dir_all(&fifo_dir) {
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
            let _ = fs::remove_file(&fifo_path);
            return Err(CcbdError::EnvironmentNotSupported {
                details: format!("open fifo {}: {err}", fifo_path.display()),
            });
        }
    };

    let tmux = ctx.tmux_server.clone();
    let agent_session = agent_session_name(agent_id);
    let lock = session_window_lock(session_id);
    let pane_id = {
        let _guard = lock.lock().await;
        if let Err(err) = tmux
            .ensure_session(agent_session.clone(), agent_cwd.clone())
            .await
        {
            drop(fifo_file);
            let _ = fs::remove_file(&fifo_path);
            return Err(err);
        }
        match tmux
            .spawn_window(agent_session, agent_id.to_string(), agent_cwd, cmd)
            .await
        {
            Ok(pane_id) => pane_id,
            Err(err) => {
                drop(fifo_file);
                let _ = fs::remove_file(&fifo_path);
                return Err(err);
            }
        }
    };
    let title = format!("{agent_id} ({provider})");
    let _ = tmux.set_pane_title(pane_id.clone(), &title).await;
    let pid = match tmux.get_pane_pid(pane_id.clone()).await {
        Ok(pid) => pid,
        Err(err) => {
            cleanup_spawn_resources(&tmux, Some(pane_id.clone()), Some(fifo_file), &fifo_path)
                .await;
            return Err(err);
        }
    };
    if let Err(err) = tmux
        .pipe_pane_to_fifo(pane_id.clone(), fifo_path.clone())
        .await
    {
        cleanup_spawn_resources(&tmux, Some(pane_id.clone()), Some(fifo_file), &fifo_path).await;
        return Err(err);
    }

    let pidfd = match monitor::pidfd_open(pid) {
        Ok(fd) => fd,
        Err(err) => {
            cleanup_spawn_resources(&tmux, Some(pane_id.clone()), Some(fifo_file), &fifo_path)
                .await;
            return Err(err);
        }
    };
    let pidfd_for_task = match pidfd.try_clone() {
        Ok(fd) => fd,
        Err(err) => {
            cleanup_spawn_resources(&tmux, Some(pane_id.clone()), Some(fifo_file), &fifo_path)
                .await;
            return Err(CcbdError::EnvironmentNotSupported {
                details: format!("clone agent pidfd for {agent_id}: {err}"),
            });
        }
    };

    let config_hash = compute_config_hash(&ConfigFingerprintInput {
        role: ConfigRole::Agent {
            provider,
            env: &spawn_env_vars,
        },
        hooks: &extensions.hooks,
        plugins: &extensions.plugins,
    })?;
    let spawn_spec = crate::db::recovery::AgentSpawnSpec {
        agent_id: agent_id.to_string(),
        provider: provider.to_string(),
        env: spawn_env_vars.clone(),
        hooks: extensions.hooks.clone(),
        plugins: extensions.plugins.clone(),
        sandbox_overrides: sandbox_overrides.clone(),
        hook_push_enabled,
    };
    match db_action {
        AgentSpawnDbAction::InsertDefault => {
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
                cleanup_spawn_resources(&tmux, Some(pane_id.clone()), Some(fifo_file), &fifo_path)
                    .await;
                return Err(err);
            }
            update_agent_config_hash(ctx.db.clone(), agent_id.to_string(), config_hash.clone())
                .await?;
            let _ = persist_agent_spawn_snapshot_after_success(&ctx.db, spawn_spec, &config_hash)?;
        }
        AgentSpawnDbAction::ReplaceKilledAndRequeue {
            expected_hash,
            captured_intent,
        } => {
            if let Err(err) = crate::db::recovery::replace_killed_agent_and_requeue_job_sync(
                &ctx.db,
                session_id,
                &spawn_spec,
                &expected_hash,
                pid as i64,
                captured_intent.as_ref(),
            ) {
                cleanup_spawn_resources(&tmux, Some(pane_id.clone()), Some(fifo_file), &fifo_path)
                    .await;
                return Err(err);
            }
        }
    }

    monitor::register(agent_id.to_string(), pidfd);
    let parser_handle = Arc::new(Mutex::new(vt100::Parser::new(200, 200, 0)));
    let matcher = Arc::new(MarkerMatcher::from_manifest(&manifest));
    parser_registry::register(agent_id.to_string(), parser_handle.clone());
    let idle_scan_enabled = Arc::new(AtomicBool::new(false));
    let reader_handle = crate::agent_io::spawn_agent_io_reader_task_with_config(
        agent_id.to_string(),
        fifo_file,
        ctx.db.clone(),
        parser_handle.clone(),
        crate::agent_io::ReaderMarkerConfig {
            matcher: matcher.clone(),
            stability_ms: manifest.stability_ms,
            idle_scan_enabled: idle_scan_enabled.clone(),
        },
    );
    let pane_id_for_startup = pane_id.clone();
    let response_pane_id = pane_id.0.clone();
    crate::agent_io::register(
        agent_id.to_string(),
        crate::agent_io::AgentIoEntry {
            session_id: session_id.to_string(),
            pane_id,
            reader_handle,
            fifo_path,
            socket_name: ctx.tmux_server.socket_name().to_string(),
            idle_scan_enabled: idle_scan_enabled.clone(),
        },
    );
    spawn_agent_pidfd_watch_task(
        agent_id.to_string(),
        pid,
        pidfd_for_task,
        Arc::new(ctx.db.clone()),
    );
    crate::provider::init_probe_task::spawn_init_probe_task(
        agent_id.to_string(),
        ctx.tmux_server.clone(),
        pane_id_for_startup,
        Arc::new(ctx.db.clone()),
        provider.to_string(),
        ctx.state_dir.clone(),
        matcher,
        manifest.init_probe,
        Duration::from_secs(manifest.readiness_timeout_s.into()),
        crate::provider::init_probe_task::STABLE_UNKNOWN_STARTUP_GRACE,
        idle_scan_enabled,
    );
    let _sandbox_dir = sandbox_guard.map(path::SandboxDirGuard::release);

    Ok(json!({ "state": "SPAWNING", "pid": pid, "pane_id": response_pane_id }))
}

pub(crate) fn hook_push_enabled_from_spawn_params(params: &Value) -> bool {
    params
        .get("hook_push_enabled")
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

pub(crate) fn build_agent_spawn_env_vars_for_hook_push(
    state_dir: &std::path::Path,
    mut extra_env_vars: HashMap<String, String>,
) -> HashMap<String, String> {
    extra_env_vars.insert(
        "CCB_SOCKET".to_string(),
        state_dir.join("ahd.sock").display().to_string(),
    );
    extra_env_vars
}

#[cfg(test)]
mod ra2_tests {
    use super::persist_agent_spawn_snapshot_after_success;
    use crate::db::agents::insert_agent_sync;
    use crate::db::recovery::query_agent_spawn_spec_sync;
    use crate::db::sessions::insert_session_sync;
    use crate::provider::extensions::HookGroup;
    use std::collections::HashMap;

    fn test_db() -> crate::db::Db {
        let file = tempfile::NamedTempFile::new().unwrap();
        crate::db::init(file.path()).unwrap()
    }

    fn sample_spec(agent_id: &str) -> crate::db::recovery::AgentSpawnSpec {
        crate::db::recovery::AgentSpawnSpec {
            agent_id: agent_id.to_string(),
            provider: "codex".to_string(),
            env: HashMap::from([("RA2_ENV".to_string(), "1".to_string())]),
            hooks: HashMap::<String, Vec<HookGroup>>::new(),
            plugins: vec!["github@openai-curated".to_string()],
            sandbox_overrides: Default::default(),
            hook_push_enabled: false,
        }
    }

    #[test]
    fn ra2_agent_spawn_success_persists_resolved_snapshot() {
        let db = test_db();
        {
            let conn = db.conn();
            insert_session_sync(&conn, "s1", "p1", "/tmp/ra2").unwrap();
            insert_agent_sync(&conn, "ra2_spawn", "s1", "codex", "SPAWNING", Some(123)).unwrap();
        }

        assert!(
            persist_agent_spawn_snapshot_after_success(&db, sample_spec("ra2_spawn"), "hash1")
                .unwrap()
        );
        let stored = query_agent_spawn_spec_sync(&db.conn(), "ra2_spawn")
            .unwrap()
            .unwrap();
        assert_eq!(stored.spec.agent_id, "ra2_spawn");
        assert_eq!(stored.spec.provider, "codex");
        assert_eq!(stored.spec.env["RA2_ENV"], "1");
        assert_eq!(stored.config_hash, "hash1");
    }

    #[test]
    fn ra2_agent_spawn_snapshot_write_failure_is_graceful() {
        let db = test_db();
        {
            let conn = db.conn();
            insert_session_sync(&conn, "s1", "p1", "/tmp/ra2").unwrap();
            insert_agent_sync(
                &conn,
                "ra2_spawn_no_snapshot",
                "s1",
                "codex",
                "SPAWNING",
                Some(123),
            )
            .unwrap();
        }

        let wrote_snapshot = persist_agent_spawn_snapshot_after_success(
            &db,
            sample_spec("missing_agent_for_fk_failure"),
            "hash1",
        )
        .unwrap();
        assert!(!wrote_snapshot);
        let state: String = db
            .conn()
            .query_row(
                "SELECT state FROM agents WHERE id = 'ra2_spawn_no_snapshot'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(state, "SPAWNING");
        assert!(
            query_agent_spawn_spec_sync(&db.conn(), "ra2_spawn_no_snapshot")
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn hook_push_worker_spawn_env_injects_deterministic_ccb_socket() {
        let state_dir = tempfile::tempdir().unwrap();
        let env = super::build_agent_spawn_env_vars_for_hook_push(
            state_dir.path(),
            HashMap::from([
                (
                    "CCB_SOCKET".to_string(),
                    "/tmp/stale-host-socket.sock".to_string(),
                ),
                ("USER_FLAG".to_string(), "1".to_string()),
            ]),
        );

        assert_eq!(
            env.get("CCB_SOCKET").map(String::as_str),
            Some(state_dir.path().join("ahd.sock").to_str().unwrap()),
            "worker spawn must inject deterministic daemon socket, not inherit host CCB_SOCKET"
        );
        assert_eq!(env.get("USER_FLAG").map(String::as_str), Some("1"));
    }

    #[test]
    fn hook_push_enabled_spawn_param_defaults_false() {
        assert!(!super::hook_push_enabled_from_spawn_params(
            &serde_json::json!({})
        ));
        assert!(!super::hook_push_enabled_from_spawn_params(
            &serde_json::json!({"hook_push_enabled": "true"})
        ));
    }

    #[test]
    fn hook_push_enabled_spawn_param_accepts_true() {
        assert!(super::hook_push_enabled_from_spawn_params(
            &serde_json::json!({"hook_push_enabled": true})
        ));
    }
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
        return Ok(json!({ "state": agent.state }));
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
    remove_agent_sandbox_dir_sync(&ctx.state_dir, &agent.session_id, agent_id);

    Ok(json!({ "state": "KILLED" }))
}

pub async fn handle_agent_notify(params: Value, ctx: &Ctx) -> Result<Value, CcbdError> {
    let agent_id = required_str(&params, "agent_id")?.to_string();
    let event = required_str(&params, "event")?.to_string();
    if event != "stop" {
        return Err(CcbdError::IpcInvalidRequest(format!(
            "unsupported event for agent.notify first release: {event}; only stop is supported"
        )));
    }
    let provider = params
        .get("provider")
        .and_then(Value::as_str)
        .map(str::to_string);
    let event_id = params
        .get("event_id")
        .and_then(Value::as_str)
        .map(str::to_string);
    let reply = params
        .get("reply")
        .and_then(Value::as_str)
        .map(str::to_string);

    let agent = query_agent(ctx.db.clone(), agent_id.clone())
        .await?
        .ok_or_else(|| CcbdError::AgentNotFound(agent_id.clone()))?;
    if let Some(provider) = provider.as_deref()
        && provider != agent.provider
    {
        return Err(CcbdError::IpcInvalidRequest(format!(
            "provider mismatch for agent {agent_id}: expected {}, got {provider}",
            agent.provider
        )));
    }
    let provider = provider.unwrap_or(agent.provider);

    let (changes, affected_job_id) = mark_agent_idle_hook_event(
        ctx.db.clone(),
        agent_id.clone(),
        provider.clone(),
        event.clone(),
        event_id.clone(),
        reply,
    )
    .await?;
    if changes > 0 {
        if let Some(job_id) = &affected_job_id {
            crate::orchestrator::pubsub::notify_job_update(job_id);
        }
        crate::completion::registry::cancel(&agent_id);
        crate::orchestrator::wake_up();
    }

    Ok(json!({
        "agent_id": agent_id,
        "event": event,
        "provider": provider,
        "event_id": event_id,
        "accepted": true,
        "transitioned": changes > 0,
        "changes": changes,
        "affected_job_id": affected_job_id,
    }))
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
    let cas_ok =
        mark_agent_waiting_for_ack(ctx.db.clone(), agent_id.to_string(), agent.state_version)
            .await?;
    if !cas_ok {
        let current_state = query_agent_state(ctx.db.clone(), agent_id.to_string())
            .await?
            .map(|(state, _)| state)
            .unwrap_or_else(|| state.clone());
        return Err(CcbdError::AgentWrongState { current_state });
    }
    crate::agent_io::set_idle_scan_enabled(agent_id, false);
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
    let write_result = crate::agent_io::send_text_to_pane_with_options(
        ctx.tmux_server.clone(),
        agent_id,
        pane_id,
        text.to_string(),
        should_press_enter_after_paste(&agent.provider, text),
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
    let marker_handle = spawn_marker_timer_task_with_prompt(
        agent_id.to_string(),
        TimerKind::Busy,
        Arc::new(ctx.db.clone()),
        parser_handle,
        Some(PromptTimerScanContext {
            provider: agent.provider.clone(),
            state_dir: ctx.state_dir.clone(),
            tmux: ctx.tmux_server.clone(),
            matcher: matcher.clone(),
        }),
    );
    registry::register(agent_id.to_string(), marker_handle);
    if let Some(capture_baseline) = capture_baseline {
        super::ack::spawn_new_capture_seed(
            ctx.db.clone(),
            ctx.tmux_server.clone(),
            agent_id.to_string(),
            agent.provider.clone(),
            ctx.state_dir.clone(),
            capture_baseline,
            matcher,
        );
    }

    Ok(json!({
        "state": crate::db::state_machine::STATE_WAITING_FOR_ACK,
        "seq_id": seq_id,
    }))
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
