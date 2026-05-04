//! Bubblewrap argument construction for MVP2 sandboxed agents.

use crate::error::CcbdError;
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
    overrides: &SandboxOverrides,
    manifest: Option<&ProviderManifest>,
) -> Result<Vec<String>, CcbdError> {
    let mut args = vec![
        "--unshare-pid".to_string(),
        "--unshare-uts".to_string(),
        "--unshare-ipc".to_string(),
    ];

    if overrides.network.as_deref() == Some("host") {
        args.push("--share-net".to_string());
    } else {
        args.push("--unshare-net".to_string());
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
    push_value(&mut args, "--dir", "/home/agent");
    args.push("--setenv".to_string());
    args.push("HOME".to_string());
    args.push("/home/agent".to_string());
    args.push("--bind".to_string());
    args.push(sandbox_dir.display().to_string());
    args.push("/workspace".to_string());

    for bind in &overrides.extra_ro_binds {
        validate_safe_path(&bind.host_path)?;
        args.push("--ro-bind".to_string());
        args.push(bind.host_path.display().to_string());
        args.push(bind.sandbox_path.display().to_string());
    }

    if let Some(manifest) = manifest {
        push_manifest_auth_mounts(&mut args, manifest)?;
        push_manifest_env(&mut args, manifest);
    }

    Ok(args)
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
    use crate::provider::manifest::{IdleDetectionMode, ProviderManifest, get_manifest};
    use std::path::PathBuf;

    fn args_for(overrides: SandboxOverrides) -> Vec<String> {
        build_args(PathBuf::from("/tmp/sandbox").as_path(), &overrides, None).unwrap()
    }

    #[test]
    fn test_build_args_default_baseline() {
        let args = args_for(SandboxOverrides::default());

        assert!(args.contains(&"--unshare-net".to_string()));
        assert!(args.contains(&"--bind".to_string()));
        assert!(args.contains(&"/tmp/sandbox".to_string()));
        assert!(args.contains(&"--ro-bind".to_string()));
        assert!(args.contains(&"/usr".to_string()));
        assert!(args.contains(&"--ro-bind-try".to_string()));
        assert!(args.contains(&"/lib64".to_string()));
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
    fn test_build_args_rejects_forbidden_extra_bind() {
        let err = build_args(
            PathBuf::from("/tmp/sandbox").as_path(),
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
        let home = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(home.path().join(".codex")).unwrap();
        std::fs::write(home.path().join(".codex").join("mock_token"), "token").unwrap();
        let old_home = std::env::var_os("HOME");
        unsafe {
            std::env::set_var("HOME", home.path());
        }

        let args = build_args(
            PathBuf::from("/tmp/sandbox").as_path(),
            &SandboxOverrides::default(),
            Some(&get_manifest("codex")),
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
            startup_sequence: &[],
            interactive_prompt_handlers: &[],
            idle_detection_mode: IdleDetectionMode::LineEndRegex,
            marker_pattern: r"$",
            stability_ms: 0,
        };

        let args = build_args(
            PathBuf::from("/tmp/sandbox").as_path(),
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
        let home = tempfile::tempdir().unwrap();
        let old_home = std::env::var_os("HOME");
        unsafe {
            std::env::set_var("HOME", home.path());
        }

        let args = build_args(
            PathBuf::from("/tmp/sandbox").as_path(),
            &SandboxOverrides::default(),
            Some(&get_manifest("codex")),
        )
        .unwrap();

        restore_home(old_home);
        assert!(!args.windows(3).any(|window| window[0] == "--ro-bind"
            && window[1].contains(".codex")
            && window[2].contains(".codex")));
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
            startup_sequence: &[],
            interactive_prompt_handlers: &[],
            idle_detection_mode: IdleDetectionMode::LineEndRegex,
            marker_pattern: r"$",
            stability_ms: 0,
        };
        let err = build_args(
            PathBuf::from("/tmp/sandbox").as_path(),
            &SandboxOverrides::default(),
            Some(&manifest),
        )
        .unwrap_err();

        assert!(matches!(err, CcbdError::SandboxMountFailed { .. }));
    }

    #[test]
    fn test_build_args_rejects_manifest_symlink_to_forbidden_path() {
        let home = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink("/etc", home.path().join(".codex")).unwrap();
        let old_home = std::env::var_os("HOME");
        unsafe {
            std::env::set_var("HOME", home.path());
        }

        let err = build_args(
            PathBuf::from("/tmp/sandbox").as_path(),
            &SandboxOverrides::default(),
            Some(&get_manifest("codex")),
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
}
