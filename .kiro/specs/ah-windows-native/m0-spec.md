# Windows Native M0 Implementation Spec: Compile Gate

## 1. Goal

M0 makes Windows target compilation real. The milestone is complete only when the root package passes:

```bash
cargo check --all-targets --target x86_64-pc-windows-msvc
```

on `windows-latest` CI. M0 is an internal engineering milestone. It does not deliver a user-usable native Windows workflow; it creates the compile boundary that M1 and M2 can safely build on. The estimate is roughly 1-2 weeks because M0 includes two real abstraction boundaries, IPC and process-monitor handles, plus broad cfg cleanup across source and integration tests.

The work order is:

1. Add Rust targets and root Windows production dependencies.
2. Add `src/platform/windows/*` modules with signature-complete stubs.
3. Replace direct Unix-only compile surfaces with cfg gates or small abstractions.
4. Make tests and benches pulled by `--all-targets` cfg-correct.
5. Add a Windows compile CI job.

## 2. Task M0-1: Rust Targets

**Scope**

- Install the primary Windows target used by CI and release validation:

```bash
rustup target add x86_64-pc-windows-msvc
```

- Install the GNU target as the local developer check target on Linux hosts:

```bash
rustup target add x86_64-pc-windows-gnu
```

**Files**

- No repository file needs to record the rustup installation.
- CI records only `x86_64-pc-windows-msvc` for the required M0 gate.
- The MSVC target gate is CI-only for most Linux developers because `rusqlite` with `bundled` requires the MSVC toolchain and `cl.exe` when checking that target. The GNU target is useful locally because MinGW cross tooling can compile the bundled C dependency from Linux, but GNU is not the MVP release ABI.

**Exit Criteria**

- `rustup target list --installed` includes `x86_64-pc-windows-msvc`.
- Local Linux developers can use `x86_64-pc-windows-gnu` for earlier local type-check feedback where MinGW is installed, but CI failure or release blocking is tied only to MSVC.

## 3. Task M0-2: Root Windows Production Dependencies

**Scope**

Add a root package Windows dependency block in `Cargo.toml`. This is not the disposable spike subcrate; it is for the production `ah` package.

Current state:

- Linux has `rusqlite = { version = "0.32", features = ["bundled"] }`.
- macOS has `rusqlite = "0.32"`.
- `nix = { version = "0.28", features = ["fs"] }` is currently in unconditional `[dependencies]` even though `nix` is Unix-only. Windows check will fail while building dependencies even if every `nix::` use site is cfg-gated.
- Windows has no root production block, so root Windows compilation cannot resolve platform persistence code once `cfg(windows)` is enabled.

Move `nix` out of unconditional dependencies:

```toml
[target.'cfg(unix)'.dependencies]
nix = { version = "0.28", features = ["fs"] }
```

Add the Windows production block:

```toml
[target.'cfg(windows)'.dependencies]
rusqlite = { version = "0.32", features = ["bundled"] }
windows-sys = { version = "0.61.2", features = [
  "Win32_Foundation",
  "Win32_Security",
  "Win32_System_Threading",
  "Win32_System_JobObjects",
  "Win32_System_Com",
  "Win32_System_TaskScheduler",
  "Win32_System_Console",
  "Win32_Storage_FileSystem",
] }
```

Do not add `portable-pty` or `alacritty_terminal` to the root package in M0. This intentionally narrows the earlier design shorthand that listed the spike PTY/grid crates in the M0 dependency block: M0 needs root Windows compile dependencies only, while the M0.5 spike remains isolated under `tests/windows_conpty_spike/`. Root PTY dependencies become production dependencies in M2 when the Windows multiplexer is introduced.

**Files**

- `Cargo.toml`
- `Cargo.lock`

**Exit Criteria**

- `cargo check --target x86_64-pc-windows-msvc` resolves dependencies.
- `cargo tree --target x86_64-pc-windows-msvc -i nix` reports no path from the root package.
- Linux `cargo test --test mvp6_acceptance test_no_legacy_pty_dependency` still passes after the M0 dependency change.

## 4. Task M0-3: Platform Module Skeleton

**Scope**

Add Windows platform modules that match the current static free-function call surface. Do not rely on the thin traits in `src/platform/mod.rs`; production code calls `crate::platform::sys::*` wrappers and concrete modules.

