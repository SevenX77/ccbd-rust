use crate::db::Db;
use crate::db::sessions::query_session_by_id;
use crate::db::system::{
    MasterDeathSessionActivity, clean_worker_runtime_resources_sync,
    snapshot_master_death_session_activity,
};
use crate::error::CcbdError;
use crate::master_revival::{
    MasterDeathDecision, MasterReviveAttemptDecision, MasterTransitionOutcome,
    classify_master_death, complete_claimed_master_transition, confirm_master_stable,
    master_spawn_lock, query_master_revive_next_retry_at, record_master_revive_attempt,
    remove_master_monitor_key_if_generation_matches, try_claim_master_transition,
};
use crate::provider::home_layout::sandbox_home_for_sandbox_dir;
use crate::rpc::Ctx;
use crate::rpc::handlers::{RealignAgentParams, spawn_realign_agent};
use crate::sandbox::{EnvState, path, systemd};
use crate::tmux::{TmuxServer, master_session_name, sanitize_tmux_name};
use rusqlite::{OptionalExtension, params};
use std::collections::HashMap;
use std::os::fd::OwnedFd;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::unix::AsyncFd;

pub fn spawn_master_pidfd_watch_task(
    session_id: String,
    expected_pid: i64,
    expected_generation: i64,
    pidfd: OwnedFd,
    db: Db,
    tmux_server: Arc<TmuxServer>,
    master_cmd: String,
    state_dir: PathBuf,
    env_state: EnvState,
    daemon_unit: Option<String>,
) {
    tokio::spawn(async move {
        let async_fd = match AsyncFd::new(pidfd) {
            Ok(fd) => fd,
            Err(err) => {
                tracing::warn!(session_id = %session_id, error = %err, "failed to create AsyncFd for master pidfd");
                remove_master_monitor_key_if_generation_matches(&session_id, expected_generation);
                return;
            }
        };

        if let Err(err) = async_fd.readable().await {
            tracing::warn!(session_id = %session_id, error = %err, "master pidfd readiness wait failed");
            remove_master_monitor_key_if_generation_matches(&session_id, expected_generation);
            return;
        }

        tracing::info!(session_id = %session_id, expected_pid, expected_generation, "master process exited");

        match classify_master_death(&db, &session_id, expected_pid, expected_generation) {
            Ok(MasterDeathDecision::Revive) => {
                if let Err(err) = revive_master_after_exit(
                    session_id.clone(),
                    expected_pid,
                    expected_generation,
                    db.clone(),
                    tmux_server.clone(),
                    master_cmd.clone(),
                    state_dir.clone(),
                    env_state.clone(),
                    daemon_unit.clone(),
                )
                .await
                {
                    tracing::warn!(session_id = %session_id, error = %err, "master revive failed");
                }
            }
            Ok(MasterDeathDecision::IntentionalExit | MasterDeathDecision::Stale) => {
                tracing::info!(session_id = %session_id, "master exit ignored for non-active or stale session")
            }
            Err(err) => {
                tracing::warn!(session_id = %session_id, error = %err, "classify master death failed");
            }
        }

        remove_master_monitor_key_if_generation_matches(&session_id, expected_generation);
    });
}

pub fn monitor_key(session_id: &str) -> String {
    format!("master:{session_id}")
}

