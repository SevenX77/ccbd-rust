use super::params::{extension_config_from_params, required_str};
use crate::db::master_cutovers::{
    MasterCutoverClaim, MasterCutoverUpdate, claim_master_cutover, get_master_cutover,
    mark_master_cutover_ack_ready, update_master_cutover_spawn_metadata,
    update_master_cutover_state,
};
use crate::db::schema::Session;
use crate::db::sessions::{
    create_session, list_session_summaries, master_tell_begin, master_tell_failed,
    query_session_by_id, set_session_master_pane_id, update_session_config_hash,
    update_session_master_cmd,
};
use crate::db::system::{
    cascade_kill_session_agents, cascade_kill_session_agents_for_daemon,
    remove_agent_sandbox_dir_sync, session_agent_ids,
};
use crate::error::CcbdError;
use crate::master_cutover::{
    HandoffBundleInput, seed_claude_project_conversation, write_handoff_bundle,
};
use crate::master_revival::{
    mark_session_intentional_killed, master_monitor_key, record_spawned_master_runtime,
    record_spawned_master_runtime_after_claim,
};
use crate::monitor;
use crate::monitor::master_watch::spawn_master_pidfd_watch_task;
use crate::monitor::session_watch::{spawn_session_watch_task, unit_name_for_session};
use crate::provider::bundles::{BundleRole, resolve_bundles_for_provider};
use crate::provider::extensions::ExtensionConfig;
use crate::provider::fingerprint::{ConfigFingerprintInput, ConfigRole, compute_config_hash};
use crate::provider::home_layout::{
    HomeLayoutRole, HookPushContext, prepare_home_layout_with_extensions_for_slot,
};
use crate::rpc::Ctx;
use crate::rpc::handlers::{RealignAgentParams, spawn_realign_agent};
use crate::sandbox::{path, systemd};
use crate::tmux::scope::{self, ScopePolicy};
use crate::tmux::{
    TmuxPaneId, TmuxWindowSize, agent_session_name, master_session_name, sanitize_tmux_name,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::process::Command;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;
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

#[derive(Debug, Clone)]
pub(crate) struct SpawnMasterPaneParams {
    pub(crate) session_id: String,
    pub(crate) cmd: String,
    pub(crate) tmux_window_size: TmuxWindowSize,
    pub(crate) extensions: ExtensionConfig,
    pub(crate) extra_env: HashMap<String, String>,
    pub(crate) claimed_master_generation: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SpawnMasterPaneOutcome {
    pub(crate) pane_id: String,
    pub(crate) home_root: Option<PathBuf>,
    pub(crate) new_pid: Option<i64>,
    pub(crate) generation: Option<i64>,
}

#[derive(Debug, Clone)]
struct MasterPanePlan {
    session: Session,
    master_cwd: PathBuf,
    master_env_vars: HashMap<String, String>,
    home_root: Option<PathBuf>,
    extensions: ExtensionConfig,
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
    let tmux_window_size = parse_tmux_window_size(&params)?;
    let extensions = extension_config_from_params(&params)?;
    let extra_env = parse_extra_env(&params)?;
    let outcome = spawn_master_pane_inner(
        ctx,
        SpawnMasterPaneParams {
            session_id: session_id.to_string(),
            cmd: cmd.to_string(),
            tmux_window_size,
            extensions,
            extra_env,
            claimed_master_generation,
        },
    )
    .await?;

    Ok(json!({ "pane_id": outcome.pane_id }))
}

fn parse_tmux_window_size(params: &Value) -> Result<TmuxWindowSize, CcbdError> {
    match params.get("tmux_window_size") {
        Some(value) => serde_json::from_value(value.clone()).map_err(|err| {
            CcbdError::IpcInvalidRequest(format!("invalid tmux_window_size: {err}"))
        }),
        None => Ok(TmuxWindowSize::Fixed),
    }
}

fn parse_extra_env(params: &Value) -> Result<HashMap<String, String>, CcbdError> {
    if let Some(value) = params
        .get("extra_env")
        .or_else(|| params.get("extra_env_vars"))
    {
        serde_json::from_value::<HashMap<String, String>>(value.clone())
            .map_err(|err| CcbdError::IpcInvalidRequest(format!("invalid extra_env: {err}")))
    } else {
        Ok(HashMap::new())
    }
}

async fn prepare_master_pane_plan(
    ctx: &Ctx,
    params: &SpawnMasterPaneParams,
) -> Result<MasterPanePlan, CcbdError> {
    let session = query_session_by_id(ctx.db.clone(), params.session_id.clone())
        .await?
        .ok_or_else(|| {
            CcbdError::IpcInvalidRequest(format!("session not found: {}", params.session_id))
        })?;
    let master_cwd: std::path::PathBuf = session.absolute_path.clone().into();
    let resolved_bundles = resolve_bundles_for_provider(
        &master_cwd,
        "claude",
        BundleRole::Master,
        &params.extensions,
    )?;
    let extensions = resolved_bundles.extensions;
    let hook_generation = if let Some(generation) = params.claimed_master_generation {
        generation
    } else {
        let conn = ctx.db.conn();
        conn.query_row(
            "SELECT master_generation + 1 FROM sessions WHERE id = ?1",
            [&params.session_id],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|err| {
            CcbdError::DbConstraintViolation(format!("query master generation: {err}"))
        })?
    };
    let mut master_env_vars = params.extra_env.clone();
    let mut home_root = None;
    let master_sandbox_dir = if ctx.env_state.unsafe_no_sandbox {
        None
    } else {
        Some(path::resolve_sandbox_dir(
            &ctx.state_dir,
            &params.session_id,
            "master",
        )?)
    };
    if let Some(dir) = master_sandbox_dir.as_ref() {
        let hook_push_ctx = HookPushContext {
            agent_id: format!("master:{}:{hook_generation}", params.session_id),
            provider: "claude".to_string(),
            ahd_socket_path: ctx.state_dir.join("ahd.sock"),
            enabled: true,
        };
        let home_overrides = prepare_home_layout_with_extensions_for_slot(
            "claude",
            dir,
            &master_cwd,
            HomeLayoutRole::Master,
            "master",
            &extensions,
            Some(&hook_push_ctx),
        )?;
        home_root = Some(home_overrides.home_root);
        master_env_vars.extend(home_overrides.extra_env);
    }

    Ok(MasterPanePlan {
        session,
        master_cwd,
        master_env_vars,
        home_root,
        extensions,
    })
}

pub(crate) async fn spawn_master_pane_inner(
    ctx: &Ctx,
    params: SpawnMasterPaneParams,
) -> Result<SpawnMasterPaneOutcome, CcbdError> {
    let plan = prepare_master_pane_plan(ctx, &params).await?;
    spawn_prepared_master_pane(ctx, params, plan, true).await
}

async fn spawn_prepared_master_pane(
    ctx: &Ctx,
    params: SpawnMasterPaneParams,
    plan: MasterPanePlan,
    arm_revival_watch: bool,
) -> Result<SpawnMasterPaneOutcome, CcbdError> {
    let tmux_cmd = systemd::master_command_with_env(
        &plan.session.project_id,
        &params.cmd,
        &ctx.env_state,
        ctx.daemon_unit.as_deref(),
        &plan.master_env_vars,
    );
    let master_session = master_session_name(&plan.session.project_id);
    ctx.tmux_server
        .ensure_session_with_window_size(
            master_session.clone(),
            plan.master_cwd.clone(),
            params.tmux_window_size,
        )
        .await?;
    let pane = ctx
        .tmux_server
        .spawn_window(
            master_session,
            sanitize_tmux_name(&plan.session.project_id),
            plan.master_cwd,
            tmux_cmd,
        )
        .await?;
    let title = format!(
        "master ({})",
        params.cmd.split_whitespace().next().unwrap_or(&params.cmd)
    );
    let _ = ctx.tmux_server.set_pane_title(pane.clone(), &title).await;
    set_session_master_pane_id(ctx.db.clone(), params.session_id.clone(), pane.0.clone()).await?;
    let config_hash = compute_config_hash(&ConfigFingerprintInput {
        role: ConfigRole::Master { cmd: &params.cmd },
        hooks: &plan.extensions.hooks,
        plugins: &plan.extensions.plugins,
        skills: &plan.extensions.skills,
        bundle: plan.extensions.bundle_digest.as_ref(),
    })?;
    update_session_config_hash(ctx.db.clone(), params.session_id.clone(), config_hash).await?;
    update_session_master_cmd(
        ctx.db.clone(),
        params.session_id.clone(),
        params.cmd.clone(),
    )
    .await?;

    let mut new_pid = None;
    let mut generation = None;
    match ctx.tmux_server.get_pane_pid(pane.clone()).await {
        Ok(pid) => {
            let recorded_generation = if let Some(claimed_generation) =
                params.claimed_master_generation
            {
                match record_spawned_master_runtime_after_claim(
                    &ctx.db,
                    &params.session_id,
                    &pane.0,
                    i64::from(pid),
                    claimed_generation,
                )? {
                    Some(generation) => generation,
                    None => {
                        let _ = ctx.tmux_server.kill_pane(pane.clone()).await;
                        tracing::warn!(
                            session_id = %params.session_id,
                            claimed_generation,
                            "claimed master spawn went stale after pane creation; killed orphan pane"
                        );
                        return Ok(SpawnMasterPaneOutcome {
                            pane_id: pane.0,
                            home_root: plan.home_root,
                            new_pid: Some(i64::from(pid)),
                            generation: None,
                        });
                    }
                }
            } else {
                record_spawned_master_runtime(&ctx.db, &params.session_id, &pane.0, i64::from(pid))?
            };
            new_pid = Some(i64::from(pid));
            generation = Some(recorded_generation);
            if arm_revival_watch
                && let Err(err) = arm_master_revival_watch(
                    ctx,
                    &params.session_id,
                    i64::from(pid),
                    recorded_generation,
                    &params.cmd,
                )
            {
                tracing::warn!(
                    session_id = %params.session_id,
                    pid,
                    generation = recorded_generation,
                    error = %err,
                    "failed to arm master revive watcher after spawn"
                );
            }
        }
        Err(err) => {
            tracing::warn!(session_id = %params.session_id, error = %err, "failed to get master pane pid");
        }
    }

    Ok(SpawnMasterPaneOutcome {
        pane_id: pane.0,
        home_root: plan.home_root,
        new_pid,
        generation,
    })
}

fn arm_master_revival_watch(
    ctx: &Ctx,
    session_id: &str,
    expected_pid: i64,
    expected_generation: i64,
    master_cmd: &str,
) -> Result<(), CcbdError> {
    let expected_pid = i32::try_from(expected_pid)
        .map_err(|_| CcbdError::PtyIoError(format!("invalid master pid: {expected_pid}")))?;
    let pidfd = monitor::pidfd_open(expected_pid)?;
    let task_fd = pidfd
        .try_clone()
        .map_err(|err| CcbdError::PtyIoError(format!("clone master pidfd for watcher: {err}")))?;
    let key = master_monitor_key(session_id, expected_generation);
    monitor::register(key, pidfd);
    spawn_master_pidfd_watch_task(
        session_id.to_string(),
        i64::from(expected_pid),
        expected_generation,
        task_fd,
        ctx.db.clone(),
        ctx.tmux_server.clone(),
        master_cmd.to_string(),
        ctx.state_dir.clone(),
        ctx.env_state.clone(),
        ctx.daemon_unit.clone(),
    );
    Ok(())
}

fn master_process_is_alive(pid: i64) -> bool {
    let Ok(pid) = i32::try_from(pid) else {
        return false;
    };
    monitor::pidfd_open(pid).is_ok()
}

async fn rollback_master_cutover_scope(
    ctx: &Ctx,
    cutover_id: &str,
    session_id: &str,
) -> Result<(), CcbdError> {
    let cutover = match get_master_cutover(&ctx.db, cutover_id) {
        Ok(cutover) => cutover,
        Err(err) => {
            tracing::warn!(%cutover_id, error = %err, "failed to load master cutover during rollback");
            None
        }
    };

    if let Some(cutover) = cutover.as_ref() {
        if let Some(pane_id) = cutover.new_master_pane_id.as_deref() {
            match TmuxPaneId::parse(pane_id) {
                Ok(pane_id) => {
                    let pane_id_label = pane_id.0.clone();
                    if let Err(err) = ctx.tmux_server.kill_pane(pane_id).await {
                        tracing::warn!(%cutover_id, pane_id = %pane_id_label, error = %err, "failed to kill new master pane during rollback");
                    }
                }
                Err(err) => {
                    tracing::warn!(%cutover_id, pane_id, error = %err, "invalid cutover master pane id during rollback");
                }
            }
        } else {
            tracing::warn!(%cutover_id, "master cutover rollback has no new master pane id");
        }
    } else {
        tracing::warn!(%cutover_id, "master cutover rollback has no cutover row; cannot kill new master pane");
    }

    if let Some(cutover) = cutover.as_ref()
        && matches!(
            cutover.state.as_str(),
            "PREPARING" | "SPAWNING" | "VERIFYING"
        )
    {
        if let Err(err) = update_master_cutover_state(&ctx.db, cutover_id, &cutover.state, "FAILED")
        {
            tracing::warn!(%cutover_id, error = %err, "failed to mark master cutover FAILED during rollback");
        }
    }

    if let Some(cutover) = cutover.as_ref() {
        let handoff_path = PathBuf::from(&cutover.handoff_path);
        if let Some(handoff_dir) = handoff_path.parent()
            && let Err(err) = std::fs::remove_dir_all(handoff_dir)
            && err.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!(%cutover_id, path = %handoff_dir.display(), error = %err, "failed to remove master cutover handoff dir");
        }
    }

    let agent_ids = match session_agent_ids(ctx.db.clone(), session_id.to_string()).await {
        Ok(agent_ids) => agent_ids,
        Err(err) => {
            tracing::warn!(%session_id, %cutover_id, error = %err, "failed to list session agents during master cutover rollback");
            Vec::new()
        }
    };
    for agent_id in agent_ids {
        crate::agent_io::cleanup_agent_runtime_resources(&agent_id);
        remove_agent_sandbox_dir_sync(&ctx.state_dir, session_id, &agent_id);
    }

    if let Some(cutover) = cutover.as_ref()
        && let Some(generation) = cutover.new_master_generation
    {
        let _ = crate::master_revival::remove_master_monitor_key_if_generation_matches(
            session_id, generation,
        );
    }

    remove_agent_sandbox_dir_sync(&ctx.state_dir, session_id, "master");

    {
        let conn = ctx.db.conn();
        match conn.execute(
            "UPDATE sessions SET status = 'FAILED' WHERE id = ?1 AND status = 'ACTIVE'",
            [session_id],
        ) {
            Ok(changes) => {
                if changes > 0 {
                    crate::orchestrator::pubsub::notify_runtime_changed(
                        crate::runtime_events::RuntimeSnapshotReason::InventoryChanged,
                    );
                }
            }
            Err(err) => {
                tracing::warn!(%session_id, %cutover_id, error = %err, "failed to mark rollback session FAILED");
            }
        }
    }

    // TODO(followup): startup reconcile in-flight cutovers.
    Ok(())
}

#[derive(Debug, Deserialize)]
struct MasterCutoverMasterParams {
    #[serde(default)]
    cmd: String,
    #[serde(default)]
    provider: Option<String>,
    #[serde(default = "default_master_readiness_timeout_s")]
    readiness_timeout_s: u64,
    #[serde(default)]
    hooks: HashMap<String, Vec<crate::provider::extensions::HookGroup>>,
    #[serde(default)]
    plugins: Vec<String>,
    #[serde(default)]
    skills: Vec<String>,
    #[serde(default)]
    bundle: Vec<String>,
    #[serde(default)]
    tmux_window_size: TmuxWindowSize,
}

fn default_master_readiness_timeout_s() -> u64 {
    120
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MasterReadinessMode {
    Ack,
    Probe,
}

impl MasterReadinessMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::Ack => "ack",
            Self::Probe => "probe",
        }
    }
}

