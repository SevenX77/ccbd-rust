//! Bubblewrap argument construction for MVP2 sandboxed agents.

use crate::error::CcbdError;
use crate::provider::home_layout::{HomeOverrides, prepare_home_layout};
use crate::provider::manifest::{ProviderManifest, collect_spawn_env};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Optional sandbox overrides accepted by `agent.spawn`.
#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
pub struct SandboxOverrides {
    pub network: Option<String>,
    #[serde(default)]
    pub extra_ro_binds: Vec<RoBind>,
}

/// One extra read-only bind mount requested by the caller.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct RoBind {
    pub host_path: PathBuf,
    pub sandbox_path: PathBuf,
}

/// Build the baseline bubblewrap argument vector plus validated overrides.
pub fn build_args(
    sandbox_dir: &Path,
    workspace_dir: &Path,
    overrides: &SandboxOverrides,
    manifest: Option<&ProviderManifest>,
) -> Result<Vec<String>, CcbdError> {
    let home_overrides = if let Some(manifest) = manifest {
        if manifest.requires_home_materialization {
            Some(prepare_home_layout(manifest.provider_name, sandbox_dir)?)
        } else {
            None
        }
    } else {
        None
    };
    let mut args = vec![
        "--unshare-pid".to_string(),
        "--unshare-uts".to_string(),
        "--unshare-ipc".to_string(),
    ];

    match overrides.network.as_deref() {
        Some("none") | Some("isolated") => args.push("--unshare-net".to_string()),
        _ => args.push("--share-net".to_string()),
    }

    push_value(&mut args, "--proc", "/proc");
    push_value(&mut args, "--dev", "/dev");
    push_value(&mut args, "--tmpfs", "/tmp");
    push_bind(&mut args, "--ro-bind", "/usr");
    push_bind(&mut args, "--ro-bind-try", "/lib");
    push_bind(&mut args, "--ro-bind-try", "/lib64");
    push_bind(&mut args, "--ro-bind-try", "/bin");
    push_bind(&mut args, "--ro-bind-try", "/sbin");
    args.push("--ro-bind".to_string());
    args.push("/etc/resolv.conf".to_string());
    args.push("/etc/resolv.conf".to_string());
    push_bind(&mut args, "--ro-bind-try", "/etc/ssl");
    push_bind(&mut args, "--ro-bind-try", "/etc/ca-certificates");
    push_bind(&mut args, "--ro-bind-try", "/etc/pki");
    if let Some(home_overrides) = &home_overrides {
        args.push("--bind".to_string());
        args.push(home_overrides.home_root.display().to_string());
        args.push("/home/agent".to_string());
        if let Some(manifest) = manifest {
            push_provider_binary_path_binds(&mut args, manifest.provider_name);
        }
    } else {
        push_value(&mut args, "--dir", "/home/agent");
    }
    args.push("--setenv".to_string());
    args.push("HOME".to_string());
    args.push("/home/agent".to_string());
    args.push("--bind".to_string());
    args.push(workspace_dir.display().to_string());
    args.push("/workspace".to_string());
    push_workspace_git_ro_bind(&mut args, workspace_dir)?;
    push_value(&mut args, "--chdir", "/workspace");

    for bind in &overrides.extra_ro_binds {
        validate_safe_path(&bind.host_path)?;
        args.push("--ro-bind".to_string());
        args.push(bind.host_path.display().to_string());
        args.push(bind.sandbox_path.display().to_string());
    }

    if let Some(manifest) = manifest {
        if home_overrides.is_none() {
            push_manifest_auth_mounts(&mut args, manifest)?;
        }
        push_manifest_env(&mut args, manifest);
        if let Some(home_overrides) = &home_overrides {
            push_home_override_env(&mut args, home_overrides);
        }
    }

    Ok(args)
}

fn push_workspace_git_ro_bind(
    args: &mut Vec<String>,
    workspace_dir: &Path,
) -> Result<(), CcbdError> {
    let git_dir = workspace_dir.join(".git");
    if !git_dir.is_dir() {
        return Ok(());
    }
    validate_safe_path(&git_dir)?;
    args.push("--ro-bind".to_string());
    args.push(git_dir.display().to_string());
    args.push("/workspace/.git".to_string());
    Ok(())
}