async fn revive_master_after_exit(
    session_id: String,
    expected_pid: i64,
    expected_generation: i64,
    db: Db,
    tmux_server: Arc<TmuxServer>,
    master_cmd: String,
    state_dir: PathBuf,
    env_state: EnvState,
    daemon_unit: Option<String>,
) -> Result<(), CcbdError> {
    let spawn_lock = master_spawn_lock(&session_id);
    let _spawn_guard = spawn_lock.lock().await;
    let snapshot = snapshot_master_death_session_activity(&db, &session_id)?;
    let daemon_marker = env_state
        .systemd_run_available
        .then(|| tmux_server.socket_name().to_string());
    let cleanup = clean_worker_runtime_resources_sync(
        &db,
        &session_id,
        &snapshot.worker_ids_to_reap,
        "MASTER_EXIT",
        daemon_marker.as_deref(),
    )?;
    tracing::warn!(
        session_id = %session_id,
        classification = ?snapshot.classification,
        workers = snapshot.worker_ids_to_reap.len(),
        db_killed = cleanup.db_killed_count,
        scope_stop_failures = cleanup.scope_stop_failures,
        pidfd_kill_failures = cleanup.pidfd_kill_failures,
        registry_cleanup_count = cleanup.registry_cleanup_count,
        "master death worker cleanup completed"
    );
    if snapshot.classification == MasterDeathSessionActivity::IdleNoWork {
        tracing::info!(
            session_id = %session_id,
            expected_pid,
            expected_generation,
            "idle master death reaped workers without reviving master"
        );
        return Ok(());
    }
    let now = unixepoch();
    if let Some(next_retry_at) =
        query_master_revive_next_retry_at(&db, &session_id, expected_pid, expected_generation)?
    {
        if now < next_retry_at {
            let delay_secs = (next_retry_at - now) as u64;
            tracing::warn!(
                session_id = %session_id,
                expected_pid,
                expected_generation,
                delay_secs,
                "master revive delayed by retry backoff"
            );
            tokio::time::sleep(Duration::from_secs(delay_secs)).await;
            if classify_master_death(&db, &session_id, expected_pid, expected_generation)?
                != MasterDeathDecision::Revive
            {
                tracing::info!(
                    session_id = %session_id,
                    expected_pid,
                    expected_generation,
                    "master revive backoff woke to stale or non-active session"
                );
                return Ok(());
            }
        }
    }
    match try_claim_master_transition(&db, &session_id, expected_pid, expected_generation)? {
        MasterTransitionOutcome::Claimed => {}
        MasterTransitionOutcome::Stale | MasterTransitionOutcome::NoChange => {
            tracing::info!(
                session_id = %session_id,
                expected_pid,
                expected_generation,
                "master revive skipped because transition claim is stale"
            );
            return Ok(());
        }
    }
    let claimed_generation = expected_generation + 1;
    match record_master_revive_attempt(
        &db,
        &session_id,
        expected_pid,
        claimed_generation,
        unixepoch(),
    )? {
        MasterReviveAttemptDecision::Spawn {
            retry_count,
            next_retry_at,
        } => tracing::warn!(
            session_id = %session_id,
            retry_count,
            next_retry_at,
            "master revive attempt recorded before spawning replacement"
        ),
        MasterReviveAttemptDecision::Fused => {
            tracing::error!(
                session_id = %session_id,
                claimed_generation,
                "master revive fuse threshold reached before spawning replacement"
            );
            return Ok(());
        }
        MasterReviveAttemptDecision::Stale => {
            tracing::info!(
                session_id = %session_id,
                claimed_generation,
                "master revive attempt went stale after transition claim"
            );
            return Ok(());
        }
    }
    let redispatch_marker_path = match write_master_revival_redispatch_marker(
        &state_dir,
        &session_id,
        expected_pid,
        expected_generation,
        claimed_generation,
        &snapshot.worker_ids_to_reap,
    ) {
        Ok(marker_path) => {
            tracing::warn!(
                session_id = %session_id,
                marker = %marker_path.display(),
                workers = snapshot.worker_ids_to_reap.len(),
                "master revive re-dispatch marker written"
            );
            Some(marker_path)
        }
        Err(err) => {
            tracing::warn!(
                session_id = %session_id,
                error = %err,
                "failed to write master revive re-dispatch marker; continuing revive"
            );
            None
        }
    };
    let session = query_session_by_id(db.clone(), session_id.clone())
        .await?
        .ok_or_else(|| CcbdError::IpcInvalidRequest(format!("session not found: {session_id}")))?;
    let master_cwd: PathBuf = session.absolute_path.clone().into();
    let master_session = master_session_name(&session.project_id);
    let mut master_env_vars = HashMap::new();
    master_env_vars.insert("AH_STATE_DIR".to_string(), state_dir.display().to_string());
    master_env_vars.insert(
        "CCB_SOCKET".to_string(),
        state_dir.join("ahd.sock").display().to_string(),
    );
    master_env_vars.insert("AH_MASTER_ROLE".to_string(), "managed".to_string());
    if let Some(marker_path) = redispatch_marker_path.as_ref() {
        master_env_vars.insert(
            "AH_REDISPATCH_MARKER".to_string(),
            marker_path.display().to_string(),
        );
    }
    let master_sandbox_home = if env_state.unsafe_no_sandbox {
        master_cwd.clone()
    } else {
        let sandbox_dir = path::resolve_sandbox_dir(&state_dir, &session_id, "master")?;
        let home_root = sandbox_home_for_sandbox_dir(&sandbox_dir)?;
        if home_root.join(".claude/.credentials.json").exists() {
            tracing::debug!(session_id = %session_id, home = %home_root.display(), "master revive reusing existing auth symlink");
        } else {
            tracing::warn!(session_id = %session_id, home = %home_root.display(), "master revive reusing existing home; auth symlink not present");
        }
        master_env_vars.insert("HOME".to_string(), home_root.display().to_string());
        master_env_vars.insert("CLAUDE_CONFIG_DIR".to_string(), ".claude".to_string());
        home_root
    };
    let tmux_cmd = if env_state.systemd_run_available || env_state.unsafe_no_sandbox {
        systemd::master_command_with_env(
            &session.project_id,
            &master_cmd,
            &env_state,
            daemon_unit.as_deref(),
            &master_env_vars,
        )
    } else {
        shell_command_with_env_prefix(&master_cmd, &master_env_vars)
    };
    tmux_server
        .ensure_session(master_session.clone(), master_cwd.clone())
        .await?;
    let pane = tmux_server
        .spawn_window(
            master_session,
            sanitize_tmux_name(&session.project_id),
            master_cwd,
            tmux_cmd,
        )
        .await?;
    let title = format!(
        "master ({})",
        master_cmd.split_whitespace().next().unwrap_or(&master_cmd)
    );
    let _ = tmux_server.set_pane_title(pane.clone(), &title).await;
    let new_pid = tmux_server.get_pane_pid(pane.clone()).await? as i64;
    let Some(outcome) = complete_claimed_master_transition(
        &db,
        &session_id,
        claimed_generation,
        new_pid,
        &pane.0,
        &master_sandbox_home,
    )?
    else {
        let _ = tmux_server.kill_pane(pane).await;
        tracing::warn!(
            session_id = %session_id,
            claimed_generation,
            "master revive finalize went stale after spawn; killed orphan pane"
        );
        return Ok(());
    };
    let pidfd = crate::monitor::pidfd_open(new_pid as i32)?;
    let task_fd = pidfd.try_clone().map_err(|err| {
        CcbdError::PtyIoError(format!("failed to clone revived master pidfd: {err}"))
    })?;
    crate::monitor::register(outcome.monitor_key.clone(), pidfd);
    spawn_master_pidfd_watch_task(
        session_id.clone(),
        new_pid,
        outcome.generation,
        task_fd,
        db.clone(),
        tmux_server.clone(),
        master_cmd,
        state_dir.clone(),
        env_state.clone(),
        daemon_unit.clone(),
    );
    reprovision_declared_workers_after_master_revive(
        &session_id,
        db.clone(),
        tmux_server.clone(),
        state_dir.clone(),
        env_state.clone(),
        daemon_unit.clone(),
    )
    .await?;
    spawn_master_confirm_timer(db, session_id, new_pid, outcome.generation);
    Ok(())
}

