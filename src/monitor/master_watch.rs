//! Master pidfd watcher that cascades session agent shutdown on master death.

use crate::db::{self, Db};
use std::os::fd::{AsRawFd, OwnedFd};
use std::sync::Arc;
use tokio::io::unix::AsyncFd;

/// Spawn an async pidfd watcher for a session master process.
pub fn spawn_master_pidfd_watch_task(session_id: String, master_pidfd: OwnedFd, db: Arc<Db>) {
    tokio::spawn(async move {
        let async_fd = match AsyncFd::new(master_pidfd) {
            Ok(async_fd) => async_fd,
            Err(err) => {
                tracing::warn!(session_id = %session_id, error = %err, "failed to create AsyncFd for master pidfd");
                let _ = crate::monitor::remove(&format!("master:{session_id}"));
                return;
            }
        };

        if let Err(err) = async_fd.readable().await {
            tracing::warn!(session_id = %session_id, error = %err, "master pidfd readiness wait failed");
            let _ = crate::monitor::remove(&format!("master:{session_id}"));
            return;
        }

        consume_exit_status(async_fd.get_ref().as_raw_fd());
        let active_agent_ids = active_agent_ids(db.as_ref().clone(), session_id.clone()).await;
        if let Err(err) = db::system::cascade_kill_session_agents(
            db.as_ref().clone(),
            session_id.clone(),
            "MASTER_DEATH".to_string(),
        )
        .await
        {
            tracing::warn!(session_id = %session_id, error = %err, "failed to cascade kill session agents");
        }
        for agent_id in active_agent_ids {
            let _ = crate::agent_io::shutdown_reader(&agent_id).await;
        }
        let _ = crate::monitor::remove(&format!("master:{session_id}"));
    });
}

async fn active_agent_ids(db: Db, session_id: String) -> Vec<String> {
    crate::db::common::spawn_db("master_watch::active_agent_ids", move || {
        let conn = db.conn();
        let mut stmt = conn
            .prepare(
                "SELECT id FROM agents WHERE session_id = ? AND state NOT IN ('CRASHED', 'KILLED')",
            )
            .map_err(|err| crate::db::common::map_db_error("prepare active session agents", err))?;
        let rows = stmt
            .query_map([session_id.as_str()], |row| row.get::<_, String>(0))
            .map_err(|err| crate::db::common::map_db_error("query active session agents", err))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|err| crate::db::common::map_db_error("collect active session agents", err))
    })
    .await
    .unwrap_or_else(|err| {
        tracing::warn!(error = %err, "failed to query active session agents for reader shutdown");
        Vec::new()
    })
}

fn consume_exit_status(pidfd_raw: i32) {
    // SAFETY: siginfo_t is a C output buffer. Zero-initialization is valid
    // before waitid fills the struct.
    let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
    // SAFETY: pidfd_raw is borrowed from AsyncFd<OwnedFd> and remains valid for
    // the call. waitid does not take ownership of the fd.
    let result = unsafe {
        libc::waitid(
            libc::P_PIDFD,
            pidfd_raw as libc::id_t,
            &mut info,
            libc::WEXITED | libc::WNOHANG,
        )
    };
    if result != 0 {
        let err = std::io::Error::last_os_error();
        tracing::warn!(error = %err, "waitid(P_PIDFD) for master failed");
    }
}

#[cfg(test)]
mod tests {
    use super::spawn_master_pidfd_watch_task;
    use crate::db;
    use crate::db::agents::insert_agent_sync;
    use crate::db::sessions::insert_session_sync;
    use crate::monitor::{contains, pidfd_open, register, remove};
    use rusqlite::OptionalExtension;
    use std::process::Command;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    async fn sleep_ms(ms: u64) {
        tokio::task::spawn_blocking(move || std::thread::sleep(Duration::from_millis(ms)))
            .await
            .unwrap();
    }

    async fn wait_for_state(db: &db::Db, agent_id: &str, expected: &str) -> bool {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            let state: Option<String> = {
                let conn = db.conn();
                conn.query_row("SELECT state FROM agents WHERE id = ?", [agent_id], |row| {
                    row.get(0)
                })
                .optional()
                .unwrap()
            };
            if state.as_deref() == Some(expected) {
                return true;
            }
            sleep_ms(50).await;
        }
        false
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_master_watch_cascade_kills_session_agents() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let db = db::init(file.path()).unwrap();
        let session_id = format!("s_{}", uuid::Uuid::new_v4());
        let agent_id = format!("ag_master_{}", uuid::Uuid::new_v4());
        let mut master = Command::new("sh")
            .arg("-c")
            .arg("sleep 60")
            .spawn()
            .unwrap();

        {
            let conn = db.conn();
            insert_session_sync(
                &conn,
                &session_id,
                "p1",
                "/tmp/master-watch",
                master.id() as i64,
            )
            .unwrap();
            insert_agent_sync(&conn, &agent_id, &session_id, "bash", "IDLE", None).unwrap();
        }

        let pidfd = pidfd_open(master.id() as i32).unwrap();
        let task_fd = pidfd.try_clone().unwrap();
        let key = format!("master:{session_id}");
        register(key.clone(), pidfd);
        spawn_master_pidfd_watch_task(session_id.clone(), task_fd, Arc::new(db.clone()));

        master.kill().unwrap();
        let _ = master.wait();

        assert!(wait_for_state(&db, &agent_id, "KILLED").await);
        assert!(!contains(&key));

        let payload: String = db
            .conn()
            .query_row(
                "SELECT payload FROM events WHERE agent_id = ? AND event_type = 'state_change'",
                [agent_id.as_str()],
                |row| row.get(0),
            )
            .unwrap();
        assert!(payload.contains("MASTER_DEATH"));
        let _ = remove(&key);
    }
}
