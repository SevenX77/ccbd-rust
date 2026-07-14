# Windows Native Port M1 Implementation Spec

Status: draft for review. Scope: implementation-level spec for M1 Win32 adaptors after M0 compile gate. This document is a work plan, not code.

## 1. Goal

M1 replaces M0's Windows compile stubs with real Win32 OS adaptors while preserving the existing Linux/macOS call surfaces. M1 is still an internal milestone: it proves the Windows process, containment, service, and IPC primitives on `windows-latest`, but the first user-usable Windows MVP remains M0 + M1 + M2 because production agent/master spawning still depends on the M2 ConPTY multiplexer.

Estimate: p50 ~3-4 weeks. This is higher than the design's early 2-3 week estimate because M1 is runtime-heavy and CI-driven: HANDLE lifetime bugs, wait callback races, COM apartment issues, and Named Pipe hangs generally require a full `windows-latest` round trip to reproduce and verify.

Primary files:

- `src/platform/windows/process.rs`
- `src/platform/windows/scope.rs`
- `src/platform/windows/service.rs`
- `src/rpc/mod.rs`
- `src/cli/rpc_client.rs`
- `Cargo.toml`
- Windows-only smoke tests under `tests/` or platform module test blocks
- `.github/workflows/ci.yml`

Recommended implementation order:

1. Process HANDLE ownership and wait primitives.
2. Job Object primitives and smoke coverage.
3. Task Scheduler COM service adaptor.
4. RPC transport seam plus Named Pipe server/client.
5. Windows CI smoke matrix and Linux regression verification.

The process, scope, and service adaptors are mostly independent. IPC is separate and should be built with the real Named Pipe implementation in hand, not as a speculative abstraction.

Incremental strategy: implement each adaptor as an independent PR with its own `windows-latest` smoke test, then merge once that slice is green. Use the M0 convergence pattern: Linux regression checks locally, Windows compile/runtime truth from CI, focused follow-up commits for CI-only failures. Do IPC last because it is the only M1 slice that changes shared cross-platform RPC flow and therefore has the highest regression blast radius.

## 2. M1-Process: Windows HANDLE Monitor Implementation

### Scope

Replace the M0 stub in `src/platform/windows/process.rs` with a real process-handle implementation behind the existing monitor wrapper API:

- `MonitorHandle`
- `BorrowedMonitorHandle<'_>`
- `pidfd_open`
- `pidfd_send_sigkill`
- `register`
- `remove`
- `with_borrowed`
- `contains`
- `list_keys`

The API must remain source-compatible with `src/monitor/mod.rs` and current call sites in `src/monitor/*`, `src/rpc/handlers/sessions.rs`, `src/rpc/handlers/agent.rs`, and `src/db/system.rs`.

### Design

`MonitorHandle` owns a real Windows process `HANDLE`.

- `pidfd_open(pid)` calls `OpenProcess` with exactly the retained-handle rights required by the current wrapper surface:
  - `SYNCHRONIZE`
  - `PROCESS_QUERY_LIMITED_INFORMATION`
  - `PROCESS_TERMINATE`
- The `PROCESS_TERMINATE` right is mandatory at open time because `pidfd_send_sigkill(handle: BorrowedMonitorHandle)` receives only the retained handle through `src/monitor/mod.rs`; it has no PID to reopen with broader rights later.
- Do not store only numeric PIDs. Retained process handles are the PID-reuse guard.
- `Drop` for `MonitorHandle` must call `CloseHandle` exactly once for each owned handle.
- `try_clone()` must use `DuplicateHandle` against the current process. The current M0 stub raw-copies the handle (`src/platform/windows/process.rs` carry-forward), which would double-close or leave dangling borrowed handles once handles become real.
- `BorrowedMonitorHandle<'a>` is a non-owning handle view tied to the registry lock/owner lifetime.
- `pidfd_send_sigkill` maps to `TerminateProcess(handle, exit_code)`. Use a stable non-zero exit code, document it in the smoke test, and treat `ERROR_ACCESS_DENIED`/already-exited cases deliberately.
- Liveness inspection uses wait state first, not exit-code polling:
  - `WaitForSingleObject(handle, 0) == WAIT_OBJECT_0` means the process has exited.
  - `WAIT_TIMEOUT` means the process is still running.
  - Only call `GetExitCodeProcess` after the wait is signaled or after the wait callback fires.
  - Do not use `GetExitCodeProcess == STILL_ACTIVE` as the sole liveness check. A process can legitimately exit with code 259, the numeric value of `STILL_ACTIVE`; polling exit code alone would misclassify that exited process as still running.
