use crate::cli::rpc_client::{CliError, RpcClient};
use serde_json::json;
use std::path::{Path, PathBuf};
use std::process::Command;

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
    let unsafe_no_sandbox = std::env::var("CCBD_UNSAFE_NO_SANDBOX").as_deref() == Ok("1");
    checks.extend(system_binary_checks(unsafe_no_sandbox));
    checks.push(daemon_check(client).await);
    checks.push(check_tmux_orphans());
    checks.extend(provider_health_checks(home_dir()));
    checks.extend(permission_checks(cwd));
    Ok(checks)
}

pub fn system_binary_checks(unsafe_no_sandbox: bool) -> Vec<DoctorCheck> {
    let binaries = if unsafe_no_sandbox {
        vec!["tmux", "systemd-run"]
    } else {
        vec!["tmux", "bwrap", "systemd-run"]
    };
    binaries
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

fn check_tmux_orphans() -> DoctorCheck {
    let socket_dir = format!("/tmp/tmux-{}", unsafe { libc::geteuid() });
    let mut alive = 0;
    let mut stale = 0;
    let mut timeouts = 0;

    if let Ok(entries) = std::fs::read_dir(socket_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if !name.starts_with("ccbd-") {
                continue;
            }

            match tmux_ls_sessions(&name) {
                TmuxLsSessionsResult::Alive => alive += 1,
                TmuxLsSessionsResult::Stale => stale += 1,
                TmuxLsSessionsResult::Timeout => {
                    stale += 1;
                    timeouts += 1;
                }
            }
        }
    }

    if stale == 0 {
        pass(
            "tmux server orphans",
            format!("0 stale ({alive} alive, timeouts: {timeouts})"),
        )
    } else {
        warn(
            "tmux server orphans",
            format!(
                "WARN {stale} stale ({alive} alive, timeouts: {timeouts}), run scripts/cleanup_orphan_tmux.sh"
            ),
            "run scripts/cleanup_orphan_tmux.sh",
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TmuxLsSessionsResult {
    Alive,
    Stale,
    Timeout,
}

fn tmux_ls_sessions(socket_name: &str) -> TmuxLsSessionsResult {
    let output = Command::new("timeout")
        .args(["2s", "tmux", "-L", socket_name, "ls-sessions"])
        .output();
    match output {
        Ok(output) if output.status.success() => TmuxLsSessionsResult::Alive,
        Ok(output) if output.status.code() == Some(124) => TmuxLsSessionsResult::Timeout,
        Ok(output) if String::from_utf8_lossy(&output.stderr).contains("unknown command") => {
            let output = Command::new("timeout")
                .args(["2s", "tmux", "-L", socket_name, "list-sessions"])
                .output();
            match output {
                Ok(output) if output.status.success() => TmuxLsSessionsResult::Alive,
                Ok(output) if output.status.code() == Some(124) => TmuxLsSessionsResult::Timeout,
                _ => TmuxLsSessionsResult::Stale,
            }
        }
        _ => TmuxLsSessionsResult::Stale,
    }
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
    use super::{DoctorStatus, permission_checks, provider_health_checks, system_binary_checks};

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

    #[test]
    fn test_system_binary_checks_skip_bwrap_when_sandbox_disabled() {
        let checks = system_binary_checks(true);
        assert!(!checks.iter().any(|check| check.name == "binary:bwrap"));
    }
}