Add to `src/platform/mod.rs`:

```rust
#[cfg(windows)]
pub mod windows;

#[cfg(windows)]
pub use windows as sys;
```

Create:

- `src/platform/windows/mod.rs`
- `src/platform/windows/process.rs`
- `src/platform/windows/proc_info.rs`
- `src/platform/windows/scope.rs`
- `src/platform/windows/service.rs`
- `src/platform/windows/identity.rs`

M0 may use `unimplemented!()` or explicit `CcbdError::EnvironmentNotSupported` for behavior. Signatures and exported names must compile.

### 4.1. `process.rs` Signature Surface

The Linux/macOS surface currently used through `src/monitor/mod.rs` is:

- `pidfd_open(pid: i32) -> Result<MonitorHandle, CcbdError>`
- `MonitorHandle::try_clone(&self) -> std::io::Result<MonitorHandle>`
- `pidfd_send_sigkill(handle: BorrowedMonitorHandle<'_>) -> Result<(), CcbdError>`
- `register(key: String, handle: MonitorHandle)`
- `remove(key: &str) -> Option<MonitorHandle>`
- `with_borrowed<R>(key: &str, f: impl FnOnce(BorrowedMonitorHandle<'_>) -> R) -> Option<R>`
- `contains(key: &str) -> bool`
- `list_keys() -> Vec<String>`

M0 should first introduce platform-neutral monitor handle aliases in `src/monitor/mod.rs` rather than exposing `std::os::fd::OwnedFd/BorrowedFd` from the wrapper. On Unix these aliases map to `OwnedFd/BorrowedFd`; on Windows they map to a stub handle type in `platform/windows/process.rs`. The clone method is part of the compile surface because watcher call sites register one handle and pass a cloned handle to the async watch task.

Recommended M0 type sketch:

```rust
// src/platform/linux/process.rs and src/platform/macos/process.rs
pub type MonitorHandle = std::os::fd::OwnedFd;
pub type BorrowedMonitorHandle<'a> = std::os::fd::BorrowedFd<'a>;

// src/platform/windows/process.rs
#[derive(Debug)]
pub struct MonitorHandle {
    // M0 may use a placeholder; M1 replaces this with an owned process HANDLE.
    raw: windows_sys::Win32::Foundation::HANDLE,
}

#[derive(Clone, Copy)]
pub struct BorrowedMonitorHandle<'a> {
    raw: windows_sys::Win32::Foundation::HANDLE,
    _lifetime: std::marker::PhantomData<&'a MonitorHandle>,
}

impl MonitorHandle {
    pub fn try_clone(&self) -> std::io::Result<Self> {
        // M0 can return Unsupported or duplicate a placeholder; M1 uses DuplicateHandle.
        unimplemented!()
    }

    pub fn borrowed(&self) -> BorrowedMonitorHandle<'_> {
        BorrowedMonitorHandle {
            raw: self.raw,
            _lifetime: std::marker::PhantomData,
        }
    }
}
```

The wrapper shape then becomes:

```rust
pub use crate::platform::sys::process::{BorrowedMonitorHandle, MonitorHandle};

pub fn with_borrowed<R>(
    key: &str,
    f: impl FnOnce(BorrowedMonitorHandle<'_>) -> R,
) -> Option<R> {
    crate::platform::sys::process::with_borrowed(key, f)
}
```

M1 will replace the Windows stub handle with a retained process `HANDLE`.

### 4.2. `proc_info.rs` Signature Surface

Provide:

- `pub enum ProcessLiveness { Alive, Dead, Unknown }`
- `kill_zero_check(pid: i32) -> ProcessLiveness`
- `is_zombie_process(pid: i32) -> bool`
- `proc_state(pid: i32) -> Option<u8>`
- `waitid_exit_code(pidfd_raw: i32) -> Option<i32>`
- `raw_fd<T>(fd: &T) -> i32` only if still required by call sites after monitor handle abstraction.

M0 stubs should compile and return conservative values. M1 maps liveness to Windows process handles and exit-code APIs.

### 4.3. `scope.rs` Signature Surface

Provide the command-construction and cleanup surface used by `src/tmux/scope.rs`, `src/sandbox/systemd.rs`, startup reconcile, and tests:

