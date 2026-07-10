#![cfg(target_os = "linux")]
//! Module B / B1 — daemon identity must come from explicit injection, not `/proc/self/cgroup`
//! sniffing.
//!
//! Root cause (C2 incident backing): `detect_current_service_unit()` sniffs the ambient
//! `/proc/self/cgroup`, so a test subprocess launched inside (or inheriting the cgroup of) a live
//! stack can misidentify itself as that live unit, and teardown then kills the live stack's cgroup.
//! Contract: when an identity is explicitly injected, `detect_current_service_unit()` returns the
//! injected value and does NOT fall back to cgroup sniffing.
//!
//! RED contract test authored by a5 (泳道2). a2 implements the fix to make it green and MUST NOT
//! edit this file.
//!
//! SEAM pinned by a5 (subject to master's confirmation): the injected identity is read from the
//! environment variable `AH_SERVICE_UNIT`. If master prefers a threaded parameter or a different
//! variable name, this single constant is the contract to renegotiate — a2 cannot touch tests, so
//! the name must be blessed before implementation.

use std::ffi::OsString;

const AH_SERVICE_UNIT: &str = "AH_SERVICE_UNIT";

/// Sets an env var for the lifetime of the guard and restores the prior value on drop.
struct EnvVarGuard {
    key: &'static str,
    prev: Option<OsString>,
}

impl EnvVarGuard {
    fn set(key: &'static str, value: &str) -> Self {
        let prev = std::env::var_os(key);
        // SAFETY: the modB acceptance suite runs single-threaded (`--test-threads=1`) and no other
        // test reads AH_SERVICE_UNIT; the guard restores the prior value on drop.
        unsafe {
            std::env::set_var(key, value);
        }
        Self { key, prev }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        unsafe {
            match &self.prev {
                Some(value) => std::env::set_var(self.key, value),
                None => std::env::remove_var(self.key),
            }
        }
    }
}

#[test]
fn injected_service_unit_wins_over_cgroup_sniffing() {
    // A value the ambient `/proc/self/cgroup` of the cargo test runner could never yield, but which
    // still satisfies `is_daemon_service_unit` (`ah-*.service`) so a validating implementation also
    // accepts it. If detection reads the injection, it returns exactly this; if it sniffs cgroups,
    // it cannot produce this synthetic unit.
    let injected = "ah-injected-b1-contract.service";
    let _guard = EnvVarGuard::set(AH_SERVICE_UNIT, injected);

    let detected = ah::systemd_unit::detect_current_service_unit();

    assert_eq!(
        detected.as_deref(),
        Some(injected),
        "with AH_SERVICE_UNIT injected, identity must use the injected value and must NOT sniff \
         /proc/self/cgroup (RED: current code always sniffs the cgroup and ignores the injection)"
    );
}