fn master_readiness_mode(provider: Option<&str>, cmd: &str) -> MasterReadinessMode {
    let provider = provider
        .filter(|value| !value.trim().is_empty())
        .map(str::trim)
        .map(crate::provider::manifest::canonicalize_provider_name)
        .map(str::to_string)
        .unwrap_or_else(|| {
            crate::provider::manifest::canonicalize_provider_name(
                cmd.split_whitespace().next().unwrap_or("custom"),
            )
            .to_string()
        });
    if provider == "claude" {
        MasterReadinessMode::Ack
    } else {
        MasterReadinessMode::Probe
    }
}

fn readiness_timeout(readiness_timeout_s: u64) -> Duration {
    Duration::from_secs(readiness_timeout_s.clamp(30, 600))
}

async fn wait_for_master_readiness(
    ctx: &Ctx,
    cutover_id: &str,
    mode: MasterReadinessMode,
    timeout: Duration,
) -> Result<String, CcbdError> {
    let deadline = tokio::time::Instant::now() + timeout;
    let mut last_probe_capture: Option<String> = None;
    let mut steady_probe_captures = 0usize;
    loop {
        let cutover = get_master_cutover(&ctx.db, cutover_id)?.ok_or_else(|| {
            CcbdError::IpcInvalidRequest(format!("master cutover not found: {cutover_id}"))
        })?;
        if cutover.state != "VERIFYING" {
            return Err(CcbdError::IpcInvalidRequest(format!(
                "master cutover {cutover_id} left VERIFYING before readiness"
            )));
        }
        if cutover.ack_ready_at.is_some() {
            return Ok(cutover
                .readiness_mode
                .unwrap_or_else(|| mode.as_str().to_string()));
        }
        if cutover
            .new_master_pid
            .is_some_and(|pid| !master_process_is_alive(pid))
        {
            return Err(CcbdError::IpcInvalidRequest(format!(
                "master cutover {cutover_id} master process exited before readiness"
            )));
        }
        if mode == MasterReadinessMode::Probe
            && let Some(pane_id) = cutover.new_master_pane_id.as_deref()
        {
            match TmuxPaneId::parse(pane_id) {
                Ok(parsed_pane_id) => match ctx.tmux_server.capture_pane(parsed_pane_id).await {
                    Ok(capture) if !capture.trim().is_empty() => {
                        if last_probe_capture.as_deref() == Some(capture.as_str()) {
                            steady_probe_captures += 1;
                        } else {
                            last_probe_capture = Some(capture);
                            steady_probe_captures = 1;
                        }
                        if steady_probe_captures >= 3 {
                            return match mark_master_cutover_ack_ready(
                                &ctx.db,
                                cutover_id,
                                mode.as_str(),
                            )? {
                                MasterCutoverUpdate::Updated => Ok(mode.as_str().to_string()),
                                MasterCutoverUpdate::Stale => Err(CcbdError::IpcInvalidRequest(
                                    format!("master cutover {cutover_id} not in VERIFYING"),
                                )),
                            };
                        }
                    }
                    Ok(_) => {
                        last_probe_capture = None;
                        steady_probe_captures = 0;
                    }
                    Err(err) => {
                        tracing::debug!(%cutover_id, pane_id, error = %err, "master readiness probe capture failed");
                        last_probe_capture = None;
                        steady_probe_captures = 0;
                    }
                },
                Err(err) => {
                    tracing::warn!(%cutover_id, pane_id, error = %err, "invalid master readiness probe pane id");
                    last_probe_capture = None;
                    steady_probe_captures = 0;
                }
            }
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(CcbdError::IpcInvalidRequest(format!(
                "master cutover {cutover_id} readiness timed out"
            )));
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

#[derive(Debug, Deserialize)]
struct MasterCutoverRequest {
    project_id: String,
    absolute_path: String,
    cwd: PathBuf,
    old_home: PathBuf,
    old_master_pid: Option<i64>,
    ah_state_dir: Option<PathBuf>,
    ah_socket_path: PathBuf,
    master: MasterCutoverMasterParams,
    #[serde(default)]
    agents: Vec<RealignAgentParams>,
    #[serde(default)]
    wait: bool,
    #[serde(default)]
    print_attach: bool,
}

type SpawnMasterFuture<'a> =
    Pin<Box<dyn Future<Output = Result<SpawnMasterPaneOutcome, CcbdError>> + Send + 'a>>;

pub async fn handle_session_master_cutover(params: Value, ctx: &Ctx) -> Result<Value, CcbdError> {
    let request: MasterCutoverRequest = serde_json::from_value(params).map_err(|err| {
        CcbdError::IpcInvalidRequest(format!("invalid master cutover params: {err}"))
    })?;
    run_master_cutover_with_spawn(ctx, request, |ctx, params, plan| {
        Box::pin(spawn_prepared_master_pane(ctx, params, plan, false))
    })
    .await
}

pub async fn handle_master_ack_ready(params: Value, ctx: &Ctx) -> Result<Value, CcbdError> {
    let cutover_id = required_str(&params, "cutover_id")?;
    match mark_master_cutover_ack_ready(&ctx.db, cutover_id, "ack")? {
        MasterCutoverUpdate::Updated => Ok(json!({
            "cutover_id": cutover_id,
            "readiness_mode": "ack",
            "ack_ready": true,
        })),
        MasterCutoverUpdate::Stale => Err(CcbdError::IpcInvalidRequest(format!(
            "master cutover {cutover_id} not in VERIFYING"
        ))),
    }
}

async fn run_master_cutover_with_spawn<S>(
    ctx: &Ctx,
    request: MasterCutoverRequest,
    spawn: S,
) -> Result<Value, CcbdError>
where
    S: for<'a> Fn(&'a Ctx, SpawnMasterPaneParams, MasterPanePlan) -> SpawnMasterFuture<'a>,
{
    let session_id = format!("sess_{}", Uuid::new_v4());
    let cutover_id = format!("cutover-{session_id}");
    let state_dir = request
        .ah_state_dir
        .clone()
        .unwrap_or_else(|| ctx.state_dir.clone());
    let handoff_path = state_dir
        .join("cutovers")
        .join(&cutover_id)
        .join("handoff.md");
    tracing::info!(%session_id, %cutover_id, "master cutover creating session");
    create_session(
        ctx.db.clone(),
        session_id.clone(),
        request.project_id.clone(),
        request.absolute_path.clone(),
    )
    .await?;

    match claim_master_cutover(
        &ctx.db,
        &cutover_id,
        &session_id,
        request.old_master_pid,
        &state_dir.display().to_string(),
        &request.ah_socket_path.display().to_string(),
        &handoff_path.display().to_string(),
    )? {
        MasterCutoverClaim::Claimed => {
            tracing::info!(%session_id, %cutover_id, "master cutover claimed PREPARING");
        }
        MasterCutoverClaim::AlreadyActive => {
            tracing::warn!(%session_id, %cutover_id, "master cutover rejected; active cutover already exists");
            return Err(CcbdError::IpcInvalidRequest(format!(
                "active master cutover already exists for session {session_id}"
            )));
        }
    }

    let mut current_state = "PREPARING";
    let result = async {
        let mut extra_env = HashMap::from([
            ("AH_STATE_DIR".to_string(), state_dir.display().to_string()),
            (
                "CCB_SOCKET".to_string(),
                request.ah_socket_path.display().to_string(),
            ),
            ("AH_CUTOVER_ID".to_string(), cutover_id.clone()),
            (
                "AH_MASTER_HANDOFF".to_string(),
                handoff_path.display().to_string(),
            ),
            ("AH_MASTER_ROLE".to_string(), "managed".to_string()),
        ]);
        let spawn_params = SpawnMasterPaneParams {
            session_id: session_id.clone(),
            cmd: request.master.cmd.clone(),
            tmux_window_size: request.master.tmux_window_size,
            extensions: ExtensionConfig {
                hooks: request.master.hooks.clone(),
                plugins: request.master.plugins.clone(),
                skills: request.master.skills.clone(),
                bundle: request.master.bundle.clone(),
                ..Default::default()
            },
            extra_env: std::mem::take(&mut extra_env),
            claimed_master_generation: None,
        };
        let plan = prepare_master_pane_plan(ctx, &spawn_params).await?;
        let master_home = plan.home_root.clone().ok_or_else(|| {
            CcbdError::EnvironmentNotSupported {
                details: "master cutover requires sandboxed master home".to_string(),
            }
        })?;
        tracing::info!(%session_id, %cutover_id, master_home = %master_home.display(), "master cutover seeding handoff before spawn");
        write_handoff_bundle(
            &state_dir,
            &HandoffBundleInput {
                cutover_id: &cutover_id,
                session_id: &session_id,
                socket_path: &request.ah_socket_path,
                state_dir: &state_dir,
            },
        )?;
        let _seed = seed_claude_project_conversation(
            &request.old_home,
            &master_home,
            &request.cwd,
            &handoff_path,
        )?;
        for agent in &request.agents {
            let expected_hash = compute_config_hash(&ConfigFingerprintInput {
                role: ConfigRole::Agent {
                    provider: &agent.provider,
                    env: &agent.env,
                },
                hooks: &agent.hooks,
                plugins: &agent.plugins,
                skills: &agent.skills,
                bundle: agent.bundle_digest.as_ref(),
            })?;
            tracing::info!(
                %session_id,
                %cutover_id,
                agent_id = %agent.agent_id,
                provider = %agent.provider,
                "master cutover provisioning declared worker before master spawn"
            );
            spawn_realign_agent(ctx, &session_id, agent, &expected_hash, false, false, None)
                .await?;
        }
        if update_master_cutover_state(&ctx.db, &cutover_id, "PREPARING", "SPAWNING")?
            != MasterCutoverUpdate::Updated
        {
            tracing::warn!(%session_id, %cutover_id, "master cutover PREPARING->SPAWNING CAS stale");
            return Err(CcbdError::IpcInvalidRequest("master cutover stale before spawn".into()));
        }
        current_state = "SPAWNING";
        tracing::info!(%session_id, %cutover_id, "master cutover spawning managed master");
        let outcome = spawn(ctx, spawn_params, plan).await?;
        let new_pid = outcome.new_pid.ok_or_else(|| {
            CcbdError::EnvironmentNotSupported {
                details: "master cutover spawn did not report new master pid".to_string(),
            }
        })?;
        let generation = outcome.generation.ok_or_else(|| {
            CcbdError::EnvironmentNotSupported {
                details: "master cutover spawn did not report new master generation".to_string(),
            }
        })?;
        if update_master_cutover_spawn_metadata(
            &ctx.db,
            &cutover_id,
            "SPAWNING",
            "VERIFYING",
            new_pid,
            generation,
            &outcome.pane_id,
        )? != MasterCutoverUpdate::Updated
        {
            tracing::warn!(%session_id, %cutover_id, "master cutover SPAWNING->VERIFYING CAS stale");
            return Err(CcbdError::IpcInvalidRequest("master cutover stale after spawn".into()));
        }
        current_state = "VERIFYING";
        let readiness_mode =
            master_readiness_mode(request.master.provider.as_deref(), &request.master.cmd);
        let readiness_mode = wait_for_master_readiness(
            ctx,
            &cutover_id,
            readiness_mode,
            readiness_timeout(request.master.readiness_timeout_s),
        )
        .await?;
        tracing::info!(%session_id, %cutover_id, readiness_mode, "master cutover readiness accepted");
        if update_master_cutover_state(&ctx.db, &cutover_id, "VERIFYING", "ACTIVE")?
            != MasterCutoverUpdate::Updated
        {
            tracing::warn!(%session_id, %cutover_id, "master cutover VERIFYING->ACTIVE CAS stale");
            return Err(CcbdError::IpcInvalidRequest("master cutover stale before ACTIVE".into()));
        }
        current_state = "ACTIVE";
        if let Err(err) =
            arm_master_revival_watch(ctx, &session_id, new_pid, generation, &request.master.cmd)
        {
            tracing::warn!(%session_id, %cutover_id, error = %err, "failed to arm master revive watcher after ACTIVE");
        }
        tracing::info!(%session_id, %cutover_id, "master cutover ACTIVE");
        let attach_command = format!("ah attach master --session {session_id}");
        let tmux_attach_command = format!(
            "tmux -L {} attach -t {}",
            ctx.tmux_server.socket_name(),
            master_session_name(&request.project_id)
        );
        let _ = (request.wait, request.print_attach);
        Ok(json!({
            "cutover_id": cutover_id,
            "session_id": session_id,
            "pane_id": outcome.pane_id,
            "attach_command": attach_command,
            "tmux_attach_command": tmux_attach_command,
            "handoff_path": handoff_path,
            "readiness_mode": readiness_mode,
        }))
    }
    .await;

    if let Err(err) = result.as_ref() {
        if current_state != "ACTIVE" {
            if let Err(rollback_err) =
                rollback_master_cutover_scope(ctx, &cutover_id, &session_id).await
            {
                tracing::warn!(%session_id, %cutover_id, error = %rollback_err, "master cutover rollback failed; possible live-master/sandbox leak");
            }
            tracing::warn!(%session_id, %cutover_id, state = current_state, error = %err, "master cutover failed; old master left running");
        }
    }
    result
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
                "master_state": session.master_state,
                "master_pane_id": session.master_pane_id,
                "active_agents": session.active_agents,
                "created_at": session.created_at,
            })
        })
        .collect::<Vec<_>>();
    Ok(json!({ "sessions": sessions }))
}

