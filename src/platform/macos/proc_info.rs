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
        let info = process::process_info(pid);
        let identity = process::registered_watch_identity(pid, info);
        let liveness = match info {
            Some(info) if info.status == libc::SZOMB => ProcessLiveness::Dead,
            Some(_) => match identity {
                process::RegisteredWatchIdentity::Matches
                | process::RegisteredWatchIdentity::Unwatched => ProcessLiveness::Alive,
                process::RegisteredWatchIdentity::Mismatches => ProcessLiveness::Dead,
            },
            None => {
                // On macOS a supervised pid can still answer kill(pid, 0) after
                // exit while proc_pidinfo no longer returns BSD info. Treat that
                // as the watched process instance being gone.
                ProcessLiveness::Dead
            }
        };
        liveness
    } else {
        let errno = std::io::Error::last_os_error().raw_os_error();
        match errno {
            Some(libc::ESRCH) => ProcessLiveness::Dead,
            Some(libc::EPERM) => ProcessLiveness::Unknown,
            _ => ProcessLiveness::Unknown,
        }
    }
}

pub fn is_zombie_process(pid: i32) -> bool {
    let info = process::process_info(pid);
    !matches!(
        process::registered_watch_identity(pid, info),
        process::RegisteredWatchIdentity::Mismatches
    ) && info.is_some_and(|info| info.status == libc::SZOMB)
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
