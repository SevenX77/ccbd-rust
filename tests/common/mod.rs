use ah::tmux::TmuxServer;
use ah::tmux::scope::{self, ScopePolicy, UnitConfig};
use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

const DAEMON_IDENTITY_ENV: &[&str] = &["CCB_SOCKET", "AH_STATE_DIR", "CCBD_STATE_DIR"];

#[allow(dead_code)]
pub fn scrub_daemon_identity_env(command: &mut Command) -> &mut Command {
    for key in DAEMON_IDENTITY_ENV {
        command.env_remove(key);
    }
    command
}

#[allow(dead_code)]
pub fn hard_gate(binary: &str) {
    assert!(
        which::which(binary).is_ok(),
        "missing {binary} binary, set CCB_TEST_SKIP_REAL_PROVIDER=1 to opt-out"
    );
    let has_auth = match binary {
        "codex" => Path::new(&format!(
            "{}/.codex/auth.json",
            std::env::var("HOME").unwrap_or_default()
        ))
        .exists(),
        "claude" => which::which("claude").is_ok(),
        _ => false,
    };
    assert!(
        has_auth,
        "{binary} has no OAuth credentials configured, set CCB_TEST_SKIP_REAL_PROVIDER=1 to opt-out"
    );
    assert!(
        which::which("tmux").is_ok() && can_use_systemd_run(),
        "real provider tests require tmux and user systemd; set CCB_TEST_SKIP_REAL_PROVIDER=1 to opt-out"
    );
}

pub fn can_use_systemd_run() -> bool {
    Command::new("systemd-run")
        .args(["--user", "--scope", "--", "true"])
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

#[allow(dead_code)]
pub fn scope_policy_for_test(socket_name: &str) -> ScopePolicy {
    scope_policy_for_test_with(
        socket_name,
        can_use_systemd_run(),
        std::env::var("CCBD_TEST_WRAPPER_SCOPE").ok(),
    )
}

#[allow(dead_code)]
pub struct TmuxServerGuard {
    server: Arc<TmuxServer>,
    socket_path: PathBuf,
}

#[allow(dead_code)]
impl TmuxServerGuard {
    pub fn new(state_dir: &Path) -> Self {
        let server = Arc::new(TmuxServer::new(state_dir));
        let socket_path = tmux_socket_path(server.socket_name());
        Self {
            server,
            socket_path,
        }
    }

    pub fn new_with_policy(state_dir: &Path, policy: ScopePolicy) -> Self {
        let server = Arc::new(TmuxServer::new_with_policy(state_dir, policy));
        let socket_path = tmux_socket_path(server.socket_name());
        Self {
            server,
            socket_path,
        }
    }

    pub fn server(&self) -> Arc<TmuxServer> {
        self.server.clone()
    }

    pub fn socket_name(&self) -> &str {
        self.server.socket_name()
    }
}

impl Drop for TmuxServerGuard {
    fn drop(&mut self) {
        let _ = Command::new("tmux")
            .args(["-L", self.server.socket_name(), "kill-server"])
            .output();
        let _ = std::fs::remove_file(&self.socket_path);
    }
}

impl Deref for TmuxServerGuard {
    type Target = TmuxServer;

    fn deref(&self) -> &Self::Target {
        &self.server
    }
}

#[allow(dead_code)]
pub fn tmux_socket_path(socket_name: &str) -> PathBuf {
    #[cfg(unix)]
    {
    let uid = unsafe { libc::geteuid() };
    PathBuf::from(format!("/tmp/tmux-{uid}")).join(socket_name)
    }
    #[cfg(not(unix))]
    {
        let _ = socket_name;
        panic!("tmux socket paths are Unix-only")
    }
}

fn scope_policy_for_test_with(
    socket_name: &str,
    systemd_available: bool,
    wrapper_scope: Option<String>,
) -> ScopePolicy {
    if !systemd_available {
        return ScopePolicy::None;
    }

    ScopePolicy::Systemd(UnitConfig {
        unit_name: scope::unit_name_for_socket(socket_name),
        slice: "ahd-agents.slice".to_string(),
        binds_to: wrapper_scope,
    })
}

#[cfg(test)]
mod tests {
    use super::{DAEMON_IDENTITY_ENV, scope_policy_for_test_with, scrub_daemon_identity_env};
    use ah::tmux::scope::{ScopePolicy, UnitConfig};
    use std::ffi::OsStr;
    use std::process::Command;

    #[test]
    fn test_scope_policy_for_test_none_without_systemd() {
        assert_eq!(
            scope_policy_for_test_with(
                "ahd-abc123def456789a",
                false,
                Some("wrapper.scope".to_string())
            ),
            ScopePolicy::None
        );
    }

    #[test]
    fn test_scope_policy_for_test_systemd_without_binds_to() {
        assert_eq!(
            scope_policy_for_test_with("ahd-abc123def456789a", true, None),
            ScopePolicy::Systemd(UnitConfig {
                unit_name: "ahd-tmux-abc123de".to_string(),
                slice: "ahd-agents.slice".to_string(),
                binds_to: None,
            })
        );
    }

    #[test]
    fn test_scope_policy_for_test_systemd_with_binds_to() {
        assert_eq!(
            scope_policy_for_test_with(
                "ahd-abc123def456789a",
                true,
                Some("wrapper.scope".to_string())
            ),
            ScopePolicy::Systemd(UnitConfig {
                unit_name: "ahd-tmux-abc123de".to_string(),
                slice: "ahd-agents.slice".to_string(),
                binds_to: Some("wrapper.scope".to_string()),
            })
        );
    }

    #[test]
    fn test_scrub_daemon_identity_env_removes_live_daemon_overrides() {
        let mut command = Command::new("ahd-test");
        command
            .env("CCB_SOCKET", "/live/ahd.sock")
            .env("AH_STATE_DIR", "/live/state")
            .env("CCBD_STATE_DIR", "/legacy/live/state")
            .env("OTHER_ENV", "preserved");

        scrub_daemon_identity_env(&mut command);

        let env = command.get_envs().collect::<Vec<_>>();
        for key in DAEMON_IDENTITY_ENV {
            assert!(
                env.iter()
                    .any(|(name, value)| *name == OsStr::new(*key) && value.is_none()),
                "{key} must be explicitly removed from spawned test daemons/helpers"
            );
        }
        assert!(
            env.iter()
                .any(|(name, value)| *name == OsStr::new("OTHER_ENV") && value.is_some())
        );
    }
}