pub async fn handle_master_tell_begin(params: Value, ctx: &Ctx) -> Result<Value, CcbdError> {
    let session_id = required_str(&params, "session_id")?.to_string();
    let request_id = required_str(&params, "request_id")?.to_string();
    let pane_id = required_str(&params, "pane_id")?.to_string();
    let changes = master_tell_begin(ctx.db.clone(), session_id.clone(), request_id.clone()).await?;
    if changes == 0 {
        return Err(CcbdError::IpcInvalidRequest(format!(
            "active session not found: {session_id}"
        )));
    }
    tracing::info!(
        command = "ah.tell",
        target = "master",
        session_id = %session_id,
        pane_id = %pane_id,
        request_id = %request_id,
        stage = "BEGIN",
        result = "registered",
        "registered master tell request"
    );
    Ok(json!({
        "session_id": session_id,
        "request_id": request_id,
        "pane_id": pane_id,
        "registered": true,
    }))
}

pub async fn handle_master_tell_failed(params: Value, ctx: &Ctx) -> Result<Value, CcbdError> {
    let session_id = required_str(&params, "session_id")?.to_string();
    let request_id = required_str(&params, "request_id")?.to_string();
    let pane_id = params.get("pane_id").and_then(Value::as_str).unwrap_or("");
    let stage = params
        .get("stage")
        .and_then(Value::as_str)
        .unwrap_or("UNKNOWN");
    let reason = params
        .get("reason")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let changes =
        master_tell_failed(ctx.db.clone(), session_id.clone(), request_id.clone()).await?;
    tracing::warn!(
        command = "ah.tell",
        target = "master",
        session_id = %session_id,
        pane_id = %pane_id,
        request_id = %request_id,
        stage = %stage,
        result = "delivery_failed",
        reason = %reason,
        cleared_pending = changes > 0,
        "master tell delivery failed"
    );
    Ok(json!({
        "session_id": session_id,
        "request_id": request_id,
        "cleared_pending": changes > 0,
    }))
}

