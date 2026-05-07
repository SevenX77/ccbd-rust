//! Sandbox environment checks and command assembly helpers for MVP2.

pub mod bwrap;
pub mod path;
pub mod systemd;

use crate::error::CcbdError;

/// Result of startup-time sandbox environment detection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvState {
    pub bwrap_available: bool,
    pub systemd_run_available: bool,
    pub unsafe_no_sandbox: bool,
    pub under_systemd: bool,
}

/// Check whether the host can run MVP2 sandboxed agents.
pub fn check_environment() -> Result<EnvState, CcbdError> {
    check_environment_with(
        std::env::var("CCBD_UNSAFE_NO_SANDBOX").as_deref() == Ok("1"),
        || which::which("bwrap").is_ok(),
        || which::which("systemd-run").is_ok(),
        std::env::var_os("INVOCATION_ID").is_some(),
    )
}

fn check_environment_with(
    unsafe_no_sandbox: bool,
    bwrap_probe: impl FnOnce() -> bool,
    systemd_run_probe: impl FnOnce() -> bool,
    under_systemd: bool,
) -> Result<EnvState, CcbdError> {
    let bwrap_available = bwrap_probe();
    let systemd_run_available = systemd_run_probe();

    if unsafe_no_sandbox {
        tracing::warn!("CCBD_UNSAFE_NO_SANDBOX=1 detected; running without sandbox isolation");
        return Ok(EnvState {
            bwrap_available,
            systemd_run_available,
            unsafe_no_sandbox,
            under_systemd,
        });
    }

    if !bwrap_available {
        return Err(CcbdError::SandboxBwrapNotFound);
    }
    if !systemd_run_available {
        return Err(CcbdError::EnvironmentNotSupported {
            details:
                "systemd-run not found in PATH; ccbd-rust requires Linux + systemd user session"
                    .into(),
        });
    }

    Ok(EnvState {
        bwrap_available,
        systemd_run_available,
        unsafe_no_sandbox,
        under_systemd,
    })
}

#[cfg(test)]
mod tests {
    use super::check_environment_with;
    use crate::error::CcbdError;

    #[test]
    fn test_check_environment_bypass_allows_missing_tools() {
        let state = check_environment_with(true, || false, || false, false).unwrap();

        assert!(state.unsafe_no_sandbox);
        assert!(!state.bwrap_available);
        assert!(!state.systemd_run_available);
    }

    #[test]
    fn test_check_environment_records_under_systemd() {
        let state = check_environment_with(false, || true, || true, true).unwrap();

        assert!(state.under_systemd);
    }

    #[test]
    fn test_check_environment_requires_bwrap_without_bypass() {
        let err = check_environment_with(false, || false, || true, false).unwrap_err();

        assert!(matches!(err, CcbdError::SandboxBwrapNotFound));
    }

    #[test]
    fn test_check_environment_requires_systemd_without_bypass() {
        let err = check_environment_with(false, || true, || false, false).unwrap_err();

        assert!(matches!(err, CcbdError::EnvironmentNotSupported { .. }));
    }
}
