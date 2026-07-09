//! Linux systemd-run scope command assembly.

use crate::provider::manifest::{ProviderManifest, collect_spawn_env};
use crate::sandbox::{EnvState, SandboxOverrides};
use crate::tmux::scope::{ScopePolicy, UnitConfig};
use std::collections::{HashMap, HashSet};
use std::io;
use std::process::Command;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoverySpawn {
    pub is_recovery: bool,
    pub args: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopeUnit {
    pub(crate) unit: String,
    pub(crate) description: String,
}

pub trait SystemctlRunner {
    fn list_scope_units(&self) -> Result<Vec<ScopeUnit>, io::Error>;
    fn unit_is_active(&self, _unit: &str) -> Result<bool, io::Error> {
        Ok(false)
    }
    fn stop_unit(&self, unit: &str) -> Result<(), io::Error>;
}

pub struct RealSystemctlRunner;

impl SystemctlRunner for RealSystemctlRunner {
    fn list_scope_units(&self) -> Result<Vec<ScopeUnit>, io::Error> {
        let output = Command::new("systemctl")
            .args([
                "--user",
                "list-units",
                "--all",
                "--type=scope",
                "--no-legend",
                "--plain",
                "--no-pager",
            ])
            .output()?;
        if !output.status.success() {
            return Err(io::Error::other(format!(
                "systemctl list-units failed: {}",
                String::from_utf8_lossy(&output.stderr)
            )));
        }
        Ok(parse_systemctl_scope_units(&String::from_utf8_lossy(
            &output.stdout,
        )))
    }

    fn stop_unit(&self, unit: &str) -> Result<(), io::Error> {
        let output = Command::new("systemctl")
            .args(["--user", "--no-block", "stop", unit])
            .output()?;
        if output.status.success() {
            Ok(())
        } else {
            Err(io::Error::other(format!(
                "systemctl --no-block stop {unit} failed: {}",
                String::from_utf8_lossy(&output.stderr)
            )))
        }
    }

    fn unit_is_active(&self, unit: &str) -> Result<bool, io::Error> {
        let output = Command::new("systemctl")
            .args(["--user", "is-active", "--quiet", unit])
            .output()?;
        Ok(output.status.success())
    }
}

pub fn parse_systemctl_scope_units(output: &str) -> Vec<ScopeUnit> {
    output
        .lines()
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            let unit = parts.next()?;
            if !unit.ends_with(".scope") {
                return None;
            }
            let description = line
                .splitn(5, char::is_whitespace)
                .nth(4)
                .unwrap_or("")
                .trim()
                .to_string();
            Some(ScopeUnit {
                unit: unit.to_string(),
                description,
            })
        })
        .collect()
}

pub fn is_own_ccbd_scope(scope: &ScopeUnit, daemon_marker: &str) -> bool {
    scope.description.contains(&format!("@{daemon_marker}"))
}