#[cfg(test)]
mod master_cutover_tests {
    use super::*;
    use crate::db;
    use crate::db::master_cutovers::get_active_master_cutover;
    use crate::db::recovery::query_agent_spawn_spec_sync;
    use crate::master_cutover::claude_project_conversation_dir;
    use crate::sandbox::EnvState;
    use crate::tmux::TmuxServer;
    use serde_json::json;
    use std::sync::{Arc, Mutex};

    fn test_ctx(state_dir: PathBuf) -> Ctx {
        let file = tempfile::NamedTempFile::new().unwrap();
        Ctx {
            db: db::init(file.path()).unwrap(),
            state_dir: state_dir.clone(),
            env_state: EnvState {
                systemd_run_available: false,
                unsafe_no_sandbox: false,
                under_systemd: false,
            },
            daemon_unit: None,
            tmux_server: Arc::new(TmuxServer::new(&state_dir)),
        }
    }

    fn request(tmp: &tempfile::TempDir, old_home: &std::path::Path) -> MasterCutoverRequest {
        MasterCutoverRequest {
            project_id: "ccbd-rust".to_string(),
            absolute_path: "/home/sevenx/coding/ccbd-rust".to_string(),
            cwd: PathBuf::from("/home/sevenx/coding/ccbd-rust"),
            old_home: old_home.to_path_buf(),
            old_master_pid: Some(111),
            ah_state_dir: Some(tmp.path().join("state")),
            ah_socket_path: tmp.path().join("state").join("ahd.sock"),
            master: MasterCutoverMasterParams {
                cmd: "claude --continue".to_string(),
                provider: Some("claude".to_string()),
                readiness_timeout_s: 1,
                hooks: HashMap::new(),
                plugins: Vec::new(),
                skills: Vec::new(),
                bundle: Vec::new(),
                tmux_window_size: TmuxWindowSize::Fixed,
            },
            agents: vec![RealignAgentParams {
                agent_id: "w1".to_string(),
                provider: "bash".to_string(),
                env: HashMap::from([("WORKER_ENV".to_string(), "1".to_string())]),
                hooks: HashMap::new(),
                plugins: Vec::new(),
                skills: Vec::new(),
                bundle: Vec::new(),
                bundle_digest: None,
                sandbox_overrides: Default::default(),
                hook_push_enabled: false,
            }],
            wait: true,
            print_attach: true,
        }
    }