- Async exit notification uses `RegisterWaitForSingleObject`:
  - Register on the retained process handle.
  - Use `WT_EXECUTEONLYONCE`. Process handles remain signaled after exit; without one-shot registration, the callback can fire repeatedly.
  - The callback must publish exactly one exit event, for example by completing a one-shot channel or setting an atomic guard before send.
  - The callback must not touch Rust state unsafely without synchronization. Prefer sending into a Tokio-safe channel or waking a small wrapper task.
  - Store the returned wait handle.
  - On cancellation/drop, call `UnregisterWaitEx(wait_handle, INVALID_HANDLE_VALUE)` so unregister blocks until any in-flight callback has returned before shared callback state is freed.
  - Do not call blocking unregister from inside the callback.

### Files

- `src/platform/windows/process.rs`: real HANDLE type, ownership, registry, `OpenProcess`, `DuplicateHandle`, `CloseHandle`, `TerminateProcess`, `WaitForSingleObject`, `GetExitCodeProcess`, `RegisterWaitForSingleObject`, `UnregisterWaitEx`.
- `src/platform/windows/proc_info.rs`: replace M0 liveness stubs with wait-state-first helpers; use `GetExitCodeProcess` only after the handle is known signaled.
- `src/monitor/agent_watch.rs` and `src/monitor/master_watch.rs`: replace M0 Windows no-op watcher branches with wait-registration based notification, while keeping Unix `AsyncFd` paths unchanged.
- `Cargo.toml`: add any missing `windows-sys` features for process wait APIs if not already present.

### Exit Gate

Add a Windows-only smoke test:

- Spawn a long-running child process.
- Open a `MonitorHandle` with `pidfd_open`.
- Register an exit notification through the real `monitor::` wrapper plus the production watcher path in `src/monitor/agent_watch.rs` or `src/monitor/master_watch.rs`. The gate must prove the M0 Windows no-op watcher branch has been replaced.
- Call `pidfd_send_sigkill` or `TerminateProcess`.
- Assert:
  - callback/notification fires exactly once within a bounded timeout;
  - `WaitForSingleObject(handle, 0)` reports signaled before reading exit code;
  - `GetExitCodeProcess` reports the expected exit code after signaled;
  - cloned handles remain valid independently;
  - closing both original and cloned handles does not double-close.

CI command remains:

```bash
cargo check --all-targets --target x86_64-pc-windows-msvc
```

The smoke must run in a `windows-latest` job.

`src/platform/windows/identity.rs` is in M1 scope only to the extent needed for daemon identity/liveness parity with the existing platform facade. If no production caller needs a richer Windows identity during M1, leave it as a minimal implementation and explicitly defer deeper daemon identity semantics to the service/IPC integration PR. Do not let identity work block process, scope, service, or IPC smoke gates.

## 3. M1-Scope: Job Object Primitives

### Scope

Replace the M0 stub behavior in `src/platform/windows/scope.rs` with real Windows Job Object containment primitives and smoke tests. M1 does not wire Job Objects into production agent/master spawning yet.

### M1/M2 Boundary

The design's MAJOR-B decision is option 2:

- `wrap_command*` and `master_command*` keep returning `Vec<String>`.
- `wrap_in_scope` keeps returning `std::process::Command`.
- These builders do not carry Job Object handles, spawn plans, or assignment callbacks.
- Job Object assignment to the production spawn path belongs to the M2 multiplexer spawn layer, because M2 owns the ConPTY-backed process spawn path.

