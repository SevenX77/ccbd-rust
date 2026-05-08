use ccbd::tmux::scope::{self, ScopePolicy, UnitConfig};
use std::path::Path;
use std::process::Command;

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
        "gemini" => Path::new(&format!(
            "{}/.gemini/oauth_creds.json",
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
        which::which("tmux").is_ok() && which::which("bwrap").is_ok() && can_use_systemd_run(),
        "real provider tests require tmux, bwrap, and user systemd; set CCB_TEST_SKIP_REAL_PROVIDER=1 to opt-out"
    );
}

pub fn can_use_systemd_run() -> bool {
    Command::new("systemd-run")
        .args(["--user", "--scope", "--", "true"])
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

pub fn scope_policy_for_test(socket_name: &str) -> ScopePolicy {
    scope_policy_for_test_with(
        socket_name,
        can_use_systemd_run(),
        std::env::var("CCBD_TEST_WRAPPER_SCOPE").ok(),
    )
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
        slice: "ccbd-agents.slice".to_string(),
        binds_to: wrapper_scope,
    })
}

#[cfg(test)]
mod tests {
    use super::scope_policy_for_test_with;
    use ccbd::tmux::scope::{ScopePolicy, UnitConfig};

    #[test]
    fn test_scope_policy_for_test_none_without_systemd() {
        assert_eq!(
            scope_policy_for_test_with(
                "ccbd-abc123def456789a",
                false,
                Some("wrapper.scope".to_string())
            ),
            ScopePolicy::None
        );
    }

    #[test]
    fn test_scope_policy_for_test_systemd_without_binds_to() {
        assert_eq!(
            scope_policy_for_test_with("ccbd-abc123def456789a", true, None),
            ScopePolicy::Systemd(UnitConfig {
                unit_name: "ccbd-tmux-abc123de".to_string(),
                slice: "ccbd-agents.slice".to_string(),
                binds_to: None,
            })
        );
    }

    #[test]
    fn test_scope_policy_for_test_systemd_with_binds_to() {
        assert_eq!(
            scope_policy_for_test_with(
                "ccbd-abc123def456789a",
                true,
                Some("wrapper.scope".to_string())
            ),
            ScopePolicy::Systemd(UnitConfig {
                unit_name: "ccbd-tmux-abc123de".to_string(),
                slice: "ccbd-agents.slice".to_string(),
                binds_to: Some("wrapper.scope".to_string()),
            })
        );
    }
}
