use crate::db::Db;
use crate::db::agents::{
    agent_exists, delete_agent, insert_agent, query_agent, query_agent_state,
    update_agent_config_hash, update_agent_state,
};
use crate::db::agents_lifecycle::mark_agent_killed;
use crate::db::events::{insert_event, query_event_by_request_id, query_events_since};
use crate::db::events_progress::record_send_progress;
use crate::db::evidence::{discard_evidence, has_job_evidence_for_path, insert_evidence_record};
use crate::db::jobs::{
    insert_job, mark_dispatched_job_cancelled_if_agent_idle, mark_queued_job_cancelled, query_job,
    request_dispatched_job_cancel, set_job_evidence_requirements,
};
use crate::db::sessions::list_session_summaries;
use crate::db::sessions::{
    create_session, query_session_by_id, set_session_master_pane_id, update_session_config_hash,
};
use crate::db::state_machine::mark_agent_waiting_for_ack;
use crate::db::state_machine_assert::assert_state_to_idle;
use crate::db::system::{
    cascade_kill_session_agents, remove_agent_sandbox_dir_sync, session_agent_ids, system_dump,
};
use crate::error::CcbdError;
use crate::marker::{
    MarkerMatcher, MatchResult, PromptTimerScanContext, TimerKind, parser_registry, registry,
    spawn_marker_timer_task_with_prompt,
};
use crate::monitor;
use crate::monitor::agent_watch::spawn_agent_pidfd_watch_task;
use crate::monitor::master_watch::spawn_master_pidfd_watch_task;
use crate::monitor::session_watch::{spawn_session_watch_task, unit_name_for_session};
use crate::pane_diff::is_meaningful_diff;
use crate::provider::extensions::ExtensionConfig;
use crate::provider::fingerprint::{ConfigFingerprintInput, ConfigRole, compute_config_hash};
use crate::provider::home_layout::{HomeLayoutRole, prepare_home_layout_with_extensions};
use crate::rpc::Ctx;
use crate::sandbox::{path, systemd};
use crate::tmux::scope::{self, ScopePolicy};
use crate::tmux::{TmuxPaneId, agent_session_name, master_session_name};
use nix::sys::stat::Mode;
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::fs;
use std::os::unix::fs::OpenOptionsExt;
use std::process::Command;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;
use tokio::sync::Mutex as AsyncMutex;
use uuid::Uuid;

static SESSION_WINDOW_LOCKS: LazyLock<Mutex<HashMap<String, Arc<AsyncMutex<()>>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

pub(crate) const CAPTURE_SEED_POLL_MS: u64 = 50;
pub(crate) const CAPTURE_SEED_STABILITY_MS: u64 = 500;
pub(crate) const ACK_IDLE_SCAN_REOPEN_DELAY_MS: u64 = 2_000;

fn session_window_lock(session_id: &str) -> Arc<AsyncMutex<()>> {
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

    if session_anchors_enabled(ctx) {
        stop_session_anchor(&unit_name_for_session(session_id));
    }

    let killed = cascade_kill_session_agents(
        ctx.db.clone(),
        session_id.to_string(),
        "SESSION_KILL".to_string(),
    )
    .await?;
    for agent_id in &agent_ids {
        remove_agent_sandbox_dir_sync(&ctx.state_dir, session_id, agent_id);
    }

    for agent_id in &agent_ids {
        if let Some(pane_id) = crate::agent_io::pane_id(agent_id) {
            let _ = ctx.tmux_server.kill_pane(pane_id).await;
        }
        let _ = ctx
            .tmux_server
            .kill_session(agent_session_name(agent_id))
            .await;
    }
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

fn session_anchors_enabled(ctx: &Ctx) -> bool {
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

fn stop_session_anchor(unit_name: &str) {
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
    let auto_shutdown_on_master_exit = params
        .get("auto_shutdown_on_master_exit")
        .and_then(Value::as_bool)
        .unwrap_or(true);
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
            session.project_id.clone(),
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
                let key = crate::monitor::master_watch::monitor_key(session_id);
                monitor::register(key, pidfd);
                spawn_master_pidfd_watch_task(
                    session_id.to_string(),
                    task_fd,
                    ctx.db.clone(),
                    ctx.tmux_server.clone(),
                    auto_shutdown_on_master_exit,
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
                "active_agents": session.active_agents,
                "created_at": session.created_at,
            })
        })
        .collect::<Vec<_>>();
    Ok(json!({ "sessions": sessions }))
}

#[derive(Debug, Deserialize, Serialize)]
struct RealignMasterParams {
    #[serde(default)]
    cmd: String,
    #[serde(default)]
    hooks: HashMap<String, Vec<crate::provider::extensions::HookGroup>>,
    #[serde(default)]
    plugins: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize)]
struct RealignAgentParams {
    agent_id: String,
    provider: String,
    #[serde(default)]
    env: HashMap<String, String>,
    #[serde(default)]
    hooks: HashMap<String, Vec<crate::provider::extensions::HookGroup>>,
    #[serde(default)]
    plugins: Vec<String>,
}

#[derive(Debug)]
struct RunningAgentConfigHash {
    id: String,
    provider: String,
    state: String,
    config_hash: Option<String>,
}