    fn seed_old_conversation(old_home: &std::path::Path) {
        let cwd = PathBuf::from("/home/sevenx/coding/ccbd-rust");
        let source_dir = claude_project_conversation_dir(old_home, &cwd);
        std::fs::create_dir_all(&source_dir).unwrap();
        std::fs::write(source_dir.join("conversation.jsonl"), b"old conversation").unwrap();
    }

    fn spawn_ack_when_verifying(ctx: &Ctx) {
        let db = ctx.db.clone();
        tokio::spawn(async move {
            for _ in 0..40 {
                let cutover_id = {
                    let conn = db.conn();
                    conn.query_row(
                        "SELECT id FROM master_cutovers WHERE state = 'VERIFYING' LIMIT 1",
                        [],
                        |row| row.get::<_, String>(0),
                    )
                    .ok()
                };
                if let Some(cutover_id) = cutover_id {
                    let _ = crate::db::master_cutovers::mark_master_cutover_ack_ready(
                        &db,
                        &cutover_id,
                        "ack",
                    );
                    return;
                }
                tokio::time::sleep(std::time::Duration::from_millis(25)).await;
            }
        });
    }

    #[tokio::test(flavor = "current_thread")]
    #[serial_test::serial(global_env)]
    async fn daemon_master_cutover_orders_seed_before_spawn_and_writes_metadata() {
        let tmp = tempfile::tempdir().unwrap();
        let old_home = tmp.path().join("old-home");
        let source_home = tmp.path().join("source-home");
        std::fs::create_dir_all(&source_home).unwrap();
        seed_old_conversation(&old_home);
        unsafe {
            std::env::set_var("HOME", &source_home);
        }
        let ctx = test_ctx(tmp.path().join("state"));
        let order = Arc::new(Mutex::new(Vec::<String>::new()));
        let seen_home = Arc::new(Mutex::new(None::<PathBuf>));
        let order_for_spawn = order.clone();
        let seen_home_for_spawn = seen_home.clone();
        let current_pid = i64::from(std::process::id());

        let response = run_master_cutover_with_spawn(
            &ctx,
            request(&tmp, &old_home),
            move |_ctx, _params, plan| {
                let order = order_for_spawn.clone();
                let seen_home = seen_home_for_spawn.clone();
                Box::pin(async move {
                    spawn_ack_when_verifying(_ctx);
                    let master_home = plan.home_root.clone().expect("master home");
                    let provisioned = _ctx
                        .db
                        .conn()
                        .query_row("SELECT state FROM agents WHERE id = 'w1'", [], |row| {
                            row.get::<_, String>(0)
                        })
                        .expect("declared worker must be provisioned before master spawn");
                    assert_eq!(provisioned, "SPAWNING");
                    let stored = query_agent_spawn_spec_sync(&_ctx.db.conn(), "w1")
                        .unwrap()
                        .expect("declared worker spawn spec must be persisted");
                    assert_eq!(stored.spec.provider, "bash");
                    assert_eq!(stored.spec.env["WORKER_ENV"], "1");
                    let seeded = claude_project_conversation_dir(
                        &master_home,
                        &PathBuf::from("/home/sevenx/coding/ccbd-rust"),
                    )
                    .join("conversation.jsonl");
                    assert!(seeded.exists(), "seed must land before spawn");
                    *seen_home.lock().unwrap() = Some(master_home);
                    order.lock().unwrap().push("spawn".to_string());
                    Ok(SpawnMasterPaneOutcome {
                        pane_id: "%42".to_string(),
                        home_root: plan.home_root,
                        new_pid: Some(current_pid),
                        generation: Some(7),
                    })
                })
            },
        )
        .await
        .unwrap();

        assert_eq!(response["pane_id"], "%42");
        let tmux_attach_command = response["tmux_attach_command"].as_str().unwrap();
        assert!(tmux_attach_command.starts_with("tmux -L "));
        assert!(tmux_attach_command.contains(ctx.tmux_server.socket_name()));
        assert!(
            !tmux_attach_command.contains("ahd.sock"),
            "tmux attach command must use the tmux socket, not the ahd RPC socket"
        );
        assert_eq!(*order.lock().unwrap(), vec!["spawn"]);
        let session_id = response["session_id"].as_str().unwrap();
        let active = get_active_master_cutover(&ctx.db, session_id)
            .unwrap()
            .expect("active cutover");
        assert_eq!(active.state, "ACTIVE");
        assert_eq!(active.new_master_pid, Some(current_pid));
        assert_eq!(active.new_master_generation, Some(7));
        assert_eq!(active.new_master_pane_id.as_deref(), Some("%42"));
        let master_home = seen_home.lock().unwrap().clone().unwrap();
        assert!(
            claude_project_conversation_dir(
                &master_home,
                &PathBuf::from("/home/sevenx/coding/ccbd-rust")
            )
            .join("conversation.jsonl")
            .exists()
        );
    }

