use super::params::{extension_config_from_params, required_str};
use crate::db::master_cutovers::{
    MasterCutoverClaim, MasterCutoverUpdate, claim_master_cutover,
    update_master_cutover_spawn_metadata, update_master_cutover_state,
};
use crate::db::schema::Session;
use crate::db::sessions::{
    create_session, list_session_summaries, query_session_by_id, set_session_master_pane_id,
    update_session_config_hash,
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
use crate::provider::extensions::ExtensionConfig;
use crate::provider::fingerprint::{ConfigFingerprintInput, ConfigRole, compute_config_hash};
use crate::provider::home_layout::{HomeLayoutRole, prepare_home_layout_with_extensions};
use crate::rpc::Ctx;
use crate::sandbox::{path, systemd};
use crate::tmux::scope::{self, ScopePolicy};
use crate::tmux::{TmuxPaneId, agent_session_name, master_session_name, sanitize_tmux_name};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
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

#[derive(Debug, Clone)]
pub(crate) struct SpawnMasterPaneParams {
    pub(crate) session_id: String,
    pub(crate) cmd: String,
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
    let extra_env = parse_extra_env(&params)?;
    let outcome = spawn_master_pane_inner(
        ctx,
        SpawnMasterPaneParams {
            session_id: session_id.to_string(),
            cmd: cmd.to_string(),
            extensions,
            extra_env,
            claimed_master_generation,
        },
    )
    .await?;

    Ok(json!({ "pane_id": outcome.pane_id }))
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
        let home_overrides = prepare_home_layout_with_extensions(
            "claude",
            dir,
            &master_cwd,
            HomeLayoutRole::Master,
            &params.extensions,
        )?;
        home_root = Some(home_overrides.home_root);
        master_env_vars.extend(home_overrides.extra_env);
    }

    Ok(MasterPanePlan {
        session,
        master_cwd,
        master_env_vars,
        home_root,
    })
}

pub(crate) async fn spawn_master_pane_inner(
    ctx: &Ctx,
    params: SpawnMasterPaneParams,
) -> Result<SpawnMasterPaneOutcome, CcbdError> {
    let plan = prepare_master_pane_plan(ctx, &params).await?;
    spawn_prepared_master_pane(ctx, params, plan).await
}

async fn spawn_prepared_master_pane(
    ctx: &Ctx,
    params: SpawnMasterPaneParams,
    plan: MasterPanePlan,
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
        .ensure_session(master_session.clone(), plan.master_cwd.clone())
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
        hooks: &params.extensions.hooks,
        plugins: &params.extensions.plugins,
    })?;
    update_session_config_hash(ctx.db.clone(), params.session_id.clone(), config_hash).await?;

    let mut new_pid = None;
    let mut generation = None;
    match ctx.tmux_server.get_pane_pid(pane.clone()).await {
        Ok(pid) => match monitor::pidfd_open(pid) {
            Ok(pidfd) => {
                let task_fd = match pidfd.try_clone() {
                    Ok(fd) => fd,
                    Err(err) => {
                        tracing::warn!(session_id = %params.session_id, error = %err, "failed to clone master pidfd");
                        return Ok(SpawnMasterPaneOutcome {
                            pane_id: pane.0,
                            home_root: plan.home_root,
                            new_pid: Some(i64::from(pid)),
                            generation: None,
                        });
                    }
                };
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
                    record_spawned_master_runtime(
                        &ctx.db,
                        &params.session_id,
                        &pane.0,
                        i64::from(pid),
                    )?
                };
                new_pid = Some(i64::from(pid));
                generation = Some(recorded_generation);
                let key = master_monitor_key(&params.session_id, recorded_generation);
                monitor::register(key, pidfd);
                spawn_master_pidfd_watch_task(
                    params.session_id.clone(),
                    i64::from(pid),
                    recorded_generation,
                    task_fd,
                    ctx.db.clone(),
                    ctx.tmux_server.clone(),
                    params.cmd.clone(),
                    ctx.state_dir.clone(),
                    ctx.env_state.clone(),
                    ctx.daemon_unit.clone(),
                );
            }
            Err(err) => {
                tracing::warn!(session_id = %params.session_id, pid, error = %err, "failed to open master pidfd");
                new_pid = Some(i64::from(pid));
            }
        },
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

#[derive(Debug, Deserialize)]
struct MasterCutoverMasterParams {
    #[serde(default)]
    cmd: String,
    #[serde(default)]
    hooks: HashMap<String, Vec<crate::provider::extensions::HookGroup>>,
    #[serde(default)]
    plugins: Vec<String>,
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
        Box::pin(spawn_prepared_master_pane(ctx, params, plan))
    })
    .await
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
            extensions: ExtensionConfig {
                hooks: request.master.hooks.clone(),
                plugins: request.master.plugins.clone(),
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
        tracing::info!(%session_id, %cutover_id, "master cutover readiness stub accepted");
        if update_master_cutover_state(&ctx.db, &cutover_id, "VERIFYING", "ACTIVE")?
            != MasterCutoverUpdate::Updated
        {
            tracing::warn!(%session_id, %cutover_id, "master cutover VERIFYING->ACTIVE CAS stale");
            return Err(CcbdError::IpcInvalidRequest("master cutover stale before ACTIVE".into()));
        }
        current_state = "ACTIVE";
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
        }))
    }
    .await;

    if let Err(err) = result.as_ref() {
        if current_state != "ACTIVE" {
            let _ = update_master_cutover_state(&ctx.db, &cutover_id, current_state, "FAILED");
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
                "master_pane_id": session.master_pane_id,
                "active_agents": session.active_agents,
                "created_at": session.created_at,
            })
        })
        .collect::<Vec<_>>();
    Ok(json!({ "sessions": sessions }))
}

#[cfg(test)]
mod master_cutover_tests {
    use super::*;
    use crate::db;
    use crate::db::master_cutovers::get_active_master_cutover;
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
                hooks: HashMap::new(),
                plugins: Vec::new(),
            },
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

        let response = run_master_cutover_with_spawn(
            &ctx,
            request(&tmp, &old_home),
            move |_ctx, _params, plan| {
                let order = order_for_spawn.clone();
                let seen_home = seen_home_for_spawn.clone();
                Box::pin(async move {
                    let master_home = plan.home_root.clone().expect("master home");
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
                        new_pid: Some(9001),
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
        assert_eq!(active.new_master_pid, Some(9001));
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
}
