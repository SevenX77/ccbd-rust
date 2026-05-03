use crate::db::{self, Db};
use std::fs::File;
use std::os::fd::OwnedFd;
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

const WATCH_INTERVAL: Duration = Duration::from_secs(3);

pub fn unit_name_for_session(session_id: &str) -> String {
    format!("ccbd-session-{session_id}.service")
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
    Command::new("systemctl")
        .args(["--user", "is-active", unit_name])
        .output()
        .map(|output| {
            output.status.success() && String::from_utf8_lossy(&output.stdout).trim() == "active"
        })
        .unwrap_or(false)
}

fn placeholder_fd() -> std::io::Result<OwnedFd> {
    // The pidfd registry is also used as a lightweight monitor registry for
    // diagnostics. Anchor watchers do not have a process fd, so /dev/null is
    // registered as a cheap owned placeholder and closed on remove().
    File::open("/dev/null").map(OwnedFd::from)
}

#[cfg(test)]
mod tests {
    use super::unit_name_for_session;

    #[test]
    fn test_unit_name_for_session() {
        assert_eq!(
            unit_name_for_session("sess_abc"),
            "ccbd-session-sess_abc.service"
        );
    }
}