pub async fn handle_session_realign(params: Value, ctx: &Ctx) -> Result<Value, CcbdError> {
    let session_id = required_str(&params, "session_id")?.to_string();
    let force = optional_bool(&params, "force", false)?;
    let skip_master = optional_bool(&params, "_skip_master", false)?;
    let master: RealignMasterParams = serde_json::from_value(
        params
            .get("master")
            .cloned()
            .ok_or_else(|| CcbdError::IpcInvalidRequest("missing field 'master'".into()))?,
    )
    .map_err(|err| CcbdError::IpcInvalidRequest(format!("invalid master: {err}")))?;
    let agents: Vec<RealignAgentParams> = serde_json::from_value(
        params
            .get("agents")
            .cloned()
            .ok_or_else(|| CcbdError::IpcInvalidRequest("missing field 'agents'".into()))?,
    )
    .map_err(|err| CcbdError::IpcInvalidRequest(format!("invalid agents: {err}")))?;

    let session = query_session_by_id(ctx.db.clone(), session_id.clone())
        .await?
        .ok_or_else(|| CcbdError::IpcInvalidRequest(format!("session not found: {session_id}")))?;
    let mut results = Vec::new();
    if skip_master {
        results.push(json!({
            "role": "master",
            "status": "SKIPPED",
            "message": "master diff skipped for agent.realign",
        }));
    } else if session.config_hash.as_deref()
        == Some(
            compute_config_hash(&ConfigFingerprintInput {
                role: ConfigRole::Master { cmd: &master.cmd },
                hooks: &master.hooks,
                plugins: &master.plugins,
            })?
            .as_str(),
        )
    {
        results.push(json!({
            "role": "master",
            "status": "NO_CHANGE",
            "message": "master up to date",
        }));
    } else if force {
        let expected_master_hash = compute_config_hash(&ConfigFingerprintInput {
            role: ConfigRole::Master { cmd: &master.cmd },
            hooks: &master.hooks,
            plugins: &master.plugins,
        })?;
        if session_anchors_enabled(ctx) {
            stop_session_anchor(&unit_name_for_session(&session_id));
        }
        let _spawned = handle_session_spawn_master_pane(
            json!({
                "session_id": session_id.clone(),
                "cmd": master.cmd.clone(),
                "hooks": master.hooks.clone(),
                "plugins": master.plugins.clone(),
            }),
            ctx,
        )
        .await?;
        update_session_config_hash(ctx.db.clone(), session_id.clone(), expected_master_hash)
            .await?;
        results.push(json!({
            "role": "master",
            "status": "REALIGNED",
            "action": "master_realign",
            "message": "master DRIFT force REALIGNED",
        }));
    } else {
        results.push(json!({
            "role": "master",
            "status": "DRIFT",
            "message": "master DRIFT audit-only",
        }));
    }

    let running_agents = running_agent_hashes(ctx, &session_id)?;
    let requested_ids = agents
        .iter()
        .map(|agent| agent.agent_id.clone())
        .collect::<std::collections::HashSet<_>>();

    for agent in &agents {
        let expected_hash = compute_config_hash(&ConfigFingerprintInput {
            role: ConfigRole::Agent {
                provider: &agent.provider,
                env: &agent.env,
            },
            hooks: &agent.hooks,
            plugins: &agent.plugins,
        })?;
        let Some(running) = running_agents
            .iter()
            .find(|running| running.id == agent.agent_id)
        else {
            spawn_realign_agent(ctx, &session_id, agent, &expected_hash, false, false).await?;
            results.push(json!({
                "agent_id": agent.agent_id,
                "status": "NEW",
                "action": "spawned",
                "message": format!("NEW agent {} spawned", agent.agent_id),
            }));
            continue;
        };
        let reason = drift_reason(running, agent);
        if running.state == "CRASHED" {
            delete_agent(ctx.db.clone(), running.id.clone()).await?;
            spawn_realign_agent(ctx, &session_id, agent, &expected_hash, true, true).await?;
            insert_event(
                ctx.db.clone(),
                agent.agent_id.clone(),
                None,
                "drift_realigned".to_string(),
                json!({ "reason": reason }).to_string(),
            )
            .await?;
            results.push(json!({
                "agent_id": agent.agent_id,
                "status": "REALIGNED",
                "event": "drift_realigned",
                "reason": reason,
                "message": format!("DRIFT {reason} REALIGNED drift_realigned"),
            }));
            continue;
        }
        if running.config_hash.as_deref() == Some(expected_hash.as_str()) {
            results.push(json!({
                "agent_id": agent.agent_id,
                "status": "NO_CHANGE",
                "message": "agent up to date",
            }));
            continue;
        }
        if running.state == "BUSY" && !force {
            insert_event(
                ctx.db.clone(),
                running.id.clone(),
                None,
                "drift_skipped".to_string(),
                json!({ "reason": reason, "state": running.state }).to_string(),
            )
            .await?;
            results.push(json!({
                "agent_id": agent.agent_id,
                "status": "SKIPPED_BUSY",
                "reason": reason,
                "message": format!("SKIPPED_BUSY: {reason}"),
            }));
            continue;
        }
        let destructive_reason = if running.state == "BUSY" {
            "DRIFT_FORCE_REALIGN"
        } else {
            "DRIFT_REALIGN"
        };
        let _ = mark_agent_killed(
            ctx.db.clone(),
            running.id.clone(),
            destructive_reason.to_string(),
        )
        .await?;
        delete_agent(ctx.db.clone(), running.id.clone()).await?;
        spawn_realign_agent(ctx, &session_id, agent, &expected_hash, true, false).await?;
        insert_event(
            ctx.db.clone(),
            agent.agent_id.clone(),
            None,
            "drift_realigned".to_string(),
            json!({ "reason": reason }).to_string(),
        )
        .await?;
        results.push(json!({
            "agent_id": agent.agent_id,
            "status": "REALIGNED",
            "event": "drift_realigned",
            "reason": reason,
            "message": format!("DRIFT {reason} REALIGNED drift_realigned"),
        }));
    }

    for running in &running_agents {
        if requested_ids.contains(&running.id) {
            continue;
        }
        if force {
            let _ = mark_agent_killed(
                ctx.db.clone(),
                running.id.clone(),
                "ORPHAN_FORCE_CLEANUP".to_string(),
            )
            .await?;
            insert_event(
                ctx.db.clone(),
                running.id.clone(),
                None,
                "agent_killed".to_string(),
                json!({ "reason": "ORPHAN_FORCE_CLEANUP" }).to_string(),
            )
            .await?;
            results.push(json!({
                "agent_id": running.id,
                "status": "ORPHAN",
                "action": "KILLED",
                "message": format!("ORPHAN {} force cleanup", running.id),
            }));
        } else {
            results.push(json!({
                "agent_id": running.id,
                "status": "ORPHAN",
                "message": format!("ORPHAN {} audit-only", running.id),
            }));
        }
    }

    Ok(json!({ "statuses": results }))
}

pub async fn handle_agent_realign(params: Value, ctx: &Ctx) -> Result<Value, CcbdError> {
    let session_id = required_str(&params, "session_id")?.to_string();
    let force = optional_bool(&params, "force", false)?;
    let agent: RealignAgentParams = serde_json::from_value(params)
        .map_err(|err| CcbdError::IpcInvalidRequest(format!("invalid agent realign: {err}")))?;
    handle_session_realign(
        json!({
            "session_id": session_id,
            "force": force,
            "_skip_master": true,
            "master": {
                "cmd": "",
                "hooks": {},
                "plugins": [],
            },
            "agents": [agent],
        }),
        ctx,
    )
    .await
}

