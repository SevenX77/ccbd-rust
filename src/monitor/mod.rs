//! pidfd registry and Linux pidfd syscall helpers for MVP2 monitoring.

use crate::error::CcbdError;
use std::os::fd::{BorrowedFd, OwnedFd};

pub mod agent_watch;
pub mod master_watch;
pub mod session_watch;

/// Open a pidfd for a live process id.
pub fn pidfd_open(pid: i32) -> Result<OwnedFd, CcbdError> {
    crate::platform::sys::process::pidfd_open(pid)
}

/// Send SIGKILL through a pidfd.
pub fn pidfd_send_sigkill(pidfd: BorrowedFd<'_>) -> Result<(), CcbdError> {
    crate::platform::sys::process::pidfd_send_sigkill(pidfd)
}

/// Register or replace a pidfd for a key.
pub fn register(key: String, fd: OwnedFd) {
    crate::platform::sys::process::register(key, fd);
}

/// Remove a pidfd from the registry, transferring ownership to the caller.
pub fn remove(key: &str) -> Option<OwnedFd> {
    crate::platform::sys::process::remove(key)
}

/// Borrow a registered pidfd while the registry lock is held.
pub fn with_borrowed<R>(key: &str, f: impl FnOnce(BorrowedFd<'_>) -> R) -> Option<R> {
    crate::platform::sys::process::with_borrowed(key, f)
}

/// Return true when a key has a registered pidfd.
pub fn contains(key: &str) -> bool {
    crate::platform::sys::process::contains(key)
}

/// Return all registered monitor keys in stable order for diagnostics.
pub fn list_keys() -> Vec<String> {
    crate::platform::sys::process::list_keys()
}

#[cfg(test)]
mod tests {
    use super::{contains, pidfd_open, register, remove, with_borrowed};
    use crate::error::CcbdError;
    use std::os::fd::AsRawFd;

    #[test]
    fn test_pidfd_open_current_process() {
        let fd = pidfd_open(std::process::id() as i32).unwrap();
        assert!(fd.as_raw_fd() >= 0);
    }

    #[test]
    fn test_pidfd_open_dead_pid_maps_unexpected_exit() {
        let err = pidfd_open(999_999_999).unwrap_err();
        assert!(matches!(err, CcbdError::AgentUnexpectedExit { .. }));
    }

    #[test]
    fn test_registry_register_borrow_remove() {
        let key = format!("test:{}", uuid::Uuid::new_v4());
        let fd = pidfd_open(std::process::id() as i32).unwrap();
        register(key.clone(), fd);

        assert!(contains(&key));
        let raw = with_borrowed(&key, |borrowed| borrowed.as_raw_fd()).unwrap();
        assert!(raw >= 0);
        assert!(remove(&key).is_some());
        assert!(!contains(&key));
    }

    #[test]
    fn test_registry_replace_drops_old_fd() {
        let key = format!("test:{}", uuid::Uuid::new_v4());
        let old_fd = pidfd_open(std::process::id() as i32).unwrap();
        let old_raw = old_fd.as_raw_fd();
        let new_fd = pidfd_open(std::process::id() as i32).unwrap();

        register(key.clone(), old_fd);
        register(key.clone(), new_fd);

        // SAFETY: fcntl(F_GETFD) only inspects the numeric fd. EBADF confirms
        // the replaced OwnedFd was dropped and closed.
        let fcntl_result = unsafe { libc::fcntl(old_raw, libc::F_GETFD) };
        assert_eq!(fcntl_result, -1);

        let _ = remove(&key);
    }
}
