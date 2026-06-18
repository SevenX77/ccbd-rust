use crate::db::{self, Db};
use rusqlite::{OptionalExtension, params};
use std::fs::File;
use std::os::fd::OwnedFd;
use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

const WATCH_INTERVAL: Duration = Duration::from_secs(3);
const DEBOUNCE_RETRIES: u32 = 2;
const DEBOUNCE_RETRY_INTERVAL: Duration = Duration::from_secs(2);

pub fn unit_name_for_session(session_id: &str) -> String {
    format!("ahd-session-{session_id}.service")
}

fn should_cascade_after_anchor_inactive(session_status: Option<&str>) -> bool {
    !matches!(session_status, Some("ACTIVE"))
}

fn should_cascade_session_watch(db: &Db, session_id: &str) -> rusqlite::Result<bool> {
    let conn = db.conn();
    let status = conn
        .query_row(
            "SELECT status FROM sessions WHERE id = ?1",
            params![session_id],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    Ok(should_cascade_after_anchor_inactive(status.as_deref()))
}

pub fn spawn_session_watch_task(session_id: String, unit_name: String, db: Arc<Db>) {
    let key = format!("anchor:{session_id}");
    match placeholder_fd() {
        Ok(fd) => crate::monitor::register(key.clone(), fd),
        Err(err) => tracing::warn!(
            session_id = %session_id,
            error = %err,
            "failed to register anchor monitor placeholder"
        ),
    }

    tokio::spawn(async move {
        let mut interval = tokio::time::interval(WATCH_INTERVAL);
        loop {
            interval.tick().await;
            if unit_is_active(&unit_name) {
                continue;
            }

            let mut confirmed = true;
            for retry_idx in 0..DEBOUNCE_RETRIES {
                tokio::time::sleep(DEBOUNCE_RETRY_INTERVAL).await;
                if unit_is_active(&unit_name) {
                    tracing::info!(
                        session_id = %session_id,
                        unit = %unit_name,
                        retry = retry_idx + 1,
                        "anchor unit recovered after transient inactive; debounce absorbed"
                    );
                    confirmed = false;
                    break;
                }
            }
            if !confirmed {
                continue;
            }

            if handle_confirmed_anchor_inactive(db.clone(), &session_id, &unit_name, &key).await {
                break;
            }
        }
    });
}

async fn handle_confirmed_anchor_inactive(
    db: Arc<Db>,
    session_id: &str,
    unit_name: &str,
    monitor_key: &str,
) -> bool {
    match should_cascade_session_watch(db.as_ref(), session_id) {
        Ok(false) => {
            tracing::info!(
                session_id,
                unit = %unit_name,
                "anchor unit inactive but session remains ACTIVE; deferring cascade"
            );
            return false;
        }
        Ok(true) => {}
        Err(err) => {
            tracing::warn!(
                session_id,
                unit = %unit_name,
                error = %err,
                "failed to query session status before anchor cascade; deferring"
            );
            return false;
        }
    }

    tracing::warn!(
        session_id,
        unit = %unit_name,
        "anchor unit confirmed inactive after debounce; cascading agent kill"
    );
    if let Err(err) = db::system::cascade_kill_session_agents(
        (*db).clone(),
        session_id.to_string(),
        "ANCHOR_UNIT_STOPPED".to_string(),
    )
    .await
    {
        tracing::warn!(
            session_id,
            unit = %unit_name,
            error = %err,
            "failed to sync killed session after anchor stopped"
        );
    }
    let _ = crate::monitor::remove(monitor_key);
    true
}

fn unit_is_active(unit_name: &str) -> bool {
    unit_is_active_with_program(unit_name, Path::new("systemctl"))
}

fn unit_is_active_with_program(unit_name: &str, program: &Path) -> bool {
    match Command::new(program)
        .args(["--user", "is-active", unit_name])
        .output()
    {
        Ok(output) => unit_is_active_from_stdout(&String::from_utf8_lossy(&output.stdout)),
        Err(err) => {
            tracing::warn!(
                unit = %unit_name,
                error = %err,
                "systemctl is-active failed; treating unit as alive"
            );
            true
        }
    }
}

fn unit_is_active_from_stdout(stdout: &str) -> bool {
    let status = stdout.trim();
    if matches!(status, "inactive" | "failed") {
        return false;
    }
    if status != "active" {
        tracing::debug!(status, "ambiguous unit state; treating unit as alive");
    }
    true
}

fn placeholder_fd() -> std::io::Result<OwnedFd> {
    // The pidfd registry is also used as a lightweight monitor registry for
    // diagnostics. Anchor watchers do not have a process fd, so /dev/null is
    // registered as a cheap owned placeholder and closed on remove().
    File::open("/dev/null").map(OwnedFd::from)
}

#[cfg(test)]
mod tests {
    use super::{
        handle_confirmed_anchor_inactive, should_cascade_after_anchor_inactive,
        unit_is_active_from_stdout,
        unit_is_active_with_program, unit_name_for_session,
    };
    use crate::agent_io::registry::{AgentIoEntry, contains, register};
    use crate::db::agents::insert_agent_sync;
    use crate::db::sessions::insert_session_sync;
    use crate::provider::home_layout::sandbox_home_for_sandbox_dir;
    use crate::tmux::TmuxPaneId;
    use std::path::Path;
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;

    #[test]
    fn test_unit_name_for_session() {
        assert_eq!(
            unit_name_for_session("sess_abc"),
            "ahd-session-sess_abc.service"
        );
    }

    #[test]
    fn test_unit_is_active_treats_inactive_as_dead() {
        assert!(!unit_is_active_from_stdout("inactive\n"));
        assert!(!unit_is_active_from_stdout("failed\n"));
    }

    #[test]
    fn test_unit_is_active_treats_unknown_status_as_alive() {
        assert!(unit_is_active_from_stdout("activating\n"));
        assert!(unit_is_active_from_stdout("unknown\n"));
        assert!(unit_is_active_from_stdout(""));
    }

    #[test]
    fn test_unit_is_active_treats_command_error_as_alive() {
        assert!(unit_is_active_with_program(
            "ahd-session-missing.service",
            Path::new("/definitely/not/a/systemctl")
        ));
    }

    #[test]
    fn session_watch_defers_cascade_while_session_is_active() {
        assert!(!should_cascade_after_anchor_inactive(Some("ACTIVE")));
    }

    #[test]
    fn session_watch_allows_cascade_after_terminal_status() {
        assert!(should_cascade_after_anchor_inactive(Some("KILLED")));
        assert!(should_cascade_after_anchor_inactive(Some("FAILED")));
        assert!(should_cascade_after_anchor_inactive(None));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn session_watch_active_session_defers_cascade_and_preserves_worker_home_until_terminal()
    {
        let db_file = tempfile::NamedTempFile::new().unwrap();
        let db = crate::db::init(db_file.path()).unwrap();
        let state_dir = tempfile::TempDir::new().unwrap();
        let session_id = "s_session_watch_race";
        let agent_id = "a_session_watch_race";
        let project_dir = tempfile::TempDir::new().unwrap();
        {
            let conn = db.conn();
            insert_session_sync(
                &conn,
                session_id,
                "p_session_watch_race",
                project_dir.path().to_str().unwrap(),
            )
            .unwrap();
            insert_agent_sync(&conn, agent_id, session_id, "codex", "BUSY", Some(123)).unwrap();
        }

        let pipes_dir = state_dir.path().join("pipes");
        let sandbox_dir = state_dir
            .path()
            .join("sandboxes")
            .join(session_id)
            .join(agent_id);
        std::fs::create_dir_all(&pipes_dir).unwrap();
        std::fs::create_dir_all(&sandbox_dir).unwrap();
        let home_root = sandbox_home_for_sandbox_dir(&sandbox_dir).unwrap();
        std::fs::create_dir_all(home_root.join(".codex")).unwrap();
        std::fs::write(home_root.join(".codex/auth.json"), b"token").unwrap();
        let fifo_path = pipes_dir.join(format!("{agent_id}.fifo"));
        std::fs::write(&fifo_path, b"").unwrap();
        let reader_handle = tokio::spawn(async {
            std::future::pending::<()>().await;
        });
        register(
            agent_id.to_string(),
            AgentIoEntry {
                session_id: session_id.to_string(),
                pane_id: TmuxPaneId("%race:1.0".to_string()),
                reader_handle,
                fifo_path: fifo_path.clone(),
                socket_name: "missing-session-watch-race-socket".to_string(),
                idle_scan_enabled: Arc::new(AtomicBool::new(true)),
            },
        );

        let db = Arc::new(db);
        let cascaded = handle_confirmed_anchor_inactive(
            db.clone(),
            session_id,
            "ahd-session-s_session_watch_race.service",
            "anchor:s_session_watch_race",
        )
        .await;

        assert!(
            !cascaded,
            "ACTIVE session must defer anchor-inactive cascade while master revive owns recovery"
        );
        assert!(contains(agent_id));
        assert!(home_root.exists(), "ACTIVE deferral must preserve worker home");
        let state: String = db
            .conn()
            .query_row("SELECT state FROM agents WHERE id = ?1", [agent_id], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(state, "BUSY");

        db.conn()
            .execute(
                "UPDATE sessions SET status = 'FAILED' WHERE id = ?1",
                [session_id],
            )
            .unwrap();
        let cascaded = handle_confirmed_anchor_inactive(
            db.clone(),
            session_id,
            "ahd-session-s_session_watch_race.service",
            "anchor:s_session_watch_race",
        )
        .await;

        assert!(cascaded, "terminal session should allow anchor cascade");
        assert!(!contains(agent_id));
        assert!(
            !home_root.exists(),
            "terminal cascade must still run the real runtime cleanup path"
        );
        let state: String = db
            .conn()
            .query_row("SELECT state FROM agents WHERE id = ?1", [agent_id], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(state, "KILLED");
    }
}