fn push_value(args: &mut Vec<String>, flag: &str, path: &str) {
    args.push(flag.to_string());
    args.push(path.to_string());
}

fn push_bind(args: &mut Vec<String>, flag: &str, path: &str) {
    args.push(flag.to_string());
    args.push(path.to_string());
    args.push(path.to_string());
}

const SANDBOX_PROVIDER_BIN_DIR: &str = "/home/agent/.local/bin-agent";

fn push_provider_binary_path_binds(args: &mut Vec<String>, provider_name: &str) {
    let Some(home) = provider_source_home() else {
        return;
    };
    push_value(args, "--dir", "/home/agent/.local");
    push_value(args, "--dir", SANDBOX_PROVIDER_BIN_DIR);
    match provider_name {
        "claude" => {
            push_ro_bind_try(
                args,
                home.join(".local/bin/claude"),
                PathBuf::from(format!("{SANDBOX_PROVIDER_BIN_DIR}/claude")),
            );
            let runtime_dir = claude_runtime_dir(&home);
            push_ro_bind_try(args, runtime_dir.clone(), runtime_dir);
        }
        "codex" | "gemini" => {
            push_ro_bind_try(
                args,
                home.join(".npm-global"),
                PathBuf::from("/home/agent/.npm-global"),
            );
            push_ro_bind_try(
                args,
                home.join(format!(".npm-global/bin/{provider_name}")),
                PathBuf::from(format!("{SANDBOX_PROVIDER_BIN_DIR}/{provider_name}")),
            );
            push_system_node_binds(args);
        }
        _ => {}
    }
}

fn push_ro_bind_try(args: &mut Vec<String>, source: PathBuf, target: PathBuf) {
    args.push("--ro-bind-try".to_string());
    args.push(source.display().to_string());
    args.push(target.display().to_string());
}

fn claude_runtime_dir(home: &Path) -> PathBuf {
    let entry = home.join(".local/bin/claude");
    if let Ok(real_entry) = std::fs::canonicalize(&entry)
        && let Some(parent) = real_entry.parent()
        && parent.starts_with(home.join(".local/share/claude"))
    {
        return parent.to_path_buf();
    }
    home.join(".local/share/claude")
}

fn push_system_node_binds(args: &mut Vec<String>) {
    for node in ["/usr/local/bin/node", "/usr/bin/node", "/bin/node"] {
        let path = PathBuf::from(node);
        push_ro_bind_try(args, path.clone(), path);
    }
}

fn provider_source_home() -> Option<PathBuf> {
    let env_home = std::env::var_os("HOME")
        .filter(|home| !home.is_empty())
        .map(PathBuf::from)?;
    let passwd_home = std::env::var("USER")
        .ok()
        .and_then(|user| passwd_home_for_user(&user));
    Some(resolve_provider_source_home(env_home, passwd_home))
}

fn resolve_provider_source_home(env_home: PathBuf, passwd_home: Option<PathBuf>) -> PathBuf {
    if is_ccb_sandbox_home(&env_home)
        && let Some(passwd_home) = passwd_home
    {
        return passwd_home;
    }
    env_home
}

fn is_ccb_sandbox_home(path: &Path) -> bool {
    let path = path.to_string_lossy();
    path.contains("/.cache/ccb/sandboxes/") || path.contains("/.cache/ccb-rs/sandboxes/")
}

fn passwd_home_for_user(user: &str) -> Option<PathBuf> {
    let passwd = std::fs::read_to_string("/etc/passwd").ok()?;
    passwd.lines().find_map(|line| {
        let mut fields = line.split(':');
        let name = fields.next()?;
        if name != user {
            return None;
        }
        let _password = fields.next()?;
        let _uid = fields.next()?;
        let _gid = fields.next()?;
        let _gecos = fields.next()?;
        let home = fields.next()?;
        if home.is_empty() {
            None
        } else {
            Some(PathBuf::from(home))
        }
    })
}

