//! systemd-run scope wrapping for tmux server processes.

use crate::systemd_unit;
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
    let unit = match policy {
        ScopePolicy::None => {
            let mut command = Command::new(base_cmd);
            command.args(base_args);
            return command;
        }
        ScopePolicy::Systemd(unit) => unit,
    };

    let mut command = Command::new("systemd-run");
    command.args([
        "--user",
        "--scope",
        "--collect",
        &format!("--unit={}", unit.unit_name),
        &format!("--slice={}", unit.slice),
    ]);
    if let Some(binds_to) = &unit.binds_to {
        command.arg(format!("--property=BindsTo={binds_to}"));
        command.arg(format!("--property=PartOf={binds_to}"));
    }
    command.arg("--").arg(base_cmd).args(base_args);
    command
}

pub fn unit_name_for_socket(socket_name: &str) -> String {
    let suffix = socket_name
        .strip_prefix("ahd-")
        .or_else(|| socket_name.strip_prefix("ccbd-"))
        .unwrap_or(socket_name);
    format!("ahd-tmux-{}", &suffix[..suffix.len().min(8)])
}

pub fn detect_scope_policy(socket_name: &str) -> ScopePolicy {
    detect_scope_policy_with_daemon_unit(
        socket_name,
        systemd_unit::detect_current_service_unit().as_deref(),
    )
}

pub fn detect_scope_policy_with_daemon_unit(
    socket_name: &str,
    daemon_unit: Option<&str>,
) -> ScopePolicy {
    if !systemd_run_available() {
        return ScopePolicy::None;
    }

    ScopePolicy::Systemd(UnitConfig {
        unit_name: unit_name_for_socket(socket_name),
        slice: "ahd-agents.slice".to_string(),
        binds_to: daemon_unit.map(str::to_string),
    })
}

fn systemd_run_available() -> bool {
    Command::new("systemd-run")
        .args(["--user", "--scope", "--", "true"])
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
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
