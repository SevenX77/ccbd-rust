//! Agent pidfd readiness task that marks unexpected process exit.

use crate::db::{self, Db};
use std::os::fd::{AsRawFd, OwnedFd};
use std::sync::Arc;
use tokio::io::unix::AsyncFd;

/// Spawn an async pidfd watcher for one agent process.
pub fn spawn_agent_pidfd_watch_task(
    agent_id: String,
    pidfd: OwnedFd,
    db: Arc<Db>,
) {
    tokio::spawn(async move {
        let async_fd = match AsyncFd::new(pidfd) {
            Ok(async_fd) => async_fd,
            Err(err) => {
                tracing::warn!(agent_id = %agent_id, error = %err, "failed to create AsyncFd for agent pidfd");
                cleanup(&agent_id).await;
                return;
            }
        };

        if let Err(err) = async_fd.readable().await {
            tracing::warn!(agent_id = %agent_id, error = %err, "agent pidfd readiness wait failed");
            cleanup(&agent_id).await;
            return;
        }

        let raw = async_fd.get_ref().as_raw_fd();
        let exit_code = waitid_exit_code(raw);
        if let Err(err) = db::agents_lifecycle::mark_agent_crashed_with_exit(
            db.as_ref().clone(),
            agent_id.clone(),
            exit_code,
        )
        .await
        {
            tracing::warn!(agent_id = %agent_id, error = %err, "failed to persist agent pidfd exit");
        }
        if let Some(handle) = crate::marker::registry::take(&agent_id) {
            let _ = handle.cancel_tx.send(());
        }
        let _ = crate::marker::parser_registry::remove(&agent_id);

        cleanup(&agent_id).await;
    });
}

fn waitid_exit_code(pidfd_raw: i32) -> i32 {
    // SAFETY: siginfo_t is a plain C output buffer and zero-initialization is
    // valid before passing it to waitid.
    let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
    // SAFETY: pidfd_raw comes from a live AsyncFd<OwnedFd>. waitid with P_PIDFD
    // only writes to `info` and does not take ownership of the fd.
    let result = unsafe {
        libc::waitid(
            libc::P_PIDFD,
            pidfd_raw as libc::id_t,
            &mut info,
            libc::WEXITED | libc::WNOHANG,
        )
    };

    if result == 0 {
        // SAFETY: waitid returned success and initialized siginfo_t; reading
        // si_status is the libc accessor for the exited child status.
        unsafe { info.si_status() }
    } else {
        let err = std::io::Error::last_os_error();
        tracing::warn!(error = %err, "waitid(P_PIDFD) failed");
        -1
    }
}

async fn cleanup(agent_id: &str) {
    let _ = crate::agent_io::shutdown_reader(agent_id).await;
    let _ = crate::monitor::remove(agent_id);
}

#[cfg(test)]
mod tests {
    use super::spawn_agent_pidfd_watch_task;
    use crate::db;
    use crate::db::agents::insert_agent_sync;
    use crate::db::agents_lifecycle::mark_agent_killed_sync;
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
    async fn test_agent_watch_marks_crashed_and_cleans_registry() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let db = db::init(file.path()).unwrap();
        let agent_id = format!("ag_pidfd_{}", uuid::Uuid::new_v4());

        {
            let conn = db.conn();
            insert_session_sync(&conn, "s1", "p1", "/tmp/foo", 999).unwrap();
            insert_agent_sync(&conn, &agent_id, "s1", "bash", "IDLE", None).unwrap();
        }

        let mut child = Command::new("sh")
            .arg("-c")
            .arg("sleep 0.2; exit 0")
            .spawn()
            .unwrap();
        let pidfd = pidfd_open(child.id() as i32).unwrap();
        let task_fd = pidfd.try_clone().unwrap();
        register(agent_id.clone(), pidfd);

        spawn_agent_pidfd_watch_task(agent_id.clone(), task_fd, Arc::new(db.clone()));

        assert!(wait_for_state(&db, &agent_id, "CRASHED").await);
        let _ = child.wait();
        assert!(!contains(&agent_id));

        let (state, exit_code): (String, Option<i64>) = db
            .conn()
            .query_row(
                "SELECT state, exit_code FROM agents WHERE id = ?",
                [agent_id.as_str()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(state, "CRASHED");
        assert_eq!(exit_code, Some(0));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_agent_watch_does_not_overwrite_killed() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let db = db::init(file.path()).unwrap();
        let agent_id = format!("ag_pidfd_killed_{}", uuid::Uuid::new_v4());

        {
            let conn = db.conn();
            insert_session_sync(&conn, "s1", "p1", "/tmp/foo", 999).unwrap();
            insert_agent_sync(&conn, &agent_id, "s1", "bash", "IDLE", None).unwrap();
        }

        let mut child = Command::new("sh")
            .arg("-c")
            .arg("sleep 60")
            .spawn()
            .unwrap();
        let pidfd = pidfd_open(child.id() as i32).unwrap();
        let task_fd = pidfd.try_clone().unwrap();
        register(agent_id.clone(), pidfd);
        mark_agent_killed_sync(&db, &agent_id, "TEST_KILL").unwrap();

        spawn_agent_pidfd_watch_task(agent_id.clone(), task_fd, Arc::new(db.clone()));
        child.kill().unwrap();

        assert!(wait_for_state(&db, &agent_id, "KILLED").await);
        let _ = child.wait();
        sleep_ms(300).await;

        let (state, event_count): (String, i64) = db
            .conn()
            .query_row(
                "SELECT agents.state, COUNT(events.seq_id) FROM agents LEFT JOIN events ON events.agent_id = agents.id WHERE agents.id = ?",
                [agent_id.as_str()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(state, "KILLED");
        assert_eq!(event_count, 1);

        let _ = remove(&agent_id);
    }
}