fn push_manifest_auth_mounts(
    args: &mut Vec<String>,
    manifest: &ProviderManifest,
) -> Result<(), CcbdError> {
    let Some(home) = std::env::var_os("HOME").filter(|home| !home.is_empty()) else {
        return Ok(());
    };
    let home = PathBuf::from(home);
    for mount_path in &manifest.auth_mount_paths {
        let mount_path = Path::new(mount_path);
        let (host_path, sandbox_path) = if mount_path.is_absolute() {
            (PathBuf::from(mount_path), PathBuf::from(mount_path))
        } else {
            (
                home.join(mount_path),
                PathBuf::from("/home/agent").join(mount_path),
            )
        };
        let metadata = match std::fs::symlink_metadata(&host_path) {
            Ok(metadata) => metadata,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => continue,
            Err(err) => {
                return Err(CcbdError::SandboxMountFailed {
                    details: format!("stat auth mount {}: {err}", host_path.display()),
                });
            }
        };
        let canonical_host_path =
            std::fs::canonicalize(&host_path).map_err(|err| CcbdError::SandboxMountFailed {
                details: format!("canonicalize auth mount {}: {err}", host_path.display()),
            })?;
        if !metadata.file_type().is_symlink() && !metadata.is_dir() {
            continue;
        }
        if !canonical_host_path.is_dir() {
            continue;
        }
        validate_safe_path(&canonical_host_path)?;
        let host_path = host_path.display().to_string();
        let sandbox_path = sandbox_path.display().to_string();
        args.push("--ro-bind".to_string());
        args.push(host_path);
        args.push(sandbox_path);
    }
    Ok(())
}

fn push_manifest_env(args: &mut Vec<String>, manifest: &ProviderManifest) {
    for (key, value) in collect_spawn_env(manifest, &HashMap::new()) {
        // bwrap owns HOME so the sandbox keeps its private /home/agent.
        if key == "HOME" {
            continue;
        }
        args.push("--setenv".to_string());
        args.push(key);
        args.push(value);
    }
}

fn push_home_override_env(args: &mut Vec<String>, home_overrides: &HomeOverrides) {
    for (key, value) in &home_overrides.extra_env {
        args.push("--setenv".to_string());
        args.push(key.clone());
        args.push(value.clone());
    }
}

