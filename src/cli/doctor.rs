use crate::cli::rpc_client::{CliError, RpcClient};
use serde_json::json;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DoctorStatus {
    Pass,
    Fail,
    Warn,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DoctorCheck {
    pub name: String,
    pub status: DoctorStatus,
    pub detail: String,
    pub suggestion: Option<String>,
}

pub async fn run_doctor(client: &impl RpcClient, cwd: &Path) -> Result<Vec<DoctorCheck>, CliError> {
    let mut checks = Vec::new();
    checks.extend(system_binary_checks());
    checks.push(daemon_check(client).await);
    checks.extend(provider_health_checks(home_dir()));
    checks.extend(permission_checks(cwd));
    Ok(checks)
}

pub fn system_binary_checks() -> Vec<DoctorCheck> {
    ["tmux", "bwrap", "systemd-run"]
        .into_iter()
        .map(|binary| match which::which(binary) {
            Ok(path) => pass(format!("binary:{binary}"), path.display().to_string()),
            Err(_) => fail(
                format!("binary:{binary}"),
                "not found in PATH",
                format!("install {binary} or adjust PATH"),
            ),
        })
        .collect()
}

async fn daemon_check(client: &impl RpcClient) -> DoctorCheck {
    match tokio::time::timeout(
        std::time::Duration::from_secs(5),
        client.call("system.dump", json!({})),
    )
    .await
    {
        Ok(Ok(_)) => pass("daemon", "ccbd responded to system.dump"),
        Ok(Err(err)) => fail(
            "daemon",
            err.to_string(),
            "start it with scripts/start-daemon.sh",
        ),
        Err(_) => fail(
            "daemon",
            "timed out after 5s",
            "check ccbd logs and socket path",
        ),
    }
}

pub fn provider_health_checks(home: Option<PathBuf>) -> Vec<DoctorCheck> {
    let Some(home) = home else {
        return vec![warn(
            "provider:home",
            "home directory unavailable",
            "set HOME before running provider checks",
        )];
    };
    [
        ("provider:codex", home.join(".codex").join("auth.json")),
        ("provider:gemini", home.join(".config").join("gemini")),
        ("provider:anthropic", home.join(".anthropic")),
        ("provider:claude", home.join(".claude")),
    ]
    .into_iter()
    .map(|(name, path)| {
        if path.exists() {
            pass(name, path.display().to_string())
        } else {
            warn(
                name,
                format!("{} not found", path.display()),
                "provider may need login before use",
            )
        }
    })
    .collect()
}

pub fn permission_checks(cwd: &Path) -> Vec<DoctorCheck> {
    let mut checks = Vec::new();
    checks.push(check_read_write("permissions:cwd", cwd));
    let ccb_dir = cwd.join(".ccb");
    if ccb_dir.exists() {
        checks.push(check_read_write("permissions:.ccb", &ccb_dir));
    } else {
        checks.push(warn(
            "permissions:.ccb",
            ".ccb directory not present",
            "it will be created when daemon state is initialized",
        ));
    }
    checks
}

pub fn print_doctor(checks: &[DoctorCheck]) {
    for check in checks {
        let (mark, color) = match check.status {
            DoctorStatus::Pass => ("✓", "\x1b[32m"),
            DoctorStatus::Fail => ("✗", "\x1b[31m"),
            DoctorStatus::Warn => ("!", "\x1b[33m"),
        };
        println!("{color}{mark}\x1b[0m {} - {}", check.name, check.detail);
        if let Some(suggestion) = &check.suggestion {
            println!("  suggestion: {suggestion}");
        }
    }
}

pub fn has_failures(checks: &[DoctorCheck]) -> bool {
    checks
        .iter()
        .any(|check| check.status == DoctorStatus::Fail)
}

fn check_read_write(name: &'static str, path: &Path) -> DoctorCheck {
    if !path.exists() {
        return fail(
            name,
            "path does not exist",
            "create the path or choose another cwd",
        );
    }
    let readable = std::fs::read_dir(path).is_ok();
    let probe = path.join(".ccb-doctor-write-test");
    let writable = std::fs::write(&probe, b"ok")
        .and_then(|_| std::fs::remove_file(&probe))
        .is_ok();
    match (readable, writable) {
        (true, true) => pass(name, path.display().to_string()),
        (false, true) => fail(name, "not readable", "fix directory read permissions"),
        (true, false) => fail(name, "not writable", "fix directory write permissions"),
        (false, false) => fail(
            name,
            "not readable or writable",
            "fix directory permissions",
        ),
    }
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

fn pass(name: impl Into<String>, detail: impl Into<String>) -> DoctorCheck {
    DoctorCheck {
        name: name.into(),
        status: DoctorStatus::Pass,
        detail: detail.into(),
        suggestion: None,
    }
}

fn fail(
    name: impl Into<String>,
    detail: impl Into<String>,
    suggestion: impl Into<String>,
) -> DoctorCheck {
    DoctorCheck {
        name: name.into(),
        status: DoctorStatus::Fail,
        detail: detail.into(),
        suggestion: Some(suggestion.into()),
    }
}

fn warn(
    name: impl Into<String>,
    detail: impl Into<String>,
    suggestion: impl Into<String>,
) -> DoctorCheck {
    DoctorCheck {
        name: name.into(),
        status: DoctorStatus::Warn,
        detail: detail.into(),
        suggestion: Some(suggestion.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::{DoctorStatus, permission_checks, provider_health_checks};

    #[test]
    fn test_provider_health_missing_home_warns() {
        let checks = provider_health_checks(None);
        assert_eq!(checks[0].status, DoctorStatus::Warn);
    }

    #[test]
    fn test_permission_checks_current_tempdir_passes() {
        let tmp = tempfile::TempDir::new().unwrap();
        std::fs::create_dir(tmp.path().join(".ccb")).unwrap();
        let checks = permission_checks(tmp.path());
        assert!(
            checks
                .iter()
                .all(|check| check.status == DoctorStatus::Pass)
        );
    }
}