async fn reprovision_declared_workers_after_master_revive(
    session_id: &str,
    db: Db,
    tmux_server: Arc<TmuxServer>,
    state_dir: PathBuf,
    env_state: EnvState,
    daemon_unit: Option<String>,
) -> Result<(), CcbdError> {
    let stored_specs = {
        let conn = db.conn();
        crate::db::recovery::query_agent_spawn_specs_for_session_sync(&conn, session_id)?
    };
    if stored_specs.is_empty() {
        return Ok(());
    }
    let ctx = Ctx {
        db: db.clone(),
        state_dir,
        env_state,
        daemon_unit,
        tmux_server,
    };
    for stored in stored_specs {
        let agent_id = stored.spec.agent_id.clone();
        let agent = RealignAgentParams {
            agent_id: stored.spec.agent_id.clone(),
            provider: stored.spec.provider.clone(),
            env: stored.spec.env.clone(),
            hooks: stored.spec.hooks.clone(),
            plugins: stored.spec.plugins.clone(),
        };
        if let Err(err) =
            revive_reprovision_one_worker(&ctx, session_id, &agent, stored.config_hash.as_str())
                .await
        {
            if let Err(restore_err) = restore_killed_worker_spawn_spec(&ctx.db, session_id, &stored)
            {
                tracing::warn!(
                    session_id,
                    agent_id = %agent_id,
                    error = %restore_err,
                    "restore killed worker spawn spec failed during revive re-provision"
                );
            }
            tracing::warn!(
                session_id,
                agent_id = %agent_id,
                error = %err,
                "master revive worker re-provision failed; restored killed worker snapshot"
            );
        }
    }
    Ok(())
}

async fn revive_reprovision_one_worker(
    ctx: &Ctx,
    session_id: &str,
    agent: &RealignAgentParams,
    expected_hash: &str,
) -> Result<(), CcbdError> {
    let current_state: Option<String> = ctx
        .db
        .conn()
        .query_row(
            "SELECT state FROM agents WHERE id = ?",
            params![agent.agent_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|err| {
            CcbdError::DbConstraintViolation(format!("query revive worker state: {err}"))
        })?;
    if current_state.as_deref() == Some("KILLED") {
        crate::db::agents::delete_agent(ctx.db.clone(), agent.agent_id.clone()).await?;
    }
    spawn_realign_agent(ctx, session_id, agent, expected_hash, false, true).await
}

fn restore_killed_worker_spawn_spec(
    db: &Db,
    session_id: &str,
    stored: &crate::db::recovery::StoredAgentSpawnSpec,
) -> Result<(), CcbdError> {
    let conn = db.conn();
    let exists: Option<i64> = conn
        .query_row(
            "SELECT 1 FROM agents WHERE id = ? LIMIT 1",
            params![stored.spec.agent_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|err| {
            CcbdError::DbConstraintViolation(format!("query revive restore worker: {err}"))
        })?;
    if exists.is_none() {
        crate::db::agents::insert_agent_sync(
            &conn,
            &stored.spec.agent_id,
            session_id,
            &stored.spec.provider,
            "KILLED",
            None,
        )?;
    }
    crate::db::recovery::persist_agent_spawn_spec_sync(&conn, &stored.spec, &stored.config_hash)
}

fn spawn_master_confirm_timer(
    db: Db,
    session_id: String,
    expected_pid: i64,
    expected_generation: i64,
) {
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(60)).await;
        if let Err(err) = confirm_master_stable(&db, &session_id, expected_pid, expected_generation)
        {
            tracing::warn!(session_id = %session_id, error = %err, "master stable confirm failed");
        }
    });
}

fn unixepoch() -> i64 {
    match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(duration) => duration.as_secs() as i64,
        Err(_) => 0,
    }
}

fn shell_command_with_env_prefix(cmd: &str, env_vars: &HashMap<String, String>) -> Vec<String> {
    if env_vars.is_empty() {
        return vec!["sh".to_string(), "-lc".to_string(), cmd.to_string()];
    }
    let mut command = vec!["env".to_string()];
    let mut entries = env_vars.iter().collect::<Vec<_>>();
    entries.sort_by(|(left, _), (right, _)| left.cmp(right));
    for (key, value) in entries {
        command.push(format!("{key}={value}"));
    }
    command.extend(["sh".to_string(), "-lc".to_string(), cmd.to_string()]);
    command
}

fn master_revival_redispatch_marker_path(
    state_dir: &Path,
    session_id: &str,
    revived_generation: i64,
) -> PathBuf {
    state_dir
        .join("master-revival")
        .join(session_id)
        .join(format!("redispatch-generation-{revived_generation}.json"))
}