fn validate_safe_path(path: &Path) -> Result<(), CcbdError> {
    let forbidden = ["/etc", "/root", "/proc", "/sys"];
    let path = path.to_str().ok_or_else(|| CcbdError::SandboxMountFailed {
        details: "host_path is not valid UTF-8".into(),
    })?;

    if forbidden
        .iter()
        .any(|prefix| path == *prefix || path.starts_with(&format!("{prefix}/")))
    {
        return Err(CcbdError::SandboxMountFailed {
            details: format!("forbidden path: {path}"),
        });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{RoBind, SandboxOverrides, build_args};
    use crate::error::CcbdError;
    use crate::provider::manifest::{
        IdleDetectionMode, InitProbeKind, ProviderManifest, get_manifest,
    };
    use std::path::PathBuf;
    use std::sync::{LazyLock, Mutex};

    static ENV_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

    fn args_for(overrides: SandboxOverrides) -> Vec<String> {
        build_args(
            PathBuf::from("/tmp/sandbox").as_path(),
            PathBuf::from("/tmp/project").as_path(),
            &overrides,
            None,
        )
        .unwrap()
    }

    #[test]
    fn test_build_args_default_baseline() {
        let args = args_for(SandboxOverrides::default());

        assert!(args.contains(&"--share-net".to_string()));
        assert!(!args.contains(&"--unshare-net".to_string()));
        assert!(args.contains(&"--bind".to_string()));
        assert!(args.contains(&"/tmp/project".to_string()));
        assert!(args.contains(&"--ro-bind".to_string()));
        assert!(args.contains(&"/usr".to_string()));
        assert!(args.contains(&"--ro-bind-try".to_string()));
        assert!(args.contains(&"/lib64".to_string()));
        assert!(
            args.windows(2)
                .any(|window| window == ["--chdir".to_string(), "/workspace".to_string()])
        );
        let workspace_bind = args
            .windows(3)
            .position(|window| {
                window
                    == [
                        "--bind".to_string(),
                        "/tmp/project".to_string(),
                        "/workspace".to_string(),
                    ]
            })
            .unwrap();
        let workspace_chdir = args
            .windows(2)
            .position(|window| window == ["--chdir".to_string(), "/workspace".to_string()])
            .unwrap();
        assert!(workspace_bind < workspace_chdir);
    }

    #[test]
    fn test_build_args_ro_binds_workspace_git_when_present() {
        let sandbox = tempfile::TempDir::new().unwrap();
        let project = tempfile::TempDir::new().unwrap();
        std::fs::create_dir(project.path().join(".git")).unwrap();

        let args = build_args(
            sandbox.path(),
            project.path(),
            &SandboxOverrides::default(),
            None,
        )
        .unwrap();

        let git_dir = project.path().join(".git").display().to_string();
        assert!(args.windows(3).any(|window| window
            == [
                "--ro-bind".to_string(),
                git_dir.clone(),
                "/workspace/.git".to_string()
            ]));
        let workspace_bind = args
            .windows(3)
            .position(|window| window[0] == "--bind" && window[2] == "/workspace")
            .unwrap();
        let git_ro_bind = args
            .windows(3)
            .position(|window| {
                window
                    == [
                        "--ro-bind".to_string(),
                        git_dir.clone(),
                        "/workspace/.git".to_string(),
                    ]
            })
            .unwrap();
        assert!(workspace_bind < git_ro_bind);
    }

    #[test]
    fn test_build_args_host_network() {
        let args = args_for(SandboxOverrides {
            network: Some("host".into()),
            extra_ro_binds: vec![],
        });

        assert!(args.contains(&"--share-net".to_string()));
        assert!(!args.contains(&"--unshare-net".to_string()));
    }

    #[test]
    fn test_build_args_includes_ssl_ca_bundle_paths() {
        let args = args_for(SandboxOverrides::default());

        for path in ["/etc/ssl", "/etc/ca-certificates", "/etc/pki"] {
            assert!(
                args.windows(3).any(|window| window
                    == [
                        "--ro-bind-try".to_string(),
                        path.to_string(),
                        path.to_string()
                    ]),
                "missing CA bundle bind for {path}: {args:?}"
            );
        }
    }

    #[test]
    fn test_build_args_can_override_to_isolated_network() {
        let args = args_for(SandboxOverrides {
            network: Some("none".into()),
            extra_ro_binds: vec![],
        });

        assert!(args.contains(&"--unshare-net".to_string()));
        assert!(!args.contains(&"--share-net".to_string()));
    }

    #[test]
    fn test_build_args_rejects_forbidden_extra_bind() {
        let err = build_args(
            PathBuf::from("/tmp/sandbox").as_path(),
            PathBuf::from("/tmp/project").as_path(),
            &SandboxOverrides {
                network: None,
                extra_ro_binds: vec![RoBind {
                    host_path: PathBuf::from("/etc/shadow"),
                    sandbox_path: PathBuf::from("/shadow"),
                }],
            },
            None,
        )
        .unwrap_err();

        assert!(matches!(err, CcbdError::SandboxMountFailed { .. }));
    }

    #[test]
    fn test_build_args_accepts_safe_extra_bind() {
        let args = args_for(SandboxOverrides {
            network: None,
            extra_ro_binds: vec![RoBind {
                host_path: PathBuf::from("/var/data"),
                sandbox_path: PathBuf::from("/data"),
            }],
        });

        assert!(args.windows(3).any(|window| window
            == [
                "--ro-bind".to_string(),
                "/var/data".to_string(),
                "/data".to_string()
            ]));
    }

    #[test]
    fn test_build_args_adds_existing_manifest_auth_mounts() {
        let _guard = ENV_LOCK.lock().unwrap();
        let home = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(home.path().join(".codex")).unwrap();
        std::fs::write(home.path().join(".codex").join("mock_token"), "token").unwrap();
        let old_home = std::env::var_os("HOME");
        unsafe {
            std::env::set_var("HOME", home.path());
        }

        let manifest = ProviderManifest {
            provider_name: "codex-auth-only",
            auth_mount_paths: vec![".codex"],
            command: &["codex"],
            env_passthrough: &[],
            injected_env_vars: &[],
            readiness_timeout_s: 1,
            requires_home_materialization: false,
            init_probe: InitProbeKind::Bash,
            idle_detection_mode: IdleDetectionMode::LineEndRegex,
            stability_ms: 0,
            idle_anti_pattern: "",
        };
        let args = build_args(
            PathBuf::from("/tmp/sandbox").as_path(),
            PathBuf::from("/tmp/project").as_path(),
            &SandboxOverrides::default(),
            Some(&manifest),
        )
        .unwrap();

        restore_home(old_home);
        let codex_path = home.path().join(".codex").display().to_string();
        assert!(args.windows(3).any(|window| window
            == [
                "--ro-bind".to_string(),
                codex_path.clone(),
                "/home/agent/.codex".to_string()
            ]));
    }

    #[test]
    fn test_build_args_maps_relative_manifest_auth_mount_to_sandbox_home() {
        let _guard = ENV_LOCK.lock().unwrap();
        let home = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(home.path().join(".codex")).unwrap();
        let old_home = std::env::var_os("HOME");
        unsafe {
            std::env::set_var("HOME", home.path());
        }
        let manifest = ProviderManifest {
            provider_name: "relative",
            auth_mount_paths: vec![".codex"],
            command: &["relative"],
            env_passthrough: &[],
            injected_env_vars: &[],
            readiness_timeout_s: 1,
            requires_home_materialization: false,
            init_probe: InitProbeKind::Bash,
            idle_detection_mode: IdleDetectionMode::LineEndRegex,
            stability_ms: 0,
            idle_anti_pattern: "",
        };

        let args = build_args(
            PathBuf::from("/tmp/sandbox").as_path(),
            PathBuf::from("/tmp/project").as_path(),
            &SandboxOverrides::default(),
            Some(&manifest),
        )
        .unwrap();

        restore_home(old_home);
        let host_path = home.path().join(".codex").display().to_string();
        assert!(args.windows(3).any(|window| window
            == [
                "--ro-bind".to_string(),
                host_path.clone(),
                "/home/agent/.codex".to_string()
            ]));
    }

    #[test]
    fn test_build_args_skips_missing_manifest_auth_mounts() {
        let _guard = ENV_LOCK.lock().unwrap();
        let home = tempfile::tempdir().unwrap();
        let old_home = std::env::var_os("HOME");
        unsafe {
            std::env::set_var("HOME", home.path());
        }

        let manifest = ProviderManifest {
            provider_name: "codex-auth-only",
            auth_mount_paths: vec![".codex"],
            command: &["codex"],
            env_passthrough: &[],
            injected_env_vars: &[],
            readiness_timeout_s: 1,
            requires_home_materialization: false,
            init_probe: InitProbeKind::Bash,
            idle_detection_mode: IdleDetectionMode::LineEndRegex,
            stability_ms: 0,
            idle_anti_pattern: "",
        };
        let args = build_args(
            PathBuf::from("/tmp/sandbox").as_path(),
            PathBuf::from("/tmp/project").as_path(),
            &SandboxOverrides::default(),
            Some(&manifest),
        )
        .unwrap();

        restore_home(old_home);
        assert!(!args.windows(3).any(|window| window[0] == "--ro-bind"
            && window[1].contains(".codex")
            && window[2].contains(".codex")));
    }

    #[test]
    fn test_build_args_binds_materialized_home_for_home_aware_manifest() {
        let _guard = ENV_LOCK.lock().unwrap();
        let home = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(home.path().join(".npm-global")).unwrap();
        std::fs::create_dir_all(home.path().join(".npm-global/bin")).unwrap();
        std::fs::create_dir_all(home.path().join(".local/bin")).unwrap();
        std::fs::create_dir_all(home.path().join(".local/share/claude")).unwrap();
        std::fs::create_dir_all(home.path().join(".codex")).unwrap();
        std::fs::create_dir_all(home.path().join(".gemini")).unwrap();
        std::fs::create_dir_all(home.path().join(".claude")).unwrap();
        std::fs::write(home.path().join(".claude.json"), "{}").unwrap();
        std::fs::write(home.path().join(".local/bin/claude"), "#!/bin/sh\n").unwrap();
        std::fs::write(home.path().join(".local/bin/ccb"), "#!/bin/sh\n").unwrap();
        std::fs::write(home.path().join(".npm-global/bin/codex"), "#!/bin/sh\n").unwrap();
        std::fs::write(home.path().join(".npm-global/bin/gemini"), "#!/bin/sh\n").unwrap();
        let cache = tempfile::tempdir().unwrap();
        let sandbox = tempfile::tempdir().unwrap();
        let old_home = std::env::var_os("HOME");
        let old_cache = std::env::var_os("XDG_CACHE_HOME");
        unsafe {
            std::env::set_var("HOME", home.path());
            std::env::set_var("XDG_CACHE_HOME", cache.path());
        }

        let args = build_args(
            sandbox.path(),
            PathBuf::from("/tmp/project").as_path(),
            &SandboxOverrides::default(),
            Some(&get_manifest("claude")),
        )
        .unwrap();

        restore_home(old_home);
        restore_xdg_cache_home(old_cache);
        assert!(args.windows(3).any(|window| {
            window[0] == "--bind"
                && window[1].contains("ccb-rs/sandboxes")
                && window[2] == "/home/agent"
        }));
        assert!(
            !args
                .windows(2)
                .any(|window| window == ["--dir".to_string(), "/home/agent".to_string()])
        );
        assert!(args.windows(3).any(|window| window
            == [
                "--setenv".to_string(),
                "CLAUDE_PROJECTS_ROOT".to_string(),
                "/home/agent/.claude/projects".to_string()
            ]));
        assert_ro_bind_try(
            &args,
            home.path().join(".local/bin/claude").display().to_string(),
            "/home/agent/.local/bin-agent/claude",
        );
        assert_ro_bind_try(
            &args,
            home.path()
                .join(".local/share/claude")
                .display()
                .to_string(),
            home.path()
                .join(".local/share/claude")
                .display()
                .to_string()
                .as_str(),
        );
        for relative in [".local/bin", ".codex", ".gemini", ".claude", ".claude.json"] {
            let path = home.path().join(relative).display().to_string();
            assert!(
                !args.windows(3).any(|window| window
                    == ["--ro-bind-try".to_string(), path.clone(), path.clone()]),
                "host {relative} should not be whole-path bound: {args:?}"
            );
        }
    }

    #[test]
    fn test_build_args_binds_codex_provider_entry_without_host_local_bin() {
        let _guard = ENV_LOCK.lock().unwrap();
        let home = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(home.path().join(".npm-global/bin")).unwrap();
        std::fs::create_dir_all(home.path().join(".local/bin")).unwrap();
        std::fs::write(home.path().join(".npm-global/bin/codex"), "#!/bin/sh\n").unwrap();
        std::fs::write(home.path().join(".local/bin/ccb"), "#!/bin/sh\n").unwrap();
        let sandbox = tempfile::tempdir().unwrap();
        let old_home = std::env::var_os("HOME");
        unsafe {
            std::env::set_var("HOME", home.path());
        }

        let args = build_args(
            sandbox.path(),
            PathBuf::from("/tmp/project").as_path(),
            &SandboxOverrides::default(),
            Some(&get_manifest("codex")),
        )
        .unwrap();

        restore_home(old_home);
        assert_ro_bind_try(
            &args,
            home.path().join(".npm-global").display().to_string(),
            "/home/agent/.npm-global",
        );
        assert_ro_bind_try(
            &args,
            home.path().join(".npm-global/bin/codex").display().to_string(),
            "/home/agent/.local/bin-agent/codex",
        );
        let local_bin = home.path().join(".local/bin").display().to_string();
        assert!(
            !args.windows(3).any(|window| window
                == [
                    "--ro-bind-try".to_string(),
                    local_bin.clone(),
                    local_bin.clone()
                ]),
            "codex must not expose host .local/bin: {args:?}"
        );
    }

    #[test]
    fn test_build_args_binds_gemini_provider_entry_without_host_local_bin() {
        let _guard = ENV_LOCK.lock().unwrap();
        let home = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(home.path().join(".npm-global/bin")).unwrap();
        std::fs::create_dir_all(home.path().join(".local/bin")).unwrap();
        std::fs::write(home.path().join(".npm-global/bin/gemini"), "#!/bin/sh\n").unwrap();
        std::fs::write(home.path().join(".local/bin/ask"), "#!/bin/sh\n").unwrap();
        let sandbox = tempfile::tempdir().unwrap();
        let old_home = std::env::var_os("HOME");
        unsafe {
            std::env::set_var("HOME", home.path());
        }

        let args = build_args(
            sandbox.path(),
            PathBuf::from("/tmp/project").as_path(),
            &SandboxOverrides::default(),
            Some(&get_manifest("gemini")),
        )
        .unwrap();

        restore_home(old_home);
        assert_ro_bind_try(
            &args,
            home.path().join(".npm-global").display().to_string(),
            "/home/agent/.npm-global",
        );
        assert_ro_bind_try(
            &args,
            home.path().join(".npm-global/bin/gemini").display().to_string(),
            "/home/agent/.local/bin-agent/gemini",
        );
        let local_bin = home.path().join(".local/bin").display().to_string();
        assert!(
            !args.windows(3).any(|window| window
                == [
                    "--ro-bind-try".to_string(),
                    local_bin.clone(),
                    local_bin.clone()
                ]),
            "gemini must not expose host .local/bin: {args:?}"
        );
    }

    #[test]
    fn test_provider_source_home_keeps_normal_home() {
        let env_home = PathBuf::from("/tmp/normal-home");
        let resolved = super::resolve_provider_source_home(
            env_home.clone(),
            Some(PathBuf::from("/home/user")),
        );

        assert_eq!(resolved, env_home);
    }

    #[test]
    fn test_provider_source_home_uses_passwd_home_from_nested_ccb_sandbox() {
        let env_home = PathBuf::from("/home/user/.cache/ccb/sandboxes/abc123");
        let passwd_home = PathBuf::from("/home/user");
        let resolved = super::resolve_provider_source_home(env_home, Some(passwd_home.clone()));

        assert_eq!(resolved, passwd_home);
    }

    #[test]
    fn test_build_args_rejects_forbidden_manifest_mount() {
        let manifest = ProviderManifest {
            provider_name: "bad",
            auth_mount_paths: vec!["/etc"],
            command: &["bad"],
            env_passthrough: &[],
            injected_env_vars: &[],
            readiness_timeout_s: 1,
            requires_home_materialization: false,
            init_probe: InitProbeKind::Bash,
            idle_detection_mode: IdleDetectionMode::LineEndRegex,
            stability_ms: 0,
            idle_anti_pattern: "",
        };
        let err = build_args(
            PathBuf::from("/tmp/sandbox").as_path(),
            PathBuf::from("/tmp/project").as_path(),
            &SandboxOverrides::default(),
            Some(&manifest),
        )
        .unwrap_err();

        assert!(matches!(err, CcbdError::SandboxMountFailed { .. }));
    }

    #[test]
    fn test_build_args_rejects_manifest_symlink_to_forbidden_path() {
        let _guard = ENV_LOCK.lock().unwrap();
        let home = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink("/etc", home.path().join(".codex")).unwrap();
        let old_home = std::env::var_os("HOME");
        unsafe {
            std::env::set_var("HOME", home.path());
        }

        let manifest = ProviderManifest {
            provider_name: "codex-auth-only",
            auth_mount_paths: vec![".codex"],
            command: &["codex"],
            env_passthrough: &[],
            injected_env_vars: &[],
            readiness_timeout_s: 1,
            requires_home_materialization: false,
            init_probe: InitProbeKind::Bash,
            idle_detection_mode: IdleDetectionMode::LineEndRegex,
            stability_ms: 0,
            idle_anti_pattern: "",
        };
        let err = build_args(
            PathBuf::from("/tmp/sandbox").as_path(),
            PathBuf::from("/tmp/project").as_path(),
            &SandboxOverrides::default(),
            Some(&manifest),
        )
        .unwrap_err();

        restore_home(old_home);
        assert!(matches!(err, CcbdError::SandboxMountFailed { .. }));
    }

    fn restore_home(old_home: Option<std::ffi::OsString>) {
        unsafe {
            if let Some(old_home) = old_home {
                std::env::set_var("HOME", old_home);
            } else {
                std::env::remove_var("HOME");
            }
        }
    }

    fn restore_xdg_cache_home(old_cache: Option<std::ffi::OsString>) {
        unsafe {
            if let Some(old_cache) = old_cache {
                std::env::set_var("XDG_CACHE_HOME", old_cache);
            } else {
                std::env::remove_var("XDG_CACHE_HOME");
            }
        }
    }

    fn assert_ro_bind_try(args: &[String], source: String, target: &str) {
        assert!(
            args.windows(3).any(|window| window
                == [
                    "--ro-bind-try".to_string(),
                    source.clone(),
                    target.to_string()
                ]),
            "missing ro-bind-try {source} -> {target}: {args:?}"
        );
    }
}
