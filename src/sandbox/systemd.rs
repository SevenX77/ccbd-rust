//! systemd-run command wrapping for agent processes.

use crate::provider::manifest::{ProviderManifest, collect_spawn_env};
use crate::sandbox::EnvState;
use std::collections::HashMap;

/// Wrap the provider entrypoint in `systemd-run --user --scope`.
pub fn wrap_command(
    agent_id: &str,
    project_id: &str,
    daemon_marker: &str,
    env_state: &EnvState,
    daemon_unit: Option<&str>,
    manifest: &ProviderManifest,
    extra_env_vars: &HashMap<String, String>,
) -> Vec<String> {
    if env_state.unsafe_no_sandbox {
        return command_with_env_prefix(manifest, extra_env_vars);
    }

    let mut cmd = vec![
        "systemd-run".to_string(),
        "--user".to_string(),
        "--scope".to_string(),
        format!("--description=ccbd-agent-{agent_id}@{daemon_marker}"),
    ];
    if env_state.under_systemd {
        cmd.push(format!("--slice={}", agent_slice_for_project(project_id)));
        append_daemon_unit_dependencies(&mut cmd, daemon_unit);
    }
    cmd.push("--".to_string());
    cmd.extend(command_with_env_prefix(manifest, extra_env_vars));
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
    let mut command = vec![
        "systemd-run".to_string(),
        "--user".to_string(),
        "--scope".to_string(),
    ];
    if env_state.under_systemd {
        command.push(format!(
            "--slice={}",
            workspace_slice_for_project(project_id)
        ));
        append_daemon_unit_dependencies(&mut command, daemon_unit);
    }
    command.push("--".to_string());
    command.extend(shell_command_with_env_prefix(cmd, extra_env_vars));
    command
}