M1-scope therefore delivers:

- Job Object creation/configuration primitives.
- Suspended spawn -> assign -> resume helper(s) used by M1 tests.
- A Windows smoke test proving cascading kill semantics.

M1-scope explicitly does not deliver:

- Agent/master production spawn containment.
- ConPTY process tree containment.
- Multiplexer-level process lifetime integration.

### Design

Job Object behavior:

- Create one Job Object per supervised scope with `CreateJobObjectW`.
- Configure `JOBOBJECT_EXTENDED_LIMIT_INFORMATION` with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`.
- Do not set `JOB_OBJECT_LIMIT_SILENT_BREAKAWAY_OK` in the MVP path. Breakaway weakens the cascade-kill guarantee and is excluded from the M1 exit gate.
- For containment-sensitive children, use:
  1. `CreateProcessW` with `CREATE_SUSPENDED`;
  2. `AssignProcessToJobObject`;
  3. `ResumeThread`;
  4. close thread/process handles according to ownership;
  5. close the job handle to trigger cascade kill.
- The smoke process must create a child after resume so the test proves process-tree cleanup, not only direct-child cleanup.

### Files

- `src/platform/windows/scope.rs`: add Job Object wrapper types and helper functions while preserving current command-builder signatures.
- `Cargo.toml`: ensure `windows-sys` has the required Job Object, process, and handle features.
- Tests may live in a Windows-only module under `src/platform/windows/scope.rs` or in a dedicated `tests/windows_job_object_smoke.rs` with `#![cfg(windows)]`.

### Exit Gate

Add a Windows-only smoke test:

- Create a Job Object with `KILL_ON_JOB_CLOSE`.
- Spawn a test command suspended.
- Assign it to the job.
- Resume it.
- Let it spawn a descendant process.
- Close the job handle.
- Assert both parent and descendant exit within a bounded timeout.
- Assert no breakaway flag is set in the tested path.

This smoke is the M1 proof of the primitive. Production spawn wiring waits for M2.

## 4. M1-Service: Task Scheduler User Daemon Adaptor

### Scope

Replace the Windows service M0 placeholder in `src/platform/windows/service.rs` with a user-level Task Scheduler adaptor. This maps the existing service helper surface to Windows semantics while keeping compile-visible symbol names used by `src/cli/service_unit.rs`.

Existing surface to preserve:

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

The names remain for cross-platform call-site compatibility even when the Windows implementation maps to Task Scheduler concepts rather than systemd units.

### Design

Use Task Scheduler COM APIs:

- Run Task Scheduler work inside `tokio::task::spawn_blocking` or an equivalent blocking boundary.
- On that same blocking thread, call `CoInitializeEx(None, COINIT_MULTITHREADED)`, perform the complete COM operation, then call `CoUninitialize` before returning.
- Do not initialize COM on one thread and use `ITaskService` interfaces on another.
- Use `ITaskService` to connect to the local Task Scheduler service.
- Create/update a per-user ah task.
- Configure an action that launches `ahd` with `AH_STATE_DIR` and permitted passthrough environment.
- Run the task on install/start.
- Query registration/state.
- Delete the task on uninstall.

Cargo dependency decision:

- `windows-sys 0.61.2` does not expose `Win32_System_TaskScheduler`; adding that feature to `windows-sys` will not compile.
- Add the `windows` crate as a Windows-only dependency under `[target.'cfg(windows)'.dependencies]`, version-aligned with the current Windows crate family, for example `windows = { version = "0.61", features = [...] }`.
- Use minimal `windows` crate features for Task Scheduler COM, at least:
  - `Win32_System_TaskScheduler`
  - `Win32_System_Com`
  - `Win32_Foundation`
  - add only the specific Security/Variant/System features required by the compiler while implementing `ITaskService`.