- `RecoverySpawn { pub is_recovery: bool, pub args: Vec<String> }`. The struct literal is constructed by `src/rpc/handlers/agent.rs`.
- `ScopeUnit { pub(crate) unit: String, pub(crate) description: String }`. `src/db/system.rs` reads both fields during scope cleanup, so Windows stubs must expose the same fields to the crate.
- `SystemctlRunner` with the same method signatures as Linux:
  - `fn list_scope_units(&self) -> Result<Vec<ScopeUnit>, std::io::Error>`
  - `fn stop_unit(&self, unit: &str) -> Result<(), std::io::Error>`
- `RealSystemctlRunner`
- `parse_systemctl_scope_units`
- `is_own_ccbd_scope`
- `is_orphan_scope`
- `wrap_in_scope`
- `unit_name_for_socket`
- `detect_scope_policy`
- `detect_scope_policy_with_daemon_unit`
- `wrap_command`
- `wrap_command_with_recovery`
- `wrap_command_with_recovery_and_sandbox_overrides`
- `master_command`
- `master_command_with_env`

M0 keeps existing return types: `wrap_command*` and `master_command*` return `Vec<String>`, and `wrap_in_scope` returns `std::process::Command`. M0 must not invent a Job Object spawn-plan type. Per design, Job Object assignment belongs to the M2 multiplexer spawn layer because the existing signatures cannot carry a job handle or callback.

### 4.4. `service.rs` Signature Surface

Provide all names used by `src/cli/service_unit.rs`, `src/cli/service_bootstrap.rs`, and `src/bin/ah.rs`:

- `ServiceUnitError`
- `derive_unit_name`
- `build_ahd_systemd_run_command`
- `build_ahd_systemd_run_command_with_env`
- `ahd_reset_failed_is_best_effort`
- `escape_systemd_env_value`
- `escape_systemd_exec_token`
- `render_unit_file`
- `resolve_user_systemd_dir`
- `resolve_user_systemd_dir_from_env`
- `atomic_write_unit`

The three names required by the design review MAJOR-A are mandatory for M0 compilation:

- `ServiceUnitError`
- `escape_systemd_env_value`
- `escape_systemd_exec_token`

On Windows the escape helpers may be no-ops or Task Scheduler argument/env escaping equivalents in M0. The names must exist.

### 4.5. `identity.rs` Signature Surface

Provide:

- `detect_current_service_unit`
- `detect_current_service_unit_from_cgroup`
- `is_daemon_service_unit`
- `unescape_systemd_unit_segment`

M0 may return `None` for Windows daemon identity until Task Scheduler support arrives in M1.

**Exit Criteria**

- The Windows modules compile when selected by `#[cfg(windows)]`.
- No production call site tries to import `crate::platform::linux::*` directly on Windows.

## 5. Task M0-4: Unix-Only Compile Surface Cleanup

This is the core M0 task. The following grep-derived list contains the non-cfg Unix-only surfaces that must be gated or abstracted for the Windows compile gate.

### 5.1. Source File Cleanup List

