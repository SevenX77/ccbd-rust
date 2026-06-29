use crate::cli::rpc_client::CliError;
use crate::cli::service_unit::{
    ServiceUnitError, atomic_write_unit, derive_unit_name, render_unit_file,
    resolve_user_systemd_dir_from_env,
};
use std::fmt;
use std::fs;
use std::io;
use std::path::Path;
use std::process::{Command, Output};

pub trait SystemctlRunner {
    fn run(&self, args: &[&str]) -> io::Result<Output>;
}

#[derive(Debug, Default)]
pub struct RealSystemctlRunner;

impl SystemctlRunner for RealSystemctlRunner {
    fn run(&self, args: &[&str]) -> io::Result<Output> {
        Command::new("systemctl").arg("--user").args(args).output()
    }
}

#[derive(Debug)]
pub enum PersistentBootstrapError {
    Recoverable(String),
    Fatal(String),
}

impl PersistentBootstrapError {
    pub fn is_fatal(&self) -> bool {
        matches!(self, Self::Fatal(_))
    }

    pub fn is_recoverable(&self) -> bool {
        matches!(self, Self::Recoverable(_))
    }

    pub fn into_cli_error(self) -> CliError {
        CliError::Config(self.to_string())
    }
}

impl fmt::Display for PersistentBootstrapError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Recoverable(message) | Self::Fatal(message) => f.write_str(message),
        }
    }
}

impl From<ServiceUnitError> for PersistentBootstrapError {
    fn from(err: ServiceUnitError) -> Self {
        Self::Recoverable(format!("failed to render ahd systemd user unit: {err}"))
    }
}

pub fn user_manager_available_from(
    xdg_runtime_dir: Option<&str>,
    probe_output: Option<&str>,
) -> bool {
    if xdg_runtime_dir.is_none_or(|value| value.is_empty()) {
        return false;
    }
    matches!(
        probe_output.map(str::trim),
        Some("running" | "degraded" | "starting" | "maintenance" | "initializing")
    )
}

pub fn systemd_user_bootstrap_available() -> bool {
    let xdg_runtime_dir = std::env::var("XDG_RUNTIME_DIR").ok();
    let output = Command::new("systemctl")
        .args(["--user", "is-system-running"])
        .output();
    let Ok(output) = output else {
        return false;
    };
    let stdout = String::from_utf8_lossy(&output.stdout);
    user_manager_available_from(xdg_runtime_dir.as_deref(), Some(stdout.trim()))
}

pub fn collect_passthrough_env() -> Vec<(String, String)> {
    crate::provider::manifest::ENV_PASSTHROUGH
        .iter()
        .filter_map(|key| {
            std::env::var(key)
                .ok()
                .map(|value| ((*key).to_string(), value))
        })
        .collect()
}

pub fn bootstrap_persistent_unit(
    runner: &impl SystemctlRunner,
    ahd_bin: &Path,
    state_dir: &Path,
    env: &[(String, String)],
    socket_accepting: bool,
) -> Result<String, PersistentBootstrapError> {
    let systemd_dir =
        resolve_user_systemd_dir_from_env().map_err(PersistentBootstrapError::from)?;
    bootstrap_persistent_unit_in_dir(
        runner,
        &systemd_dir,
        ahd_bin,
        state_dir,
        env,
        socket_accepting,
    )
}

pub fn bootstrap_persistent_unit_in_dir(
    runner: &impl SystemctlRunner,
    systemd_dir: &Path,
    ahd_bin: &Path,
    state_dir: &Path,
    env: &[(String, String)],
    socket_accepting: bool,
) -> Result<String, PersistentBootstrapError> {
    let unit_name = derive_unit_name(state_dir);
    let unit_path = systemd_dir.join(&unit_name);
    let desired = render_unit_file(&unit_name, ahd_bin, state_dir, env)?;
    let existing = fs::read_to_string(&unit_path).ok();
    let changed = existing.as_deref() != Some(desired.as_str());

    if changed {
        atomic_write_unit(&unit_path, &desired).map_err(|err| {
            PersistentBootstrapError::Recoverable(format!(
                "failed to write ahd systemd user unit {}: {err}",
                unit_path.display()
            ))
        })?;
    }

    run_required(runner, &["daemon-reload"], FailureKind::Recoverable)?;

    run_reset_failed_best_effort(runner, &unit_name);
    run_required(runner, &["enable", &unit_name], FailureKind::Fatal)?;

    // The current caller reaches this path only after the socket is not accepting.
    // Restart handles both inactive units and active-but-hung daemons.
    let action = if socket_accepting && !changed {
        "start"
    } else {
        "restart"
    };
    run_required(runner, &[action, &unit_name], FailureKind::Fatal)?;

    Ok(unit_name)
}

pub fn linger_note_for_user(user: &str, output: &str) -> Option<String> {
    let linger = output
        .lines()
        .find_map(|line| line.strip_prefix("Linger="))
        .map(str::trim)?;
    if linger == "no" {
        Some(format!(
            "note: ahd user service is enabled for the user manager; boot-before-login/logout survival requires: loginctl enable-linger {user}"
        ))
    } else {
        None
    }
}