- Keep the existing `windows-sys` dependency for lower-level process, Job Object, handle, console, file-system, and Named Pipe APIs. The `windows` and `windows-sys` crates can coexist; use `windows` where COM interface projections are needed and `windows-sys` where raw FFI is already sufficient.

Escaping:

- `escape_systemd_env_value` and `escape_systemd_exec_token` become Windows Task Scheduler argument/env escaping equivalents.
- They may be no-ops only for values that are safe under Task Scheduler XML/action encoding.
- Preserve control-character rejection.

Persistence:

- `render_unit_file` may render a diagnostic/metadata representation, but the authoritative persistent object is the Task Scheduler registration.
- `atomic_write_unit` remains for any metadata file the CLI wants to keep under `%LOCALAPPDATA%\ah\`; it is not a substitute for registering the task.
- `resolve_user_systemd_dir*` should return the Windows ah config/metadata directory under `%LOCALAPPDATA%\ah\` or `%USERPROFILE%\AppData\Local\ah\`.

### Files

- `src/platform/windows/service.rs`: MTA COM initialization inside blocking operation boundaries, task registration/run/delete/query helpers, escaping, metadata path resolution.
- `src/cli/service_unit.rs` and related CLI service commands: M1 wires `ah service install`/restart/delete-style CLI paths to the Windows Task Scheduler adaptor. The Task Scheduler adaptor smoke alone is not enough; the service CLI path must call the Windows adaptor in M1.
- `Cargo.toml`: Windows-only `windows` crate Task Scheduler COM dependency; do not add a nonexistent `Win32_System_TaskScheduler` feature to `windows-sys`.
- `.github/workflows/ci.yml`: Windows smoke job.

### Exit Gate

Add a Windows-only smoke test:

- Register a uniquely named per-user test task.
- Configure it to run a harmless command that writes a marker file under a temp directory.
- Run the task.
- Poll for marker file or task completion with a bounded timeout.
- Query the task registration/state.
- Delete the task.
- Assert the task is gone.
- Exercise the same adaptor functions that the Windows `ah service install`/run/delete CLI branch uses, or include a separate CLI-level smoke if the adaptor and CLI branch are not identical.

The smoke must run without admin elevation on `windows-latest`.

## 5. M1-IPC Transport Seam and Named Pipes

### Scope

M0 left Windows RPC server/client paths unsupported. M1 builds the real transport seam with Named Pipes in hand.

Carry-forward evidence:

- `src/rpc/mod.rs` Windows `run_server` returns `Unsupported`.
- Unix accept/dispatch loop is inside `#[cfg(unix)]`.
- `bind_rpc_listener` returns `UnixListener`.
- `src/cli/rpc_client.rs` has a useful `RpcClient` trait, but Windows `rpc_call` and `rpc_stream_first` return `Unsupported`.

### Design

Extract shared request handling from transport-specific accept/read/write:

- Shared daemon function:
  - parse one JSON line;
  - detect `event.subscribe`;
  - call `handlers::stream_event_subscribe` for streams;
  - call `router::dispatch` for request/response;
  - serialize response line.
- Unix transport keeps current UDS behavior.
- Windows transport uses Named Pipes.

Named Pipe requirements:

- Pipe name should be derived from state layout, not `/tmp`; use a stable user-scoped name such as `\\.\pipe\ahd-<hash>`.
- Build a deterministic security descriptor:
  - resolve the current user SID;
  - construct a DACL that grants the current user the required pipe access;
  - avoid broad Everyone/Authenticated Users write access;
  - expose the ACL construction through a small inspectable helper so tests can assert the intended ACEs without relying only on connection behavior.
- Daemon side supports:
  - create/listen;
  - accept multiple connections;
  - read line-delimited JSON;
  - write response lines;
  - support streaming `event.subscribe`.
- CLI side supports:
  - `rpc_call`;
  - `rpc_stream_first`;
  - connection-refused/not-running error mapping equivalent to Unix.

Implementation options:

