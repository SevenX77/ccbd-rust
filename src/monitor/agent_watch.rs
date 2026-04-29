//! Agent pidfd readiness task that marks unexpected process exit.

use crate::db::{self, Db};
use std::os::fd::{AsRawFd, OwnedFd};
use std::sync::Arc;
use tokio::io::unix::AsyncFd;

/// Spawn an async pidfd watcher for one agent process.
pub fn spawn_agent_pidfd_watch_task(
    agent_id: String,
    pidfd: OwnedFd,
    child: Box<dyn portable_pty::Child + Send + Sync>,
    db: Arc<Db>,
) {
    tokio::spawn(async move {
        let _child = child;
        let async_fd = match AsyncFd::new(pidfd) {
            Ok(async_fd) => async_fd,
            Err(err) => {
                tracing::warn!(agent_id = %agent_id, error = %err, "failed to create AsyncFd for agent pidfd");
                cleanup(&agent_id);
                return;
            }
        };

        if let Err(err) = async_fd.readable().await {
            tracing::warn!(agent_id = %agent_id, error = %err, "agent pidfd readiness wait failed");
            cleanup(&agent_id);
            return;
        }

        let raw = async_fd.get_ref().as_raw_fd();
        let exit_code = waitid_exit_code(raw);
        if let Err(err) = db::agents::mark_agent_crashed_with_exit(&db, &agent_id, exit_code) {
            tracing::warn!(agent_id = %agent_id, error = %err, "failed to persist agent pidfd exit");
        }
        if let Some(handle) = crate::marker::registry::take(&agent_id) {
            let _ = handle.cancel_tx.send(());
        }
        let _ = crate::marker::parser_registry::remove(&agent_id);

        cleanup(&agent_id);
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

fn cleanup(agent_id: &str) {
    match crate::pty::PTY_MAP.lock() {
        Ok(mut pty_map) => {
            let _ = pty_map.remove(agent_id);
        }
        Err(err) => {
            tracing::warn!(agent_id, error = %err, "PTY_MAP mutex poisoned during pidfd cleanup")
        }
    }
    let _ = crate::monitor::remove(agent_id);
}

#[cfg(test)]
mod tests {
    use super::spawn_agent_pidfd_watch_task;
    use crate::db;
    use crate::db::agents::{insert_agent, mark_agent_killed};
    use crate::db::sessions::insert_session;
    use crate::monitor::{contains, pidfd_open, register, remove};
    use crate::pty::{PTY_MAP, spawn_agent};
    use rusqlite::OptionalExtension;
    use std::io::Write;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    fn write_to_agent(agent_id: &str, bytes: &[u8]) {
        let mut pty_map = PTY_MAP.lock().unwrap();
        let writer = pty_map
            .get_mut(agent_id)
            .unwrap_or_else(|| panic!("missing PTY writer for {agent_id}"));
        writer.write_all(bytes).unwrap();
        writer.flush().unwrap();
    }

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
            insert_session(&conn, "s1", "p1", "/tmp/foo", 999).unwrap();
            insert_agent(&conn, &agent_id, "s1", "bash", "IDLE", None).unwrap();
        }

        let spawn_result = spawn_agent(&agent_id, "bash").unwrap();
        let pidfd = pidfd_open(spawn_result.pid as i32).unwrap();
        let task_fd = pidfd.try_clone().unwrap();
        register(agent_id.clone(), pidfd);

        spawn_agent_pidfd_watch_task(
            agent_id.clone(),
            task_fd,
            spawn_result.child,
            Arc::new(db.clone()),
        );
        write_to_agent(&agent_id, b"exit\n");

        assert!(wait_for_state(&db, &agent_id, "CRASHED").await);
        assert!(!contains(&agent_id));
        assert!(!PTY_MAP.lock().unwrap().contains_key(&agent_id));

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
            insert_session(&conn, "s1", "p1", "/tmp/foo", 999).unwrap();
            insert_agent(&conn, &agent_id, "s1", "bash", "IDLE", None).unwrap();
        }

        let spawn_result = spawn_agent(&agent_id, "bash").unwrap();
        let pidfd = pidfd_open(spawn_result.pid as i32).unwrap();
        let task_fd = pidfd.try_clone().unwrap();
        register(agent_id.clone(), pidfd);
        mark_agent_killed(&db, &agent_id, "TEST_KILL").unwrap();

        spawn_agent_pidfd_watch_task(
            agent_id.clone(),
            task_fd,
            spawn_result.child,
            Arc::new(db.clone()),
        );
        write_to_agent(&agent_id, b"exit\n");

        assert!(wait_for_state(&db, &agent_id, "KILLED").await);
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
        let _ = PTY_MAP.lock().unwrap().remove(&agent_id);
    }
}
