//! systemd-run command wrapping for sandboxed agent processes.

use crate::provider::manifest::ProviderManifest;
use crate::sandbox::EnvState;

/// Wrap bwrap and provider entrypoint in `systemd-run --user --scope`.
pub fn wrap_command(
    agent_id: &str,
    env_state: &EnvState,
    bwrap_args: &[String],
    manifest: &ProviderManifest,
) -> Vec<String> {
    if env_state.unsafe_no_sandbox {
        return command_with_env_prefix(manifest);
    }

    let mut cmd = vec![
        "systemd-run".to_string(),
        "--user".to_string(),
        "--scope".to_string(),
        "--slice=ccbd-agents.slice".to_string(),
        "--property=BindsTo=ccbd-rust.service".to_string(),
        format!("--description=ccbd-agent-{agent_id}"),
        "bwrap".to_string(),
    ];
    cmd.extend(bwrap_args.iter().cloned());
    for (key, value) in manifest.env_vars {
        cmd.push("--setenv".to_string());
        cmd.push((*key).to_string());
        cmd.push((*value).to_string());
    }
    cmd.extend(manifest.command.iter().map(|part| (*part).to_string()));
    cmd
}

fn command_with_env_prefix(manifest: &ProviderManifest) -> Vec<String> {
    let mut cmd = Vec::new();
    if !manifest.env_vars.is_empty() {
        cmd.push("env".to_string());
        cmd.extend(
            manifest
                .env_vars
                .iter()
                .map(|(key, value)| format!("{key}={value}")),
        );
    }
    cmd.extend(manifest.command.iter().map(|part| (*part).to_string()));
    cmd
}

#[cfg(test)]
mod tests {
    use super::wrap_command;
    use crate::provider::manifest::get_manifest;
    use crate::sandbox::EnvState;

    fn env_state(unsafe_no_sandbox: bool) -> EnvState {
        EnvState {
            bwrap_available: !unsafe_no_sandbox,
            systemd_run_available: !unsafe_no_sandbox,
            unsafe_no_sandbox,
        }
    }

    fn argv_strings(args: &[String]) -> Vec<String> {
        args.to_vec()
    }

    #[test]
    fn test_wrap_command_uses_systemd_scope() {
        let cmd = wrap_command(
            "ag_1",
            &env_state(false),
            &["--unshare-net".into(), "--proc".into(), "/proc".into()],
            &get_manifest("bash"),
        );
        let argv = argv_strings(&cmd);

        assert_eq!(argv[0], "systemd-run");
        assert!(argv.contains(&"--user".to_string()));
        assert!(argv.contains(&"--scope".to_string()));
        assert!(argv.contains(&"--slice=ccbd-agents.slice".to_string()));
        assert!(argv.contains(&"--property=BindsTo=ccbd-rust.service".to_string()));
        assert!(argv.contains(&"--description=ccbd-agent-ag_1".to_string()));
        assert!(argv.contains(&"bwrap".to_string()));
        assert!(argv.contains(&"bash".to_string()));
        assert!(
            argv.windows(3)
                .any(|window| window
                    == ["--setenv".to_string(), "PS1".to_string(), "$ ".to_string()])
        );
        assert!(!argv.contains(&"--no-block".to_string()));
    }

    #[test]
    fn test_wrap_command_bypass_returns_provider() {
        let cmd = wrap_command(
            "ag_1",
            &env_state(true),
            &["--unshare-net".into()],
            &get_manifest("bash"),
        );
        let argv = argv_strings(&cmd);

        assert_eq!(
            argv,
            vec![
                "env".to_string(),
                "PS1=$ ".to_string(),
                "bash".to_string(),
                "--noprofile".to_string(),
                "--norc".to_string(),
                "-i".to_string()
            ]
        );
    }
}
