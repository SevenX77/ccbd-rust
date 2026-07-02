//! macOS process liveness compile skeleton.

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
        return ProcessLiveness::Alive;
    }

    match std::io::Error::last_os_error().raw_os_error() {
        Some(libc::ESRCH) => ProcessLiveness::Dead,
        Some(libc::EPERM) => ProcessLiveness::Unknown,
        _ => ProcessLiveness::Unknown,
    }
}

pub fn is_zombie_process(_pid: i32) -> bool {
    false
}

pub fn proc_state(_pid: i32) -> Option<u8> {
    None
}

pub fn waitid_exit_code(_pidfd_raw: i32) -> Option<i32> {
    None
}

pub fn raw_fd<T: AsRawFd>(fd: &T) -> i32 {
    fd.as_raw_fd()
}