pub fn detect_linger_note() -> Option<String> {
    let user = std::env::var("USER")
        .ok()
        .filter(|value| !value.is_empty())?;
    let output = Command::new("loginctl")
        .args(["show-user", &user, "-p", "Linger"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    linger_note_for_user(&user, &stdout)
}

#[derive(Clone, Copy)]
enum FailureKind {
    Recoverable,
    Fatal,
}

fn run_required(
    runner: &impl SystemctlRunner,
    args: &[&str],
    failure_kind: FailureKind,
) -> Result<(), PersistentBootstrapError> {
    let output = runner.run(args).map_err(|err| {
        classified_error(
            failure_kind,
            format!(
                "systemctl --user {} failed to execute: {err}",
                args.join(" ")
            ),
        )
    })?;
    if output.status.success() {
        return Ok(());
    }
    Err(classified_error(
        failure_kind,
        format!(
            "systemctl --user {} failed: status={}, stderr={}",
            args.join(" "),
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ),
    ))
}

fn run_reset_failed_best_effort(runner: &impl SystemctlRunner, unit_name: &str) {
    match runner.run(&["reset-failed", unit_name]) {
        Ok(output) if output.status.success() => {}
        Ok(output) => {
            tracing::warn!(
                unit = unit_name,
                status = ?output.status,
                stderr = %String::from_utf8_lossy(&output.stderr).trim(),
                "systemctl reset-failed failed before ahd bootstrap"
            );
        }
        Err(err) => {
            tracing::warn!(
                unit = unit_name,
                error = %err,
                "systemctl reset-failed failed before ahd bootstrap"
            );
        }
    }
}

fn classified_error(kind: FailureKind, message: String) -> PersistentBootstrapError {
    match kind {
        FailureKind::Recoverable => PersistentBootstrapError::Recoverable(message),
        FailureKind::Fatal => PersistentBootstrapError::Fatal(message),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::fs;
    use std::os::unix::process::ExitStatusExt;
    use std::path::PathBuf;
    use std::process::{ExitStatus, Output};

    #[derive(Debug)]
    struct FakeRunner {
        calls: RefCell<Vec<Vec<String>>>,
        failures: Vec<Vec<String>>,
    }

    impl FakeRunner {
        fn new() -> Self {
            Self {
                calls: RefCell::new(Vec::new()),
                failures: Vec::new(),
            }
        }

        fn failing(args: &[&str]) -> Self {
            Self {
                calls: RefCell::new(Vec::new()),
                failures: vec![args.iter().map(|arg| (*arg).to_string()).collect()],
            }
        }

        fn calls(&self) -> Vec<Vec<String>> {
            self.calls.borrow().clone()
        }
    }

    impl SystemctlRunner for FakeRunner {
        fn run(&self, args: &[&str]) -> std::io::Result<Output> {
            let call = args
                .iter()
                .map(|arg| (*arg).to_string())
                .collect::<Vec<_>>();
            self.calls.borrow_mut().push(call.clone());
            let success = !self.failures.iter().any(|failure| failure == &call);
            Ok(output(if success { 0 } else { 1 }, ""))
        }
    }

    fn output(code: i32, stdout: &str) -> Output {
        Output {
            status: ExitStatus::from_raw(code),
            stdout: stdout.as_bytes().to_vec(),
            stderr: Vec::new(),
        }
    }

    fn setup() -> (tempfile::TempDir, PathBuf, PathBuf, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let systemd_dir = tmp.path().join("systemd/user");
        let state_dir = tmp.path().join("state");
        let ahd_bin = tmp.path().join("bin/ahd");
        fs::create_dir_all(ahd_bin.parent().unwrap()).unwrap();
        fs::write(&ahd_bin, "").unwrap();
        fs::create_dir_all(&state_dir).unwrap();
        (tmp, systemd_dir, state_dir, ahd_bin)
    }

    #[test]
    fn service_bootstrap_user_manager_probe_accepts_real_manager_states() {
        for state in [
            "running",
            "degraded",
            "starting",
            "maintenance",
            "initializing",
        ] {
            assert!(user_manager_available_from(
                Some("/run/user/1000"),
                Some(state)
            ));
        }
    }

    #[test]
    fn service_bootstrap_user_manager_probe_rejects_offline_missing_runtime_or_failed_probe() {
        assert!(!user_manager_available_from(
            Some("/run/user/1000"),
            Some("offline")
        ));
        assert!(!user_manager_available_from(None, Some("running")));
        assert!(!user_manager_available_from(Some(""), Some("running")));
        assert!(!user_manager_available_from(Some("/run/user/1000"), None));
    }

    #[test]
    fn service_bootstrap_linger_note_parses_linger_status() {
        assert!(linger_note_for_user("alice", "Linger=no").is_some());
        assert!(linger_note_for_user("alice", "Linger=yes").is_none());
        assert!(linger_note_for_user("alice", "").is_none());
    }

    #[test]
    fn service_bootstrap_fresh_install_reloads_enables_and_restarts() {
        let (_tmp, systemd_dir, state_dir, ahd_bin) = setup();
        let runner = FakeRunner::new();

        let unit = bootstrap_persistent_unit_in_dir(
            &runner,
            &systemd_dir,
            &ahd_bin,
            &state_dir,
            &[],
            false,
        )
        .unwrap();

        assert!(unit.starts_with("ah-"));
        assert!(systemd_dir.join(&unit).exists());
        assert_eq!(
            runner.calls(),
            vec![
                vec!["daemon-reload"],
                vec!["reset-failed", unit.as_str()],
                vec!["enable", unit.as_str()],
                vec!["restart", unit.as_str()],
            ]
        );
    }

    #[test]
    fn service_bootstrap_idempotent_install_does_not_rewrite_but_enables_and_starts() {
        let (_tmp, systemd_dir, state_dir, ahd_bin) = setup();
        let runner = FakeRunner::new();
        let unit = bootstrap_persistent_unit_in_dir(
            &runner,
            &systemd_dir,
            &ahd_bin,
            &state_dir,
            &[],
            false,
        )
        .unwrap();
        let unit_path = systemd_dir.join(&unit);
        let before = fs::metadata(&unit_path).unwrap().modified().unwrap();

        let runner = FakeRunner::new();
        let second = bootstrap_persistent_unit_in_dir(
            &runner,
            &systemd_dir,
            &ahd_bin,
            &state_dir,
            &[],
            false,
        )
        .unwrap();

        assert_eq!(unit, second);
        assert_eq!(
            fs::metadata(&unit_path).unwrap().modified().unwrap(),
            before
        );
        assert_eq!(
            runner.calls(),
            vec![
                vec!["daemon-reload"],
                vec!["reset-failed", unit.as_str()],
                vec!["enable", unit.as_str()],
                vec!["restart", unit.as_str()],
            ]
        );
    }

    #[test]
    fn service_bootstrap_changed_content_reloads_and_restarts() {
        let (_tmp, systemd_dir, state_dir, ahd_bin) = setup();
        let unit = crate::cli::service_unit::derive_unit_name(&state_dir);
        fs::create_dir_all(&systemd_dir).unwrap();
        fs::write(systemd_dir.join(&unit), "old content").unwrap();
        let runner = FakeRunner::new();

        bootstrap_persistent_unit_in_dir(&runner, &systemd_dir, &ahd_bin, &state_dir, &[], false)
            .unwrap();

        assert_eq!(
            runner.calls(),
            vec![
                vec!["daemon-reload"],
                vec!["reset-failed", unit.as_str()],
                vec!["enable", unit.as_str()],
                vec!["restart", unit.as_str()],
            ]
        );
    }

    #[test]
    fn service_bootstrap_reset_failed_failure_is_ignored() {
        let (_tmp, systemd_dir, state_dir, ahd_bin) = setup();
        let unit = crate::cli::service_unit::derive_unit_name(&state_dir);
        let runner = FakeRunner::failing(&["reset-failed", &unit]);

        bootstrap_persistent_unit_in_dir(&runner, &systemd_dir, &ahd_bin, &state_dir, &[], false)
            .unwrap();

        assert!(
            runner
                .calls()
                .contains(&vec!["enable".into(), unit.clone()])
        );
        assert!(runner.calls().contains(&vec!["restart".into(), unit]));
    }

    #[test]
    fn service_bootstrap_daemon_reload_failure_is_recoverable() {
        let (_tmp, systemd_dir, state_dir, ahd_bin) = setup();
        let runner = FakeRunner::failing(&["daemon-reload"]);

        let err = bootstrap_persistent_unit_in_dir(
            &runner,
            &systemd_dir,
            &ahd_bin,
            &state_dir,
            &[],
            false,
        )
        .unwrap_err();

        assert!(err.is_recoverable());
    }

    #[test]
    fn service_bootstrap_enable_and_restart_failures_are_fatal() {
        let (_tmp, systemd_dir, state_dir, ahd_bin) = setup();
        let unit = crate::cli::service_unit::derive_unit_name(&state_dir);

        let enable_runner = FakeRunner::failing(&["enable", &unit]);
        assert!(
            bootstrap_persistent_unit_in_dir(
                &enable_runner,
                &systemd_dir,
                &ahd_bin,
                &state_dir,
                &[],
                false,
            )
            .unwrap_err()
            .is_fatal()
        );

        let (_tmp, systemd_dir, state_dir, ahd_bin) = setup();
        let unit = crate::cli::service_unit::derive_unit_name(&state_dir);
        let start_runner = FakeRunner::failing(&["restart", &unit]);
        assert!(
            bootstrap_persistent_unit_in_dir(
                &start_runner,
                &systemd_dir,
                &ahd_bin,
                &state_dir,
                &[],
                false,
            )
            .unwrap_err()
            .is_fatal()
        );
    }
}
