use crate::db::{self, Db};
use crate::error::CcbdError;
use crate::tmux::{TmuxServer, agent_session_name};
use std::os::fd::OwnedFd;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::unix::AsyncFd;

type ShutdownTrigger = Arc<dyn Fn() + Send + Sync + 'static>;
const MASTER_EXIT_SHUTDOWN_GRACE: Duration = Duration::from_secs(5);

pub fn spawn_master_pidfd_watch_task(
    session_id: String,
    pidfd: OwnedFd,
    db: Db,
    tmux_server: Arc<TmuxServer>,
    auto_shutdown_on_master_exit: bool,
) {
    let key = monitor_key(&session_id);
    tokio::spawn(async move {
        let async_fd = match AsyncFd::new(pidfd) {
            Ok(fd) => fd,
            Err(err) => {
                tracing::warn!(session_id = %session_id, error = %err, "failed to create AsyncFd for master pidfd");
                let _ = crate::monitor::remove(&key);
                return;
            }
        };

        if let Err(err) = async_fd.readable().await {
            tracing::warn!(session_id = %session_id, error = %err, "master pidfd readiness wait failed");
            let _ = crate::monitor::remove(&key);
            return;
        }

        tracing::info!(session_id = %session_id, "master process exited, cascading session kill");

        if let Err(err) = db::system::cascade_kill_session_agents(
            db.clone(),
            session_id.clone(),
            "MASTER_EXIT".to_string(),
        )
        .await
        {
            tracing::warn!(session_id = %session_id, error = %err, "cascade kill failed after master exit");
        }

        let agent_ids = db::system::session_agent_ids(db.clone(), session_id.clone())
            .await
            .unwrap_or_default();
        for agent_id in &agent_ids {
            if let Some(pane_id) = crate::agent_io::pane_id(agent_id) {
                let _ = tmux_server.kill_pane(pane_id).await;
            }
            let _ = tmux_server.kill_session(agent_session_name(agent_id)).await;
        }

        spawn_master_exit_shutdown_check(
            db.clone(),
            auto_shutdown_on_master_exit,
            MASTER_EXIT_SHUTDOWN_GRACE,
            daemon_shutdown_trigger(),
        );

        let _ = crate::monitor::remove(&key);
    });
}

pub fn monitor_key(session_id: &str) -> String {
    format!("master:{session_id}")
}

fn spawn_master_exit_shutdown_check(
    db: Db,
    auto_shutdown_on_master_exit: bool,
    grace: Duration,
    shutdown_trigger: ShutdownTrigger,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        if let Err(err) = schedule_daemon_shutdown_if_idle(
            db,
            auto_shutdown_on_master_exit,
            grace,
            shutdown_trigger,
        )
        .await
        {
            tracing::warn!(error = %err, "failed to evaluate master-exit daemon shutdown");
        }
    })
}

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

    let active_agents = db::system::count_active_agents_daemon_wide(db.clone()).await?;
    if active_agents != 0 {
        tracing::info!(
            active_agents,
            "daemon-wide active agents remain after master exit; shutdown not scheduled"
        );
        return Ok(());
    }

    tracing::info!("daemon-wide active agents == 0 after master exit, scheduling shutdown in 5s");
    tokio::time::sleep(grace).await;

    let active_agents = db::system::count_active_agents_daemon_wide(db).await?;
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

fn daemon_shutdown_trigger() -> ShutdownTrigger {
    Arc::new(|| {
        tokio::spawn(async {
            tokio::time::sleep(Duration::from_millis(50)).await;
            // SAFETY: getpid returns the current daemon process and SIGTERM is a constant signal.
            unsafe { libc::kill(libc::getpid(), libc::SIGTERM) };
        });
    })
}

#[cfg(test)]
mod tests {
    use super::{schedule_daemon_shutdown_if_idle, spawn_master_pidfd_watch_task};
    use crate::db;
    use crate::db::agents::insert_agent_sync;
    use crate::db::agents_lifecycle::mark_agent_killed_sync;
    use crate::db::sessions::insert_session_sync;
    use crate::monitor::{contains, pidfd_open, register};
    use crate::tmux::TmuxServer;
    use std::process::Command;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::{Duration, Instant};

    async fn sleep_ms(ms: u64) {
        tokio::task::spawn_blocking(move || std::thread::sleep(Duration::from_millis(ms)))
            .await
            .unwrap();
    }

    async fn wait_for_session_state(db: &db::Db, session_id: &str, expected: &str) -> bool {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            let status: Option<String> = {
                let conn = db.conn();
                conn.query_row(
                    "SELECT status FROM sessions WHERE id = ?",
                    [session_id],
                    |row| row.get(0),
                )
                .ok()
            };
            if status.as_deref() == Some(expected) {
                return true;
            }
            sleep_ms(50).await;
        }
        false
    }

    async fn wait_for_agent_state(db: &db::Db, agent_id: &str, expected: &str) -> bool {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            let state: Option<String> = {
                let conn = db.conn();
                conn.query_row("SELECT state FROM agents WHERE id = ?", [agent_id], |row| {
                    row.get(0)
                })
                .ok()
            };
            if state.as_deref() == Some(expected) {
                return true;
            }
            sleep_ms(50).await;
        }
        false
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
    async fn test_master_watch_cascades_kill_on_exit() {
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
        let key = super::monitor_key(&session_id);
        register(key.clone(), pidfd);

        let tmux = Arc::new(TmuxServer::new(&state_dir));
        spawn_master_pidfd_watch_task(session_id.clone(), task_fd, db.clone(), tmux, true);

        assert!(wait_for_session_state(&db, &session_id, "KILLED").await);
        assert!(wait_for_agent_state(&db, &agent_id, "KILLED").await);
        assert!(wait_until_monitor_absent(&key).await);
        let _ = child.wait();

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
