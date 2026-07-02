//! macOS process watcher/reaper compile skeleton.

use crate::error::CcbdError;
use std::collections::HashMap;
use std::fs::File;
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::sync::{Arc, LazyLock, Mutex};

pub static PIDFD_REGISTRY: LazyLock<Arc<Mutex<HashMap<String, OwnedFd>>>> =
    LazyLock::new(|| Arc::new(Mutex::new(HashMap::new())));

pub fn pidfd_open(pid: i32) -> Result<OwnedFd, CcbdError> {
    Err(CcbdError::EnvironmentNotSupported {
        details: format!("macOS: pidfd_open({pid}) is unsupported until PR-3 kqueue watcher"),
    })
}

pub fn pidfd_send_sigkill(_pidfd: BorrowedFd<'_>) -> Result<(), CcbdError> {
    Err(CcbdError::EnvironmentNotSupported {
        details: "macOS: pidfd_send_signal is unsupported until PR-3/PR-4 process reaper"
            .to_string(),
    })
}

pub fn register(key: String, fd: OwnedFd) {
    match PIDFD_REGISTRY.lock() {
        Ok(mut registry) => {
            registry.insert(key, fd);
        }
        Err(err) => {
            tracing::warn!(error = %err, "macOS pidfd registry mutex poisoned during register")
        }
    }
}

pub fn remove(key: &str) -> Option<OwnedFd> {
    match PIDFD_REGISTRY.lock() {
        Ok(mut registry) => registry.remove(key),
        Err(err) => {
            tracing::warn!(error = %err, "macOS pidfd registry mutex poisoned during remove");
            None
        }
    }
}

pub fn with_borrowed<R>(key: &str, f: impl FnOnce(BorrowedFd<'_>) -> R) -> Option<R> {
    match PIDFD_REGISTRY.lock() {
        Ok(registry) => registry.get(key).map(|fd| f(fd.as_fd())),
        Err(err) => {
            tracing::warn!(error = %err, "macOS pidfd registry mutex poisoned during borrow");
            None
        }
    }
}

pub fn contains(key: &str) -> bool {
    match PIDFD_REGISTRY.lock() {
        Ok(registry) => registry.contains_key(key),
        Err(err) => {
            tracing::warn!(error = %err, "macOS pidfd registry mutex poisoned during contains");
            false
        }
    }
}

pub fn list_keys() -> Vec<String> {
    match PIDFD_REGISTRY.lock() {
        Ok(registry) => {
            let mut keys = registry.keys().cloned().collect::<Vec<_>>();
            keys.sort();
            keys
        }
        Err(err) => {
            tracing::warn!(error = %err, "macOS pidfd registry mutex poisoned during list_keys");
            Vec::new()
        }
    }
}

pub fn placeholder_fd() -> Result<OwnedFd, CcbdError> {
    File::open("/dev/null")
        .map(OwnedFd::from)
        .map_err(|err| CcbdError::PtyIoError(format!("macOS placeholder fd open failed: {err}")))
}
