use super::params::{extension_config_from_params, required_str};
use crate::db::sessions::{
    create_session, list_session_summaries, query_session_by_id, set_session_master_pane_id,
    update_session_config_hash,
};
use crate::db::system::{
    cascade_kill_session_agents, cascade_kill_session_agents_for_daemon,
    remove_agent_sandbox_dir_sync, session_agent_ids,
};
use crate::error::CcbdError;
use crate::master_revival::{
    mark_session_intentional_killed, master_monitor_key, record_spawned_master_runtime,
    record_spawned_master_runtime_after_claim,
};
use crate::monitor;
use crate::monitor::master_watch::spawn_master_pidfd_watch_task;
use crate::monitor::session_watch::{spawn_session_watch_task, unit_name_for_session};
use crate::provider::fingerprint::{ConfigFingerprintInput, ConfigRole, compute_config_hash};
use crate::provider::home_layout::{HomeLayoutRole, prepare_home_layout_with_extensions};
use crate::rpc::Ctx;
use crate::sandbox::{path, systemd};
use crate::tmux::scope::{self, ScopePolicy};
use crate::tmux::{TmuxPaneId, agent_session_name, master_session_name, sanitize_tmux_name};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::process::Command;
use std::sync::{Arc, LazyLock, Mutex};
use tokio::sync::Mutex as AsyncMutex;
use uuid::Uuid;

static SESSION_WINDOW_LOCKS: LazyLock<Mutex<HashMap<String, Arc<AsyncMutex<()>>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

pub(super) fn session_window_lock(session_id: &str) -> Arc<AsyncMutex<()>> {
    let mut locks = SESSION_WINDOW_LOCKS
        .lock()
        .expect("session window locks mutex poisoned");
    locks
        .entry(session_id.to_string())
        .or_insert_with(|| Arc::new(AsyncMutex::new(())))
        .clone()
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

    if session_anchors_enabled(ctx) {
        let unit_name = unit_name_for_session(&session_id);
        if let Err(err) = create_session_anchor(&unit_name) {
            let _ = cascade_kill_session_agents(
                ctx.db.clone(),
                session_id.clone(),
                "ANCHOR_CREATE_FAILED".to_string(),
            )
            .await;
            return Err(err);
        }
        spawn_session_watch_task(session_id.clone(), unit_name, Arc::new(ctx.db.clone()));
    }

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

    mark_session_intentional_killed(&ctx.db, session_id)?;

    if session_anchors_enabled(ctx) {
        stop_session_anchor(&unit_name_for_session(session_id));
    }

    let killed = cascade_kill_session_agents_for_daemon(
        ctx.db.clone(),
        session_id.to_string(),
        "SESSION_KILL".to_string(),
        ctx.tmux_server.socket_name().to_string(),
    )
    .await?;
    for agent_id in &agent_ids {
        if let Some(pane_id) = crate::agent_io::pane_id(agent_id) {
            let _ = ctx.tmux_server.kill_pane(pane_id).await;
        }
        let _ = ctx
            .tmux_server
            .kill_session(agent_session_name(agent_id))
            .await;
    }
    for agent_id in &agent_ids {
        remove_agent_sandbox_dir_sync(&ctx.state_dir, session_id, agent_id);
    }
    remove_agent_sandbox_dir_sync(&ctx.state_dir, session_id, "master");
    let _ = ctx
        .tmux_server
        .kill_session(master_session_name(&session.project_id))
        .await;
    if force {
        tracing::debug!(
            session_id,
            "session.kill force requested; runtime teardown is already best-effort by default"
        );
    }
    let master_pane_killed = if let Some(master_pane_id) = session.master_pane_id {
        match TmuxPaneId::parse(&master_pane_id) {
            Ok(pane_id) => {
                let _ = ctx.tmux_server.kill_pane(pane_id).await;
                true
            }
            Err(err) => {
                tracing::warn!(session_id, pane_id = %master_pane_id, error = %err, "invalid stored master pane id");
                false
            }
        }
    } else {
        false
    };
    Ok(json!({
        "session_id": session_id,
        "state": "KILLED",
        "killed_agents": killed,
        "master_pane_killed": master_pane_killed,
    }))
}

pub(super) fn session_anchors_enabled(ctx: &Ctx) -> bool {
    ctx.env_state.systemd_run_available
        && (ctx.env_state.unsafe_no_sandbox || ctx.env_state.under_systemd)
        && matches!(
            scope::detect_scope_policy(ctx.tmux_server.socket_name()),
            ScopePolicy::Systemd(_)
        )
}

