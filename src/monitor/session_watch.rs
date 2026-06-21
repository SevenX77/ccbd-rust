use crate::db::{self, Db};
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

fn should_defer_anchor_cascade_for_master_revive(session_id: &str) -> bool {
    crate::master_revival::master_revive_in_flight(session_id)
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
    if should_defer_anchor_cascade_for_master_revive(session_id) {
        tracing::info!(
            session_id,
            unit = %unit_name,
            "anchor unit inactive while master revive is in flight; deferring cascade"
        );
        return false;
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
        handle_confirmed_anchor_inactive, should_defer_anchor_cascade_for_master_revive,
        unit_is_active_from_stdout, unit_is_active_with_program, unit_name_for_session,
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
    fn session_watch_does_not_defer_cascade_without_master_revive() {
        assert!(!should_defer_anchor_cascade_for_master_revive(
            "s_no_master_revive"
        ));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn session_watch_defers_cascade_while_master_revive_lock_is_held() {
        let session_id = format!("s_revive_lock_{}", uuid::Uuid::new_v4());
        let lock = crate::master_revival::master_spawn_lock(&session_id);
        let _guard = lock.lock().await;

        assert!(should_defer_anchor_cascade_for_master_revive(&session_id));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn session_watch_defers_only_while_master_revive_in_flight_and_cascades_active_without_it()
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
        let revive_lock = crate::master_revival::master_spawn_lock(session_id);
        let revive_guard = revive_lock.lock().await;
        let cascaded = handle_confirmed_anchor_inactive(
            db.clone(),
            session_id,
            "ahd-session-s_session_watch_race.service",
            "anchor:s_session_watch_race",
        )
        .await;

        assert!(
            !cascaded,
            "anchor-inactive cascade must defer while master revive owns recovery"
        );
        assert!(contains(agent_id));
        assert!(
            home_root.exists(),
            "revive deferral must preserve worker home"
        );
        let state: String = db
            .conn()
            .query_row(
                "SELECT state FROM agents WHERE id = ?1",
                [agent_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(state, "BUSY");

        drop(revive_guard);
        let cascaded = handle_confirmed_anchor_inactive(
            db.clone(),
            session_id,
            "ahd-session-s_session_watch_race.service",
            "anchor:s_session_watch_race",
        )
        .await;

        assert!(
            cascaded,
            "ACTIVE session without master revive in flight should allow anchor cascade"
        );
        assert!(!contains(agent_id));
        assert!(
            !home_root.exists(),
            "legal anchor cascade must still run the real runtime cleanup path"
        );
        let state: String = db
            .conn()
            .query_row(
                "SELECT state FROM agents WHERE id = ?1",
                [agent_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(state, "KILLED");
    }
}