async fn spawn_realign_agent(
    ctx: &Ctx,
    session_id: &str,
    agent: &RealignAgentParams,
    expected_hash: &str,
    killed_before_spawn: bool,
    is_recovery: bool,
) -> Result<(), CcbdError> {
    handle_agent_spawn_with_recovery(
        json!({
            "session_id": session_id,
            "agent_id": agent.agent_id.clone(),
            "provider": agent.provider.clone(),
            "extra_env_vars": agent.env.clone(),
            "hooks": agent.hooks.clone(),
            "plugins": agent.plugins.clone(),
        }),
        ctx,
        is_recovery,
    )
    .await?;
    update_agent_config_hash(
        ctx.db.clone(),
        agent.agent_id.clone(),
        expected_hash.to_string(),
    )
    .await?;
    update_agent_state(ctx.db.clone(), agent.agent_id.clone(), "IDLE".to_string()).await?;
    if killed_before_spawn {
        insert_event(
            ctx.db.clone(),
            agent.agent_id.clone(),
            None,
            "agent_killed".to_string(),
            json!({ "reason": "DRIFT_REALIGN" }).to_string(),
        )
        .await?;
    }
    insert_event(
        ctx.db.clone(),
        agent.agent_id.clone(),
        None,
        "agent_spawned".to_string(),
        json!({
            "reason": if killed_before_spawn { "DRIFT_REALIGN" } else { "NEW" },
            "is_recovery": is_recovery,
        })
        .to_string(),
    )
    .await?;
    Ok(())
}

fn running_agent_hashes(
    ctx: &Ctx,
    session_id: &str,
) -> Result<Vec<RunningAgentConfigHash>, CcbdError> {
    let conn = ctx.db.conn();
    let mut stmt = conn
        .prepare(
            "SELECT id, provider, state, config_hash FROM agents \
             WHERE session_id = ? AND state != 'KILLED' ORDER BY id ASC",
        )
        .map_err(|err| {
            CcbdError::DbConstraintViolation(format!("prepare realign agents: {err}"))
        })?;
    let rows = stmt
        .query_map([session_id], |row| {
            Ok(RunningAgentConfigHash {
                id: row.get(0)?,
                provider: row.get(1)?,
                state: row.get(2)?,
                config_hash: row.get(3)?,
            })
        })
        .map_err(|err| CcbdError::DbConstraintViolation(format!("query realign agents: {err}")))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|err| CcbdError::DbConstraintViolation(format!("collect realign agents: {err}")))
}

fn drift_reason(running: &RunningAgentConfigHash, expected: &RealignAgentParams) -> &'static str {
    if running.provider != expected.provider {
        "provider changed"
    } else if !expected.plugins.is_empty() {
        "plugins changed"
    } else if !expected.hooks.is_empty() {
        "hooks changed"
    } else if !expected.env.is_empty() {
        "env changed"
    } else {
        "config changed"
    }
}

pub async fn handle_agent_spawn(params: Value, ctx: &Ctx) -> Result<Value, CcbdError> {
    handle_agent_spawn_with_recovery(params, ctx, false).await
}

