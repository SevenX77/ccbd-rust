use crate::db::Db;
use crate::db::sessions::query_session_by_id;
use crate::error::CcbdError;
use crate::master_revival::{
    MasterDeathDecision, MasterTransitionOutcome, classify_master_death,
    complete_claimed_master_transition, confirm_master_stable, master_spawn_lock,
    record_master_revive_failure, remove_master_monitor_key_if_generation_matches,
    try_claim_master_transition,
};
use crate::provider::home_layout::sandbox_home_for_sandbox_dir;
use crate::sandbox::{EnvState, path, systemd};
use crate::tmux::{TmuxServer, master_session_name, sanitize_tmux_name};
use std::collections::HashMap;
use std::os::fd::OwnedFd;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::unix::AsyncFd;

#[cfg(test)]
type ShutdownTrigger = Arc<dyn Fn() + Send + Sync + 'static>;

pub fn spawn_master_pidfd_watch_task(
    session_id: String,
    expected_pid: i64,
    expected_generation: i64,
    pidfd: OwnedFd,
    db: Db,
    tmux_server: Arc<TmuxServer>,
    auto_shutdown_on_master_exit: bool,
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
                    auto_shutdown_on_master_exit,
                    master_cmd.clone(),
                    state_dir.clone(),
                    env_state.clone(),
                    daemon_unit.clone(),
                )
                .await
                {
                    tracing::warn!(session_id = %session_id, error = %err, "master revive failed");
                    let now = unixepoch();
                    if let Err(err) = record_master_revive_failure(
                        &db,
                        &session_id,
                        expected_pid,
                        expected_generation,
                        now,
                    ) {
                        tracing::warn!(session_id = %session_id, error = %err, "record master revive failure failed");
                    }
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
    auto_shutdown_on_master_exit: bool,
    master_cmd: String,
    state_dir: PathBuf,
    env_state: EnvState,
    daemon_unit: Option<String>,
) -> Result<(), CcbdError> {
    let spawn_lock = master_spawn_lock(&session_id);
    let _spawn_guard = spawn_lock.lock().await;
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
    let session = query_session_by_id(db.clone(), session_id.clone())
        .await?
        .ok_or_else(|| CcbdError::IpcInvalidRequest(format!("session not found: {session_id}")))?;
    let master_cwd: PathBuf = session.absolute_path.clone().into();
    let master_session = master_session_name(&session.project_id);
    let mut master_env_vars = HashMap::new();
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
        tmux_server,
        auto_shutdown_on_master_exit,
        master_cmd,
        state_dir,
        env_state,
        daemon_unit,
    );
    spawn_master_confirm_timer(db, session_id, new_pid, outcome.generation);
    Ok(())
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

