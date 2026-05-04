//! systemd-run command wrapping for sandboxed agent processes.

use crate::provider::manifest::{ProviderManifest, collect_spawn_env};
use crate::sandbox::EnvState;
use std::collections::HashMap;

/// Wrap bwrap and provider entrypoint in `systemd-run --user --scope`.
pub fn wrap_command(
    agent_id: &str,
    session_id: &str,
    binds_to_session_anchor: bool,
    env_state: &EnvState,
    bwrap_args: &[String],
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
        "--slice=ccbd-agents.slice".to_string(),
        format!("--description=ccbd-agent-{agent_id}"),
    ];
    if binds_to_session_anchor {
        cmd.push(format!(
            "--property=BindsTo=ccbd-session-{session_id}.service"
        ));
    }
    cmd.push("bwrap".to_string());
    cmd.extend(bwrap_args.iter().cloned());
    for (key, value) in sorted_extra_env(extra_env_vars) {
        cmd.push("--setenv".to_string());
        cmd.push(key);
        cmd.push(value);
    }
    cmd.extend(manifest.command.iter().map(|part| (*part).to_string()));
    cmd
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

fn sorted_extra_env(extra_env_vars: &HashMap<String, String>) -> Vec<(String, String)> {
    let mut env = extra_env_vars
        .iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect::<Vec<_>>();
    env.sort_by(|(left, _), (right, _)| left.cmp(right));
    env
}

#[cfg(test)]
mod tests {
    use super::wrap_command;
    use crate::provider::manifest::get_manifest;
    use crate::sandbox::EnvState;
    use crate::sandbox::bwrap::{self, SandboxOverrides};
    use std::path::Path;

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

    fn extra_env() -> std::collections::HashMap<String, String> {
        std::collections::HashMap::new()
    }

    fn bwrap_args_with_manifest(provider: &str) -> Vec<String> {
        bwrap::build_args(
            Path::new("/tmp/sandbox"),
            &SandboxOverrides::default(),
            Some(&get_manifest(provider)),
        )
        .unwrap()
    }

    #[test]
    fn test_wrap_command_uses_systemd_scope() {
        let cmd = wrap_command(
            "ag_1",
            "s_1",
            true,
            &env_state(false),
            &bwrap_args_with_manifest("bash"),
            &get_manifest("bash"),
            &extra_env(),
        );
        let argv = argv_strings(&cmd);

        assert_eq!(argv[0], "systemd-run");
        assert!(argv.contains(&"--user".to_string()));
        assert!(argv.contains(&"--scope".to_string()));
        assert!(argv.contains(&"--slice=ccbd-agents.slice".to_string()));
        assert!(argv.contains(&"--property=BindsTo=ccbd-session-s_1.service".to_string()));
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
            "s_1",
            false,
            &env_state(true),
            &["--unshare-net".into()],
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
            "s_1",
            true,
            &env_state(false),
            &bwrap_args_with_manifest("bash"),
            &get_manifest("bash"),
            &extra,
        );

        assert!(cmd.windows(3).any(|window| window
            == [
                "--setenv".to_string(),
                "CCB_GLOBAL".to_string(),
                "1".to_string()
            ]));
        assert!(cmd.windows(3).any(|window| window
            == [
                "--setenv".to_string(),
                "PS1".to_string(),
                "override> ".to_string()
            ]));
    }

    #[test]
    fn test_wrap_command_dev_scope_omits_binds_to() {
        let cmd = wrap_command(
            "ag_1",
            "s_1",
            false,
            &env_state(false),
            &["--unshare-net".into()],
            &get_manifest("bash"),
            &extra_env(),
        );

        assert!(!cmd.iter().any(|arg| arg.starts_with("--property=BindsTo=")));
    }

    #[test]
    fn test_wrap_command_injects_passthrough_and_forced_env() {
        unsafe {
            std::env::set_var("ANTHROPIC_API_KEY", "host-anthropic");
        }
        let cmd = wrap_command(
            "ag_1",
            "s_1",
            true,
            &env_state(false),
            &bwrap_args_with_manifest("claude"),
            &get_manifest("claude"),
            &extra_env(),
        );
        unsafe {
            std::env::remove_var("ANTHROPIC_API_KEY");
        }

        assert!(cmd.contains(&"--property=BindsTo=ccbd-session-s_1.service".to_string()));
        assert!(cmd.windows(3).any(|window| window
            == [
                "--setenv".to_string(),
                "ANTHROPIC_API_KEY".to_string(),
                "host-anthropic".to_string()
            ]));
        assert!(cmd.windows(3).any(|window| window
            == [
                "--setenv".to_string(),
                "CCB_CLAUDE_MD_MODE".to_string(),
                "route".to_string()
            ]));
    }
}