| # | Location | Unix-only surface | M0 strategy |
| --- | --- | --- | --- |
| 1 | `Cargo.toml:37` | unconditional `nix = { version = "0.28", features = ["fs"] }` | Move `nix` to `[target.'cfg(unix)'.dependencies]`; otherwise Windows check fails while compiling dependencies before source cfg can help. |
| 2 | `src/rpc/mod.rs:5` | `std::os::unix::net::UnixStream` for stale socket probing | Move RPC transport behind a listener abstraction. Unix backend keeps UDS; Windows backend stubs Named Pipe listener enough to compile or returns unsupported before use. |
| 3 | `src/rpc/mod.rs:9` | `tokio::net::UnixListener` return type | Replace `bind_rpc_listener` return type with platform listener enum/type alias; `run_server` must not name `UnixListener` on Windows. |
| 4 | `src/rpc/mod.rs:30` | Unix socket permission chmod via `PermissionsExt` | Gate with `#[cfg(unix)]`; no-op on Windows Named Pipe path. |
| 5 | `src/cli/rpc_client.rs:6` | `std::os::unix::net::UnixStream` import | Introduce a platform RPC client transport. Unix backend uses `UnixStream`; Windows backend compiles against Named Pipe/stub transport. |
| 6 | `src/cli/rpc_client.rs:120` | `UnixStream::connect` in request RPC | Route through platform RPC client transport. |
| 7 | `src/cli/rpc_client.rs:164` | `UnixStream::connect` in streaming/event RPC | Route through platform RPC client transport. |
| 8 | `src/monitor/mod.rs:4` | `OwnedFd/BorrowedFd` in public monitor wrapper | Introduce `MonitorHandle` and `BorrowedMonitorHandle` aliases/types from `platform::sys::process`. |
| 9 | `src/monitor/mod.rs:11` | `pidfd_open` returns `OwnedFd` | Return platform-neutral `MonitorHandle`. |
| 10 | `src/monitor/mod.rs:16` | `pidfd_send_sigkill` takes `BorrowedFd` | Take `BorrowedMonitorHandle<'_>` or a closure-compatible platform borrow wrapper. |
| 11 | `src/monitor/mod.rs:21-31` | registry functions expose `OwnedFd/BorrowedFd` | Update wrapper signatures to platform-neutral handle types. |
| 12 | `src/monitor/mod.rs:49,88` | unit test imports `AsRawFd` and calls `libc::fcntl` | Gate pidfd registry tests with `#[cfg(unix)]` or add Windows-specific monitor handle tests later. |
| 13 | `src/monitor/agent_watch.rs:5` | `OwnedFd` pidfd argument | Use platform-neutral monitor handle or gate Unix implementation and provide Windows stub. |
| 14 | `src/monitor/agent_watch.rs:7,13` | `tokio::io::unix::AsyncFd` | Move readiness waiting behind monitor-watch abstraction. Windows M0 can stub; M1 uses process wait handles. |
| 15 | `src/monitor/agent_watch.rs:97` | `waitid_exit_code(pidfd_raw)` path | Gate Unix implementation; Windows exit status comes from process handle in M1. |
| 16 | `src/monitor/master_watch.rs:31` | `OwnedFd` master pidfd | Same monitor handle abstraction as agent watcher. |
| 17 | `src/monitor/master_watch.rs:37,430` | `tokio::io::unix::AsyncFd` for pidfd readiness | Move behind monitor-watch abstraction or `#[cfg(unix)]` implementation with Windows compile stub. |
| 18 | `src/monitor/master_watch.rs:217-221` | `pidfd.try_clone()` then `register` and spawn master watcher | Preserve `MonitorHandle::try_clone()` or add a wrapper such as `monitor::clone_handle_for_watch`; update call site to platform-neutral handle. |
| 19 | `src/monitor/master_watch.rs:842-849` | revived master `pidfd.try_clone()` and watcher spawn | Same monitor clone abstraction as row 18. |
| 20 | `src/monitor/master_watch.rs:1364,1369` | fallback `libc::kill` and `ESRCH` | Route through platform process kill/liveness helpers; Windows compile path must not name `libc::SIGKILL`/`ESRCH`. |
| 21 | `src/monitor/session_watch.rs:5,194,198` | `OwnedFd` placeholder from `/dev/null` | Replace placeholder monitor anchor with platform-neutral handle or gate anchor watcher implementation. |
| 22 | `src/agent_io/reader.rs:7,44` | FIFO reader uses `tokio::io::unix::AsyncFd` | Gate Unix FIFO reader and introduce a stream-reader abstraction. Windows implementation may be compile-only in M0; M2 replaces FIFO with ConPTY stream. |
| 23 | `src/rpc/handlers/agent.rs:28` | `nix::sys::stat::Mode` import | Gate Unix FIFO creation helper or move the import into a Unix-only helper module. |
| 24 | `src/rpc/handlers/agent.rs:32` | `OpenOptionsExt` import | Gate Unix FIFO open path or move into Unix-only helper. |
| 25 | `src/rpc/handlers/agent.rs:190` | `nix::unistd::mkfifo` | Gate Unix agent IO path; Windows M0 returns unsupported before spawn or compiles a stub. |
| 26 | `src/rpc/handlers/agent.rs:203` | `libc::O_NONBLOCK` | Gate Unix FIFO open path; Windows stream path lands in M2. |
| 27 | `src/rpc/handlers/agent.rs:258-270` | `pidfd_open`, `pidfd.try_clone()`, and watcher handle split | Use platform-neutral `MonitorHandle::try_clone()` or wrapper before registering/spawning the watch task. |
| 28 | `src/rpc/handlers/agent.rs:558` | test/helper `libc::kill(SIGKILL)` | Gate Unix-only tests or route through platform kill helper. |
| 29 | `src/rpc/handlers/sessions.rs:453-459` | `pidfd_open`, `pidfd.try_clone()`, register, and master watcher spawn | Use platform-neutral `MonitorHandle::try_clone()` or wrapper. |
| 30 | `src/rpc/handlers/system.rs:14` | shutdown self-signal uses `libc::kill(SIGTERM)` | Replace with platform shutdown abstraction or cfg Unix self-signal and Windows process exit/unsupported path. |
| 31 | `src/db/system.rs:21` | `std::os::unix::fs::OpenOptionsExt` in root lib module | Gate startup-reconcile FIFO reopen helper or move Unix FIFO reattach into a Unix-only module. |
| 32 | `src/db/system.rs:974-978` | reopen FIFO with `libc::O_NONBLOCK` | Gate Unix FIFO reattach path; Windows M0 should compile an unsupported/no-reattach path until M2 stream reattach exists. |
| 33 | `src/db/system.rs:998-1008` | `libc::kill(pid, 0)`, `EPERM`, `ESRCH` | Route through `platform::sys::proc_info::kill_zero_check` or a platform liveness wrapper. |
| 34 | `src/db/system.rs:1076-1089` | `pidfd_open().try_clone()`, register, and `spawn_agent_pidfd_watch_task` | Use platform-neutral monitor handle clone/watcher abstraction. |
| 35 | `src/db/system.rs:1222-1225` | stale tmux sweep uses `/tmp/tmux-<uid>` and `libc::geteuid` | Gate tmux socket sweeping to Unix; Windows M0 returns zero/no-op until ConPTY session cleanup exists. |
| 36 | `src/db/system.rs:1324-1325` | unit test fake pidfd uses `std::os::fd::OwnedFd` | Gate fake pidfd tests with `#[cfg(unix)]` or provide Windows monitor-handle test fixture. |
| 37 | `src/bin/ahd.rs:16,117-124` | `tokio::signal::unix::{signal, SignalKind}` | Gate Unix signal handling and add Windows ctrl/shutdown handling stub. |
| 38 | `src/bin/ahd.rs:179` | tmux socket path uses `/tmp/tmux-<uid>` and `libc::geteuid` | Gate tmux cleanup path to Unix; Windows has no tmux server cleanup in M0. |
| 39 | `src/bin/ah.rs:543,613` | daemon socket probing uses `std::os::unix::net::UnixStream` | Route through the same platform RPC client probe abstraction as `src/cli/rpc_client.rs`. |
| 40 | `src/bin/ah.rs:1032-1033` | tmux socket path uses `/tmp/tmux-<uid>` and `libc::geteuid` | Gate attach path to Unix or return Windows unsupported until M2 attach work. |
| 41 | `src/bin/ah.rs:1050` | `std::os::unix::process::CommandExt::exec` | Gate Unix `exec tmux attach`; Windows M0 should compile with an unsupported attach path. |
| 42 | `src/cli/doctor.rs:218` | tmux orphan scan uses `/tmp/tmux-<uid>` and `libc::geteuid` | Gate Unix doctor check; Windows doctor can omit tmux orphan scan. |
| 43 | `src/cli/service_unit.rs:54` | test helper uses Unix symlink | Gate the symlink test or rewrite with portable temp layout. |
| 44 | `src/cli/service_bootstrap.rs:498` | test imports `std::os::unix::process::ExitStatusExt` | Gate Unix-only status construction test or replace with portable assertion. |
| 45 | `src/provider/home_layout.rs:15,602,1764,1788,1877` | `PermissionsExt` and Unix symlink materialization | Split provider home layout filesystem operations by platform. M0 may gate symlink tests and provide Windows compile stubs; M1/M2 decide junction/copy semantics. |
| 46 | `src/provider/skills.rs:194` | test uses Unix symlink | Gate test with `#[cfg(unix)]` or use a Windows-safe fixture. |
| 47 | `src/prompt_handler/kb.rs:6,9,244` | `nix::fcntl::flock` and `AsRawFd` | Gate Unix file lock implementation and add Windows compile stub using `EnvironmentNotSupported` until a Windows lock is chosen. |
| 48 | `src/prompt_handler/integration.rs:842-843` | test harness tmux path uses `/tmp/tmux-<uid>` and `libc::geteuid` | Gate tmux integration tests for Unix. |
| 49 | `src/tmux/mod.rs:50` | `nix::sys::stat::Mode` test import | Gate Unix-specific tmux/FIFO tests. |
| 50 | `src/tmux/mod.rs:52` | `OpenOptionsExt` test import | Gate Unix-specific tmux/FIFO tests. |
| 51 | `src/tmux/mod.rs:66-67` | tmux socket path uses `/tmp/tmux-<uid>` and `libc::geteuid` | Gate Unix-specific tmux tests. |
| 52 | `src/tmux/mod.rs:160,335,339` | tests create FIFO with `mkfifo` and `O_NONBLOCK` | Gate Unix-specific tmux/FIFO tests; production tmux module can remain Unix-only until M2 introduces multiplexer boundary. |
| 53 | `src/rpc/handlers.rs:66` | router test uses `PermissionsExt` | Gate or rewrite permission assertion for Windows. |