#[cfg(test)]
async fn schedule_daemon_shutdown_if_idle(
    db: Db,
    auto_shutdown_on_master_exit: bool,
    grace: Duration,
    shutdown_trigger: ShutdownTrigger,
) -> Result<(), CcbdError> {
    if !auto_shutdown_on_master_exit {
        tracing::info!("master-exit daemon shutdown disabled by config");
        return Ok(());
    }

    let active_agents = crate::db::system::count_active_agents_daemon_wide(db.clone()).await?;
    if active_agents != 0 {
        tracing::info!(
            active_agents,
            "daemon-wide active agents remain after master exit; shutdown not scheduled"
        );
        return Ok(());
    }

    tracing::info!("daemon-wide active agents == 0 after master exit, scheduling shutdown in 5s");
    tokio::time::sleep(grace).await;

    let active_agents = crate::db::system::count_active_agents_daemon_wide(db).await?;
    if active_agents == 0 {
        tracing::info!("daemon-wide active agents still 0 after grace; triggering shutdown");
        shutdown_trigger();
    } else {
        tracing::info!(
            active_agents,
            "active agent appeared during grace, shutdown cancelled"
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        revive_master_after_exit, schedule_daemon_shutdown_if_idle, spawn_master_pidfd_watch_task,
    };
    use crate::db;
    use crate::db::agents::insert_agent_sync;
    use crate::db::agents_lifecycle::mark_agent_killed_sync;
    use crate::db::sessions::insert_session_sync;
    use crate::monitor::{contains, pidfd_open, register};
    use crate::tmux::{TmuxServer, master_session_name};
    use std::process::Command;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
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
            insert_agent_sync(&conn, &agent_id, &session_id, "bash", "IDLE", None).unwrap();
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
            true,
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
        assert_eq!(agent_state, "IDLE");
        assert!(!contains(&key));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_master_revive_production_lock_blocks_spawn_and_stale_cas_skips_spawn() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let state_dir = tempfile::TempDir::new().unwrap().keep();
        let project_dir = tempfile::TempDir::new().unwrap();
        let db = db::init(file.path()).unwrap();
        let session_id = format!("sess_lock_{}", uuid::Uuid::new_v4());
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
            true,
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
        assert!(marker.exists());
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
        revive_master_after_exit(
            session_id.clone(),
            111,
            5,
            db.clone(),
            tmux.clone(),
            true,
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
    async fn test_master_exit_idle_daemon_triggers_shutdown_after_grace() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let db = db::init(file.path()).unwrap();
        let triggered = Arc::new(AtomicBool::new(false));
        let trigger_seen = triggered.clone();

        schedule_daemon_shutdown_if_idle(
            db,
            true,
            Duration::from_millis(25),
            Arc::new(move || {
                trigger_seen.store(true, Ordering::SeqCst);
            }),
        )
        .await
        .unwrap();

        assert!(triggered.load(Ordering::SeqCst));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_master_exit_with_active_agent_does_not_trigger_shutdown() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let db = db::init(file.path()).unwrap();
        {
            let conn = db.conn();
            insert_session_sync(&conn, "sess_active", "p_active", "/tmp/active").unwrap();
            insert_agent_sync(&conn, "ag_active", "sess_active", "bash", "IDLE", None).unwrap();
        }
        let triggered = Arc::new(AtomicBool::new(false));
        let trigger_seen = triggered.clone();

        schedule_daemon_shutdown_if_idle(
            db,
            true,
            Duration::from_millis(25),
            Arc::new(move || {
                trigger_seen.store(true, Ordering::SeqCst);
            }),
        )
        .await
        .unwrap();

        assert!(!triggered.load(Ordering::SeqCst));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_master_exit_shutdown_cancelled_when_agent_appears_during_grace() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let db = db::init(file.path()).unwrap();
        {
            let conn = db.conn();
            insert_session_sync(&conn, "sess_recover", "p_recover", "/tmp/recover").unwrap();
        }
        let triggered = Arc::new(AtomicBool::new(false));
        let trigger_seen = triggered.clone();
        let db_for_insert = db.clone();
        let task = tokio::spawn(async move {
            schedule_daemon_shutdown_if_idle(
                db_for_insert,
                true,
                Duration::from_millis(100),
                Arc::new(move || {
                    trigger_seen.store(true, Ordering::SeqCst);
                }),
            )
            .await
        });

        tokio::time::sleep(Duration::from_millis(25)).await;
        {
            let conn = db.conn();
            insert_agent_sync(&conn, "ag_recover", "sess_recover", "bash", "IDLE", None).unwrap();
        }

        task.await.unwrap().unwrap();
        assert!(!triggered.load(Ordering::SeqCst));

        mark_agent_killed_sync(&db, "ag_recover", "TEST_CLEANUP").unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_master_exit_shutdown_disabled_by_config() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let db = db::init(file.path()).unwrap();
        let triggered = Arc::new(AtomicBool::new(false));
        let trigger_seen = triggered.clone();

        schedule_daemon_shutdown_if_idle(
            db,
            false,
            Duration::from_millis(25),
            Arc::new(move || {
                trigger_seen.store(true, Ordering::SeqCst);
            }),
        )
        .await
        .unwrap();

        assert!(!triggered.load(Ordering::SeqCst));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_master_exit_shutdown_disabled_returns_without_waiting_grace() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let db = db::init(file.path()).unwrap();
        let triggered = Arc::new(AtomicBool::new(false));
        let trigger_seen = triggered.clone();

        tokio::time::timeout(
            Duration::from_millis(50),
            schedule_daemon_shutdown_if_idle(
                db,
                false,
                Duration::from_secs(60),
                Arc::new(move || {
                    trigger_seen.store(true, Ordering::SeqCst);
                }),
            ),
        )
        .await
        .expect("disabled auto-shutdown should not wait for master-exit grace")
        .unwrap();

        assert!(!triggered.load(Ordering::SeqCst));
    }
}
