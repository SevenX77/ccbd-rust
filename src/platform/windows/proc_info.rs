//! Windows process liveness stubs for the M0 compile gate.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessLiveness {
    Alive,
    Dead,
    Unknown,
}

pub fn kill_zero_check(_pid: i32) -> ProcessLiveness {
    ProcessLiveness::Unknown
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

pub fn raw_fd<T>(_fd: &T) -> i32 {
    -1
}