pub fn is_orphan_scope(scope: &ScopeUnit, live_refs: &HashSet<String>) -> bool {
    let searchable = format!("{} {}", scope.unit, scope.description);
    !live_refs
        .iter()
        .any(|reference| searchable.contains(reference))
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
    detect_scope_policy_with_daemon_unit(socket_name, None)
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

pub fn active_daemon_unit_or_none(daemon_unit: Option<String>) -> Option<String> {
    active_daemon_unit_or_none_with_runner(daemon_unit, &RealSystemctlRunner)
}

pub fn active_daemon_unit_or_none_with_runner(
    daemon_unit: Option<String>,
    runner: &dyn SystemctlRunner,
) -> Option<String> {
    let Some(unit) = daemon_unit else {
        return None;
    };
    match runner.unit_is_active(&unit) {
        Ok(true) => Some(unit),
        Ok(false) => {
            tracing::warn!(
                unit = %unit,
                "derived daemon unit is not active; spawning child scopes without BindsTo/PartOf"
            );
            None
        }
        Err(err) => {
            tracing::warn!(
                unit = %unit,
                error = %err,
                "failed to verify derived daemon unit; spawning child scopes without BindsTo/PartOf"
            );
            None
        }
    }
}

fn systemd_run_available() -> bool {
    Command::new("systemd-run")
        .args(["--user", "--scope", "--", "true"])
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

/// Wrap the provider entrypoint in `systemd-run --user --scope`.
pub fn wrap_command(
    agent_id: &str,
    project_id: &str,
    daemon_marker: &str,
    env_state: &EnvState,
    // True only when session.realign matched a CRASHED agent.
    is_recovery: bool,
    daemon_unit: Option<&str>,
    manifest: &ProviderManifest,
    extra_env_vars: &HashMap<String, String>,
) -> Vec<String> {
    let recovery = RecoverySpawn {
        is_recovery,
        args: Vec::new(),
    };
    if env_state.unsafe_no_sandbox {
        return command_with_env_prefix(manifest, extra_env_vars, &recovery);
    }

    let mut cmd = vec![
        "systemd-run".to_string(),
        "--user".to_string(),
        "--scope".to_string(),
        "--collect".to_string(),
        format!("--description=ccbd-agent-{agent_id}@{daemon_marker}"),
    ];
    if env_state.under_systemd {
        cmd.push(format!("--slice={}", agent_slice_for_project(project_id)));
        append_daemon_unit_dependencies(&mut cmd, daemon_unit);
    }
    cmd.push("--".to_string());
    cmd.extend(command_with_env_prefix(manifest, extra_env_vars, &recovery));
    cmd
}

pub fn wrap_command_with_recovery(
    agent_id: &str,
    project_id: &str,
    daemon_marker: &str,
    env_state: &EnvState,
    recovery: RecoverySpawn,
    daemon_unit: Option<&str>,
    manifest: &ProviderManifest,
    extra_env_vars: &HashMap<String, String>,
) -> Vec<String> {
    wrap_command_with_recovery_and_sandbox_overrides(
        agent_id,
        project_id,
        daemon_marker,
        env_state,
        recovery,
        daemon_unit,
        manifest,
        extra_env_vars,
        &SandboxOverrides::default(),
    )
}

pub fn wrap_command_with_recovery_and_sandbox_overrides(
    agent_id: &str,
    project_id: &str,
    daemon_marker: &str,
    env_state: &EnvState,
    recovery: RecoverySpawn,
    daemon_unit: Option<&str>,
    manifest: &ProviderManifest,
    extra_env_vars: &HashMap<String, String>,
    sandbox_overrides: &SandboxOverrides,
) -> Vec<String> {
    if env_state.unsafe_no_sandbox {
        return command_with_env_prefix(manifest, extra_env_vars, &recovery);
    }

    let mut cmd = vec![
        "systemd-run".to_string(),
        "--user".to_string(),
        "--scope".to_string(),
        "--collect".to_string(),
        format!("--description=ccbd-agent-{agent_id}@{daemon_marker}"),
    ];
    if env_state.under_systemd {
        cmd.push(format!("--slice={}", agent_slice_for_project(project_id)));
        append_daemon_unit_dependencies(&mut cmd, daemon_unit);
    }
    append_read_only_bind_overrides(&mut cmd, sandbox_overrides);
    cmd.push("--".to_string());
    cmd.extend(command_with_env_prefix(manifest, extra_env_vars, &recovery));
    cmd
}

pub fn master_command(
    project_id: &str,
    cmd: &str,
    env_state: &EnvState,
    daemon_unit: Option<&str>,
) -> Vec<String> {
    master_command_with_env(project_id, cmd, env_state, daemon_unit, &HashMap::new())
}

pub fn master_command_with_env(
    project_id: &str,
    cmd: &str,
    env_state: &EnvState,
    daemon_unit: Option<&str>,
    extra_env_vars: &HashMap<String, String>,
) -> Vec<String> {
    if env_state.unsafe_no_sandbox || !env_state.systemd_run_available {
        return shell_command_with_env_prefix(cmd, extra_env_vars);
    }

    let mut command = vec![
        "systemd-run".to_string(),
        "--user".to_string(),
        "--scope".to_string(),
        "--collect".to_string(),
    ];
    if env_state.under_systemd {
        command.push(format!(
            "--slice={}",
            workspace_slice_for_project(project_id)
        ));
        append_daemon_unit_dependencies(&mut command, daemon_unit);
    }
    command.push("--".to_string());
    command.extend(master_shell_command_with_env_prefix(cmd, extra_env_vars));
    command
}

fn append_daemon_unit_dependencies(cmd: &mut Vec<String>, daemon_unit: Option<&str>) {
    let Some(unit) = daemon_unit else {
        return;
    };
    cmd.push(format!("--property=BindsTo={unit}"));
    cmd.push(format!("--property=PartOf={unit}"));
}

fn append_read_only_bind_overrides(cmd: &mut Vec<String>, overrides: &SandboxOverrides) {
    for bind in &overrides.extra_ro_binds {
        cmd.push(format!(
            "--property=BindReadOnlyPaths={}:{}",
            bind.host_path, bind.sandbox_path
        ));
    }
}

fn agent_slice_for_project(project_id: &str) -> String {
    format!("ccb-{}-ccbd-agents.slice", sanitize_project_id(project_id))
}

fn workspace_slice_for_project(project_id: &str) -> String {
    format!(
        "ccb-{}-ccbd-workspace.slice",
        sanitize_project_id(project_id)
    )
}

fn sanitize_project_id(project_id: &str) -> String {
    let sanitized = project_id
        .bytes()
        .map(|byte| {
            if byte.is_ascii_alphanumeric() || byte == b'-' {
                byte as char
            } else {
                '-'
            }
        })
        .collect::<String>();
    let sanitized = sanitized.trim_matches('-');
    if sanitized.is_empty() {
        "project"
    } else {
        sanitized
    }
    .to_string()
}

fn command_with_env_prefix(
    manifest: &ProviderManifest,
    extra_env_vars: &HashMap<String, String>,
    recovery: &RecoverySpawn,
) -> Vec<String> {
    let mut cmd = Vec::new();
    let spawn_env = collect_spawn_env(manifest, extra_env_vars);
    if !spawn_env.is_empty() {
        cmd.push("env".to_string());
        cmd.extend(
            spawn_env
                .into_iter()
                .map(|(key, value)| format!("{key}={value}")),
        );
    }
    cmd.extend(manifest.command.iter().map(|part| (*part).to_string()));
    if recovery.is_recovery {
        if recovery.args.is_empty() {
            cmd.extend(manifest.resume_args.iter().map(|part| (*part).to_string()));
        } else {
            cmd.extend(recovery.args.iter().cloned());
        }
    }
    cmd
}

fn shell_command_with_env_prefix(
    shell_cmd: &str,
    extra_env_vars: &HashMap<String, String>,
) -> Vec<String> {
    let mut cmd = Vec::new();
    if !extra_env_vars.is_empty() {
        cmd.push("env".to_string());
        let mut env_entries = extra_env_vars
            .iter()
            .map(|(key, value)| format!("{key}={value}"))
            .collect::<Vec<_>>();
        env_entries.sort();
        cmd.extend(env_entries);
    }
    cmd.extend(["sh".to_string(), "-lc".to_string(), shell_cmd.to_string()]);
    cmd
}

fn master_shell_command_with_env_prefix(
    shell_cmd: &str,
    extra_env_vars: &HashMap<String, String>,
) -> Vec<String> {
    let mut cmd = Vec::new();
    if !extra_env_vars.is_empty() {
        cmd.push("env".to_string());
        let mut env_entries = extra_env_vars
            .iter()
            .map(|(key, value)| format!("{key}={value}"))
            .collect::<Vec<_>>();
        env_entries.sort();
        cmd.extend(env_entries);
    }
    cmd.extend([
        "sh".to_string(),
        "-lc".to_string(),
        "echo 500 > /proc/self/oom_score_adj 2>/dev/null || { echo 'ccbd master failed to set oom_score_adj=500' >&2; exit 126; }; exec sh -lc \"$1\"".to_string(),
        "sh".to_string(),
        shell_cmd.to_string(),
    ]);
    cmd
}

#[cfg(test)]
mod tests {
    use super::{
        ScopeUnit, SystemctlRunner, active_daemon_unit_or_none_with_runner,
        detect_scope_policy_with_daemon_unit, wrap_in_scope,
    };
    use std::io;

    struct UnitActiveRunner {
        active: bool,
    }

    impl SystemctlRunner for UnitActiveRunner {
        fn list_scope_units(&self) -> Result<Vec<ScopeUnit>, io::Error> {
            Ok(Vec::new())
        }

        fn unit_is_active(&self, _unit: &str) -> Result<bool, io::Error> {
            Ok(self.active)
        }

        fn stop_unit(&self, _unit: &str) -> Result<(), io::Error> {
            Ok(())
        }
    }

    fn command_args(command: &std::process::Command) -> Vec<String> {
        command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn inactive_declared_daemon_unit_omits_binds_to_from_tmux_scope() {
        let daemon_unit = active_daemon_unit_or_none_with_runner(
            Some("ah-test.service".to_string()),
            &UnitActiveRunner { active: false },
        );
        let policy =
            detect_scope_policy_with_daemon_unit("ahd-abc123def456789a", daemon_unit.as_deref());
        let command = wrap_in_scope("tmux", &["-L", "test", "new-session"], &policy);
        let args = command_args(&command);

        assert_eq!(command.get_program(), "systemd-run");
        assert!(
            !args
                .iter()
                .any(|arg| arg.starts_with("--property=BindsTo="))
        );
        assert!(!args.iter().any(|arg| arg.starts_with("--property=PartOf=")));
    }

    #[test]
    fn active_declared_daemon_unit_keeps_binds_to_in_tmux_scope() {
        let daemon_unit = active_daemon_unit_or_none_with_runner(
            Some("ah-test.service".to_string()),
            &UnitActiveRunner { active: true },
        );
        let policy =
            detect_scope_policy_with_daemon_unit("ahd-abc123def456789a", daemon_unit.as_deref());
        let command = wrap_in_scope("tmux", &["-L", "test", "new-session"], &policy);
        let args = command_args(&command);

        assert_eq!(command.get_program(), "systemd-run");
        assert!(args.contains(&"--property=BindsTo=ah-test.service".to_string()));
        assert!(args.contains(&"--property=PartOf=ah-test.service".to_string()));
    }
}