Tasks M0-5 through M0-8 below are implementation-oriented breakdowns of this table. They are not additive scope on top of the 53 cleanup rows; they group the same work by abstraction boundary so implementers can avoid double counting.

### 5.2. Test Target Cleanup List

`cargo check --all-targets --target x86_64-pc-windows-msvc` compiles integration tests. The 44 unique files below contain Unix, tmux, systemd, pidfd, FIFO, or permission assumptions and must be file-gated, module-gated, or rewritten before the M0 gate can pass:

- `tests/ack_fallback_lifecycle.rs`
- `tests/ah_dogfooding.rs`
- `tests/ah_full_e2e_drift.rs`
- `tests/ah_full_e2e_main.rs`
- `tests/ah_full_e2e_realign_extra.rs`
- `tests/common/mod.rs`
- `tests/dispatch_atomicity.rs`
- `tests/e2e_bundle_materialization_a4.rs`
- `tests/mvp10_acceptance.rs`
- `tests/mvp11_acceptance.rs`
- `tests/mvp11_real_claude.rs`
- `tests/mvp11_real_codex.rs`
- `tests/mvp12_home_layout.rs`
- `tests/mvp2_acceptance.rs`
- `tests/mvp3_acceptance.rs`
- `tests/mvp4_acceptance.rs`
- `tests/mvp6_acceptance.rs`
- `tests/mvp7_acceptance.rs`
- `tests/mvp7_real_codex.rs`
- `tests/mvp8_acceptance.rs`
- `tests/mvp8_real_codex.rs`
- `tests/mvp9_acceptance.rs`
- `tests/mvp9_real_codex_claude.rs`
- `tests/orphan_reap.rs`
- `tests/pr1_bug_f_state_layout.rs`
- `tests/pr1b_readfirst_hook.rs`
- `tests/pr3_codex_bundle.rs`
- `tests/pr4_antigravity_bundle.rs`
- `tests/pr4a_lifecycle_contract.rs`
- `tests/pr4c_hooks_plugins.rs`
- `tests/pr4d_auto_provisioning.rs`
- `tests/pr4e_up_fingerprint.rs`
- `tests/pr7_tests_first.rs`
- `tests/prompt_handler_e2e.rs`
- `tests/r1_bindsto_alignment.rs`
- `tests/r1_master_exit_shutdown.rs`
- `tests/r1_r3_joint.rs`
- `tests/r1_session_lifecycle.rs`
- `tests/r1_session_naming.rs`
- `tests/r1_shutdown_cleanup.rs`
- `tests/r2_master_scope_spawn.rs`
- `tests/r3_absolute_path_propagation.rs`
- `tests/r4_attach_mapping.rs`
- `tests/r4_doctor_migration.rs`

