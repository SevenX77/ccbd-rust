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

            tracing::warn!(
                session_id = %session_id,
                unit = %unit_name,
                "anchor unit confirmed inactive after debounce; cascading agent kill"
            );
            if let Err(err) = db::system::cascade_kill_session_agents(
                (*db).clone(),
                session_id.clone(),
                "ANCHOR_UNIT_STOPPED".to_string(),
            )
            .await
            {
                tracing::warn!(
                    session_id = %session_id,
                    unit = %unit_name,
                    error = %err,
                    "failed to sync killed session after anchor stopped"
                );
            }
            let _ = crate::monitor::remove(&key);
            break;
        }
    });
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
    use super::{unit_is_active_from_stdout, unit_is_active_with_program, unit_name_for_session};
    use std::path::Path;

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
}
