//! Linux `/proc` and process liveness helpers.

use std::os::fd::AsRawFd;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessLiveness {
    Alive,
    Dead,
    Unknown,
}

pub fn kill_zero_check(pid: i32) -> ProcessLiveness {
    // SAFETY: kill(pid, 0) does not send a signal; it only checks process existence.
    let result = unsafe { libc::kill(pid, 0) };
    if result == 0 {
        if is_zombie_process(pid) {
            return ProcessLiveness::Dead;
        }
        return ProcessLiveness::Alive;
    }

    match std::io::Error::last_os_error().raw_os_error() {
        Some(libc::ESRCH) => ProcessLiveness::Dead,
        Some(libc::EPERM) => ProcessLiveness::Unknown,
        _ => ProcessLiveness::Unknown,
    }
}

pub fn is_zombie_process(pid: i32) -> bool {
    proc_state(pid).is_some_and(|state| state == b'Z')
}

pub fn proc_state(pid: i32) -> Option<u8> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let state = stat.rsplit_once(") ")?.1.as_bytes().first().copied()?;
    Some(state)
}

pub fn waitid_exit_code(pidfd_raw: i32) -> Option<i32> {
    // SAFETY: siginfo_t is a plain C output buffer and zero-initialization is
    // valid before passing it to waitid.
    let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
    // SAFETY: pidfd_raw comes from a live AsyncFd<OwnedFd>. waitid with P_PIDFD
    // only writes to `info` and does not take ownership of the fd.
    let result = unsafe {
        libc::waitid(
            libc::P_PIDFD,
            pidfd_raw as libc::id_t,
            &mut info,
            libc::WEXITED | libc::WNOHANG,
        )
    };

    if result == 0 {
        // SAFETY: waitid returned success and initialized siginfo_t; reading
        // si_status is the libc accessor for the exited child status.
        Some(unsafe { info.si_status() })
    } else {
        let err = std::io::Error::last_os_error();
        if err.raw_os_error() != Some(libc::ECHILD) {
            tracing::warn!(error = %err, "waitid(P_PIDFD) failed");
        } else {
            tracing::debug!("waitid(P_PIDFD) unavailable for non-child agent process");
        }
        None
    }
}

pub fn raw_fd<T: AsRawFd>(fd: &T) -> i32 {
    fd.as_raw_fd()
}