Preferred M0 strategy:

- Use `#![cfg(unix)]` for integration tests whose purpose is explicitly tmux/systemd/pidfd behavior.
- Use targeted `#[cfg(unix)]` modules for tests that also contain portable assertions.
- Do not delete tests. Preserve Linux coverage.
- Add Windows-specific compile-only tests only when they prove a new abstraction boundary.

**Exit Criteria**

- Running the exact M0 command no longer fails on parser/import errors from Unix-only names.
- Linux `cargo test --all-targets` remains green.

## 6. Task M0-5: IPC Boundary

**Scope**

Introduce a minimal IPC transport boundary so the daemon and CLI no longer directly name Unix sockets in shared Windows builds.

The boundary must cover:

- daemon listener bind, accept, split read/write stream
- stale endpoint probe
- CLI request/response connect
- CLI event stream connect
- endpoint path/name construction

M0 can implement the Windows backend as compile-only `NamedPipe` placeholders returning `EnvironmentNotSupported` for runtime paths not yet used by acceptance. The type signatures must not expose `UnixStream` or `UnixListener` outside Unix-only modules.

Prefer platform-conditional concrete types over trait objects for M0. A low-cost shape is:

```rust
#[cfg(unix)]
type RpcListener = tokio::net::UnixListener;

#[cfg(windows)]
struct RpcListener {
    // M0 placeholder; M1/M2 can replace with Named Pipe listener state.
}
```

