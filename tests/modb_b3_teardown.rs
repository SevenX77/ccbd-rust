#![cfg(target_os = "linux")]
//! Module B / B3① — e2e teardown must reap the spawned `ahd` child even on the panic/failure
//! path.
//!
//! Root cause (C2 investigation, CONFIRMED): the e2e harness spawns `ahd` as a bare `Child` and
//! kills it only on the success path (e.g. `terminate_daemon(child)` at the end of a test). When an
//! assertion panics first, that kill is skipped and `ahd` escapes — real evidence showed leaked
//! `.../bin/ahd` processes surviving for days. Contract: teardown must be RAII/Drop-guaranteed so
//! the child is reaped on unwind.
//!
//! RED contract test authored by a5 (泳道2), harness owner. The fix lives in the RAII guard
//! `common::ReapOnDropDaemon::drop` (a seam stub today). Whoever implements it MUST NOT edit this
//! file's assertions.

mod common;

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

fn pid_alive(pid: i32) -> bool {
    unsafe { libc::kill(pid, 0) == 0 }
}

#[test]
fn ahd_child_is_reaped_when_test_body_panics() {
    let state_dir = tempfile::TempDir::new().unwrap();
    let pid_cell: Arc<Mutex<Option<i32>>> = Arc::new(Mutex::new(None));
    let pid_recorder = pid_cell.clone();

    // Model a real e2e body: spawn the daemon under a teardown guard, then hit a failing assertion
    // BEFORE any explicit cleanup. On unwind, the guard is dropped and must reap the child.
    let result = catch_unwind(AssertUnwindSafe(|| {
        let daemon = common::ReapOnDropDaemon::spawn(state_dir.path());
        *pid_recorder.lock().unwrap() = Some(daemon.pid());
        panic!("simulated mid-test failure before explicit teardown");
    }));
    assert!(result.is_err(), "the test body must have panicked");

    let pid = pid_cell
        .lock()
        .unwrap()
        .expect("daemon pid should have been recorded before the panic");

    // Give the Drop guard a moment to reap.
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline && pid_alive(pid) {
        std::thread::sleep(Duration::from_millis(50));
    }

    let leaked = pid_alive(pid);
    if leaked {
        // Safety net: reap the escaped child so a RED run never leaves a real `ahd` running.
        unsafe {
            libc::kill(pid, libc::SIGKILL);
        }
    }

    assert!(
        !leaked,
        "ahd child escaped the panic path — teardown must reap it via an RAII Drop guard (B3①)"
    );
}