fn write_master_revival_redispatch_marker(
    state_dir: &Path,
    session_id: &str,
    expected_pid: i64,
    observed_generation: i64,
    revived_generation: i64,
    worker_ids_to_reap: &[String],
) -> std::io::Result<PathBuf> {
    let marker_path =
        master_revival_redispatch_marker_path(state_dir, session_id, revived_generation);
    if let Some(parent) = marker_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let body = serde_json::json!({
        "session_id": session_id,
        "expected_pid": expected_pid,
        "observed_generation": observed_generation,
        "revived_generation": revived_generation,
        "worker_ids_to_reap": worker_ids_to_reap,
        "redispatch_required": true,
        "hint": "Master was revived after master death; workers were reaped; inspect ah ps/logs and re-dispatch missing work."
    })
    .to_string();
    std::fs::write(&marker_path, body)?;
    Ok(marker_path)
}

#[cfg(test)]
mod tests {
    use super::{
        master_revival_redispatch_marker_path, revive_master_after_exit,
        spawn_master_pidfd_watch_task,
    };
    use crate::db;
    use crate::db::agents::insert_agent_sync;
    use crate::db::recovery::{AgentSpawnSpec, persist_agent_spawn_spec_sync};
    use crate::db::jobs::insert_job_sync;
    use crate::db::sessions::insert_session_sync;
    use crate::monitor::{contains, pidfd_open, register};
    use crate::tmux::{TmuxServer, master_session_name};
    use std::process::Command;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    async fn sleep_ms(ms: u64) {
        tokio::task::spawn_blocking(move || std::thread::sleep(Duration::from_millis(ms)))
            .await
            .unwrap();
    }

