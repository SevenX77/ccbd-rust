//! systemd-run command wrapping for sandboxed agent processes.

use crate::sandbox::EnvState;
use portable_pty::CommandBuilder;

/// Wrap bwrap and provider entrypoint in `systemd-run --user --scope`.
pub fn wrap_command(
    agent_id: &str,
    env_state: &EnvState,
    bwrap_args: &[String],
    provider_entrypoint: &str,
) -> CommandBuilder {
    if env_state.unsafe_no_sandbox {
        return CommandBuilder::new(provider_entrypoint);
    }

    let mut cmd = CommandBuilder::new("systemd-run");
    cmd.arg("--user");
    cmd.arg("--scope");
    cmd.arg("--slice=ccbd-agents.slice");
    cmd.arg("--property=BindsTo=ccbd-rust.service");
    cmd.arg(format!("--description=ccbd-agent-{agent_id}"));
    cmd.arg("bwrap");
    cmd.args(bwrap_args.iter().map(String::as_str));
    cmd.arg(provider_entrypoint);
    cmd
}

#[cfg(test)]
mod tests {
    use super::wrap_command;
    use crate::sandbox::EnvState;

    fn env_state(unsafe_no_sandbox: bool) -> EnvState {
        EnvState {
            bwrap_available: !unsafe_no_sandbox,
            systemd_run_available: !unsafe_no_sandbox,
            unsafe_no_sandbox,
        }
    }

    fn argv_strings(cmd: &portable_pty::CommandBuilder) -> Vec<String> {
        cmd.get_argv()
            .iter()
            .map(|arg| arg.to_string_lossy().to_string())
            .collect()
    }

    #[test]
    fn test_wrap_command_uses_systemd_scope() {
        let cmd = wrap_command(
            "ag_1",
            &env_state(false),
            &["--unshare-net".into(), "--proc".into(), "/proc".into()],
            "bash",
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
        assert!(!argv.contains(&"--no-block".to_string()));
    }

    #[test]
    fn test_wrap_command_bypass_returns_provider() {
        let cmd = wrap_command("ag_1", &env_state(true), &["--unshare-net".into()], "bash");
        let argv = argv_strings(&cmd);

        assert_eq!(argv, vec!["bash".to_string()]);
    }
}