fn append_daemon_unit_dependencies(cmd: &mut Vec<String>, daemon_unit: Option<&str>) {
    let Some(unit) = daemon_unit else {
        return;
    };
    cmd.push(format!("--property=BindsTo={unit}"));
    cmd.push(format!("--property=PartOf={unit}"));
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

#[cfg(test)]
mod tests {
    use super::{master_command, master_command_with_env, wrap_command};
    use crate::provider::manifest::get_manifest;
    use crate::sandbox::EnvState;
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

    #[test]
    fn test_wrap_command_uses_systemd_scope() {
        let cmd = wrap_command(
            "ag_1",
            "p1",
            "ccbd-test",
            &env_state(false),
            Some("ccbd.service"),
            &get_manifest("bash"),
            &extra_env(),
        );
        let argv = argv_strings(&cmd);

        assert_eq!(argv[0], "systemd-run");
        assert!(argv.contains(&"--user".to_string()));
        assert!(argv.contains(&"--scope".to_string()));
        assert!(argv.contains(&"--slice=ccb-p1-ccbd-agents.slice".to_string()));
        assert!(argv.contains(&"--property=BindsTo=ccbd.service".to_string()));
        assert!(argv.contains(&"--property=PartOf=ccbd.service".to_string()));
        assert!(argv.contains(&"--description=ccbd-agent-ag_1@ccbd-test".to_string()));
        assert!(!argv.contains(&"bwrap".to_string()));
        assert!(argv.contains(&"bash".to_string()));
        let separator = argv.iter().position(|arg| arg == "--").unwrap();
        assert_eq!(argv[separator + 1], "env");
        assert!(argv.contains(&"PS1=$ ".to_string()));
        assert!(!argv.contains(&"--no-block".to_string()));
    }

    #[test]
    fn test_wrap_command_description_contains_daemon_marker() {
        let cmd = wrap_command(
            "ag_1",
            "p1",
            "ccbd-1234567890abcdef",
            &env_state_with_systemd(false),
            Some("ccbd.service"),
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
            Some("ccbd.service"),
            &get_manifest("bash"),
            &extra_env(),
        );

        assert!(cmd.contains(&"--property=BindsTo=ccbd.service".to_string()));
        assert!(
            !cmd.iter()
                .any(|arg| arg.starts_with("--property=BindsTo=ccbd-session-"))
        );
    }

    #[test]
    fn test_wrap_command_uses_project_slice_hierarchy() {
        let cmd = wrap_command(
            "ag_1",
            "p1",
            "ccbd-test",
            &env_state(false),
            Some("ccbd.service"),
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
            Some("ccbd.service"),
        );

        assert_eq!(cmd[0], "systemd-run");
        assert!(cmd.contains(&"--user".to_string()));
        assert!(cmd.contains(&"--scope".to_string()));
        assert!(cmd.contains(&"--slice=ccb-p1-ccbd-workspace.slice".to_string()));
        assert!(cmd.contains(&"--property=BindsTo=ccbd.service".to_string()));
        assert!(cmd.contains(&"--property=PartOf=ccbd.service".to_string()));
        assert!(cmd.contains(&"claude".to_string()));
    }

    #[test]
    fn test_master_command_passes_complex_argv_through_sh_lc() {
        let master_cmd = "claude --dangerously-skip-permissions --continue /remote-control";
        let cmd = master_command(
            "p1",
            master_cmd,
            &env_state_with_systemd(true),
            Some("ccbd.service"),
        );
        let sh_pos = cmd.iter().position(|arg| arg == "sh").unwrap();

        assert_eq!(cmd[sh_pos], "sh");
        assert_eq!(cmd[sh_pos + 1], "-lc");
        assert_eq!(cmd[sh_pos + 2], master_cmd);
    }

    #[test]
    fn test_master_command_preserves_quotes_and_spacing_as_single_shell_arg() {
        let master_cmd = r#"claude   --model "sonnet latest"   /remote-control"#;
        let cmd = master_command(
            "p1",
            master_cmd,
            &env_state_with_systemd(false),
            Some("ccbd.service"),
        );
        let sh_pos = cmd.iter().position(|arg| arg == "sh").unwrap();

        assert_eq!(cmd[sh_pos + 1], "-lc");
        assert_eq!(cmd[sh_pos + 2], master_cmd);
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
        assert_eq!(cmd[env_pos + 5], "claude");
    }

    #[test]
    fn test_wrap_command_under_systemd_uses_slice_and_binds_to() {
        let cmd = wrap_command(
            "ag_1",
            "p1",
            "ccbd-test",
            &env_state_with_systemd(true),
            Some("ccbd.service"),
            &get_manifest("bash"),
            &extra_env(),
        );

        assert!(cmd.contains(&"--slice=ccb-p1-ccbd-agents.slice".to_string()));
        assert!(cmd.contains(&"--property=BindsTo=ccbd.service".to_string()));
        assert!(cmd.contains(&"--property=PartOf=ccbd.service".to_string()));
    }

    #[test]
    fn test_wrap_command_binds_to_detected_ah_service() {
        let cmd = wrap_command(
            "ag_1",
            "p1",
            "ccbd-test",
            &env_state_with_systemd(true),
            Some("ah-p1.service"),
            &get_manifest("bash"),
            &extra_env(),
        );

        assert!(cmd.contains(&"--slice=ccb-p1-ccbd-agents.slice".to_string()));
        assert!(cmd.contains(&"--property=BindsTo=ah-p1.service".to_string()));
        assert!(cmd.contains(&"--property=PartOf=ah-p1.service".to_string()));
        assert!(
            !cmd.iter()
                .any(|arg| arg == "--property=BindsTo=ccbd.service")
        );
    }

    #[test]
    fn test_wrap_command_no_detected_daemon_unit_omits_dependencies() {
        let cmd = wrap_command(
            "ag_1",
            "p1",
            "ccbd-test",
            &env_state_with_systemd(true),
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
            Some("ccbd.service"),
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
            Some("ccbd.service"),
        );

        assert!(cmd.contains(&"--slice=ccb-p1-ccbd-workspace.slice".to_string()));
        assert!(cmd.contains(&"--property=BindsTo=ccbd.service".to_string()));
        assert!(cmd.contains(&"--property=PartOf=ccbd.service".to_string()));
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
                .any(|arg| arg == "--property=BindsTo=ccbd.service")
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
            Some("ccbd.service"),
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
            Some("ccbd.service"),
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
            Some("ccbd.service"),
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
            Some("ccbd.service"),
            &get_manifest("bash"),
            &extra_env(),
        );

        assert!(cmd.contains(&"--property=BindsTo=ccbd.service".to_string()));
        assert!(cmd.contains(&"--property=PartOf=ccbd.service".to_string()));
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
            Some("ccbd.service"),
            &get_manifest("claude"),
            &extra_env(),
        );
        unsafe {
            std::env::remove_var("ANTHROPIC_API_KEY");
        }

        assert!(cmd.contains(&"--property=BindsTo=ccbd.service".to_string()));
        assert!(cmd.contains(&"--property=PartOf=ccbd.service".to_string()));
        assert!(cmd.contains(&"ANTHROPIC_API_KEY=host-anthropic".to_string()));
        assert!(cmd.contains(&"CCB_CLAUDE_MD_MODE=route".to_string()));
    }
}
