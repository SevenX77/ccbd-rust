//! systemd-run scope wrapping for tmux server processes.

use std::process::Command;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnitConfig {
    pub unit_name: String,
    pub slice: String,
    pub binds_to: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ScopePolicy {
    /// Wrap command in `systemd-run --user --scope`, optionally with BindsTo.
    Systemd(UnitConfig),
    /// Spawn the command directly.
    None,
}

pub fn wrap_in_scope(base_cmd: &str, base_args: &[&str], policy: &ScopePolicy) -> Command {
    crate::platform::sys::scope::wrap_in_scope(base_cmd, base_args, policy)
}

pub fn unit_name_for_socket(socket_name: &str) -> String {
    crate::platform::sys::scope::unit_name_for_socket(socket_name)
}

pub fn detect_scope_policy(socket_name: &str) -> ScopePolicy {
    crate::platform::sys::scope::detect_scope_policy(socket_name)
}

pub fn detect_scope_policy_with_daemon_unit(
    socket_name: &str,
    daemon_unit: Option<&str>,
) -> ScopePolicy {
    crate::platform::sys::scope::detect_scope_policy_with_daemon_unit(socket_name, daemon_unit)
}

#[cfg(test)]
mod tests {
    use super::{
        ScopePolicy, UnitConfig, detect_scope_policy_with_daemon_unit, unit_name_for_socket,
        wrap_in_scope,
    };

    fn systemd_policy(binds_to: Option<&str>) -> ScopePolicy {
        ScopePolicy::Systemd(UnitConfig {
            unit_name: "ahd-tmux-abc123de".to_string(),
            slice: "ahd-agents.slice".to_string(),
            binds_to: binds_to.map(str::to_string),
        })
    }

    fn args(command: &std::process::Command) -> Vec<String> {
        command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn test_wrap_in_scope_with_systemd() {
        let command = wrap_in_scope(
            "tmux",
            &["-L", "test", "new-session"],
            &systemd_policy(Some("ahd.service")),
        );

        assert_eq!(command.get_program(), "systemd-run");
        assert_eq!(
            args(&command),
            vec![
                "--user",
                "--scope",
                "--collect",
                "--unit=ahd-tmux-abc123de",
                "--slice=ahd-agents.slice",
                "--property=BindsTo=ahd.service",
                "--property=PartOf=ahd.service",
                "--",
                "tmux",
                "-L",
                "test",
                "new-session",
            ]
        );
    }

    #[test]
    fn test_wrap_in_scope_with_detected_ah_service() {
        let command = wrap_in_scope(
            "tmux",
            &["-L", "test", "new-session"],
            &systemd_policy(Some("ah-p1.service")),
        );

        assert_eq!(command.get_program(), "systemd-run");
        assert_eq!(
            args(&command),
            vec![
                "--user",
                "--scope",
                "--collect",
                "--unit=ahd-tmux-abc123de",
                "--slice=ahd-agents.slice",
                "--property=BindsTo=ah-p1.service",
                "--property=PartOf=ah-p1.service",
                "--",
                "tmux",
                "-L",
                "test",
                "new-session",
            ]
        );
    }

    #[test]
    fn test_wrap_in_scope_with_systemd_no_binds_to() {
        let command = wrap_in_scope(
            "tmux",
            &["-L", "test", "new-session"],
            &systemd_policy(None),
        );
        let args = args(&command);

        assert_eq!(command.get_program(), "systemd-run");
        assert!(
            !args
                .iter()
                .any(|arg| arg.starts_with("--property=BindsTo="))
        );
        assert!(!args.iter().any(|arg| arg.starts_with("--property=PartOf=")));
        assert_eq!(
            args,
            vec![
                "--user",
                "--scope",
                "--collect",
                "--unit=ahd-tmux-abc123de",
                "--slice=ahd-agents.slice",
                "--",
                "tmux",
                "-L",
                "test",
                "new-session",
            ]
        );
    }

    #[test]
    fn test_wrap_in_scope_fallback() {
        let command = wrap_in_scope("tmux", &["-L", "test", "new-session"], &ScopePolicy::None);

        assert_eq!(command.get_program(), "tmux");
        assert_eq!(args(&command), vec!["-L", "test", "new-session"]);
    }

    #[test]
    fn test_unit_name_for_socket() {
        assert_eq!(
            unit_name_for_socket("ahd-abc123def456789a"),
            "ahd-tmux-abc123de"
        );
    }

    #[test]
    fn test_detect_scope_policy_uses_supplied_daemon_unit_without_renaming_scope() {
        let policy =
            detect_scope_policy_with_daemon_unit("ahd-abc123def456789a", Some("ah-p1.service"));

        assert_eq!(
            policy,
            ScopePolicy::Systemd(UnitConfig {
                unit_name: "ahd-tmux-abc123de".to_string(),
                slice: "ahd-agents.slice".to_string(),
                binds_to: Some("ah-p1.service".to_string()),
            })
        );
    }
}