async fn handle_agent_spawn_with_recovery(
    params: Value,
    ctx: &Ctx,
    is_recovery: bool,
) -> Result<Value, CcbdError> {
    let session_id = required_str(&params, "session_id")?;
    let agent_id = required_str(&params, "agent_id")?;
    let provider = required_str(&params, "provider")?;
    let manifest = crate::provider::manifest::get_manifest(provider);
    let extensions = extension_config_from_params(&params)?;
    let extra_env_vars = match params.get("extra_env_vars") {
        Some(value) => {
            serde_json::from_value::<HashMap<String, String>>(value.clone()).map_err(|err| {
                CcbdError::IpcInvalidRequest(format!("invalid extra_env_vars: {err}"))
            })?
        }
        None => HashMap::new(),
    };
    if agent_exists(ctx.db.clone(), agent_id.to_string()).await? {
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
    let mut spawn_env_vars = extra_env_vars;
    if let Some(dir) = sandbox_guard.as_ref().and_then(|guard| guard.path())
        && manifest.requires_home_materialization
    {
        let home_overrides = prepare_home_layout_with_extensions(
            manifest.provider_name,
            dir,
            &agent_cwd,
            HomeLayoutRole::Worker,
            &extensions,
        )?;
        spawn_env_vars.extend(home_overrides.extra_env);
    }
    let cmd = systemd::wrap_command(
        agent_id,
        &session.project_id,
        ctx.tmux_server.socket_name(),
        &ctx.env_state,
        is_recovery,
        ctx.daemon_unit.as_deref(),
        &manifest,
        &spawn_env_vars,
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
        return Err(err);
    }
    let config_hash = compute_config_hash(&ConfigFingerprintInput {
        role: ConfigRole::Agent {
            provider,
            env: &spawn_env_vars,
        },
        hooks: &extensions.hooks,
        plugins: &extensions.plugins,
    })?;
    update_agent_config_hash(ctx.db.clone(), agent_id.to_string(), config_hash).await?;

    let pidfd_for_task = match pidfd.try_clone() {
        Ok(fd) => fd,
        Err(err) => {
            cleanup_spawn_resources(&tmux, Some(pane_id.clone()), Some(fifo_file), &fifo_path)
                .await;
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
        idle_scan_enabled,
    );
    let _sandbox_dir = sandbox_guard.map(path::SandboxDirGuard::release);

    Ok(json!({ "state": "SPAWNING", "pid": pid, "pane_id": response_pane_id }))
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

pub async fn handle_evidence_insert(params: Value, ctx: &Ctx) -> Result<Value, CcbdError> {
    let agent_id = required_str(&params, "agent_id")?;
    let job_id = required_str(&params, "job_id")?;
    let evidence_type = required_str(&params, "evidence_type")?;
    let subject_path = required_str(&params, "subject_path")?;
    let payload = params.get("payload").cloned().unwrap_or_else(|| json!({}));

    let evidence_id = insert_evidence_record(
        ctx.db.clone(),
        agent_id.to_string(),
        Some(job_id.to_string()),
        evidence_type.to_string(),
        Some(subject_path.to_string()),
        payload,
    )
    .await?;

    Ok(json!({
        "evidence_id": evidence_id,
        "recorded": true,
    }))
}

pub async fn handle_job_has_evidence(params: Value, ctx: &Ctx) -> Result<Value, CcbdError> {
    let job_id = required_str(&params, "job_id")?;
    let evidence_type = required_str(&params, "evidence_type")?;
    let subject_path = required_str(&params, "subject_path")?;

    let has_evidence = has_job_evidence_for_path(
        ctx.db.clone(),
        job_id.to_string(),
        evidence_type.to_string(),
        subject_path.to_string(),
    )
    .await?;

    Ok(json!({ "has_evidence": has_evidence }))
}

pub async fn handle_job_mark_requires_evidence(
    params: Value,
    ctx: &Ctx,
) -> Result<Value, CcbdError> {
    let job_id = required_str(&params, "job_id")?;
    let job = query_job(ctx.db.clone(), job_id.to_string())
        .await?
        .ok_or_else(|| CcbdError::IpcInvalidRequest(format!("job_id not found: {job_id}")))?;
    set_job_evidence_requirements(
        ctx.db.clone(),
        job_id.to_string(),
        true,
        job.requires_test_evidence,
    )
    .await?;

    Ok(json!({
        "job_id": job_id,
        "requires_physical_evidence": true,
        "requires_test_evidence": job.requires_test_evidence,
    }))
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
    remove_agent_sandbox_dir_sync(&ctx.state_dir, &agent.session_id, agent_id);

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
        spawn_new_capture_seed(
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

pub async fn handle_agent_resolve_prompt(params: Value, ctx: &Ctx) -> Result<Value, CcbdError> {
    let agent_id = required_str(&params, "agent_id")?.to_string();
    let action_value = params
        .get("action")
        .cloned()
        .ok_or_else(|| CcbdError::IpcInvalidRequest("missing field 'action'".to_string()))?;
    let save_to_kb = optional_bool(&params, "save_to_kb", false)?;
    tracing::info!(
        agent_id,
        save_to_kb,
        "agent.resolve_prompt request received"
    );

    let actions = crate::prompt_handler::resolve::normalize_action_value(action_value)?;
    let agent = query_agent(ctx.db.clone(), agent_id.clone())
        .await?
        .ok_or_else(|| CcbdError::AgentNotFound(agent_id.clone()))?;
    let pane_id = crate::agent_io::pane_id(&agent_id)
        .ok_or_else(|| CcbdError::PtyIoError(format!("tmux pane not registered for {agent_id}")))?;
    let io = crate::prompt_handler::runner::TmuxPromptIo::new((*ctx.tmux_server).clone());
    let result = crate::prompt_handler::resolve::resolve_prompt_with_io(
        crate::prompt_handler::resolve::ResolvePromptRequest {
            db: ctx.db.clone(),
            agent_id: agent_id.clone(),
            pane_id,
            provider: agent.provider,
            kb_path: ctx.state_dir.join("prompt-cases.json"),
            actions,
            save_to_kb,
        },
        &io,
    )
    .await?;

    if result.resolved_state == crate::db::state_machine::STATE_IDLE {
        crate::orchestrator::wake_up();
    }

    Ok(json!({
        "status": "ok",
        "resolved_state": result.resolved_state,
        "saved_to_kb": result.saved_to_kb,
        "case_id": result.case_id,
        "action_sent": result.action_labels,
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

pub async fn handle_system_shutdown(_params: Value, _ctx: &Ctx) -> Result<Value, CcbdError> {
    tokio::spawn(async {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        unsafe { libc::kill(libc::getpid(), libc::SIGTERM) };
    });
    Ok(serde_json::json!({"status": "shutting_down"}))
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

fn extension_config_from_params(params: &Value) -> Result<ExtensionConfig, CcbdError> {
    Ok(ExtensionConfig {
        hooks: match params.get("hooks") {
            Some(value) => serde_json::from_value(value.clone())
                .map_err(|err| CcbdError::IpcInvalidRequest(format!("invalid hooks: {err}")))?,
            None => Default::default(),
        },
        plugins: match params.get("plugins") {
            Some(value) => serde_json::from_value(value.clone())
                .map_err(|err| CcbdError::IpcInvalidRequest(format!("invalid plugins: {err}")))?,
            None => Default::default(),
        },
    })
}

fn optional_bool(params: &Value, field: &str, default: bool) -> Result<bool, CcbdError> {
    match params.get(field) {
        Some(value) => value
            .as_bool()
            .ok_or_else(|| CcbdError::IpcInvalidRequest(format!("invalid field '{field}'"))),
        None => Ok(default),
    }
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

pub(crate) fn spawn_new_capture_seed(
    db: crate::db::Db,
    tmux: Arc<crate::tmux::TmuxServer>,
    agent_id: String,
    provider: String,
    state_dir: std::path::PathBuf,
    baseline: String,
    matcher: Arc<MarkerMatcher>,
) {
    tokio::spawn(async move {
        let allow_direct_idle =
            matcher.mode() != crate::provider::manifest::IdleDetectionMode::ObservedStability;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        let mut processed_len = 0_usize;
        let stability = Duration::from_millis(CAPTURE_SEED_STABILITY_MS);
        let ack_started_at = tokio::time::Instant::now();
        let mut busy_marked = false;
        let mut last_meaningful_diff_at: Option<tokio::time::Instant> = None;
        while tokio::time::Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(CAPTURE_SEED_POLL_MS)).await;
            let Some(pane_id) = crate::agent_io::pane_id(&agent_id) else {
                if let Err(err) =
                    fallback_ack_to_crashed(db.clone(), &agent_id, "pane_unregistered_during_ack")
                        .await
                {
                    tracing::warn!(agent_id = %agent_id, error = %err, "failed to mark ACK fallback CRASHED after pane unregister");
                }
                return;
            };
            let Some(parser_handle) = parser_registry::get(&agent_id) else {
                if let Err(err) =
                    fallback_ack_to_crashed(db.clone(), &agent_id, "reader_unregistered_during_ack")
                        .await
                {
                    tracing::warn!(agent_id = %agent_id, error = %err, "failed to mark ACK fallback CRASHED after reader unregister");
                }
                return;
            };
            let Ok(capture) = tmux.capture_pane(pane_id.clone()).await else {
                if let Err(err) =
                    fallback_ack_to_stuck(db.clone(), &agent_id, "tmux_capture_failed_during_ack")
                        .await
                {
                    tracing::warn!(agent_id = %agent_id, error = %err, "failed to mark ACK fallback STUCK after capture failure");
                }
                return;
            };
            let now = tokio::time::Instant::now();
            if !is_meaningful_diff(&baseline, &capture) {
                if !busy_marked && now.duration_since(ack_started_at) >= stability {
                    if let Err(err) = crate::db::state_machine::transit_agent_state(
                        db.clone(),
                        agent_id.clone(),
                        vec![crate::db::state_machine::STATE_WAITING_FOR_ACK.to_string()],
                        crate::db::state_machine::STATE_BUSY.to_string(),
                        Some("ACK_STABILITY_WINDOW".to_string()),
                    )
                    .await
                    {
                        tracing::warn!(agent_id = %agent_id, error = %err, "failed to mark agent BUSY after ACK stability window");
                    }
                    busy_marked = true;
                }
                if last_meaningful_diff_at
                    .is_some_and(|last_change| now.duration_since(last_change) >= stability)
                {
                    return;
                }
                continue;
            }
            let first_meaningful_diff = last_meaningful_diff_at.is_none();
            last_meaningful_diff_at = Some(now);
            if first_meaningful_diff && !busy_marked {
                if crate::prompt_handler::integration::is_prompt_handling_provider(&provider) {
                    match crate::prompt_handler::integration::scan_prompt_and_apply_outcome(
                        crate::prompt_handler::integration::PromptScanRequest {
                            db: db.clone(),
                            agent_id: agent_id.clone(),
                            provider: provider.clone(),
                            pane_id: pane_id.clone(),
                            tmux: tmux.clone(),
                            state_dir: state_dir.clone(),
                            marker_matcher: matcher.clone(),
                            max_depth: 3,
                        },
                    )
                    .await
                    {
                        Ok(crate::prompt_handler::integration::PromptScanDisposition::Handled {
                            depth,
                        }) => {
                            tracing::info!(
                                agent_id = %agent_id,
                                depth,
                                "prompt scan auto-handled prompt during ACK visual diff; continuing ACK loop"
                            );
                            processed_len = 0;
                            last_meaningful_diff_at = None;
                            continue;
                        }
                        Ok(crate::prompt_handler::integration::PromptScanDisposition::Pending {
                            depth,
                            block_reason,
                        }) => {
                            tracing::info!(
                                agent_id = %agent_id,
                                depth,
                                block_reason,
                                "prompt scan moved agent to PROMPT_PENDING during ACK visual diff"
                            );
                            if let Some(handle) = registry::take(&agent_id) {
                                let _ = handle.cancel_tx.send(());
                            }
                            crate::orchestrator::wake_up();
                            return;
                        }
                        Ok(crate::prompt_handler::integration::PromptScanDisposition::Deferred {
                            depth,
                            block_reason,
                        }) => {
                            tracing::info!(
                                agent_id = %agent_id,
                                depth,
                                block_reason,
                                "prompt scan deferred during ACK visual diff; resuming ACK handling"
                            );
                        }
                        Ok(crate::prompt_handler::integration::PromptScanDisposition::NoActionNeeded {
                            ..
                        }) => {
                            tracing::info!(
                                agent_id = %agent_id,
                                "prompt scan found no prompt during ACK visual diff; resuming ACK handling"
                            );
                        }
                        Err(err) => {
                            tracing::warn!(
                                agent_id = %agent_id,
                                reason = %err,
                                impact = "prompt scan failed; preserving existing ACK visual diff behavior",
                                "prompt scan failed during ACK visual diff"
                            );
                        }
                    }
                }
                if let Err(err) = crate::db::state_machine::transit_agent_state(
                    db.clone(),
                    agent_id.clone(),
                    vec![crate::db::state_machine::STATE_WAITING_FOR_ACK.to_string()],
                    crate::db::state_machine::STATE_BUSY.to_string(),
                    Some("ACK_VISUAL_DIFF".to_string()),
                )
                .await
                {
                    tracing::warn!(agent_id = %agent_id, error = %err, "failed to mark agent BUSY after ACK visual diff");
                }
                tokio::time::sleep(Duration::from_millis(ACK_IDLE_SCAN_REOPEN_DELAY_MS)).await;
                crate::agent_io::set_idle_scan_enabled(&agent_id, true);
                busy_marked = true;
                let matched_after_reopen = match parser_handle.lock() {
                    Ok(parser) => matcher.scan(&parser),
                    Err(err) => {
                        tracing::warn!(agent_id = %agent_id, error = %err, "parser mutex poisoned while reopening idle scan after ACK");
                        MatchResult::NoMatch
                    }
                };
                if matched_after_reopen == MatchResult::Matched {
                    match crate::db::state_machine::mark_agent_idle_matched(
                        db.clone(),
                        agent_id.clone(),
                    )
                    .await
                    {
                        Ok((changes, affected_job)) if changes > 0 => {
                            // R-2 (mvp12): notify hoisted from state_machine wrapper for clearer dispatcher boundary.
                            if let Some(job_id) = affected_job {
                                crate::orchestrator::pubsub::notify_job_update(&job_id);
                            }
                            crate::orchestrator::wake_up();
                        }
                        Ok(_) => {}
                        Err(err) => {
                            tracing::warn!(agent_id = %agent_id, error = %err, "failed to mark agent IDLE after reopening idle scan");
                        }
                    }
                    if let Some(handle) = registry::take(&agent_id) {
                        let _ = handle.cancel_tx.send(());
                    }
                    return;
                }
                continue;
            }
            if !capture.starts_with(&baseline) && !capture.contains(&baseline) {
                let mut parser = vt100::Parser::new(200, 200, 0);
                parser.process(capture.as_bytes());
                if allow_direct_idle && capture_seed_matches(&parser, &matcher) {
                    match crate::db::state_machine::mark_agent_idle_matched(
                        db.clone(),
                        agent_id.clone(),
                    )
                    .await
                    {
                        Ok((changes, affected_job)) if changes > 0 => {
                            // R-2 (mvp12): notify hoisted from state_machine wrapper for clearer dispatcher boundary.
                            if let Some(job_id) = affected_job {
                                crate::orchestrator::pubsub::notify_job_update(&job_id);
                            }
                            crate::orchestrator::wake_up();
                        }
                        Ok(_) => {}
                        Err(err) => {
                            tracing::warn!(agent_id = %agent_id, error = %err, "failed to mark agent IDLE from replacement tmux capture seed");
                        }
                    }
                    if let Some(handle) = registry::take(&agent_id) {
                        let _ = handle.cancel_tx.send(());
                    }
                    return;
                }
                continue;
            }
            let Some(suffix) = capture.strip_prefix(&baseline) else {
                if let Err(err) = fallback_ack_to_stuck(
                    db.clone(),
                    &agent_id,
                    "capture_baseline_mismatch_during_ack",
                )
                .await
                {
                    tracing::warn!(agent_id = %agent_id, error = %err, "failed to mark ACK fallback STUCK after baseline mismatch");
                }
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
                match crate::db::state_machine::mark_agent_idle_matched(
                    db.clone(),
                    agent_id.clone(),
                )
                .await
                {
                    Ok((changes, affected_job)) if changes > 0 => {
                        // R-2 (mvp12): notify hoisted from state_machine wrapper for clearer dispatcher boundary.
                        if let Some(job_id) = affected_job {
                            crate::orchestrator::pubsub::notify_job_update(&job_id);
                        }
                        crate::orchestrator::wake_up();
                    }
                    Ok(_) => {}
                    Err(err) => {
                        tracing::warn!(agent_id = %agent_id, error = %err, "failed to mark agent IDLE from new tmux capture seed");
                    }
                }
                if let Some(handle) = registry::take(&agent_id) {
                    let _ = handle.cancel_tx.send(());
                }
                return;
            }
        }
        if let Err(err) = fallback_ack_to_stuck(db, &agent_id, "ack_deadline_timeout").await {
            tracing::warn!(agent_id = %agent_id, error = %err, "failed to mark ACK fallback STUCK after capture seed deadline");
        }
    });
}

#[doc(hidden)]
pub async fn fallback_ack_to_stuck(
    db: crate::db::Db,
    agent_id: &str,
    reason: &str,
) -> Result<usize, CcbdError> {
    let agent_id = agent_id.to_string();
    let reason = reason.to_string();
    crate::db::common::spawn_db("handlers::fallback_ack_to_stuck", move || {
        let mut conn = db.conn();
        let tx = conn
            .transaction()
            .map_err(|err| crate::db::common::map_db_error("begin ACK stuck fallback", err))?;
        let current = tx
            .query_row(
                "SELECT state, state_version FROM agents WHERE id = ?",
                rusqlite::params![agent_id.as_str()],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
            )
            .optional()
            .map_err(|err| crate::db::common::map_db_error("query ACK fallback state", err))?;
        let Some((current_state, state_version)) = current else {
            tx.rollback()
                .map_err(|err| crate::db::common::map_db_error("rollback missing ACK fallback", err))?;
            return Ok(0);
        };
        if current_state != crate::db::state_machine::STATE_WAITING_FOR_ACK {
            tx.rollback()
                .map_err(|err| crate::db::common::map_db_error("rollback ignored ACK fallback", err))?;
            return Ok(0);
        }

        let changes = tx
            .execute(
                "UPDATE agents SET state = 'STUCK', state_version = state_version + 1, updated_at = unixepoch() WHERE id = ? AND state = 'WAITING_FOR_ACK' AND state_version = ?",
                rusqlite::params![agent_id.as_str(), state_version],
            )
            .map_err(|err| crate::db::common::map_db_error("mark ACK fallback stuck", err))?;
        if changes == 1 {
            let payload = json!({
                "from": current_state,
                "to": crate::db::state_machine::STATE_STUCK,
                "reason": reason,
            })
            .to_string();
            tx.execute(
                "INSERT INTO events (agent_id, request_id, event_type, payload) VALUES (?, NULL, 'state_change', ?)",
                rusqlite::params![agent_id.as_str(), payload],
            )
            .map_err(|err| crate::db::common::map_db_error("insert ACK fallback stuck state_change", err))?;
        }
        tx.commit()
            .map_err(|err| crate::db::common::map_db_error("commit ACK stuck fallback", err))?;
        Ok(changes)
    })
    .await
}

#[doc(hidden)]
pub async fn fallback_ack_to_crashed(
    db: crate::db::Db,
    agent_id: &str,
    reason: &str,
) -> Result<usize, CcbdError> {
    let state = crate::db::agents::query_agent_state(db.clone(), agent_id.to_string()).await?;
    if state
        .as_ref()
        .is_none_or(|(state, _)| state != crate::db::state_machine::STATE_WAITING_FOR_ACK)
    {
        return Ok(0);
    }
    crate::db::agents_lifecycle::mark_agent_crashed_with_reason(
        db,
        agent_id.to_string(),
        None,
        reason.to_string(),
    )
    .await
}

fn capture_seed_matches(parser: &vt100::Parser, matcher: &MarkerMatcher) -> bool {
    if matcher.mode() == crate::provider::manifest::IdleDetectionMode::ObservedStability {
        return false;
    }
    matcher.scan(parser) == MatchResult::Matched
}

#[cfg(test)]
mod tests {
    use super::{
        CAPTURE_SEED_POLL_MS, CAPTURE_SEED_STABILITY_MS, capture_seed_matches,
        handle_agent_assert_state, handle_agent_discard_evidence, handle_agent_kill,
        handle_agent_read, handle_agent_send, handle_agent_spawn, handle_agent_watch,
        handle_job_submit, handle_job_wait, handle_session_create, handle_session_kill,
        handle_session_spawn_master_pane, handle_system_dump,
    };
    use crate::db;
    use crate::db::agents::{insert_agent_sync, query_agent_state_sync, update_agent_state_sync};
    use crate::db::jobs::{insert_job_sync, query_job_sync};
    use crate::db::sessions::insert_session_sync;
    use crate::db::state_machine::mark_agent_unknown_sync;
    use crate::error::CcbdError;
    use crate::marker::MarkerMatcher;
    use crate::provider::manifest::{IdleDetectionMode, InitProbeKind, ProviderManifest};
    use crate::rpc::Ctx;
    use crate::sandbox::EnvState;
    use crate::tmux::TmuxServer;
    use serde_json::json;
    use std::os::unix::fs::PermissionsExt;
    use std::process::Command;
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
                systemd_run_available: false,
                unsafe_no_sandbox: true,
                under_systemd: false,
            },
            daemon_unit: None,
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
            command: &["fake"],
            resume_args: &[],
            env_passthrough: &[],
            injected_env_vars: &[],
            readiness_timeout_s: 1,
            requires_home_materialization: false,
            init_probe: InitProbeKind::Bash,
            idle_detection_mode: IdleDetectionMode::ObservedStability,
            stability_ms: 300,
            idle_anti_pattern: "",
        };
        let matcher = MarkerMatcher::from_manifest(&manifest);
        let mut parser = vt100::Parser::new(200, 200, 0);
        parser.process(b"old screen\nREADY_PROMPT\nnew output\n");

        assert!(!capture_seed_matches(&parser, &matcher));
    }

    #[test]
    fn test_capture_seed_does_not_match_historical_prompt() {
        let matcher = MarkerMatcher::default();
        let mut parser = vt100::Parser::new(200, 200, 0);
        parser.process(b"$ first command\noutput\n$ sleep 3\n");

        assert!(!capture_seed_matches(&parser, &matcher));
    }

    #[test]
    fn test_capture_seed_poll_and_stability_windows() {
        assert_eq!(CAPTURE_SEED_POLL_MS, 50);
        assert_eq!(CAPTURE_SEED_STABILITY_MS, 500);
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

    fn restore_env(key: &str, old_value: Option<std::ffi::OsString>) {
        unsafe {
            if let Some(old_value) = old_value {
                std::env::set_var(key, old_value);
            } else {
                std::env::remove_var(key);
            }
        }
    }

    fn insert_session_with_temp_dir(
        ctx: &Ctx,
        session_id: &str,
        project_id: &str,
    ) -> tempfile::TempDir {
        let project_dir = tempfile::TempDir::new().unwrap();
        let conn = ctx.db.conn();
        insert_session_sync(
            &conn,
            session_id,
            project_id,
            project_dir.path().to_string_lossy().as_ref(),
        )
        .unwrap();
        project_dir
    }

    fn tmux_pane_current_path(ctx: &Ctx, pane_id: &str) -> String {
        let output = Command::new("tmux")
            .args([
                "-L",
                ctx.tmux_server.socket_name(),
                "display-message",
                "-p",
                "-t",
                pane_id,
                "#{pane_current_path}",
            ])
            .output()
            .expect("tmux display-message should run");
        assert!(
            output.status.success(),
            "tmux display-message failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }

    async fn wait_for_file(path: &std::path::Path, timeout: Duration) -> String {
        let deadline = std::time::Instant::now() + timeout;
        while std::time::Instant::now() < deadline {
            if let Ok(data) = std::fs::read_to_string(path)
                && !data.is_empty()
            {
                return data;
            }
            sleep_ms(100).await;
        }
        panic!("{} was not written within {timeout:?}", path.display());
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
    #[serial_test::serial(global_env)]
    async fn test_handle_session_create_no_anchor_unit() {
        let mut ctx = test_ctx();
        ctx.env_state.systemd_run_available = true;
        ctx.env_state.unsafe_no_sandbox = false;
        let bin_dir = tempfile::TempDir::new().unwrap();
        let log_file = tempfile::NamedTempFile::new().unwrap();
        let systemd_run = bin_dir.path().join("systemd-run");
        std::fs::write(
            &systemd_run,
            format!(
                "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\nexit 0\n",
                log_file.path().display()
            ),
        )
        .unwrap();
        let mut perms = std::fs::metadata(&systemd_run).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&systemd_run, perms).unwrap();
        let old_path = std::env::var_os("PATH");
        let new_path = format!(
            "{}:{}",
            bin_dir.path().display(),
            old_path
                .as_deref()
                .map(|path| path.to_string_lossy())
                .unwrap_or_default()
        );
        unsafe {
            std::env::set_var("PATH", new_path);
        }

        let result = handle_session_create(
            json!({
                "project_id": "p1",
                "absolute_path": "/tmp/t4-session-no-anchor",
            }),
            &ctx,
        )
        .await
        .unwrap();

        restore_env("PATH", old_path);
        assert!(result["session_id"].as_str().unwrap().starts_with("sess_"));
        let log = std::fs::read_to_string(log_file.path()).unwrap_or_default();
        assert!(
            !log.contains("ahd-session-"),
            "session.create should not create systemd anchor unit, log: {log}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_handle_session_spawn_master_pane_returns_pane_id() {
        let ctx = test_ctx();
        let master_dir = tempfile::TempDir::new().unwrap();
        let master_path = master_dir.path().to_string_lossy().to_string();
        {
            let conn = ctx.db.conn();
            insert_session_sync(&conn, "s_master", "p_master", &master_path).unwrap();
        }

        let result = handle_session_spawn_master_pane(
            json!({
                "session_id": "s_master",
                "cmd": "bash",
            }),
            &ctx,
        )
        .await
        .unwrap();

        assert!(result["pane_id"].as_str().unwrap().starts_with('%'));
        let pane_path = Command::new("tmux")
            .args([
                "-L",
                ctx.tmux_server.socket_name(),
                "display-message",
                "-p",
                "-t",
                result["pane_id"].as_str().unwrap(),
                "#{pane_current_path}",
            ])
            .output()
            .expect("tmux display-message should run");
        assert!(
            pane_path.status.success(),
            "tmux display-message failed: {}",
            String::from_utf8_lossy(&pane_path.stderr)
        );
        assert_eq!(
            String::from_utf8_lossy(&pane_path.stdout).trim(),
            master_path
        );
        let stored: String = ctx
            .db
            .conn()
            .query_row(
                "SELECT master_pane_id FROM sessions WHERE id = 's_master'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(stored, result["pane_id"].as_str().unwrap());
        let _ = Command::new("tmux")
            .args(["-L", ctx.tmux_server.socket_name(), "kill-server"])
            .output();
    }

    #[tokio::test(flavor = "multi_thread")]
    #[serial_test::serial(global_env)]
    async fn test_handle_session_spawn_master_pane_uses_isolated_claude_home() {
        let mut ctx = test_ctx();
        ctx.env_state.unsafe_no_sandbox = false;
        let host_home = tempfile::TempDir::new().unwrap();
        let cache_home = tempfile::TempDir::new().unwrap();
        let project_dir = tempfile::TempDir::new().unwrap();
        let env_file = tempfile::NamedTempFile::new().unwrap();
        let env_path = env_file.path().to_path_buf();
        let old_home = std::env::var_os("HOME");
        let old_cache = std::env::var_os("XDG_CACHE_HOME");
        unsafe {
            std::env::set_var("HOME", host_home.path());
            std::env::set_var("XDG_CACHE_HOME", cache_home.path());
        }
        {
            let conn = ctx.db.conn();
            insert_session_sync(
                &conn,
                "s_master_isolated",
                "p_master_isolated",
                project_dir.path().to_string_lossy().as_ref(),
            )
            .unwrap();
        }

        let cmd = format!(
            "printf 'HOME=%s\\nCLAUDE_CONFIG_DIR=%s\\n' \"$HOME\" \"$CLAUDE_CONFIG_DIR\" > {}; sleep 30",
            env_path.display()
        );
        let result = handle_session_spawn_master_pane(
            json!({
                "session_id": "s_master_isolated",
                "cmd": cmd,
            }),
            &ctx,
        )
        .await
        .unwrap();
        let env_dump = wait_for_file(&env_path, Duration::from_secs(5)).await;
        let _ = handle_session_kill(json!({ "session_id": "s_master_isolated" }), &ctx).await;
        let _ = Command::new("tmux")
            .args(["-L", ctx.tmux_server.socket_name(), "kill-server"])
            .output();
        restore_env("HOME", old_home);
        restore_env("XDG_CACHE_HOME", old_cache);

        assert!(result["pane_id"].as_str().unwrap().starts_with('%'));
        let home = env_dump
            .lines()
            .find_map(|line| line.strip_prefix("HOME="))
            .unwrap();
        let claude_config_dir = env_dump
            .lines()
            .find_map(|line| line.strip_prefix("CLAUDE_CONFIG_DIR="))
            .unwrap();
        assert_ne!(home, host_home.path().to_string_lossy());
        assert!(
            home.starts_with(&cache_home.path().join("ah/sandboxes").display().to_string()),
            "master HOME must be an ah sandbox, env dump: {env_dump}"
        );
        assert_eq!(
            claude_config_dir,
            format!("{home}/.claude"),
            "master CLAUDE_CONFIG_DIR must point at its sandbox .claude, env dump: {env_dump}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_session_kill_cleans_up_master_pane() {
        let ctx = test_ctx();
        {
            let conn = ctx.db.conn();
            insert_session_sync(&conn, "s_kill_master", "p_kill_master", "/tmp/kill-master")
                .unwrap();
            conn.execute(
                "UPDATE sessions SET master_pane_id = ? WHERE id = ?",
                ["%999", "s_kill_master"],
            )
            .unwrap();
        }

        let result = handle_session_kill(json!({ "session_id": "s_kill_master" }), &ctx)
            .await
            .unwrap();

        assert_eq!(result["state"], "KILLED");
        assert_eq!(result["master_pane_killed"], true);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_handle_agent_spawn_returns_idle_and_inserts_pid() {
        let ctx = test_ctx();
        let project_dir = insert_session_with_temp_dir(&ctx, "s1", "p1");
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
        let pane_id = result["pane_id"].as_str().unwrap();
        assert!(pane_id.starts_with('%'));
        assert_eq!(
            tmux_pane_current_path(&ctx, pane_id),
            project_dir.path().to_string_lossy().as_ref()
        );
        assert!(pid.is_some());
        wait_for_state(&ctx, &agent_id, "IDLE").await;
        cleanup_agent(&ctx, &agent_id).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_handle_agent_spawn_ignores_layout_hint_route() {
        let ctx = test_ctx();
        let _project_dir = insert_session_with_temp_dir(&ctx, "s_layout_ignored", "p_layout");
        let agent_id = format!("ag_layout_{}", Uuid::new_v4());
        let result = handle_agent_spawn(
            json!({
                "session_id": "s_layout_ignored",
                "agent_id": agent_id,
                "provider": "bash",
                "layout_parent_pane_id": "%999999",
                "layout_direction": "right",
                "layout_percent": 40,
            }),
            &ctx,
        )
        .await
        .unwrap();

        assert_eq!(result["state"], "SPAWNING");
        assert!(result["pane_id"].as_str().unwrap().starts_with('%'));
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
        let _project_dir = insert_session_with_temp_dir(&ctx, "s1", "p1");
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
        let _project_dir = insert_session_with_temp_dir(&ctx, "s1", "p1");
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
        let _project_dir = insert_session_with_temp_dir(&ctx, "s1", "p1");
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
    async fn test_handle_agent_kill_removes_sandbox_dir() {
        let ctx = test_ctx();
        let agent_id = format!("ag_kill_sandbox_{}", Uuid::new_v4());
        let sandbox_dir = ctx
            .state_dir
            .join("sandboxes")
            .join("s_kill_sandbox")
            .join(&agent_id);
        std::fs::create_dir_all(&sandbox_dir).unwrap();
        {
            let conn = ctx.db.conn();
            insert_session_sync(&conn, "s_kill_sandbox", "p1", "/tmp/t4-kill-sandbox").unwrap();
            insert_agent_sync(&conn, &agent_id, "s_kill_sandbox", "bash", "IDLE", None).unwrap();
        }

        handle_agent_kill(json!({ "agent_id": agent_id }), &ctx)
            .await
            .unwrap();

        assert!(!sandbox_dir.exists());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_handle_session_kill_removes_all_agent_sandbox_dirs() {
        let ctx = test_ctx();
        let agent_a = format!("ag_session_kill_a_{}", Uuid::new_v4());
        let agent_b = format!("ag_session_kill_b_{}", Uuid::new_v4());
        let sandbox_a = ctx
            .state_dir
            .join("sandboxes")
            .join("s_session_kill_sandbox")
            .join(&agent_a);
        let sandbox_b = ctx
            .state_dir
            .join("sandboxes")
            .join("s_session_kill_sandbox")
            .join(&agent_b);
        std::fs::create_dir_all(&sandbox_a).unwrap();
        std::fs::create_dir_all(&sandbox_b).unwrap();
        {
            let conn = ctx.db.conn();
            insert_session_sync(
                &conn,
                "s_session_kill_sandbox",
                "p1",
                "/tmp/t4-session-kill-sandbox",
            )
            .unwrap();
            insert_agent_sync(
                &conn,
                &agent_a,
                "s_session_kill_sandbox",
                "bash",
                "IDLE",
                None,
            )
            .unwrap();
            insert_agent_sync(
                &conn,
                &agent_b,
                "s_session_kill_sandbox",
                "bash",
                "BUSY",
                None,
            )
            .unwrap();
        }

        handle_session_kill(json!({ "session_id": "s_session_kill_sandbox" }), &ctx)
            .await
            .unwrap();

        assert!(!sandbox_a.exists());
        assert!(!sandbox_b.exists());
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
    async fn test_handle_agent_send_rejects_unknown_with_new_request() {
        let ctx = test_ctx();
        let agent_id = format!("ag_unknown_send_{}", Uuid::new_v4());
        let _project_dir = insert_session_with_temp_dir(&ctx, "s_unknown", "p_unknown");
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

        let err = handle_agent_send(
            json!({
                "agent_id": agent_id,
                "text": "echo recover\n",
                "request_id": "recover-1",
            }),
            &ctx,
        )
        .await
        .unwrap_err();

        assert!(matches!(
            err,
            CcbdError::AgentWrongState { current_state } if current_state == "UNKNOWN"
        ));
        assert_eq!(command_count(&ctx, &agent_id), 0);
        let _ = handle_agent_kill(json!({ "agent_id": agent_id }), &ctx).await;
        let _ = crate::marker::registry::take(&agent_id);
    }
}
