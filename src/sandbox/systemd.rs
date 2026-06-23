//! systemd-run command wrapping for agent processes.

use crate::provider::manifest::{ProviderManifest, collect_spawn_env};
use crate::sandbox::{EnvState, SandboxOverrides};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoverySpawn {
    pub is_recovery: bool,
    pub args: Vec<String>,
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
        master_command, master_command_with_env, wrap_command,
        wrap_command_with_recovery_and_sandbox_overrides,
    };
    use crate::provider::manifest::get_manifest;
    use crate::sandbox::{EnvState, ReadOnlyBind, SandboxOverrides};
    use std::collections::HashMap;

    fn env_state(unsafe_no_sandbox: bool) -> EnvState {
        EnvState {
            systemd_run_available: !unsafe_no_sandbox,
            unsafe_no_sandbox,
            under_systemd: !unsafe_no_sandbox,
        }
    }

    fn env_state_with_systemd(under_systemd: bool) -> EnvState {
        EnvState {
            systemd_run_available: true,
            unsafe_no_sandbox: false,
            under_systemd,
        }
    }

    fn argv_strings(args: &[String]) -> Vec<String> {
        args.to_vec()
    }

    fn extra_env() -> std::collections::HashMap<String, String> {
        std::collections::HashMap::new()
    }

    fn assert_collect_after_scope_before_separator(cmd: &[String]) {
        let scope = cmd.iter().position(|arg| arg == "--scope").unwrap();
        let collect = cmd.iter().position(|arg| arg == "--collect").unwrap();
        let separator = cmd.iter().position(|arg| arg == "--").unwrap();

        assert!(scope < collect, "--collect must appear after --scope");
        assert!(collect < separator, "--collect must appear before --");
    }

    #[test]
    fn test_wrap_command_uses_systemd_scope() {
        let cmd = wrap_command(
            "ag_1",
            "p1",
            "ccbd-test",
            &env_state(false),
            false,
            Some("ahd.service"),
            &get_manifest("bash"),
            &extra_env(),
        );
        let argv = argv_strings(&cmd);

        assert_eq!(argv[0], "systemd-run");
        assert!(argv.contains(&"--user".to_string()));
        assert!(argv.contains(&"--scope".to_string()));
        assert_collect_after_scope_before_separator(&argv);
        assert!(argv.contains(&"--slice=ccb-p1-ccbd-agents.slice".to_string()));
        assert!(argv.contains(&"--property=BindsTo=ahd.service".to_string()));
        assert!(argv.contains(&"--property=PartOf=ahd.service".to_string()));
        assert!(argv.contains(&"--description=ccbd-agent-ag_1@ccbd-test".to_string()));
        assert!(!argv.contains(&"bwrap".to_string()));
        assert!(argv.contains(&"bash".to_string()));
        let separator = argv.iter().position(|arg| arg == "--").unwrap();
        assert_eq!(argv[separator + 1], "env");
        assert!(argv.contains(&"PS1=$ ".to_string()));
        assert!(!argv.contains(&"--no-block".to_string()));
    }

    #[test]
    fn test_wrap_command_applies_read_only_bind_overrides_before_separator() {
        let overrides = SandboxOverrides {
            extra_ro_binds: vec![ReadOnlyBind {
                host_path: "/opt/tools".to_string(),
                sandbox_path: "/mnt/tools".to_string(),
            }],
        };
        let cmd = wrap_command_with_recovery_and_sandbox_overrides(
            "ag_1",
            "p1",
            "ccbd-test",
            &env_state(false),
            super::RecoverySpawn {
                is_recovery: false,
                args: Vec::new(),
            },
            Some("ahd.service"),
            &get_manifest("bash"),
            &extra_env(),
            &overrides,
        );

        let bind = "--property=BindReadOnlyPaths=/opt/tools:/mnt/tools".to_string();
        let bind_pos = cmd
            .iter()
            .position(|arg| arg == &bind)
            .expect("bind property");
        let separator = cmd.iter().position(|arg| arg == "--").unwrap();
        assert!(bind_pos < separator);
    }

    #[test]
    fn test_wrap_command_description_contains_daemon_marker() {
        let cmd = wrap_command(
            "ag_1",
            "p1",
            "ccbd-1234567890abcdef",
            &env_state_with_systemd(false),
            false,
            Some("ahd.service"),
            &get_manifest("bash"),
            &extra_env(),
        );

        assert!(cmd.contains(&"--description=ccbd-agent-ag_1@ccbd-1234567890abcdef".to_string()));
    }

    #[test]
    fn test_wrap_command_binds_to_daemon_not_session_anchor() {
        let cmd = wrap_command(
            "ag_1",
            "p1",
            "ccbd-test",
            &env_state(false),
            false,
            Some("ahd.service"),
            &get_manifest("bash"),
            &extra_env(),
        );

        assert!(cmd.contains(&"--property=BindsTo=ahd.service".to_string()));
        assert!(
            !cmd.iter()
                .any(|arg| arg.starts_with("--property=BindsTo=ahd-session-"))
        );
    }

    #[test]
    fn test_wrap_command_uses_project_slice_hierarchy() {
        let cmd = wrap_command(
            "ag_1",
            "p1",
            "ccbd-test",
            &env_state(false),
            false,
            Some("ahd.service"),
            &get_manifest("bash"),
            &extra_env(),
        );

        assert!(cmd.contains(&"--slice=ccb-p1-ccbd-agents.slice".to_string()));
        assert!(!cmd.contains(&"--slice=ccbd-agents.slice".to_string()));
    }

    #[test]
    fn test_master_command_uses_systemd_run_workspace_slice() {
        let cmd = master_command(
            "p1",
            "claude",
            &env_state_with_systemd(true),
            Some("ahd.service"),
        );

        assert_eq!(cmd[0], "systemd-run");
        assert!(cmd.contains(&"--user".to_string()));
        assert!(cmd.contains(&"--scope".to_string()));
        assert_collect_after_scope_before_separator(&cmd);
        assert!(!cmd.contains(&"--property=OOMScoreAdjust=500".to_string()));
        assert!(cmd.contains(&"--slice=ccb-p1-ccbd-workspace.slice".to_string()));
        assert!(cmd.contains(&"--property=BindsTo=ahd.service".to_string()));
        assert!(cmd.contains(&"--property=PartOf=ahd.service".to_string()));
        assert!(
            cmd.iter()
                .any(|arg| arg.contains("/proc/self/oom_score_adj"))
        );
        assert!(cmd.iter().any(|arg| arg.contains("exec sh -lc")));
        assert!(cmd.contains(&"claude".to_string()));
    }

    #[test]
    fn test_master_command_passes_complex_argv_through_sh_lc() {
        let master_cmd = "claude --dangerously-skip-permissions --continue /remote-control";
        let cmd = master_command(
            "p1",
            master_cmd,
            &env_state_with_systemd(true),
            Some("ahd.service"),
        );
        let sh_pos = cmd.iter().position(|arg| arg == "sh").unwrap();

        assert_eq!(cmd[sh_pos], "sh");
        assert_eq!(cmd[sh_pos + 1], "-lc");
        assert!(cmd[sh_pos + 2].contains("/proc/self/oom_score_adj"));
        assert!(cmd[sh_pos + 2].contains("exec sh -lc"));
        assert_eq!(cmd[sh_pos + 3], "sh");
        assert_eq!(cmd[sh_pos + 4], master_cmd);
    }

    #[test]
    fn test_master_command_preserves_quotes_and_spacing_as_single_shell_arg() {
        let master_cmd = r#"claude   --model "sonnet latest"   /remote-control"#;
        let cmd = master_command(
            "p1",
            master_cmd,
            &env_state_with_systemd(false),
            Some("ahd.service"),
        );
        let sh_pos = cmd.iter().position(|arg| arg == "sh").unwrap();

        assert_eq!(cmd[sh_pos + 1], "-lc");
        assert!(cmd[sh_pos + 2].contains("/proc/self/oom_score_adj"));
        assert_eq!(cmd[sh_pos + 3], "sh");
        assert_eq!(cmd[sh_pos + 4], master_cmd);
        assert_eq!(cmd.iter().filter(|arg| *arg == master_cmd).count(), 1);
    }

    #[test]
    fn test_master_command_injects_isolated_home_env_before_shell() {
        let mut extra_env = HashMap::new();
        extra_env.insert("HOME".to_string(), "/tmp/ah-home".to_string());
        extra_env.insert(
            "CLAUDE_CONFIG_DIR".to_string(),
            "/tmp/ah-home/.claude".to_string(),
        );

        let cmd = master_command_with_env(
            "p1",
            "claude",
            &env_state_with_systemd(false),
            None,
            &extra_env,
        );
        let env_pos = cmd.iter().position(|arg| arg == "env").unwrap();

        assert_eq!(cmd[env_pos], "env");
        assert!(cmd.contains(&"HOME=/tmp/ah-home".to_string()));
        assert!(cmd.contains(&"CLAUDE_CONFIG_DIR=/tmp/ah-home/.claude".to_string()));
        assert_eq!(cmd[env_pos + 3], "sh");
        assert_eq!(cmd[env_pos + 4], "-lc");
        assert!(cmd[env_pos + 5].contains("/proc/self/oom_score_adj"));
        assert_eq!(cmd[env_pos + 6], "sh");
        assert_eq!(cmd[env_pos + 7], "claude");
    }

    #[test]
    fn spawn_master_pane_accepts_cutover_extra_env() {
        let mut extra_env = HashMap::new();
        extra_env.insert("AH_CUTOVER_ID".to_string(), "cutover-sess_1".to_string());
        extra_env.insert(
            "AH_MASTER_HANDOFF".to_string(),
            "/tmp/ah-state/cutovers/cutover-sess_1/handoff.md".to_string(),
        );

        let cmd = master_command_with_env(
            "p1",
            "claude --continue /remote-control",
            &env_state_with_systemd(false),
            None,
            &extra_env,
        );
        let env_pos = cmd.iter().position(|arg| arg == "env").unwrap();
        let sh_pos = cmd.iter().position(|arg| arg == "sh").unwrap();

        assert!(env_pos < sh_pos);
        assert!(cmd.contains(&"AH_CUTOVER_ID=cutover-sess_1".to_string()));
        assert!(cmd.contains(
            &"AH_MASTER_HANDOFF=/tmp/ah-state/cutovers/cutover-sess_1/handoff.md".to_string()
        ));
    }

    #[test]
    fn cutover_master_env_contains_ccb_socket_and_ah_state_dir() {
        let mut extra_env = HashMap::new();
        extra_env.insert("AH_STATE_DIR".to_string(), "/tmp/ah-state".to_string());
        extra_env.insert(
            "CCB_SOCKET".to_string(),
            "/tmp/ah-state/ahd.sock".to_string(),
        );
        extra_env.insert("AH_CUTOVER_ID".to_string(), "cutover-sess_1".to_string());
        extra_env.insert(
            "AH_MASTER_HANDOFF".to_string(),
            "/tmp/ah-state/cutovers/cutover-sess_1/handoff.md".to_string(),
        );
        extra_env.insert("AH_MASTER_ROLE".to_string(), "managed".to_string());

        let cmd = master_command_with_env(
            "p1",
            "claude --continue /remote-control",
            &env_state_with_systemd(false),
            None,
            &extra_env,
        );

        assert!(cmd.contains(&"CCB_SOCKET=/tmp/ah-state/ahd.sock".to_string()));
        assert!(cmd.contains(&"AH_STATE_DIR=/tmp/ah-state".to_string()));
        assert!(cmd.contains(&"AH_MASTER_ROLE=managed".to_string()));
        assert!(cmd.contains(&"AH_CUTOVER_ID=cutover-sess_1".to_string()));
        assert!(cmd.contains(
            &"AH_MASTER_HANDOFF=/tmp/ah-state/cutovers/cutover-sess_1/handoff.md".to_string()
        ));
    }

    #[test]
    fn ordinary_start_master_env_unchanged() {
        let cmd = master_command(
            "p1",
            "claude --continue /remote-control",
            &env_state_with_systemd(false),
            None,
        );
        let joined = cmd.join("\n");

        for key in [
            "AH_CUTOVER_ID=",
            "AH_MASTER_HANDOFF=",
            "AH_MASTER_ROLE=",
            "CCB_SOCKET=",
            "AH_STATE_DIR=",
        ] {
            assert!(
                !joined.contains(key),
                "ordinary master command must not contain cutover env {key}: {cmd:?}"
            );
        }
    }

    #[test]
    fn test_master_command_unsafe_path_does_not_inject_oom_wrapper() {
        let cmd = master_command("p1", "claude", &env_state(true), Some("ahd.service"));

        assert!(!cmd.iter().any(|arg| arg.contains("oom_score_adj")));
        assert!(!cmd.contains(&"--property=OOMScoreAdjust=500".to_string()));
        let sh_pos = cmd.iter().position(|arg| arg == "sh").unwrap();
        assert_eq!(cmd[sh_pos + 1], "-lc");
        assert_eq!(cmd[sh_pos + 2], "claude");
    }

    #[test]
    fn test_wrap_command_under_systemd_uses_slice_and_binds_to() {
        let cmd = wrap_command(
            "ag_1",
            "p1",
            "ccbd-test",
            &env_state_with_systemd(true),
            false,
            Some("ahd.service"),
            &get_manifest("bash"),
            &extra_env(),
        );

        assert!(cmd.contains(&"--slice=ccb-p1-ccbd-agents.slice".to_string()));
        assert!(cmd.contains(&"--property=BindsTo=ahd.service".to_string()));
        assert!(cmd.contains(&"--property=PartOf=ahd.service".to_string()));
    }

    #[test]
    fn test_wrap_command_binds_to_detected_ah_service() {
        let cmd = wrap_command(
            "ag_1",
            "p1",
            "ccbd-test",
            &env_state_with_systemd(true),
            false,
            Some("ah-p1.service"),
            &get_manifest("bash"),
            &extra_env(),
        );

        assert!(cmd.contains(&"--slice=ccb-p1-ccbd-agents.slice".to_string()));
        assert!(cmd.contains(&"--property=BindsTo=ah-p1.service".to_string()));
        assert!(cmd.contains(&"--property=PartOf=ah-p1.service".to_string()));
        assert!(
            !cmd.iter()
                .any(|arg| arg == "--property=BindsTo=ahd.service")
        );
    }

    #[test]
    fn test_wrap_command_no_detected_daemon_unit_omits_dependencies() {
        let cmd = wrap_command(
            "ag_1",
            "p1",
            "ccbd-test",
            &env_state_with_systemd(true),
            false,
            None,
            &get_manifest("bash"),
            &extra_env(),
        );

        assert!(cmd.contains(&"--slice=ccb-p1-ccbd-agents.slice".to_string()));
        assert!(!cmd.iter().any(|arg| arg.starts_with("--property=BindsTo=")));
        assert!(!cmd.iter().any(|arg| arg.starts_with("--property=PartOf=")));
    }

    #[test]
    fn test_wrap_command_dev_mode_omits_slice_and_binds_to() {
        let cmd = wrap_command(
            "ag_1",
            "p1",
            "ccbd-test",
            &env_state_with_systemd(false),
            false,
            Some("ahd.service"),
            &get_manifest("bash"),
            &extra_env(),
        );

        assert!(cmd.contains(&"systemd-run".to_string()));
        assert!(cmd.contains(&"--user".to_string()));
        assert!(cmd.contains(&"--scope".to_string()));
        assert!(!cmd.iter().any(|arg| arg.starts_with("--slice=")));
        assert!(!cmd.iter().any(|arg| arg.starts_with("--property=BindsTo=")));
        assert!(!cmd.iter().any(|arg| arg.starts_with("--property=PartOf=")));
    }

    #[test]
    fn test_master_command_under_systemd_uses_workspace_slice() {
        let cmd = master_command(
            "p1",
            "claude",
            &env_state_with_systemd(true),
            Some("ahd.service"),
        );

        assert!(cmd.contains(&"--slice=ccb-p1-ccbd-workspace.slice".to_string()));
        assert!(cmd.contains(&"--property=BindsTo=ahd.service".to_string()));
        assert!(cmd.contains(&"--property=PartOf=ahd.service".to_string()));
    }

    #[test]
    fn test_master_command_binds_to_detected_ah_service() {
        let cmd = master_command(
            "p1",
            "claude",
            &env_state_with_systemd(true),
            Some("ah-p1.service"),
        );

        assert!(cmd.contains(&"--slice=ccb-p1-ccbd-workspace.slice".to_string()));
        assert!(cmd.contains(&"--property=BindsTo=ah-p1.service".to_string()));
        assert!(cmd.contains(&"--property=PartOf=ah-p1.service".to_string()));
        assert!(
            !cmd.iter()
                .any(|arg| arg == "--property=BindsTo=ahd.service")
        );
    }

    #[test]
    fn test_master_command_no_detected_daemon_unit_omits_dependencies() {
        let cmd = master_command("p1", "claude", &env_state_with_systemd(true), None);

        assert!(cmd.contains(&"--slice=ccb-p1-ccbd-workspace.slice".to_string()));
        assert!(!cmd.iter().any(|arg| arg.starts_with("--property=BindsTo=")));
        assert!(!cmd.iter().any(|arg| arg.starts_with("--property=PartOf=")));
    }

    #[test]
    fn test_master_command_dev_mode_no_slice() {
        let cmd = master_command(
            "p1",
            "claude",
            &env_state_with_systemd(false),
            Some("ahd.service"),
        );

        assert_eq!(cmd[0], "systemd-run");
        assert!(cmd.contains(&"--user".to_string()));
        assert!(cmd.contains(&"--scope".to_string()));
        assert!(!cmd.iter().any(|arg| arg.starts_with("--slice=")));
        assert!(!cmd.iter().any(|arg| arg.starts_with("--property=BindsTo=")));
        assert!(!cmd.iter().any(|arg| arg.starts_with("--property=PartOf=")));
        assert!(cmd.contains(&"claude".to_string()));
    }

    #[test]
    fn test_wrap_command_bypass_returns_provider() {
        let cmd = wrap_command(
            "ag_1",
            "p1",
            "ccbd-test",
            &env_state(true),
            false,
            Some("ahd.service"),
            &get_manifest("bash"),
            &extra_env(),
        );
        let argv = argv_strings(&cmd);

        assert_eq!(argv.first().map(String::as_str), Some("env"));
        assert!(argv.contains(&"PS1=$ ".to_string()));
        assert_eq!(
            &argv[argv.len() - 4..],
            ["bash", "--noprofile", "--norc", "-i"]
        );
    }

    #[test]
    fn test_wrap_command_appends_extra_env_after_manifest_env() {
        let mut extra = std::collections::HashMap::new();
        extra.insert("CCB_GLOBAL".to_string(), "1".to_string());
        extra.insert("PS1".to_string(), "override> ".to_string());

        let cmd = wrap_command(
            "ag_1",
            "p1",
            "ccbd-test",
            &env_state(false),
            false,
            Some("ahd.service"),
            &get_manifest("bash"),
            &extra,
        );

        assert!(cmd.contains(&"CCB_GLOBAL=1".to_string()));
        assert!(cmd.contains(&"PS1=override> ".to_string()));
    }

    #[test]
    fn test_wrap_command_systemd_scope_binds_to_daemon() {
        let cmd = wrap_command(
            "ag_1",
            "p1",
            "ccbd-test",
            &env_state(false),
            false,
            Some("ahd.service"),
            &get_manifest("bash"),
            &extra_env(),
        );

        assert!(cmd.contains(&"--property=BindsTo=ahd.service".to_string()));
        assert!(cmd.contains(&"--property=PartOf=ahd.service".to_string()));
    }

    #[test]
    fn test_wrap_command_with_recovery_appends_resume_args() {
        let claude_recovery = wrap_command(
            "ag_1",
            "p1",
            "ccbd-test",
            &env_state(true),
            true,
            Some("ahd.service"),
            &get_manifest("claude"),
            &extra_env(),
        );
        assert!(claude_recovery.contains(&"claude".to_string()));
        assert!(claude_recovery.contains(&"--dangerously-skip-permissions".to_string()));
        assert!(claude_recovery.contains(&"--continue".to_string()));

        let claude_new = wrap_command(
            "ag_1",
            "p1",
            "ccbd-test",
            &env_state(true),
            false,
            Some("ahd.service"),
            &get_manifest("claude"),
            &extra_env(),
        );
        assert!(!claude_new.contains(&"--continue".to_string()));

        for provider in ["bash", "codex"] {
            let cmd = wrap_command(
                "ag_1",
                "p1",
                "ccbd-test",
                &env_state(true),
                true,
                Some("ahd.service"),
                &get_manifest(provider),
                &extra_env(),
            );
            assert!(
                !cmd.contains(&"--continue".to_string()),
                "{provider} should not add Claude resume args: {cmd:?}"
            );
        }
    }

    #[test]
    #[serial_test::serial(global_env)]
    fn test_wrap_command_injects_passthrough_and_forced_env() {
        unsafe {
            std::env::set_var("ANTHROPIC_API_KEY", "host-anthropic");
        }
        let cmd = wrap_command(
            "ag_1",
            "p1",
            "ccbd-test",
            &env_state(false),
            false,
            Some("ahd.service"),
            &get_manifest("claude"),
            &extra_env(),
        );
        unsafe {
            std::env::remove_var("ANTHROPIC_API_KEY");
        }

        assert!(cmd.contains(&"--property=BindsTo=ahd.service".to_string()));
        assert!(cmd.contains(&"--property=PartOf=ahd.service".to_string()));
        assert!(cmd.contains(&"ANTHROPIC_API_KEY=host-anthropic".to_string()));
        assert!(cmd.contains(&"CCB_CLAUDE_MD_MODE=route".to_string()));
    }
}
