//! macOS kqueue process watcher and registry.

use crate::error::CcbdError;
use std::collections::HashMap;
use std::mem;
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, FromRawFd, OwnedFd, RawFd};
use std::ptr;
use std::sync::{Arc, LazyLock, Mutex};

pub static PIDFD_REGISTRY: LazyLock<Arc<Mutex<HashMap<String, OwnedFd>>>> =
    LazyLock::new(|| Arc::new(Mutex::new(HashMap::new())));

static WATCH_IDENTITIES: LazyLock<Mutex<HashMap<RawFd, MacProcessWatch>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

#[derive(Debug)]
struct MacProcessWatch {
    identity: MacProcessIdentity,
    probe_fd: OwnedFd,
    exited: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MacProcessStartTime {
    pub sec: u64,
    pub usec: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MacProcessInfo {
    pub pid: i32,
    pub start_time: Option<MacProcessStartTime>,
    pub status: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MacProcessIdentity {
    pub pid: i32,
    pub generation: Option<i64>,
    pub start_time: Option<MacProcessStartTime>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RegisteredWatchIdentity {
    Unwatched,
    Matches,
    Mismatches,
}

pub fn pidfd_open(pid: i32) -> Result<OwnedFd, CcbdError> {
    let identity = capture_identity(pid)?;
    open_kqueue_for_identity(identity)
}

pub fn pidfd_send_sigkill(_pidfd: BorrowedFd<'_>) -> Result<(), CcbdError> {
    Err(CcbdError::EnvironmentNotSupported {
        details: "macOS: pidfd_send_signal is unsupported until PR-4 process reaper".to_string(),
    })
}

pub fn register(key: String, fd: OwnedFd) {
    match PIDFD_REGISTRY.lock() {
        Ok(mut registry) => {
            if let Some(old_fd) = registry.insert(key, fd) {
                unregister_identity(old_fd.as_raw_fd());
            }
        }
        Err(err) => {
            tracing::warn!(error = %err, "macOS pidfd registry mutex poisoned during register")
        }
    }
}

pub fn remove(key: &str) -> Option<OwnedFd> {
    match PIDFD_REGISTRY.lock() {
        Ok(mut registry) => {
            let fd = registry.remove(key)?;
            unregister_identity(fd.as_raw_fd());
            Some(fd)
        }
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

pub(crate) fn process_info(pid: i32) -> Option<MacProcessInfo> {
    // SAFETY: proc_bsdinfo is a plain output buffer. proc_pidinfo fills it when
    // the pid exists and the caller has sufficient visibility.
    let mut info: libc::proc_bsdinfo = unsafe { mem::zeroed() };
    let size = mem::size_of::<libc::proc_bsdinfo>();
    let result = unsafe {
        libc::proc_pidinfo(
            pid,
            libc::PROC_PIDTBSDINFO,
            0,
            (&mut info as *mut libc::proc_bsdinfo).cast(),
            size as libc::c_int,
        )
    };

    if result != size as libc::c_int {
        return None;
    }

    Some(MacProcessInfo {
        pid: info.pbi_pid as i32,
        start_time: Some(MacProcessStartTime {
            sec: info.pbi_start_tvsec,
            usec: info.pbi_start_tvusec,
        }),
        status: info.pbi_status,
    })
}

pub(crate) fn registered_watch_identity(
    pid: i32,
    current: Option<MacProcessInfo>,
) -> RegisteredWatchIdentity {
    let Ok(identities) = WATCH_IDENTITIES.lock() else {
        tracing::warn!("macOS watch identity mutex poisoned during identity check");
        return RegisteredWatchIdentity::Mismatches;
    };

    let watched = identities
        .values()
        .filter(|watch| watch.identity.pid == pid)
        .collect::<Vec<_>>();
    if watched.is_empty() {
        return RegisteredWatchIdentity::Unwatched;
    }

    if watched
        .into_iter()
        .any(|watch| process_info_matches_identity(current, &watch.identity))
    {
        RegisteredWatchIdentity::Matches
    } else {
        RegisteredWatchIdentity::Mismatches
    }
}

pub(crate) fn emit_liveness_diagnostic(
    label: &str,
    pid: i32,
    kill_result: i32,
    kill_errno: Option<i32>,
    current: Option<MacProcessInfo>,
    identity: RegisteredWatchIdentity,
    observed_exit: bool,
) {
    emit_watch_diagnostic(format_args!(
        "macOS liveness diagnostic label={label} pid={pid} kill_result={kill_result} kill_errno={kill_errno:?} current={current:?} identity={identity:?} observed_exit={observed_exit} registry={}",
        registry_snapshot(pid)
    ));
}

// TODO(PR-3 cleanup): round-8 diagnostics showed the probe kqueue is
// independently registered, but the final liveness table no longer depends on
// NOTE_EXIT probing. Keep this diagnostic path until the PR is green, then
// remove it in the cleanup commit instead of deepening the probe design here.
#[allow(dead_code)]
pub(crate) fn registered_watch_observed_exit(pid: i32, current: Option<MacProcessInfo>) -> bool {
    let Ok(mut watches) = WATCH_IDENTITIES.lock() else {
        tracing::warn!("macOS watch identity mutex poisoned during exit probe");
        emit_watch_diagnostic(format_args!(
            "macOS observed-exit diagnostic pid={pid} mutex_poisoned=true registry=<unavailable>"
        ));
        return false;
    };

    let matching_pid_count = watches
        .values()
        .filter(|watch| watch.identity.pid == pid)
        .count();
    emit_watch_diagnostic(format_args!(
        "macOS observed-exit diagnostic pid={pid} current={current:?} matching_pid_count={matching_pid_count} registry={}",
        registry_snapshot_locked(pid, &watches)
    ));

    let observed = watches
        .iter_mut()
        .filter(|(_, watch)| watch.identity.pid == pid)
        .any(|(primary_fd, watch)| {
            if !process_info_matches_identity(current, &watch.identity) {
                emit_watch_diagnostic(format_args!(
                    "macOS observed-exit diagnostic pid={pid} action=skip-identity-mismatch primary_fd={} probe_fd={} identity={:?} current={current:?} exited={}",
                    primary_fd,
                    watch.probe_fd.as_raw_fd(),
                    watch.identity,
                    watch.exited
                ));
                return false;
            }
            if watch.exited {
                emit_watch_diagnostic(format_args!(
                    "macOS observed-exit diagnostic pid={pid} action=cache-hit primary_fd={} probe_fd={} identity={:?}",
                    primary_fd,
                    watch.probe_fd.as_raw_fd(),
                    watch.identity
                ));
                return true;
            }
            if probe_note_exit(watch.probe_fd.as_raw_fd(), pid) {
                watch.exited = true;
                emit_watch_diagnostic(format_args!(
                    "macOS observed-exit diagnostic pid={pid} action=probe-hit primary_fd={} probe_fd={} identity={:?}",
                    primary_fd,
                    watch.probe_fd.as_raw_fd(),
                    watch.identity
                ));
                return true;
            }
            emit_watch_diagnostic(format_args!(
                "macOS observed-exit diagnostic pid={pid} action=probe-miss primary_fd={} probe_fd={} identity={:?}",
                primary_fd,
                watch.probe_fd.as_raw_fd(),
                watch.identity
            ));
            false
        });
    emit_watch_diagnostic(format_args!(
        "macOS observed-exit diagnostic pid={pid} observed={observed} registry_after={}",
        registry_snapshot_locked(pid, &watches)
    ));
    observed
}

fn capture_identity(pid: i32) -> Result<MacProcessIdentity, CcbdError> {
    let info = process_info(pid).ok_or_else(|| CcbdError::AgentUnexpectedExit {
        details: format!("pid {pid} is not alive"),
    })?;

    Ok(identity_from_process_info(info))
}

fn identity_from_process_info(info: MacProcessInfo) -> MacProcessIdentity {
    MacProcessIdentity {
        pid: info.pid,
        generation: None,
        start_time: info.start_time,
    }
}

fn open_kqueue_for_identity(identity: MacProcessIdentity) -> Result<OwnedFd, CcbdError> {
    if !identity_matches_current_process(&identity) {
        return Err(CcbdError::AgentUnexpectedExit {
            details: format!(
                "pid {} identity changed before watch registration",
                identity.pid
            ),
        });
    }

    open_kqueue_for_identity_unchecked(identity)
}

fn open_kqueue_for_identity_unchecked(identity: MacProcessIdentity) -> Result<OwnedFd, CcbdError> {
    let fd = open_proc_exit_kqueue(identity, "primary")?;
    let probe_fd = open_proc_exit_kqueue(identity, "probe")?;
    emit_watch_diagnostic(format_args!(
        "macOS kqueue registration diagnostic pid={} primary_fd={} probe_fd={} identity={:?}",
        identity.pid,
        fd.as_raw_fd(),
        probe_fd.as_raw_fd(),
        identity
    ));
    register_identity(fd.as_raw_fd(), identity, probe_fd);
    Ok(fd)
}

fn open_proc_exit_kqueue(identity: MacProcessIdentity, label: &str) -> Result<OwnedFd, CcbdError> {
    // SAFETY: kqueue returns a new file descriptor on success, or -1 with errno.
    let raw = unsafe { libc::kqueue() };
    if raw < 0 {
        let err = std::io::Error::last_os_error();
        return Err(CcbdError::SandboxMountFailed {
            details: format!("kqueue({label}) failed for pid {}: {err}", identity.pid),
        });
    }

    let event = proc_exit_event(identity.pid);
    // SAFETY: raw is a valid kqueue fd and event points to initialized kevent
    // storage. This call only registers the process-exit filter; the kqueue fd
    // becomes readable later when NOTE_EXIT fires and AsyncFd observes it.
    let result = unsafe { libc::kevent(raw, &event, 1, ptr::null_mut(), 0, ptr::null()) };
    if result < 0 {
        let err = std::io::Error::last_os_error();
        let errno = err.raw_os_error();
        // SAFETY: raw is owned here and must be closed on registration failure.
        unsafe {
            libc::close(raw);
        }
        if errno == Some(libc::ESRCH) {
            return Err(CcbdError::AgentUnexpectedExit {
                details: format!("pid {} is not alive", identity.pid),
            });
        }
        return Err(CcbdError::SandboxMountFailed {
            details: format!(
                "kevent(EVFILT_PROC NOTE_EXIT {label}) failed for pid {}: {err}",
                identity.pid
            ),
        });
    }

    // SAFETY: raw is a fresh kqueue descriptor and OwnedFd becomes its owner.
    Ok(unsafe { OwnedFd::from_raw_fd(raw) })
}

fn identity_matches_current_process(identity: &MacProcessIdentity) -> bool {
    process_info_matches_identity(process_info(identity.pid), identity)
}

fn process_info_matches_identity(
    info: Option<MacProcessInfo>,
    identity: &MacProcessIdentity,
) -> bool {
    let Some(info) = info else {
        return false;
    };
    info.pid == identity.pid && info.start_time == identity.start_time
}

fn register_identity(fd: RawFd, identity: MacProcessIdentity, probe_fd: OwnedFd) {
    match WATCH_IDENTITIES.lock() {
        Ok(mut identities) => {
            let probe_raw = probe_fd.as_raw_fd();
            identities.insert(
                fd,
                MacProcessWatch {
                    identity,
                    probe_fd,
                    exited: false,
                },
            );
            emit_watch_diagnostic(format_args!(
                "macOS registry diagnostic action=register pid={} primary_fd={fd} probe_fd={probe_raw} identity={identity:?} registry={}",
                identity.pid,
                registry_snapshot_locked(identity.pid, &identities)
            ));
        }
        Err(err) => {
            tracing::warn!(error = %err, "macOS watch identity mutex poisoned during register")
        }
    }
}

fn unregister_identity(fd: RawFd) {
    match WATCH_IDENTITIES.lock() {
        Ok(mut identities) => {
            let removed = identities.remove(&fd);
            emit_watch_diagnostic(format_args!(
                "macOS registry diagnostic action=unregister primary_fd={fd} removed_pid={:?}",
                removed.as_ref().map(|watch| watch.identity.pid)
            ));
        }
        Err(err) => {
            tracing::warn!(error = %err, "macOS watch identity mutex poisoned during unregister")
        }
    }
}

#[allow(dead_code)]
fn probe_note_exit(fd: RawFd, pid: i32) -> bool {
    const PROBE_ATTEMPTS: usize = 40;
    const PROBE_TIMEOUT_NS: libc::c_long = 50_000_000;

    let started = std::time::Instant::now();
    let mut last_result = 0;
    let mut last_errno = None;
    let mut last_hit_note_exit = false;

    for attempt in 1..=PROBE_ATTEMPTS {
        let mut event = zeroed_kevent();
        let timeout = libc::timespec {
            tv_sec: 0,
            tv_nsec: PROBE_TIMEOUT_NS,
        };
        // SAFETY: fd is a registered probe kqueue owned by the watch registry.
        // The short timeout gives NOTE_EXIT time to arrive while only consuming
        // the independent probe fd, never the production AsyncFd kqueue.
        let result = unsafe {
            libc::kevent(
                fd,
                ptr::null(),
                0,
                &mut event,
                1,
                &timeout as *const libc::timespec,
            )
        };
        last_result = result;
        let errno = if result < 0 {
            std::io::Error::last_os_error().raw_os_error()
        } else {
            None
        };
        let hit_note_exit = result > 0
            && event.filter == libc::EVFILT_PROC
            && (event.fflags & libc::NOTE_EXIT) == libc::NOTE_EXIT;
        emit_watch_diagnostic(format_args!(
            "macOS NOTE_EXIT probe kevent pid={pid} fd={fd} attempt={attempt} result={result} errno={errno:?} filter={} flags={} fflags={} hit_note_exit={hit_note_exit} elapsed_ms={}",
            event.filter,
            event.flags,
            event.fflags,
            started.elapsed().as_millis()
        ));
        if result < 0 {
            let err = std::io::Error::last_os_error();
            last_errno = err.raw_os_error();
            emit_probe_diagnostic(
                "failed",
                pid,
                attempt,
                result,
                last_errno,
                last_hit_note_exit,
                started.elapsed().as_millis(),
            );
            tracing::warn!(%pid, %err, attempt, "macOS NOTE_EXIT probe kevent failed");
            return false;
        }
        if result == 0 {
            continue;
        }

        last_hit_note_exit = hit_note_exit;
        if last_hit_note_exit {
            emit_probe_diagnostic(
                "hit",
                pid,
                attempt,
                result,
                last_errno,
                last_hit_note_exit,
                started.elapsed().as_millis(),
            );
            return true;
        }

        emit_probe_diagnostic(
            "unexpected-event",
            pid,
            attempt,
            result,
            last_errno,
            last_hit_note_exit,
            started.elapsed().as_millis(),
        );
        tracing::warn!(
            %pid,
            attempt,
            result,
            filter = event.filter,
            fflags = event.fflags,
            "macOS NOTE_EXIT probe received unexpected event"
        );
        return false;
    }

    emit_probe_diagnostic(
        "timed-out",
        pid,
        PROBE_ATTEMPTS,
        last_result,
        last_errno,
        last_hit_note_exit,
        started.elapsed().as_millis(),
    );
    tracing::warn!(
        %pid,
        attempts = PROBE_ATTEMPTS,
        last_result,
        ?last_errno,
        last_hit_note_exit,
        "macOS NOTE_EXIT probe timed out"
    );
    false
}

fn proc_exit_event(pid: i32) -> libc::kevent {
    let mut event = zeroed_kevent();
    event.ident = pid as libc::uintptr_t;
    event.filter = libc::EVFILT_PROC;
    event.flags = libc::EV_ADD | libc::EV_ENABLE | libc::EV_ONESHOT;
    event.fflags = libc::NOTE_EXIT;
    event
}

fn zeroed_kevent() -> libc::kevent {
    // SAFETY: kevent is a C POD struct where zeroed fields are valid defaults.
    unsafe { mem::zeroed() }
}

fn registry_snapshot(pid: i32) -> String {
    let Ok(watches) = WATCH_IDENTITIES.lock() else {
        return "mutex_poisoned=true".to_string();
    };
    registry_snapshot_locked(pid, &watches)
}

fn registry_snapshot_locked(pid: i32, watches: &HashMap<RawFd, MacProcessWatch>) -> String {
    let entries = watches
        .iter()
        .filter(|(_, watch)| watch.identity.pid == pid)
        .map(|(primary_fd, watch)| {
            format!(
                "{{primary_fd={primary_fd}, probe_fd={}, identity={:?}, exited={}}}",
                watch.probe_fd.as_raw_fd(),
                watch.identity,
                watch.exited
            )
        })
        .collect::<Vec<_>>();
    if entries.is_empty() {
        "registered=false entries=[]".to_string()
    } else {
        format!("registered=true entries=[{}]", entries.join(", "))
    }
}

#[cfg(all(test, target_os = "macos"))]
fn emit_watch_diagnostic(args: std::fmt::Arguments<'_>) {
    let message = format!("{args}\n");
    unsafe {
        libc::write(libc::STDERR_FILENO, message.as_ptr().cast(), message.len());
    }
}

#[cfg(not(all(test, target_os = "macos")))]
fn emit_watch_diagnostic(_args: std::fmt::Arguments<'_>) {}

#[cfg(all(test, target_os = "macos"))]
#[allow(dead_code)]
fn emit_probe_diagnostic(
    reason: &str,
    pid: i32,
    attempts: usize,
    result: i32,
    errno: Option<i32>,
    hit_note_exit: bool,
    elapsed_ms: u128,
) {
    let message = format!(
        "macOS NOTE_EXIT probe diagnostic reason={reason} pid={pid} attempts={attempts} result={result} errno={errno:?} hit_note_exit={hit_note_exit} elapsed_ms={elapsed_ms}\n"
    );
    // Bypass libtest's captured print macros so CI logs retain the diagnostic.
    unsafe {
        libc::write(libc::STDERR_FILENO, message.as_ptr().cast(), message.len());
    }
}

#[cfg(not(all(test, target_os = "macos")))]
#[allow(dead_code)]
fn emit_probe_diagnostic(
    _reason: &str,
    _pid: i32,
    _attempts: usize,
    _result: i32,
    _errno: Option<i32>,
    _hit_note_exit: bool,
    _elapsed_ms: u128,
) {
}

#[cfg(all(test, target_os = "macos"))]
fn pidfd_open_with_identity_for_test(identity: MacProcessIdentity) -> Result<OwnedFd, CcbdError> {
    open_kqueue_for_identity_unchecked(identity)
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::{capture_identity, pidfd_open, pidfd_open_with_identity_for_test};
    use crate::platform::macos::proc_info::{ProcessLiveness, kill_zero_check, waitid_exit_code};
    use std::fmt::Write as _;
    use std::os::fd::AsRawFd;
    use std::process::Command;
    use std::time::Duration;
    use tokio::io::{Interest, unix::AsyncFd};

    #[tokio::test]
    async fn kqueue_reports_external_death() {
        let mut child = Command::new("/bin/sleep").arg("30").spawn().unwrap();
        let pid = child.id() as i32;
        let fd = pidfd_open(pid).unwrap();
        let async_fd = AsyncFd::with_interest(fd, Interest::READABLE)
            .expect("AsyncFd::with_interest(kqueue fd, READABLE) failed in external death test");

        child.kill().unwrap();
        let _ = tokio::time::timeout(Duration::from_secs(5), async_fd.readable())
            .await
            .expect("kqueue exit event timed out")
            .expect("kqueue exit event failed");

        emit_proc_info_diagnostic("external_death_after_note_exit", pid);
        assert_eq!(kill_zero_check(pid), ProcessLiveness::Dead);
        let _ = child.wait();
    }

    #[tokio::test]
    async fn stale_identity_is_dead_for_supervised_instance() {
        let mut child = Command::new("/bin/sleep").arg("30").spawn().unwrap();
        let pid = child.id() as i32;
        let mut stale = capture_identity(pid).unwrap();
        stale.generation = Some(999);
        stale.start_time = stale.start_time.map(|start| super::MacProcessStartTime {
            sec: start.sec.saturating_add(1),
            usec: start.usec,
        });
        let fd = pidfd_open_with_identity_for_test(stale).unwrap();
        let async_fd = AsyncFd::with_interest(fd, Interest::READABLE)
            .expect("AsyncFd::with_interest(kqueue fd, READABLE) failed in stale identity test");

        assert_eq!(
            super::registered_watch_identity(pid, super::process_info(pid)),
            super::RegisteredWatchIdentity::Mismatches
        );
        // A mismatched registered identity means the supervised process
        // instance is gone under pidfd-style semantics, even if the numeric pid
        // currently names a live process. The PR-4 reaper owns the separate
        // kill-before-reap identity fence that prevents acting on reused pids.
        assert_eq!(kill_zero_check(pid), ProcessLiveness::Dead);

        child.kill().unwrap();
        let _ = tokio::time::timeout(Duration::from_secs(5), async_fd.readable())
            .await
            .expect("kqueue stale exit event timed out")
            .expect("kqueue stale exit event failed");

        emit_proc_info_diagnostic("stale_identity_after_note_exit", pid);
        assert_eq!(kill_zero_check(pid), ProcessLiveness::Dead);
        let _ = child.wait();
    }

    #[tokio::test]
    async fn kqueue_exit_code_remains_none_for_non_child_semantics() {
        let mut child = Command::new("/bin/sleep").arg("30").spawn().unwrap();
        let pid = child.id() as i32;
        let fd = pidfd_open(pid).unwrap();
        let raw = fd.as_raw_fd();
        let async_fd = AsyncFd::with_interest(fd, Interest::READABLE)
            .expect("AsyncFd::with_interest(kqueue fd, READABLE) failed in exit code test");

        child.kill().unwrap();
        let _ = tokio::time::timeout(Duration::from_secs(5), async_fd.readable())
            .await
            .expect("kqueue exit event timed out")
            .expect("kqueue exit event failed");

        assert_eq!(waitid_exit_code(raw), None);
        let _ = child.wait();
    }

    fn emit_proc_info_diagnostic(label: &str, pid: i32) {
        let mut message = format!(
            "macOS proc_pidinfo diagnostic label={label} pid={pid} SZOMB={}\n",
            libc::SZOMB
        );
        match super::process_info(pid) {
            Some(info) => {
                let _ = writeln!(
                    message,
                    "macOS proc_pidinfo result=Some pbi_pid={} status={} start_sec={} start_usec={}",
                    info.pid,
                    info.status,
                    info.start_time.map_or(0, |start| start.sec),
                    info.start_time.map_or(0, |start| start.usec)
                );
            }
            None => {
                let _ = writeln!(message, "macOS proc_pidinfo result=None");
            }
        }

        // Bypass libtest's captured print macros so CI logs retain the
        // diagnostic even when the test passes.
        unsafe {
            libc::write(libc::STDERR_FILENO, message.as_ptr().cast(), message.len());
        }
    }
}
