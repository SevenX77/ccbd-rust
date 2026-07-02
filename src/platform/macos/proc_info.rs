//! macOS process liveness compile skeleton.

use crate::platform::macos::process;
use std::os::fd::AsRawFd;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessLiveness {
    Alive,
    Dead,
    Unknown,
}

pub fn kill_zero_check(pid: i32) -> ProcessLiveness {
    let result = unsafe { libc::kill(pid, 0) };
    if result == 0 {
        if !process::identity_matches_registered_watch(pid) {
            return ProcessLiveness::Alive;
        }
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
    process::process_info(pid).is_some_and(|info| {
        process::identity_matches_registered_watch(pid) && info.status == libc::SZOMB
    })
}

pub fn proc_state(pid: i32) -> Option<u8> {
    process::process_info(pid).map(|info| {
        if info.status == libc::SZOMB {
            b'Z'
        } else {
            b'R'
        }
    })
}

pub fn waitid_exit_code(_pidfd_raw: i32) -> Option<i32> {
    tracing::debug!("macOS kqueue process watcher does not synthesize exit codes");
    None
}

pub fn raw_fd<T: AsRawFd>(fd: &T) -> i32 {
    fd.as_raw_fd()
}