If shared call sites need one name for accepted streams, use a small enum or platform-specific type alias at the transport module boundary. Do not push `Box<dyn AsyncRead + AsyncWrite>` through the whole RPC stack unless a concrete type boundary proves insufficient.

**Files**

- `src/rpc/mod.rs`
- `src/cli/rpc_client.rs`
- callers in `src/bin/ah.rs` that probe the daemon socket

**Exit Criteria**

- No unconditional `UnixStream`, `UnixListener`, or `tokio::net::Unix*` import remains reachable in Windows builds.

## 7. Task M0-6: Monitor Boundary

**Scope**

Replace the current public `OwnedFd/BorrowedFd` monitor API with platform-neutral types. This is required because `std::os::fd` compiles only on Unix-like targets.

M0 shape:

- `src/monitor/mod.rs` re-exports or aliases `crate::platform::sys::process::MonitorHandle`.
- `pidfd_open` keeps its existing name for call-site churn control, but returns the platform monitor handle.
- `MonitorHandle` must expose `try_clone()` or the monitor module must expose an equivalent `clone_handle_for_watch(&MonitorHandle) -> std::io::Result<MonitorHandle>`.
- `with_borrowed` continues to support closure-style access for call sites such as `master_watch`, using a platform-neutral borrowed handle type.
- `agent_watch` and `master_watch` hide Unix `AsyncFd` readiness behind a platform watch function. Windows M0 may compile as unsupported; M1 implements wait handles.

Known direct clone call sites to update:

- `src/monitor/master_watch.rs:217-221`
- `src/monitor/master_watch.rs:842-849`
- `src/rpc/handlers/sessions.rs:453-459`
- `src/rpc/handlers/agent.rs:258-270`
- `src/db/system.rs:1076-1089`

Prefer platform-conditional type aliases or small newtypes over trait objects for M0. The handle is stored in a registry and borrowed under a mutex, so static dispatch keeps the borrow and clone signatures visible to the compiler and avoids object-safety churn.

**Files**

- `src/monitor/mod.rs`
- `src/monitor/agent_watch.rs`
- `src/monitor/master_watch.rs`
- `src/monitor/session_watch.rs`
- `src/platform/{linux,macos,windows}/process.rs`

**Exit Criteria**

- No unconditional `std::os::fd`, `OwnedFd`, `BorrowedFd`, or `tokio::io::unix::AsyncFd` import remains reachable in Windows builds.

## 8. Task M0-7: Agent IO Boundary

**Scope**

Make the Unix FIFO path compile-gated and introduce a future-compatible stream reader boundary for M2.

M0 requirements:

- Keep current FIFO implementation on Unix.
- Gate `nix::unistd::mkfifo`, `OpenOptionsExt`, and `libc::O_NONBLOCK`.
- Gate `tokio::io::unix::AsyncFd` in `agent_io::reader`.
- Provide a Windows compile path that either returns `EnvironmentNotSupported` before agent spawn or compiles a no-op stream registration boundary.

**Files**

- `src/rpc/handlers/agent.rs`
- `src/agent_io/reader.rs`
- `src/agent_io/registry.rs`

**Exit Criteria**

- Windows `--all-targets` compile does not see Unix FIFO APIs.
- The design's M2 requirement is preserved: Windows must later replace `pipe_pane_to_fifo` with stream registration, not emulate Unix FIFOs.