    async fn wait_until_monitor_absent(key: &str) -> bool {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if !contains(key) {
                return true;
            }
            sleep_ms(50).await;
        }
        false
    }

    async fn wait_until_path_exists(path: &std::path::Path) -> bool {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if path.exists() {
                return true;
            }
            sleep_ms(50).await;
        }
        false
    }

    async fn wait_for_session_status(db: &db::Db, session_id: &str, expected: &str) -> bool {
        let deadline = Instant::now() + Duration::from_secs(25);
        while Instant::now() < deadline {
            let status = db
                .conn()
                .query_row(
                    "SELECT status FROM sessions WHERE id = ?1",
                    [session_id],
                    |row| row.get::<_, String>(0),
                )
                .ok();
            if status.as_deref() == Some(expected) {
                return true;
            }
            sleep_ms(100).await;
        }
        false
    }

    async fn wait_for_agent_state(db: &db::Db, agent_id: &str, expected: &str) -> bool {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            let state = db
                .conn()
                .query_row(
                    "SELECT state FROM agents WHERE id = ?1",
                    [agent_id],
                    |row| row.get::<_, String>(0),
                )
                .ok();
            if state.as_deref() == Some(expected) {
                return true;
            }
            sleep_ms(50).await;
        }
        false
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn active_work_master_death_reaps_worker_revives_master_and_requires_redispatch_marker() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let state_dir = tempfile::TempDir::new().unwrap().keep();
        let project_dir = tempfile::TempDir::new().unwrap();
        let db = db::init(file.path()).unwrap();
        let session_id = format!("sess_batch_f_active_{}", uuid::Uuid::new_v4());
        let agent_id = format!("ag_batch_f_active_{}", uuid::Uuid::new_v4());
        let project_id = format!("p_batch_f_active_{}", uuid::Uuid::new_v4());
        {
            let conn = db.conn();
            insert_session_sync(
                &conn,
                &session_id,
                &project_id,
                project_dir.path().to_str().unwrap(),
            )
            .unwrap();
            conn.execute(
                "UPDATE sessions
                 SET status = 'ACTIVE', master_pid = 111, master_generation = 5
                 WHERE id = ?1",
                [&session_id],
            )
            .unwrap();
            insert_agent_sync(&conn, &agent_id, &session_id, "bash", "BUSY", Some(10)).unwrap();
            insert_job_sync(
                &conn,
                "job_batch_f_active",
                &agent_id,
                None,
                "in flight",
            )
            .unwrap();
            conn.execute(
                "UPDATE jobs SET status = 'DISPATCHED', dispatched_at = unixepoch() WHERE id = 'job_batch_f_active'",
                [],
            )
            .unwrap();
        }

        let spawn_marker = state_dir.join("batch-f-active-master-spawned");
        let redispatch_marker = master_revival_redispatch_marker_path(&state_dir, &session_id, 6);
        let tmux = Arc::new(TmuxServer::new(&state_dir));
        revive_master_after_exit(
            session_id.clone(),
            111,
            5,
            db.clone(),
            tmux.clone(),
            format!("touch {}; sleep 5", spawn_marker.display()),
            state_dir.clone(),
            crate::sandbox::EnvState {
                systemd_run_available: false,
                unsafe_no_sandbox: true,
                under_systemd: false,
            },
            None,
        )
        .await
        .unwrap();

        assert!(wait_for_agent_state(&db, &agent_id, "KILLED").await);
        assert!(wait_until_path_exists(&spawn_marker).await);
        let marker_body = std::fs::read_to_string(&redispatch_marker).unwrap_or_else(|err| {
            panic!(
                "expected redispatch marker at {}: {err}",
                redispatch_marker.display()
            )
        });
        assert!(marker_body.contains(&session_id));
        assert!(marker_body.contains(&agent_id));
        assert!(marker_body.contains("\"revived_generation\":6"));
        assert!(marker_body.contains("\"redispatch_required\":true"));
        assert!(marker_body.contains("re-dispatch"));
        let _ = tmux.kill_session(master_session_name(&project_id)).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn active_work_master_revival_reprovisions_declared_killed_worker() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let state_dir = tempfile::TempDir::new().unwrap().keep();
        let project_dir = tempfile::TempDir::new().unwrap();
        let db = db::init(file.path()).unwrap();
        let session_id = format!("sess_gap1b_{}", uuid::Uuid::new_v4());
        let agent_id = format!("ag_gap1b_{}", uuid::Uuid::new_v4());
        let project_id = format!("p_gap1b_{}", uuid::Uuid::new_v4());
        {
            let conn = db.conn();
            insert_session_sync(
                &conn,
                &session_id,
                &project_id,
                project_dir.path().to_str().unwrap(),
            )
            .unwrap();
            conn.execute(
                "UPDATE sessions
                 SET status = 'ACTIVE', master_pid = 111, master_generation = 5
                 WHERE id = ?1",
                [&session_id],
            )
            .unwrap();
            insert_agent_sync(&conn, &agent_id, &session_id, "bash", "BUSY", Some(10)).unwrap();
            persist_agent_spawn_spec_sync(
                &conn,
                &AgentSpawnSpec {
                    agent_id: agent_id.clone(),
                    provider: "bash".to_string(),
                    env: std::collections::HashMap::from([(
                        "REVIVE_WORKER_ENV".to_string(),
                        "1".to_string(),
                    )]),
                    hooks: std::collections::HashMap::new(),
                    plugins: Vec::new(),
                },
                "hash-gap1b",
            )
            .unwrap();
            insert_job_sync(&conn, "job_gap1b", &agent_id, None, "in flight").unwrap();
            conn.execute(
                "UPDATE jobs SET status = 'DISPATCHED', dispatched_at = unixepoch() WHERE id = 'job_gap1b'",
                [],
            )
            .unwrap();
        }

        let spawn_marker = state_dir.join("gap1b-master-spawned");
        let tmux = Arc::new(TmuxServer::new(&state_dir));
        revive_master_after_exit(
            session_id.clone(),
            111,
            5,
            db.clone(),
            tmux.clone(),
            format!("touch {}; sleep 5", spawn_marker.display()),
            state_dir.clone(),
            crate::sandbox::EnvState {
                systemd_run_available: false,
                unsafe_no_sandbox: true,
                under_systemd: false,
            },
            None,
        )
        .await
        .unwrap();

        assert!(wait_until_path_exists(&spawn_marker).await);
        assert!(
            wait_for_agent_state(&db, &agent_id, "IDLE").await,
            "declared worker should be re-provisioned after master revive instead of remaining KILLED"
        );
        let _ = tmux.kill_session(master_session_name(&project_id)).await;
        let _ = tmux.kill_session(crate::tmux::agent_session_name(&agent_id)).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn idle_master_death_reaps_without_revive() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let state_dir = tempfile::TempDir::new().unwrap().keep();
        let project_dir = tempfile::TempDir::new().unwrap();
        let db = db::init(file.path()).unwrap();
        let session_id = format!("sess_batch_f_idle_{}", uuid::Uuid::new_v4());
        let agent_id = format!("ag_batch_f_idle_{}", uuid::Uuid::new_v4());
        let project_id = format!("p_batch_f_idle_{}", uuid::Uuid::new_v4());
        {
            let conn = db.conn();
            insert_session_sync(
                &conn,
                &session_id,
                &project_id,
                project_dir.path().to_str().unwrap(),
            )
            .unwrap();
            conn.execute(
                "UPDATE sessions
                 SET status = 'ACTIVE', master_pid = 111, master_generation = 5
                 WHERE id = ?1",
                [&session_id],
            )
            .unwrap();
            insert_agent_sync(&conn, &agent_id, &session_id, "bash", "IDLE", Some(10)).unwrap();
        }

        let spawn_marker = state_dir.join("batch-f-idle-master-spawned");
        let redispatch_marker = master_revival_redispatch_marker_path(&state_dir, &session_id, 6);
        let tmux = Arc::new(TmuxServer::new(&state_dir));
        revive_master_after_exit(
            session_id.clone(),
            111,
            5,
            db.clone(),
            tmux.clone(),
            format!("touch {}", spawn_marker.display()),
            state_dir.clone(),
            crate::sandbox::EnvState {
                systemd_run_available: false,
                unsafe_no_sandbox: true,
                under_systemd: false,
            },
            None,
        )
        .await
        .unwrap();

        assert!(wait_for_agent_state(&db, &agent_id, "KILLED").await);
        assert!(
            !spawn_marker.exists(),
            "IdleNoWork master death must reap workers without reviving master"
        );
        assert!(
            !redispatch_marker.exists(),
            "IdleNoWork master death must not create a re-dispatch marker"
        );
        let _ = tmux.kill_session(master_session_name(&project_id)).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn revived_master_env_contains_state_dir_socket_and_redispatch_marker() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let state_dir = tempfile::TempDir::new().unwrap().keep();
        let project_dir = tempfile::TempDir::new().unwrap();
        let db = db::init(file.path()).unwrap();
        let session_id = format!("sess_batch_g_env_{}", uuid::Uuid::new_v4());
        let agent_id = format!("ag_batch_g_env_{}", uuid::Uuid::new_v4());
        let project_id = format!("p_batch_g_env_{}", uuid::Uuid::new_v4());
        {
            let conn = db.conn();
            insert_session_sync(
                &conn,
                &session_id,
                &project_id,
                project_dir.path().to_str().unwrap(),
            )
            .unwrap();
            conn.execute(
                "UPDATE sessions
                 SET status = 'ACTIVE', master_pid = 111, master_generation = 5
                 WHERE id = ?1",
                [&session_id],
            )
            .unwrap();
            insert_agent_sync(&conn, &agent_id, &session_id, "bash", "BUSY", Some(10)).unwrap();
            insert_job_sync(&conn, "job_batch_g_env", &agent_id, None, "in flight").unwrap();
            conn.execute(
                "UPDATE jobs SET status = 'DISPATCHED', dispatched_at = unixepoch() WHERE id = 'job_batch_g_env'",
                [],
            )
            .unwrap();
        }

        let env_capture = state_dir.join("batch-g-revived-master-env");
        let redispatch_marker = master_revival_redispatch_marker_path(&state_dir, &session_id, 6);
        let expected_socket = state_dir.join("ahd.sock");
        let tmux = Arc::new(TmuxServer::new(&state_dir));
        revive_master_after_exit(
            session_id.clone(),
            111,
            5,
            db.clone(),
            tmux.clone(),
            format!(
                "printf '%s\n%s\n%s\n%s\n' \"$AH_STATE_DIR\" \"$CCB_SOCKET\" \"$AH_MASTER_ROLE\" \"$AH_REDISPATCH_MARKER\" > {}; sleep 5",
                env_capture.display()
            ),
            state_dir.clone(),
            crate::sandbox::EnvState {
                systemd_run_available: false,
                unsafe_no_sandbox: true,
                under_systemd: false,
            },
            None,
        )
        .await
        .unwrap();

        assert!(wait_for_agent_state(&db, &agent_id, "KILLED").await);
        assert!(wait_until_path_exists(&env_capture).await);
        let env_lines = std::fs::read_to_string(&env_capture)
            .unwrap()
            .lines()
            .map(str::to_string)
            .collect::<Vec<_>>();
        assert_eq!(env_lines[0], state_dir.display().to_string());
        assert_eq!(env_lines[1], expected_socket.display().to_string());
        assert_eq!(env_lines[2], "managed");
        assert_eq!(env_lines[3], redispatch_marker.display().to_string());
        assert!(redispatch_marker.exists());
        let _ = tmux.kill_session(master_session_name(&project_id)).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "manual true-scope dogfood harness; requires a real ah pane and external master kill"]
    async fn dogfood_master_revive_redispatch_marker_true_scope() {
        // Manual Batch F acceptance:
        // 1. Start/attach a green ah-managed master pane.
        // 2. From that pane, dispatch a real worker task that runs for more than 10 seconds.
        // 3. From an external shell, kill -9 the master pid.
        // 4. Confirm ahd reaps the old worker, revives master, and the revived master observes the
        //    recovery marker before re-dispatching the lost work to successful completion.
        // 5. Repeat with an idle master: kill -9 should reap workers but not revive master.
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_master_watch_revives_active_session_on_master_exit() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let state_dir = tempfile::TempDir::new().unwrap().keep();
        let db = db::init(file.path()).unwrap();
        let session_id = format!("sess_master_{}", uuid::Uuid::new_v4());
        let agent_id = format!("ag_mw_{}", uuid::Uuid::new_v4());

        {
            let conn = db.conn();
            insert_session_sync(&conn, &session_id, "p1", "/tmp/foo").unwrap();
            insert_agent_sync(&conn, &agent_id, &session_id, "bash", "BUSY", None).unwrap();
            insert_job_sync(
                &conn,
                "job_master_watch_active",
                &agent_id,
                None,
                "in flight",
            )
            .unwrap();
            conn.execute(
                "UPDATE jobs SET status = 'DISPATCHED', dispatched_at = unixepoch() WHERE id = 'job_master_watch_active'",
                [],
            )
            .unwrap();
        }

        let mut child = Command::new("sh")
            .arg("-c")
            .arg("sleep 0.2; exit 0")
            .spawn()
            .unwrap();
        let pidfd = pidfd_open(child.id() as i32).unwrap();
        let task_fd = pidfd.try_clone().unwrap();
        let key = crate::master_revival::master_monitor_key(&session_id, 0);
        register(key.clone(), pidfd);

        let tmux = Arc::new(TmuxServer::new(&state_dir));
        spawn_master_pidfd_watch_task(
            session_id.clone(),
            0,
            0,
            task_fd,
            db.clone(),
            tmux,
            "/bin/sleep 5".to_string(),
            state_dir,
            crate::sandbox::EnvState {
                systemd_run_available: false,
                unsafe_no_sandbox: true,
                under_systemd: false,
            },
            None,
        );

        assert!(wait_until_monitor_absent(&key).await);
        let _ = child.wait();

        let session_status: String = db
            .conn()
            .query_row(
                "SELECT status FROM sessions WHERE id = ?",
                [&session_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(session_status, "ACTIVE");

        let agent_state: String = db
            .conn()
            .query_row(
                "SELECT state FROM agents WHERE id = ?",
                [&agent_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(agent_state, "KILLED");
        assert!(!contains(&key));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_master_revive_production_lock_blocks_spawn_and_stale_cas_skips_spawn() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let state_dir = tempfile::TempDir::new().unwrap().keep();
        let project_dir = tempfile::TempDir::new().unwrap();
        let db = db::init(file.path()).unwrap();
        let session_id = format!("sess_lock_{}", uuid::Uuid::new_v4());
        let agent_id = format!("ag_lock_{}", uuid::Uuid::new_v4());
        let project_id = format!("p_lock_{}", uuid::Uuid::new_v4());
        {
            let conn = db.conn();
            insert_session_sync(
                &conn,
                &session_id,
                &project_id,
                project_dir.path().to_str().unwrap(),
            )
            .unwrap();
            conn.execute(
                "UPDATE sessions
                 SET status = 'ACTIVE', master_pid = 111, master_generation = 5
                 WHERE id = ?1",
                [&session_id],
            )
            .unwrap();
            insert_agent_sync(&conn, &agent_id, &session_id, "bash", "BUSY", Some(10)).unwrap();
            insert_job_sync(&conn, "job_lock_active", &agent_id, None, "in flight").unwrap();
            conn.execute(
                "UPDATE jobs SET status = 'DISPATCHED', dispatched_at = unixepoch() WHERE id = 'job_lock_active'",
                [],
            )
            .unwrap();
        }

        let marker = state_dir.join("master-lock-spawned");
        let tmux = Arc::new(TmuxServer::new(&state_dir));
        let lock = crate::master_revival::master_spawn_lock(&session_id);
        let guard = lock.lock().await;
        let task = tokio::spawn(revive_master_after_exit(
            session_id.clone(),
            111,
            5,
            db.clone(),
            tmux.clone(),
            format!("touch {}; sleep 5", marker.display()),
            state_dir.clone(),
            crate::sandbox::EnvState {
                systemd_run_available: false,
                unsafe_no_sandbox: true,
                under_systemd: false,
            },
            None,
        ));

        sleep_ms(150).await;
        assert!(
            !marker.exists(),
            "revive_master_after_exit must acquire master_spawn_lock before spawning"
        );
        let generation_before_unlock: i64 = db
            .conn()
            .query_row(
                "SELECT master_generation FROM sessions WHERE id = ?1",
                [&session_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(generation_before_unlock, 5);

        drop(guard);
        task.await.unwrap().unwrap();
        assert!(wait_until_path_exists(&marker).await);
        let generation_after_spawn: i64 = db
            .conn()
            .query_row(
                "SELECT master_generation FROM sessions WHERE id = ?1",
                [&session_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(generation_after_spawn, 6);

        let stale_marker = state_dir.join("master-stale-spawned");
        {
            let conn = db.conn();
            let stale_agent_id = format!("ag_lock_stale_{}", uuid::Uuid::new_v4());
            insert_agent_sync(
                &conn,
                &stale_agent_id,
                &session_id,
                "bash",
                "BUSY",
                Some(11),
            )
            .unwrap();
            insert_job_sync(
                &conn,
                "job_lock_stale_active",
                &stale_agent_id,
                None,
                "in flight",
            )
            .unwrap();
            conn.execute(
                "UPDATE jobs SET status = 'DISPATCHED', dispatched_at = unixepoch() WHERE id = 'job_lock_stale_active'",
                [],
            )
            .unwrap();
        }
        revive_master_after_exit(
            session_id.clone(),
            111,
            5,
            db.clone(),
            tmux.clone(),
            format!("touch {}", stale_marker.display()),
            state_dir.clone(),
            crate::sandbox::EnvState {
                systemd_run_available: false,
                unsafe_no_sandbox: true,
                under_systemd: false,
            },
            None,
        )
        .await
        .unwrap();
        assert!(
            !stale_marker.exists(),
            "stale CAS must return before spawning a replacement master"
        );

        db.conn()
            .execute(
                "UPDATE sessions SET status = 'KILLED' WHERE id = ?1",
                [&session_id],
            )
            .unwrap();
        let _ = tmux.kill_session(master_session_name(&project_id)).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_master_revive_reaps_worker_before_backoff_sleep() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let state_dir = tempfile::TempDir::new().unwrap().keep();
        let project_dir = tempfile::TempDir::new().unwrap();
        let db = db::init(file.path()).unwrap();
        let session_id = format!("sess_backoff_{}", uuid::Uuid::new_v4());
        let agent_id = format!("ag_backoff_{}", uuid::Uuid::new_v4());
        let project_id = format!("p_backoff_{}", uuid::Uuid::new_v4());
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        {
            let conn = db.conn();
            insert_session_sync(
                &conn,
                &session_id,
                &project_id,
                project_dir.path().to_str().unwrap(),
            )
            .unwrap();
            insert_agent_sync(&conn, &agent_id, &session_id, "bash", "BUSY", Some(10)).unwrap();
            insert_job_sync(&conn, "job_backoff", &agent_id, None, "in flight").unwrap();
            conn.execute(
                "UPDATE sessions
                 SET status = 'ACTIVE',
                     master_pid = 111,
                     master_generation = 5,
                     master_next_retry_at = ?2
                 WHERE id = ?1",
                rusqlite::params![&session_id, now + 1],
            )
            .unwrap();
            conn.execute(
                "UPDATE jobs SET status = 'DISPATCHED', dispatched_at = unixepoch() WHERE id = 'job_backoff'",
                [],
            )
            .unwrap();
        }

        let marker = state_dir.join("master-backoff-spawned");
        let tmux = Arc::new(TmuxServer::new(&state_dir));
        let task = tokio::spawn(revive_master_after_exit(
            session_id.clone(),
            111,
            5,
            db.clone(),
            tmux.clone(),
            format!("touch {}; sleep 5", marker.display()),
            state_dir.clone(),
            crate::sandbox::EnvState {
                systemd_run_available: false,
                unsafe_no_sandbox: true,
                under_systemd: false,
            },
            None,
        ));

        assert!(wait_for_agent_state(&db, &agent_id, "KILLED").await);
        assert!(
            !marker.exists(),
            "worker must be reaped before delayed master revive is allowed to spawn"
        );
        task.await.unwrap().unwrap();
        let _ = tmux.kill_session(master_session_name(&project_id)).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_master_revive_fuse_reaps_worker_and_does_not_spawn() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let state_dir = tempfile::TempDir::new().unwrap().keep();
        let project_dir = tempfile::TempDir::new().unwrap();
        let db = db::init(file.path()).unwrap();
        let session_id = format!("sess_fuse_reap_{}", uuid::Uuid::new_v4());
        let agent_id = format!("ag_fuse_reap_{}", uuid::Uuid::new_v4());
        let project_id = format!("p_fuse_reap_{}", uuid::Uuid::new_v4());
        {
            let conn = db.conn();
            insert_session_sync(
                &conn,
                &session_id,
                &project_id,
                project_dir.path().to_str().unwrap(),
            )
            .unwrap();
            insert_agent_sync(&conn, &agent_id, &session_id, "bash", "BUSY", Some(10)).unwrap();
            insert_job_sync(&conn, "job_fuse_reap", &agent_id, None, "in flight").unwrap();
            conn.execute(
                "UPDATE sessions
                 SET status = 'ACTIVE',
                     master_pid = 111,
                     master_generation = 5,
                     master_retry_count = 4
                 WHERE id = ?1",
                [&session_id],
            )
            .unwrap();
            conn.execute(
                "UPDATE jobs SET status = 'DISPATCHED', dispatched_at = unixepoch() WHERE id = 'job_fuse_reap'",
                [],
            )
            .unwrap();
        }

        let marker = state_dir.join("master-fuse-spawned");
        let tmux = Arc::new(TmuxServer::new(&state_dir));
        revive_master_after_exit(
            session_id.clone(),
            111,
            5,
            db.clone(),
            tmux.clone(),
            format!("touch {}", marker.display()),
            state_dir.clone(),
            crate::sandbox::EnvState {
                systemd_run_available: false,
                unsafe_no_sandbox: true,
                under_systemd: false,
            },
            None,
        )
        .await
        .unwrap();

        assert!(
            !marker.exists(),
            "fused revive must not spawn replacement master"
        );
        assert!(wait_for_session_status(&db, &session_id, "FAILED").await);
        assert!(wait_for_agent_state(&db, &agent_id, "KILLED").await);
        let _ = tmux.kill_session(master_session_name(&project_id)).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_master_revive_startup_crash_loop_is_backed_off_and_fused() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let state_dir = tempfile::TempDir::new().unwrap().keep();
        let project_dir = tempfile::TempDir::new().unwrap();
        let db = db::init(file.path()).unwrap();
        let session_id = format!("sess_crash_loop_{}", uuid::Uuid::new_v4());
        let agent_id = format!("ag_crash_loop_{}", uuid::Uuid::new_v4());
        let project_id = format!("p_crash_loop_{}", uuid::Uuid::new_v4());
        {
            let conn = db.conn();
            insert_session_sync(
                &conn,
                &session_id,
                &project_id,
                project_dir.path().to_str().unwrap(),
            )
            .unwrap();
            insert_agent_sync(&conn, &agent_id, &session_id, "bash", "BUSY", Some(10)).unwrap();
            insert_job_sync(&conn, "job_crash_loop", &agent_id, None, "in flight").unwrap();
            conn.execute(
                "UPDATE jobs SET status = 'DISPATCHED', dispatched_at = unixepoch() WHERE id = 'job_crash_loop'",
                [],
            )
            .unwrap();
            conn.execute(
                "UPDATE sessions SET master_retry_count = 4 WHERE id = ?1",
                [&session_id],
            )
            .unwrap();
        }

        let mut child = Command::new("sh")
            .arg("-c")
            .arg("sleep 0.1; exit 0")
            .spawn()
            .unwrap();
        let pidfd = pidfd_open(child.id() as i32).unwrap();
        let task_fd = pidfd.try_clone().unwrap();
        let key = crate::master_revival::master_monitor_key(&session_id, 0);
        register(key.clone(), pidfd);

        let tmux = Arc::new(TmuxServer::new(&state_dir));
        spawn_master_pidfd_watch_task(
            session_id.clone(),
            0,
            0,
            task_fd,
            db.clone(),
            tmux.clone(),
            "sleep 0.1; exit 0".to_string(),
            state_dir,
            crate::sandbox::EnvState {
                systemd_run_available: false,
                unsafe_no_sandbox: true,
                under_systemd: false,
            },
            None,
        );

        assert!(wait_for_session_status(&db, &session_id, "FAILED").await);
        let _ = child.wait();
        let retry_count: i64 = db
            .conn()
            .query_row(
                "SELECT master_retry_count FROM sessions WHERE id = ?1",
                [&session_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(retry_count, 5);
        let generation: i64 = db
            .conn()
            .query_row(
                "SELECT master_generation FROM sessions WHERE id = ?1",
                [&session_id],
                |row| row.get(0),
            )
            .unwrap();
        assert!(
            generation <= 5,
            "startup crash loop must be bounded by fuse threshold, got generation {generation}"
        );
        let agent_state: String = db
            .conn()
            .query_row(
                "SELECT state FROM agents WHERE id = ?1",
                [&agent_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(agent_state, "KILLED");
        assert!(!contains(&key));
        let _ = tmux.kill_session(master_session_name(&project_id)).await;
    }
}
