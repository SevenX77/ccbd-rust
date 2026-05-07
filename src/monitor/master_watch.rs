use crate::db::{self, Db};
use crate::tmux::TmuxServer;
use std::os::fd::OwnedFd;
use std::sync::Arc;
use tokio::io::unix::AsyncFd;

pub fn spawn_master_pidfd_watch_task(
    session_id: String,
    pidfd: OwnedFd,
    db: Db,
    tmux_server: Arc<TmuxServer>,
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

        let agent_ids = db::system::session_agent_ids(db, session_id.clone())
            .await
            .unwrap_or_default();
        for agent_id in &agent_ids {
            if let Some(pane_id) = crate::agent_io::pane_id(agent_id) {
                let _ = tmux_server.kill_pane(pane_id).await;
            }
        }

        let _ = crate::monitor::remove(&key);
    });
}

pub fn monitor_key(session_id: &str) -> String {
    format!("master:{session_id}")
}

#[cfg(test)]
mod tests {
    use super::spawn_master_pidfd_watch_task;
    use crate::db;
    use crate::db::agents::insert_agent_sync;
    use crate::db::sessions::insert_session_sync;
    use crate::monitor::{contains, pidfd_open, register};
    use crate::tmux::TmuxServer;
    use std::process::Command;
    use std::sync::Arc;
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

    #[tokio::test(flavor = "multi_thread")]
    async fn test_master_watch_cascades_kill_on_exit() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let state_dir = tempfile::TempDir::new().unwrap().keep();
        let db = db::init(file.path()).unwrap();
        let session_id = format!("sess_master_{}", uuid::Uuid::new_v4());

        {
            let conn = db.conn();
            insert_session_sync(&conn, &session_id, "p1", "/tmp/foo").unwrap();
            insert_agent_sync(&conn, "ag_mw_1", &session_id, "bash", "IDLE", None).unwrap();
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
        spawn_master_pidfd_watch_task(session_id.clone(), task_fd, db.clone(), tmux);

        assert!(wait_for_session_state(&db, &session_id, "KILLED").await);
        let _ = child.wait();

        let agent_state: String = db
            .conn()
            .query_row(
                "SELECT state FROM agents WHERE id = 'ag_mw_1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(agent_state, "KILLED");
        assert!(!contains(&key));
    }
}