- Prefer `tokio::net::windows::named_pipe` if it is available for the pinned Tokio version and supports the needed server/client behavior. If available, it materially reduces overlapped-I/O risk.
- If using raw Win32 APIs through `windows-sys`, isolate overlapped I/O details inside a Windows transport module and keep router/handler code transport-neutral.

### Files

- `src/rpc/mod.rs`: extract shared dispatch/stream helpers and call them from Unix and Windows transports.
- New optional module, for example `src/rpc/transport.rs` or `src/rpc/windows_pipe.rs`.
- `src/cli/rpc_client.rs`: implement Windows Named Pipe client while preserving Unix UDS code.
- `src/state_layout.rs` if a pipe-name helper belongs with state layout.
- `Cargo.toml`: Windows-only Named Pipe dependency or `windows-sys` features.

### Exit Gate

Add a Windows-only smoke test:

- Start the daemon RPC server bound to a test Named Pipe name with a temp state DB/context.
- Assert the constructed pipe ACL grants the current user and does not grant broad write access to Everyone/Authenticated Users.
- Connect via CLI-side Named Pipe client.
- Send a simple request/response method that does not need tmux/ConPTY.
- Assert the response is valid JSON and contains expected result data.
- Exercise `rpc_stream_first` against `event.subscribe` or a narrow test-only stream if practical.
- Assert unauthorized/stale pipe cases map to deterministic `CliError` variants.

## 6. Windows CI and Local Verification

### Local

Required local checks before pushing M1 implementation increments:

```bash
CARGO_BUILD_JOBS=1 cargo check --all-targets
CARGO_BUILD_JOBS=1 cargo build --release
```

These validate Linux and cfg stability only.

The current development environment does not have MinGW available, so `cargo check --target x86_64-pc-windows-gnu` stops before Rust source checking when C dependencies need `x86_64-w64-mingw32-gcc`. Do not treat local GNU target failure from missing toolchain as a Rust failure. If a future worker has MinGW installed, GNU check is useful because it exercises `cfg(windows)`, but the authoritative target remains MSVC CI.

### CI

`windows-latest` is the truth source for M1 behavior:

- `cargo check --all-targets --target x86_64-pc-windows-msvc`
- process HANDLE smoke;
- Job Object cascading-kill smoke;
- Task Scheduler COM CRUD smoke;
- Named Pipe RPC round-trip smoke.

Keep the existing Linux and macOS jobs green. M1 must not regress the ConPTY spike job.

Each adaptor PR should add its own Windows smoke job or smoke step next to the existing Windows check. Do not batch all M1 runtime validation into a final PR; that recreates the slowest possible CI feedback loop.

## 7. Test Matrix

| Gate | File/Area | Environment | Assertion |
| --- | --- | --- | --- |
| Windows compile | whole crate | windows-latest | `cargo check --all-targets --target x86_64-pc-windows-msvc` passes. |
| Process HANDLE wait | `platform/windows/process.rs` + real watcher path | windows-latest | `OpenProcess` handle survives PID reuse risk, `RegisterWaitForSingleObject` one-shot fires exactly once after termination, wait state is signaled before `GetExitCodeProcess` is read. |
| Handle clone ownership | `platform/windows/process.rs` | windows-latest | `try_clone` uses `DuplicateHandle`; dropping original and clone does not double-close or invalidate the other prematurely. |
| Job Object cascade | `platform/windows/scope.rs` | windows-latest | Suspended parent is assigned to a `KILL_ON_JOB_CLOSE` job, resumed, creates a child, and both exit when job handle closes. |
| Task Scheduler CRUD | `platform/windows/service.rs` + service CLI branch | windows-latest | Create, query, run, observe marker, delete a non-admin per-user task through the same adaptor used by Windows service commands. |
| Named Pipe RPC | `rpc` + `cli/rpc_client` | windows-latest | Daemon binds pipe with inspected current-user ACL, CLI connects, request/response succeeds, streaming first response works or is explicitly covered by a focused stream smoke. |
| Linux regression | whole crate | Linux local + CI | `cargo check --all-targets` and `cargo build --release` stay green. |