    #[tokio::test(flavor = "current_thread")]
    #[serial_test::serial(global_env)]
    async fn daemon_master_cutover_requires_ack_before_active() {
        let tmp = tempfile::tempdir().unwrap();
        let old_home = tmp.path().join("old-home");
        let source_home = tmp.path().join("source-home");
        std::fs::create_dir_all(&source_home).unwrap();
        seed_old_conversation(&old_home);
        unsafe {
            std::env::set_var("HOME", &source_home);
        }
        let ctx = test_ctx(tmp.path().join("state"));
        let current_pid = i64::from(std::process::id());

        let response = run_master_cutover_with_spawn(
            &ctx,
            request(&tmp, &old_home),
            move |ctx, _params, plan| {
                spawn_ack_when_verifying(ctx);
                Box::pin(async move {
                    Ok(SpawnMasterPaneOutcome {
                        pane_id: "%44".to_string(),
                        home_root: plan.home_root,
                        new_pid: Some(current_pid),
                        generation: Some(9),
                    })
                })
            },
        )
        .await
        .unwrap();

        let session_id = response["session_id"].as_str().unwrap();
        let active = get_active_master_cutover(&ctx.db, session_id)
            .unwrap()
            .expect("active cutover");
        assert_eq!(active.state, "ACTIVE");
        assert!(active.ack_ready_at.is_some());
        assert_eq!(active.readiness_mode.as_deref(), Some("ack"));
        assert_eq!(response["readiness_mode"], "ack");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn wait_for_master_readiness_fails_fast_when_verifying_master_pid_is_dead() {
        let tmp = tempfile::tempdir().unwrap();
        let ctx = test_ctx(tmp.path().join("state"));
        let session_id = "s_dead_verify";
        let cutover_id = "cutover_dead_verify";
        create_session(
            ctx.db.clone(),
            session_id.to_string(),
            "p_dead_verify".to_string(),
            tmp.path().display().to_string(),
        )
        .await
        .unwrap();
        assert_eq!(
            claim_master_cutover(
                &ctx.db,
                cutover_id,
                session_id,
                Some(111),
                &ctx.state_dir.display().to_string(),
                &ctx.state_dir.join("ahd.sock").display().to_string(),
                &ctx.state_dir
                    .join("cutovers")
                    .join(cutover_id)
                    .join("handoff.md")
                    .display()
                    .to_string(),
            )
            .unwrap(),
            MasterCutoverClaim::Claimed
        );
        assert_eq!(
            update_master_cutover_state(&ctx.db, cutover_id, "PREPARING", "SPAWNING").unwrap(),
            MasterCutoverUpdate::Updated
        );
        assert_eq!(
            update_master_cutover_spawn_metadata(
                &ctx.db,
                cutover_id,
                "SPAWNING",
                "VERIFYING",
                999_999_999,
                3,
                "%404",
            )
            .unwrap(),
            MasterCutoverUpdate::Updated
        );

        let err = wait_for_master_readiness(
            &ctx,
            cutover_id,
            MasterReadinessMode::Ack,
            Duration::from_millis(200),
        )
        .await
        .unwrap_err();

        assert!(
            err.to_string().contains("exited before readiness"),
            "expected process-death error, got {err}"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn probe_readiness_times_out_without_stable_capture() {
        let tmp = tempfile::tempdir().unwrap();
        let ctx = test_ctx(tmp.path().join("state"));
        let session_id = "s_probe_timeout";
        let cutover_id = "cutover_probe_timeout";
        create_session(
            ctx.db.clone(),
            session_id.to_string(),
            "p_probe_timeout".to_string(),
            tmp.path().display().to_string(),
        )
        .await
        .unwrap();
        assert_eq!(
            claim_master_cutover(
                &ctx.db,
                cutover_id,
                session_id,
                Some(111),
                &ctx.state_dir.display().to_string(),
                &ctx.state_dir.join("ahd.sock").display().to_string(),
                &ctx.state_dir
                    .join("cutovers")
                    .join(cutover_id)
                    .join("handoff.md")
                    .display()
                    .to_string(),
            )
            .unwrap(),
            MasterCutoverClaim::Claimed
        );
        assert_eq!(
            update_master_cutover_state(&ctx.db, cutover_id, "PREPARING", "SPAWNING").unwrap(),
            MasterCutoverUpdate::Updated
        );
        assert_eq!(
            update_master_cutover_spawn_metadata(
                &ctx.db,
                cutover_id,
                "SPAWNING",
                "VERIFYING",
                i64::from(std::process::id()),
                4,
                "%missing",
            )
            .unwrap(),
            MasterCutoverUpdate::Updated
        );

        let err = wait_for_master_readiness(
            &ctx,
            cutover_id,
            MasterReadinessMode::Probe,
            Duration::from_millis(120),
        )
        .await
        .unwrap_err();

        assert!(
            err.to_string().contains("readiness timed out"),
            "expected probe timeout, got {err}"
        );
        let cutover = get_master_cutover(&ctx.db, cutover_id)
            .unwrap()
            .expect("cutover row");
        assert_eq!(cutover.state, "VERIFYING");
        assert!(cutover.ack_ready_at.is_none());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn spawn_master_pane_does_not_arm_revival_watch_before_active() {
        let tmp = tempfile::tempdir().unwrap();
        let mut ctx = test_ctx(tmp.path().join("state"));
        ctx.env_state.unsafe_no_sandbox = true;
        create_session(
            ctx.db.clone(),
            "s_watch_delay".to_string(),
            "p_watch_delay".to_string(),
            tmp.path().display().to_string(),
        )
        .await
        .unwrap();

        let params = SpawnMasterPaneParams {
            session_id: "s_watch_delay".to_string(),
            cmd: "sleep 60".to_string(),
            tmux_window_size: TmuxWindowSize::Fixed,
            extensions: ExtensionConfig::default(),
            extra_env: HashMap::new(),
            claimed_master_generation: None,
        };
        let plan = prepare_master_pane_plan(&ctx, &params).await.unwrap();
        let outcome = spawn_prepared_master_pane(&ctx, params, plan, false)
            .await
            .unwrap();
        let generation = outcome.generation.expect("master generation");
        let key = master_monitor_key("s_watch_delay", generation);
        let armed_before_active = monitor::remove(&key).is_some();
        let _ = ctx
            .tmux_server
            .kill_pane(TmuxPaneId::parse(&outcome.pane_id).unwrap())
            .await;

        assert!(
            !armed_before_active,
            "master revive watcher must not arm before cutover reaches ACTIVE"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn rollback_master_cutover_scope_kills_new_pane_but_keeps_old_master_session() {
        let tmp = tempfile::tempdir().unwrap();
        let ctx = test_ctx(tmp.path().join("state"));
        let project_id = "p_scope_rollback";
        let session_id = "s_scope_rollback";
        let cutover_id = "cutover_scope_rollback";
        create_session(
            ctx.db.clone(),
            session_id.to_string(),
            project_id.to_string(),
            tmp.path().display().to_string(),
        )
        .await
        .unwrap();
        {
            let conn = ctx.db.conn();
            crate::db::agents::insert_agent_sync(
                &conn,
                "w_scope_rollback",
                session_id,
                "bash",
                "SPAWNING",
                Some(1234),
            )
            .unwrap();
        }
        let master_session = master_session_name(project_id);
        ctx.tmux_server
            .ensure_session(master_session.clone(), tmp.path().to_path_buf())
            .await
            .unwrap();
        let _old_pane = ctx
            .tmux_server
            .spawn_window(
                master_session.clone(),
                "old-master".to_string(),
                tmp.path().to_path_buf(),
                vec!["sleep".to_string(), "60".to_string()],
            )
            .await
            .unwrap();
        let new_pane = ctx
            .tmux_server
            .spawn_window(
                master_session.clone(),
                "new-master".to_string(),
                tmp.path().to_path_buf(),
                vec!["sleep".to_string(), "60".to_string()],
            )
            .await
            .unwrap();
        let handoff_dir = ctx.state_dir.join("cutovers").join(cutover_id);
        std::fs::create_dir_all(&handoff_dir).unwrap();
        std::fs::write(handoff_dir.join("handoff.md"), b"handoff").unwrap();
        let master_sandbox = ctx
            .state_dir
            .join("sandboxes")
            .join(session_id)
            .join("master");
        let worker_sandbox = ctx
            .state_dir
            .join("sandboxes")
            .join(session_id)
            .join("w_scope_rollback");
        std::fs::create_dir_all(&master_sandbox).unwrap();
        std::fs::create_dir_all(&worker_sandbox).unwrap();
        assert_eq!(
            claim_master_cutover(
                &ctx.db,
                cutover_id,
                session_id,
                Some(111),
                &ctx.state_dir.display().to_string(),
                &ctx.state_dir.join("ahd.sock").display().to_string(),
                &handoff_dir.join("handoff.md").display().to_string(),
            )
            .unwrap(),
            MasterCutoverClaim::Claimed
        );
        assert_eq!(
            update_master_cutover_state(&ctx.db, cutover_id, "PREPARING", "SPAWNING").unwrap(),
            MasterCutoverUpdate::Updated
        );
        assert_eq!(
            update_master_cutover_spawn_metadata(
                &ctx.db,
                cutover_id,
                "SPAWNING",
                "VERIFYING",
                9999,
                77,
                &new_pane.0,
            )
            .unwrap(),
            MasterCutoverUpdate::Updated
        );

        rollback_master_cutover_scope(&ctx, cutover_id, session_id)
            .await
            .unwrap();

        assert!(
            ctx.tmux_server
                .window_exists(master_session.clone(), "old-master".to_string())
                .await
                .unwrap(),
            "rollback must not kill the shared master tmux session or old master window"
        );
        assert!(!handoff_dir.exists());
        assert!(!master_sandbox.exists());
        assert!(!worker_sandbox.exists());
        let cutover = get_master_cutover(&ctx.db, cutover_id)
            .unwrap()
            .expect("cutover row");
        assert_eq!(cutover.state, "FAILED");

        let _ = ctx.tmux_server.kill_session(master_session).await;
    }

    #[tokio::test(flavor = "current_thread")]
    #[serial_test::serial(global_env)]
    async fn daemon_master_cutover_spawn_failure_marks_failed_and_keeps_old_master() {
        let tmp = tempfile::tempdir().unwrap();
        let old_home = tmp.path().join("old-home");
        let source_home = tmp.path().join("source-home");
        std::fs::create_dir_all(&source_home).unwrap();
        seed_old_conversation(&old_home);
        unsafe {
            std::env::set_var("HOME", &source_home);
        }
        let ctx = test_ctx(tmp.path().join("state"));
        let calls = Arc::new(Mutex::new(Vec::<String>::new()));
        let calls_for_spawn = calls.clone();

        let err = run_master_cutover_with_spawn(
            &ctx,
            request(&tmp, &old_home),
            move |_ctx, _params, _plan| {
                let calls = calls_for_spawn.clone();
                Box::pin(async move {
                    calls.lock().unwrap().push("spawn".to_string());
                    Err(CcbdError::EnvironmentNotSupported {
                        details: "injected spawn failure".to_string(),
                    })
                })
            },
        )
        .await
        .unwrap_err();

        assert!(err.to_string().contains("injected spawn failure"));
        assert_eq!(*calls.lock().unwrap(), vec!["spawn"]);
        let conn = ctx.db.conn();
        let state: String = conn
            .query_row("SELECT state FROM master_cutovers", [], |row| row.get(0))
            .unwrap();
        assert_eq!(state, "FAILED");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn dispatch_session_master_cutover_method_registered() {
        let tmp = tempfile::tempdir().unwrap();
        let ctx = test_ctx(tmp.path().join("state"));
        let response = crate::rpc::router::dispatch(
            &json!({
                "jsonrpc": "2.0",
                "method": "session.master_cutover",
                "params": {},
                "id": 77
            })
            .to_string(),
            &ctx,
        )
        .await;
        let obj: serde_json::Value = serde_json::from_str(&response).unwrap();
        assert_eq!(obj["id"], 77);
        assert_eq!(obj["error"]["code"], -32000);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn dispatch_master_ack_ready_method_registered_and_rejects_missing_cutover() {
        let tmp = tempfile::tempdir().unwrap();
        let ctx = test_ctx(tmp.path().join("state"));
        let response = crate::rpc::router::dispatch(
            &json!({
                "jsonrpc": "2.0",
                "method": "master.ack_ready",
                "params": {"cutover_id": "cutover_missing"},
                "id": 78
            })
            .to_string(),
            &ctx,
        )
        .await;
        let obj: serde_json::Value = serde_json::from_str(&response).unwrap();
        assert_eq!(obj["id"], 78);
        assert_eq!(obj["error"]["code"], -32000);
        assert!(
            obj["error"]["message"]
                .as_str()
                .unwrap()
                .contains("not in VERIFYING")
        );
    }

    #[test]
    fn handoff_and_master_rules_include_ack_ready_instruction() {
        let tmp = tempfile::tempdir().unwrap();
        let handoff = write_handoff_bundle(
            tmp.path(),
            &HandoffBundleInput {
                cutover_id: "cutover_rules",
                session_id: "sess_rules",
                socket_path: &tmp.path().join("ahd.sock"),
                state_dir: tmp.path(),
            },
        )
        .unwrap();
        let handoff_body = std::fs::read_to_string(handoff).unwrap();
        assert!(handoff_body.contains("ah master ack-ready --cutover-id \"$AH_CUTOVER_ID\""));
        assert!(
            crate::provider::builtin::MASTER_KERNEL
                .contains("ah master ack-ready --cutover-id \"$AH_CUTOVER_ID\"")
        );
    }
}