fn create_session_anchor(unit_name: &str) -> Result<(), CcbdError> {
    let output = Command::new("systemd-run")
        .args([
            "--user",
            &format!("--unit={unit_name}"),
            "--remain-after-exit",
            "/usr/bin/true",
        ])
        .output()
        .map_err(|err| CcbdError::EnvironmentNotSupported {
            details: format!("create session anchor {unit_name}: {err}"),
        })?;
    if output.status.success() {
        return Ok(());
    }
    Err(CcbdError::EnvironmentNotSupported {
        details: format!(
            "create session anchor {unit_name} failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ),
    })
}

pub(super) fn stop_session_anchor(unit_name: &str) {
    match Command::new("systemctl")
        .args(["--user", "stop", unit_name])
        .output()
    {
        Ok(output) if output.status.success() => {}
        Ok(output) => tracing::warn!(
            unit = %unit_name,
            status = ?output.status,
            stderr = %String::from_utf8_lossy(&output.stderr).trim(),
            "failed to stop session anchor"
        ),
        Err(err) => tracing::warn!(unit = %unit_name, error = %err, "failed to run systemctl stop"),
    }
}

pub async fn handle_session_spawn_master_pane(
    params: Value,
    ctx: &Ctx,
) -> Result<Value, CcbdError> {
    let session_id = required_str(&params, "session_id")?;
    let cmd = required_str(&params, "cmd")?;
    let claimed_master_generation = params
        .get("_claimed_master_generation")
        .and_then(Value::as_i64);
    let extensions = extension_config_from_params(&params)?;
    let session = query_session_by_id(ctx.db.clone(), session_id.to_string())
        .await?
        .ok_or_else(|| CcbdError::IpcInvalidRequest(format!("session not found: {session_id}")))?;
    let master_cwd: std::path::PathBuf = session.absolute_path.clone().into();
    let mut master_env_vars = HashMap::new();
    let master_sandbox_dir = if ctx.env_state.unsafe_no_sandbox {
        None
    } else {
        Some(path::resolve_sandbox_dir(
            &ctx.state_dir,
            session_id,
            "master",
        )?)
    };
    if let Some(dir) = master_sandbox_dir.as_ref() {
        let home_overrides = prepare_home_layout_with_extensions(
            "claude",
            dir,
            &master_cwd,
            HomeLayoutRole::Master,
            &extensions,
        )?;
        master_env_vars.extend(home_overrides.extra_env);
    }
    let tmux_cmd = systemd::master_command_with_env(
        &session.project_id,
        cmd,
        &ctx.env_state,
        ctx.daemon_unit.as_deref(),
        &master_env_vars,
    );
    let master_session = master_session_name(&session.project_id);
    ctx.tmux_server
        .ensure_session(master_session.clone(), master_cwd.clone())
        .await?;
    let pane = ctx
        .tmux_server
        .spawn_window(
            master_session,
            sanitize_tmux_name(&session.project_id),
            master_cwd,
            tmux_cmd,
        )
        .await?;
    let title = format!("master ({})", cmd.split_whitespace().next().unwrap_or(cmd));
    let _ = ctx.tmux_server.set_pane_title(pane.clone(), &title).await;
    set_session_master_pane_id(ctx.db.clone(), session_id.to_string(), pane.0.clone()).await?;
    let config_hash = compute_config_hash(&ConfigFingerprintInput {
        role: ConfigRole::Master { cmd },
        hooks: &extensions.hooks,
        plugins: &extensions.plugins,
    })?;
    update_session_config_hash(ctx.db.clone(), session_id.to_string(), config_hash).await?;

    match ctx.tmux_server.get_pane_pid(pane.clone()).await {
        Ok(pid) => match monitor::pidfd_open(pid) {
            Ok(pidfd) => {
                let task_fd = match pidfd.try_clone() {
                    Ok(fd) => fd,
                    Err(err) => {
                        tracing::warn!(session_id, error = %err, "failed to clone master pidfd");
                        return Ok(json!({ "pane_id": pane.0 }));
                    }
                };
                let generation = if let Some(claimed_generation) = claimed_master_generation {
                    match record_spawned_master_runtime_after_claim(
                        &ctx.db,
                        session_id,
                        &pane.0,
                        i64::from(pid),
                        claimed_generation,
                    )? {
                        Some(generation) => generation,
                        None => {
                            let _ = ctx.tmux_server.kill_pane(pane.clone()).await;
                            tracing::warn!(
                                session_id,
                                claimed_generation,
                                "claimed master spawn went stale after pane creation; killed orphan pane"
                            );
                            return Ok(json!({ "pane_id": pane.0, "status": "STALE" }));
                        }
                    }
                } else {
                    record_spawned_master_runtime(&ctx.db, session_id, &pane.0, i64::from(pid))?
                };
                let key = master_monitor_key(session_id, generation);
                monitor::register(key, pidfd);
                spawn_master_pidfd_watch_task(
                    session_id.to_string(),
                    i64::from(pid),
                    generation,
                    task_fd,
                    ctx.db.clone(),
                    ctx.tmux_server.clone(),
                    cmd.to_string(),
                    ctx.state_dir.clone(),
                    ctx.env_state.clone(),
                    ctx.daemon_unit.clone(),
                );
            }
            Err(err) => {
                tracing::warn!(session_id, pid, error = %err, "failed to open master pidfd");
            }
        },
        Err(err) => {
            tracing::warn!(session_id, error = %err, "failed to get master pane pid");
        }
    }

    Ok(json!({ "pane_id": pane.0 }))
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
                "status": session.status,
                "master_pane_id": session.master_pane_id,
                "active_agents": session.active_agents,
                "created_at": session.created_at,
            })
        })
        .collect::<Vec<_>>();
    Ok(json!({ "sessions": sessions }))
}