## 8. Risks and Mitigations

1. CI-only runtime feedback:
   - Risk: without local Windows execution or MinGW, every runtime bug can cost a full CI turn.
   - Mitigation: keep PRs small and adaptor-specific, add the smoke with the implementation, and avoid combining process/scope/service/IPC runtime changes in one branch.

2. HANDLE ownership and wait registration:
   - Risk: raw handle copies can double-close or invalidate borrowed handles.
   - Mitigation: make `MonitorHandle` the only owner, implement `Drop`, and make `try_clone` call `DuplicateHandle`. Add explicit clone/drop smoke coverage.

3. `RegisterWaitForSingleObject` callback safety:
   - Risk: callback runs on a Windows threadpool thread and can outlive Rust state if not unregistered.
   - Mitigation: register with `WT_EXECUTEONLYONCE`, publish through a one-shot/guarded channel, unregister with `UnregisterWaitEx(wait, INVALID_HANDLE_VALUE)` outside the callback before freeing shared state, and avoid calling Tokio APIs directly from unsafe callback context unless routed through a safe wake mechanism.

4. Exit-code liveness trap:
   - Risk: treating `GetExitCodeProcess == STILL_ACTIVE` as authoritative can misclassify an exited process whose real exit code is 259.
   - Mitigation: use `WaitForSingleObject(handle, 0)` or the wait callback for liveness; read exit code only after signaled.

5. Job Object semantics:
   - Risk: assigning after spawn lets children escape; breakaway flags weaken cascade kill.
   - Mitigation: use `CREATE_SUSPENDED -> AssignProcessToJobObject -> ResumeThread`, never set `SILENT_BREAKAWAY_OK` in MVP, and test a parent-created descendant.

6. Task Scheduler COM:
   - Risk: COM apartment model and interface lifetime can fail under Tokio worker threads.
   - Mitigation: use `spawn_blocking`; on that same thread call `CoInitializeEx(None, COINIT_MULTITHREADED)`, use `ITaskService`, and call `CoUninitialize` before returning.

7. Task Scheduler binding availability:
   - Risk: using `windows-sys` for Task Scheduler is a compile-time dead end; the feature/module is absent in `windows-sys 0.61.2`.
   - Mitigation: use the Windows-only `windows` crate for Task Scheduler COM projections and keep `windows-sys` for lower-level non-COM APIs.

8. Named Pipe ACLs and async I/O:
   - Risk: a pipe that works functionally but has broad ACLs would be a local privilege/security regression; overlapped I/O can also produce flaky hangs.
   - Mitigation: construct an explicit current-user SID DACL, expose inspectable ACL construction tests, keep pipe read/write line protocol small, prefer `tokio::net::windows::named_pipe` if available, and isolate Windows transport code behind a narrow module.

9. Scope/production-spawn boundary:
   - Risk: reviewers may expect M1 Job Objects to contain real agents.
   - Mitigation: document and test the primitive in M1; production assignment is an M2 multiplexer responsibility because only M2 owns the ConPTY spawn path.

## 9. Definition of Done

M1 is complete when:

- Windows process monitor handles are real owned HANDLEs with safe clone/drop behavior.
- Windows process wait/termination smoke drives the real `monitor::` wrapper and watcher registration path and passes on `windows-latest`.
- Job Object cascade smoke passes using suspended assign/resume and no breakaway flag.
- Task Scheduler per-user CRUD smoke passes without admin elevation using the Windows-only `windows` crate COM bindings, and Windows service CLI paths are wired to that adaptor.
- Named Pipe RPC request/response smoke passes with deterministic current-user ACL construction and inspection.
- `cargo check --all-targets --target x86_64-pc-windows-msvc` is green in CI.
- Linux `cargo check --all-targets` and `cargo build --release` remain green.

M1 is not complete merely because Windows compiles. The M0 compile gate already covers that; M1 must prove the real Win32 primitives.
