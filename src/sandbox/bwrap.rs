//! Bubblewrap argument construction for MVP2 sandboxed agents.

use crate::error::CcbdError;
use serde::Deserialize;
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

fn validate_safe_path(path: &Path) -> Result<(), CcbdError> {
    let forbidden = ["/etc/", "/root", "/proc", "/sys"];
    let path = path.to_str().ok_or_else(|| CcbdError::SandboxMountFailed {
        details: "host_path is not valid UTF-8".into(),
    })?;

    if forbidden.iter().any(|prefix| path.starts_with(prefix)) {
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
    use std::path::PathBuf;

    fn args_for(overrides: SandboxOverrides) -> Vec<String> {
        build_args(PathBuf::from("/tmp/sandbox").as_path(), &overrides).unwrap()
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
}