## 9. Task M0-8: tmux/systemd Runtime Gates

**Scope**

M0 does not replace tmux or systemd behavior. It only prevents Unix runtime code from breaking Windows compilation.

Gate or stub:

- tmux attach execution in `src/bin/ah.rs`
- tmux cleanup in `src/bin/ahd.rs`
- tmux doctor orphan scan in `src/cli/doctor.rs`
- systemd bootstrap paths in `src/bin/ah.rs`, `src/cli/service_bootstrap.rs`, and `src/sandbox/systemd.rs`
- tmux module Unix tests in `src/tmux/mod.rs`

**Exit Criteria**

- Windows compile paths either call Windows stubs or produce explicit unsupported errors.
- Linux/macOS tmux/systemd behavior remains unchanged.

## 10. Task M0-9: CI Job

**Scope**

Extend `.github/workflows/ci.yml` with a root-package Windows compile job separate from the disposable spike job.

Suggested job:

```yaml
windows-msvc-check:
  runs-on: windows-latest
  env:
    CARGO_BUILD_JOBS: "1"
  steps:
    - name: Checkout
      uses: actions/checkout@v4
    - name: Install Rust
      uses: dtolnay/rust-toolchain@stable
      with:
        targets: x86_64-pc-windows-msvc
    - name: Check Windows target
      run: cargo check --all-targets --target x86_64-pc-windows-msvc
```

Keep the existing `windows-conpty-spike` job until M2 production code supersedes it.

**Exit Criteria**

- PR CI has a red/green signal for the M0 compile gate.
- The job runs serial cargo via `CARGO_BUILD_JOBS=1`.

## 11. Task M0-10: `test_no_legacy_pty_dependency` Guard Narrowing

**Scope**

The current guard in `tests/mvp6_acceptance.rs` protects against accidentally reintroducing legacy PTY dependencies. It must not be deleted. It should be narrowed so that future M2 production Windows PTY dependencies can be explicitly allowed while non-target or accidental dependencies remain blocked.

M0 stance:

- Do not add root `portable-pty` in M0 unless production compile paths require it.
- Keep the guard passing after M0.
- Update the test wording or helper shape so the intended future rule is clear.

M2-ready rule:

- Allow `portable-pty` and `alacritty_terminal` only under `[target.'cfg(windows)'.dependencies]`.
- Continue rejecting those crates in unconditional `[dependencies]`, Unix target dependency blocks, and non-spike dev-dependencies.
- Make the guard intent-aware rather than substring-only:
  - Parse `Cargo.toml` and build an explicit whitelist from the root package's `[target.'cfg(windows)'.dependencies]` block.
  - Fail if `portable-pty`, `alacritty_terminal`, or another guarded PTY/grid crate appears in root unconditional `[dependencies]`, Unix target blocks, or non-spike dev-dependencies.
  - Allow a guarded crate to appear in `Cargo.lock` if and only if it is explicitly present in the root Windows target whitelist.
  - Fail if `Cargo.lock` contains a guarded crate that the root manifest did not whitelist for Windows. This catches accidental transitive drift while still permitting intentional M2 Windows PTY dependencies.

**Files**

- `tests/mvp6_acceptance.rs`
- `Cargo.toml`
- `Cargo.lock`

**Exit Criteria**

- M0 keeps `cargo test --test mvp6_acceptance test_no_legacy_pty_dependency` green.
- The guard preserves its original intent: accidental PTY dependency drift remains visible.

## 12. Final M0 Exit Gate

M0 is complete when all of the following pass:

```bash
cargo check --all-targets --target x86_64-pc-windows-msvc
cargo test --test mvp6_acceptance test_no_legacy_pty_dependency
cargo test --all-targets
```

and CI includes the Windows MSVC check job with the first command.

The required external CI signal is:

- `windows-latest`: `cargo check --all-targets --target x86_64-pc-windows-msvc` passes.

## 13. Non-Goals

- Implementing Job Object behavior.
- Implementing Task Scheduler CRUD.
- Replacing tmux with ConPTY multiplexer production code.
- Making `ah attach` work on Windows.
- Running Windows behavioral tests beyond compile validation.
- Adding root `portable-pty`/`alacritty_terminal` before the M2 production multiplexer requires them.
