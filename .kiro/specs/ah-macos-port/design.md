# macOS Native Support Design: Platform Abstraction Boundary

Status: design only. This document takes `.kiro/specs/ah-macos-port/research.md` as input and proposes the abstraction boundary for later implementation. No source changes are made in this phase.

Primary rule: Linux behavior is the compatibility oracle. The first implementation PR must move the existing Linux code behind platform traits with zero semantic change before any macOS behavior is added.

## 1. Platform Abstraction Boundary

Add a narrow `src/platform/` module that owns OS-specific behavior. Callers should depend on traits and small value types, not on `systemd-run`, pidfd syscalls, cgroup paths, or `/proc` directly.

Suggested shape:

```text
src/platform/
  mod.rs
  linux/
    process.rs
    scope.rs
    service.rs
    identity.rs
    proc_info.rs
  macos/
    process.rs
    scope.rs
    service.rs
    identity.rs
    proc_info.rs
```

Compile-time selection:

```rust
#[cfg(target_os = "linux")]
pub use linux::Platform;

#[cfg(target_os = "macos")]
pub use macos::Platform;
```

The Linux implementation should be a direct move of the current code paths. Do not rewrite behavior while extracting; make function bodies call the same commands/syscalls with the same arguments and error handling.

### 1.1 Proposed traits

| Trait | Owns | Current sites covered | Linux implementation | macOS candidate |
| --- | --- | --- | --- | --- |
| `ProcessWatcher` | Registering master/worker process exit watches; watcher task spawning; liveness checks; exit event shape. | `src/monitor/mod.rs:13`, `:17`; `src/monitor/agent_watch.rs:159`; `src/monitor/master_watch.rs:159`, `:417`; `src/rpc/handlers/agent.rs:250`; `src/rpc/handlers/sessions.rs:432`. | Move pidfd registry, `pidfd_open`, `AsyncFd`, `waitid(P_PIDFD)` behavior exactly. | kqueue `EVFILT_PROC` / `NOTE_EXIT`, explicit process identity fencing. |
| `ProcessReaper` | Signal delivery, `SIGKILL`, `SIGTERM`, kill fallback, optional process-group kill. | `src/monitor/mod.rs:44`; `src/monitor/master_watch.rs:1340`; `src/db/system.rs:240`, `:400`; `src/rpc/handlers/agent.rs:542`; `src/rpc/handlers/system.rs:14`. | Move `pidfd_send_signal` first, `kill(pid, SIGKILL)` fallback exactly. | `kill(pid, sig)` / `killpg(pgid, sig)` with pid/generation fencing. |
| `ScopeManager` | Containment for master/worker/tmux processes, scope naming, cascade ownership, startup orphan reconcile. | `src/sandbox/systemd.rs:13`, `:72`, `:104`, `:142`; `src/tmux/scope.rs:21`, `:55`; `src/db/system.rs:258`, `:436`, `:557`, `:621`; `src/rpc/handlers/sessions.rs:163`, `:192`. | Move `systemd-run --user --scope`, `BindsTo`, `PartOf`, slice/unit naming, `systemctl list-units/stop`, parsing exactly. | Process-group ownership tree plus durable DB ownership records and startup reconcile; optional launchd job boundary if PM chooses. |
| `ServiceSupervisor` | Persistent ahd service install/start/restart/GC/migration. | `src/cli/service_unit.rs:22`, `:39`, `:83`; `src/cli/service_bootstrap.rs:12`, `:59`, `:95`, `:159`, `:271`, `:402`; `src/cli/start.rs:30`, `:74`; `src/bin/ah.rs:537`, `:581`. | Move persistent systemd user service renderer/bootstrap plus transient fallback exactly. | LaunchAgent plist plus `launchctl bootstrap/bootout/kickstart`; no legacy systemd migration. |
| `DaemonIdentity` | Detecting current daemon identity and recursion/nesting guard. | `src/systemd_unit.rs:1`, `:16`; `src/bin/ah.rs:538`, `:633`; `src/bin/ahd.rs:34`, `:52`; `src/tmux/scope.rs:55`. | Move `/proc/self/cgroup` parser and `ahd.service`/`ah-*.service` detection exactly. | Environment marker and/or launchd label plus ahd-owned runtime identity file. |
| `ProcInfo` | Zombie/liveness metadata and process start identity. | `src/monitor/agent_watch.rs:117`; `src/db/system.rs:1086`; readiness checks in `master_watch`. | Move `/proc/{pid}/stat` zombie parse and `kill(pid,0)` helpers exactly. | `sysctl KERN_PROC_PID` or `libproc` for state/start-time; `kill(pid,0)` only as non-authoritative liveness. |
| `PlatformDiagnostics` | Doctor checks and platform capability errors. | `src/sandbox/mod.rs:33`, `:62`, `:100`; `src/cli/doctor.rs:34`; `src/error.rs:244`. | Same `systemd-run` and user-manager checks. | tmux + launchd/process-group capability checks; explicit degraded sandbox notes. |

### 1.2 Value types

Keep platform-specific handles opaque:

```rust
struct ProcessIdentity {
    pid: i32,
    generation: Option<i64>,
    start_time: Option<ProcessStartTime>,
}

enum ProcessExit {
    Exited { pid: i32, exit_code: Option<i32> },
    WatchLost { pid: i32, reason: String },
}

struct ScopeHandle {
    id: String,
    owner_session_id: Option<String>,
    owner_agent_id: Option<String>,
    process_group_id: Option<i32>,
}

enum CascadeTarget {
    Session { session_id: String },
    Agent { agent_id: String },
    TmuxServer { socket_name: String },
    Master { session_id: String, generation: i64 },
}
```

`ProcessExit.exit_code` must remain optional. The current Linux path often cannot obtain exit code for non-child tmux-launched processes; the macOS path must not silently invent stronger semantics.

### 1.3 Three-grade treatment

#### Direct replacement / no heavy trait

These are POSIX or already Unix-portable. Keep local, or put behind tiny helpers only when call sites need a common import:

| Item | Current examples | Strategy |
| --- | --- | --- |
| Unix domain sockets | `src/rpc/mod.rs:5`, `src/cli/rpc_client.rs:6`, `src/bin/ah.rs:503` | Keep as Unix; macOS supports them. Watch path length in tests. |
| FIFO + `O_NONBLOCK` | `src/rpc/handlers/agent.rs:182`, `:195`; `src/tmux/mod.rs:160`, `:339` | Keep POSIX FIFO. Verify Tokio readiness on macOS. |
| `flock` | `src/prompt_handler/kb.rs:6`, `:244` | Keep nix `flock`; confirm macOS support in CI. |
| symlink | `src/provider/home_layout.rs:504`, `:1213`, `:1237`, `:1326` | Keep existing `cfg(unix)` path. |
| Unix permissions | `src/rpc/handlers.rs:64`, `src/rpc/mod.rs:30`, `src/provider/home_layout.rs:9` | Keep with minor test coverage on macOS. |
| `geteuid` tmux socket path | `src/bin/ahd.rs:179`, `src/db/system.rs:1310`, `src/bin/ah.rs:989`, `src/tmux/mod.rs:67` | Keep initially; verify Homebrew tmux uses `/tmp/tmux-<uid>`. |
| direct `kill(pid,0)` | `src/monitor/agent_watch.rs:97`, `src/db/system.rs:1086` | POSIX, but not authoritative for pid identity. Use only as liveness hint under `ProcInfo`. |
| direct `kill(SIGTERM/SIGKILL)` | `src/rpc/handlers/system.rs:14`, `src/rpc/handlers/agent.rs:542`, fallback in `src/monitor/master_watch.rs:1361` | Keep as low-level signal primitive; authoritative cascade must be fenced by `ProcessReaper`. |
| `CommandExt::exec` | `src/bin/ah.rs:1006` | Unix-portable. |

Do not over-design these in PR-1. The port risk is not these calls.

#### Needs design: process supervision, pidfd to kqueue

Goal: preserve current process-watch semantics, including current limitations.

Current Linux semantics:
- A pidfd is acquired with `SYS_pidfd_open` (`src/monitor/mod.rs:17`) and stored in `PIDFD_REGISTRY` (`src/monitor/mod.rs:13`).
- Worker and master watcher tasks wait on pidfd readiness through Tokio `AsyncFd` (`src/monitor/agent_watch.rs:159`, `src/monitor/master_watch.rs:417`).
- Agent exit-code lookup uses `waitid(P_PIDFD, ..., WEXITED | WNOHANG)` (`src/monitor/agent_watch.rs:124`).
- tmux-launched workers are normally not children of ahd. The Linux code already handles `ECHILD` by logging "waitid(P_PIDFD) unavailable for non-child agent process" and returns no exit code (`src/monitor/agent_watch.rs:143`). This is a required compatibility behavior, not a bug to fix in the macOS port.

macOS design:
- `MacProcessWatcher` registers `EVFILT_PROC` with `NOTE_EXIT` for `pid`.
- Registration must capture a `ProcessIdentity`: pid plus DB generation and, when available, start-time from `sysctl KERN_PROC_PID`/`libproc`.
- Every exit event is accepted only if the current process identity still matches the registered identity. This is the explicit replacement for pidfd's identity token.
- For non-child processes, return `ProcessExit::Exited { exit_code: None }`. Do not use shell tricks or child wait assumptions to synthesize exit code. This preserves Linux's non-child `ECHILD` behavior.
- For child processes that ah directly spawned, `waitpid` may provide exit status, but the trait should still expose `Option<i32>` so callers do not depend on macOS-only stronger data.
- `master_process_is_alive` becomes a trait method. On Linux it remains `pidfd_open(pid).is_ok()`. On macOS it checks `kill(pid,0)` plus process identity if known. If only pid is known, result is "alive hint" rather than authoritative identity proof.

Pid reuse handling:
- DB rows already carry session/agent ids and master generation. Watch registration should store expected generation and pid.
- For macOS, the platform layer must also capture start-time at registration and re-read it before destructive actions.
- Reaper methods must accept `ProcessIdentity`, not raw pid, where the caller has one. Raw pid kill remains allowed only for legacy fallback paths that already lack pidfd authority today.

Required invariants:
- Linux behavior does not change: pidfd readiness, pidfd SIGKILL, and `ECHILD => exit_code=None` stay intact.
- macOS exit detection is comparable to Linux pidfd readiness, but exit-code availability remains optional and often `None`.
- Stale watcher events must not mutate a newer generation's DB state.

#### Needs design: anti-orphan cascade, systemd BindsTo to mac ownership tree

This is the highest-risk subsystem.

Current Linux guarantee:
- Master, worker, and tmux commands are wrapped in systemd user scopes (`src/sandbox/systemd.rs:13`, `:72`, `:104`; `src/tmux/scope.rs:21`).
- Scopes use `BindsTo` and `PartOf` to tie lifetime to the ahd service or anchor unit (`src/sandbox/systemd.rs:142`; `src/tmux/scope.rs:40`).
- Cascade paths stop systemd scopes before pid fallback (`src/db/system.rs:258`, `:436`).
- Startup reconcile lists `systemctl --user list-units --type=scope` and stops orphan scopes (`src/db/system.rs:557`, `:621`).
- Session watch uses anchor unit active/inactive as cascade input (`src/monitor/session_watch.rs:163`).
- DB recovery windows protect healthy master revival from over-eager cascade while still enforcing eventual reap.

macOS candidate:
- Use process groups as the containment primitive. Spawn each tmux server, master, and worker under an ah-owned process group/session where possible.
- Store durable ownership records in DB for every platform scope: session id, agent id, pane id, pid, process group id, generation, created_at, and owner marker.
- On normal cascade, call `killpg(pgid, SIGKILL)` for the owned group, then per-pid fallback through `ProcessReaper`.
- On ahd startup, run reconcile before normal operation: load live DB ownership records, verify pids/pgids via `ProcInfo`, and reap orphan groups whose owner session/agent is no longer live or whose recovery window has expired.
- For ahd crash/restart, there is no macOS equivalent of systemd automatically killing `BindsTo` scopes. The replacement guarantee is "bounded orphan lifetime": launchd restarts ahd; startup reconcile scans ownership records and reaps stale groups before accepting normal work.
- For active master recovery, reuse the existing recovery-window invariant: unexpired recovery windows prevent false cascade; expired windows release cascade. Startup reconcile must check recovery windows before orphan group cleanup, matching the current Linux Phase 4 ordering.

Linux BindsTo vs mac ownership tree:

| Property | Linux today | macOS proposed | Residual risk |
| --- | --- | --- | --- |
| ahd dies | systemd service relationship can cascade scopes. | launchd restarts ahd; startup reconcile reaps stale process groups from durable ownership records. | Orphans can live until ahd restarts and reconcile runs. |
| session killed | stop matching systemd scopes, then pidfd fallback. | kill owned process groups, then per-process fallback. | Must ensure process group contains disowned grandchildren; shell/provider may call `setsid`. |
| worker/master not child of ahd | pidfd watches and systemd scopes still work. | kqueue watches by pid and DB-owned pgid records. | pid reuse requires start-time/generation fencing. |
| startup orphan cleanup | systemctl list scopes and parse descriptions. | scan DB ownership records and live process table; optionally scan ah-owned pgid markers. | DB loss would weaken cleanup unless runtime marker files also exist. |
| recovery window defer | DB `master_recovery_windows` prevents home wipe and enforces expiry. | Same DB state machine; platform reaper consults it before killing groups. | Reconcile ordering must remain strict. |
| healthy revived session | P3/P4/R2/R3 guards keep live master/ack-complete sessions from cascade. | Same guards, with mac `ProcessWatcher` liveness and readiness signals. | `killpg` must be generation-fenced to avoid killing a newer healthy group. |

Residual risk and mitigation:
- Process groups do not automatically die with ahd. This is a real semantic gap versus systemd. The mitigation is persistent ownership plus launchd restart plus startup reconcile, not pretending process groups are equivalent.
- Some providers may spawn grandchildren outside the group. The mac implementation should prefer launching provider shells as process-group leaders and test disowned grandchildren. If providers escape with `setsid`, only user-space discovery/reaper can catch them.
- If DB is corrupted or deleted, ownership records may be unavailable. Consider a secondary runtime marker directory per group in later implementation, but keep PR-1 focused on extraction.

#### No corresponding macOS behavior

| Linux behavior | Current site | macOS platform strategy |
| --- | --- | --- |
| `OOMScoreAdjust=-900` for ahd unit | `src/cli/service_unit.rs:68` | No-op on macOS with one-line log: "OOMScoreAdjust unsupported on macOS". |
| `--property=OOMScoreAdjust=-900` transient ahd | `src/cli/start.rs:55` | No-op in mac supervisor. |
| `/proc/self/oom_score_adj=500` for master | `src/sandbox/systemd.rs:251` | No-op/degraded log. Do not claim OOM preference on macOS. |
| systemd slices | `src/sandbox/systemd.rs:41`, `:159`; `src/tmux/scope.rs:36` | No-op. Use DB ownership/project labels for accounting only. |
| `BindReadOnlyPaths=` | `src/sandbox/systemd.rs:150` | No direct equivalent. macOS starts with degraded sandbox isolation; log once when a requested read-only bind is ignored. |
| `loginctl enable-linger` note | `src/cli/service_bootstrap.rs:402` | No-op; launchd has different login/boot semantics. |
| legacy transient `ahd.service` migration | `src/cli/service_bootstrap.rs:271` | Linux-only migration. macOS has no legacy systemd transient service. |
| cgroup path identity | `src/systemd_unit.rs:1`; `src/bin/ah.rs:538`, `:635` | Replace with explicit daemon identity marker; no cgroup equivalent. |

Platform strategy: no corresponding behavior must be explicit. The macOS backend should log once at startup or command render time for degraded resource controls/sandbox controls, not silently pretend to enforce them.

## 2. Linux Zero-Regression Rule

The first implementation PR is extraction only:
- `#[cfg(target_os = "linux")]` platform implementation calls the current functions and commands with current arguments.
- Existing functions can temporarily become thin wrappers that call `platform::linux::*` to keep public API stable while call sites migrate.
- Tests should prove byte-for-byte command generation where existing tests do so today, especially `systemd-run`, unit rendering, `BindsTo`, `PartOf`, `OOMScoreAdjust`, and cgroup parsing.
- Full Linux test suite must remain green, not only `--lib`. This means `cargo test` including `tests/` integration tests is the PR-1 gate.

Non-negotiable Linux invariants:
- Worker/master process watch behavior remains pidfd-backed.
- Non-child worker exit code remains `None` when `waitid(P_PIDFD)` returns `ECHILD`.
- systemd scope stop remains the authoritative subtree reap path before pidfd fallback.
- `master_recovery_windows` continues to enforce invariant A/B: revive windows defer cascade only while bounded; expired/failed windows eventually cascade; healthy recovered sessions are not killed.
- Persistent ahd user service behavior remains systemd user service based.

## 3. Test Strategy

The repository currently has more than the 16 integration files called out in the task prompt. Grep found 23 `tests/` files referencing pidfd/systemd/proc/scope terms. Treat all of them explicitly.

### 3.1 Integration test inventory and platform plan

| Test file | Linux-only reference | Strategy |
| --- | --- | --- |
| `tests/common/mod.rs:33` | `systemd-run --user --scope` probe and `ScopePolicy::Systemd` helper. | Split helper into platform test capability. Linux keeps current helper; mac returns mac scope policy or skips systemd-only tests. |
| `tests/pr7_tests_first.rs:12`, `:471`, `:681`, `:710` | Direct `pidfd_open`, pidfd watch, systemd-run command, cgroup recursion tests. | Split: pidfd watcher tests Linux-only; recursion guard gets platform-specific identity tests; command rendering remains Linux-only. |
| `tests/prompt_handler_e2e.rs:21`, `:371` | Direct pidfd registry/watch in prompt pending test. | Add process-watcher trait test harness; Linux pidfd test remains; mac gets kqueue-backed equivalent. |
| `tests/mvp2_acceptance.rs:148`, `:159`, `:259` | `/proc/{pid}` liveness/comm and pidfd external death. | Replace proc helpers with `ProcInfo` test helper; pidfd-specific assertions Linux-only; mac watcher equivalent expected `exit_code=None`. |
| `tests/ah_full_e2e_drift.rs:618` | `/proc/<pid>` existence. | Use platform liveness helper. |
| `tests/ah_full_e2e_realign_extra.rs:618`, `:1057` | `/proc/<pid>` existence and pidfd non-child `NULL` exit-code note. | Use platform liveness helper; keep non-child `exit_code=None` assertion on both platforms. |
| `tests/mvp6_acceptance.rs:12`, `:326` | systemd scope helper and pidfd kill pane test. | Gate scope-specific path; keep platform-neutral pane cleanup test. |
| `tests/mvp7_acceptance.rs:13` | scope policy helper. | Platform capability helper. |
| `tests/mvp7_real_codex.rs:39`, `:144` | scope policy and systemctl cleanup. | Real provider Linux scope cleanup gate; later mac real-provider job uses mac backend. |
| `tests/mvp8_acceptance.rs:14` | scope policy helper. | Platform capability helper. |
| `tests/mvp8_real_codex.rs:40`, `:173` | scope policy and systemctl cleanup. | Gate Linux cleanup; add mac cleanup once backend exists. |
| `tests/mvp9_acceptance.rs:18` | scope policy helper. | Platform capability helper. |
| `tests/mvp9_real_codex_claude.rs:44`, `:222` | scope policy and systemctl cleanup. | Gate Linux cleanup; later mac real-provider coverage. |
| `tests/mvp10_acceptance.rs:48`, `:77`, `:180`, `:196`, `:219`, `:285` | tmux scope, `/proc/<pid>/cgroup`, wrapper `systemd-run`, `systemctl stop/list-units`. | Linux-only for true systemd cascade; mac gets separate process-group cascade test. |
| `tests/mvp11_acceptance.rs:10`, `:98`, `:142`, `:248` | systemd capability, master pidfd watch, anchor stop via systemctl. | Split DB/recovery assertions platform-neutral; anchor stop Linux-only until mac anchor exists. |
| `tests/mvp11_real_codex.rs:40`, `:173` | scope policy and systemctl cleanup. | Gate Linux cleanup; later mac real-provider job. |
| `tests/mvp11_real_claude.rs:40`, `:176` | scope policy and systemctl cleanup. | Gate Linux cleanup; later mac real-provider job. |
| `tests/orphan_reap.rs:10`, `:98`, `:207`, `:230`, `:252` | systemd scope discovery, cgroup pids, `systemctl is-active`. | Keep as Linux-only anti-orphan acceptance; add mac process-group orphan-reap acceptance in later PR. |
| `tests/r1_bindsto_alignment.rs:1`, `:20` | asserts `BindsTo`/`PartOf` command args. | Linux-only command generation test; mac equivalent asserts ownership record/process-group config. |
| `tests/r1_master_exit_shutdown.rs:32`, `:414` | systemctl stop and `/proc` scan. | Gate Linux scope shutdown; use platform liveness helper for process scan. |
| `tests/r1_shutdown_cleanup.rs:2` | `ScopePolicy` usage. | Platform helper. |
| `tests/r2_master_scope_spawn.rs:19`, `:315`, `:363`, `:388` | explicit ignored systemd dogfood, `systemctl list-units`, `/proc/<pid>/oom_score_adj`. | Keep Linux-only ignored/manual; add mac manual dogfood later for process-group ownership. |
| `tests/pr1b_readfirst_hook.rs:165` | Only matched by "scope" in test name/text, not OS-specific code. | Platform-neutral. |

### 3.2 Test tiers

Linux PR-1 gate:
- `cargo test` full suite on Linux.
- Existing ignored/manual systemd dogfood remains ignored unless explicitly invoked.
- Add no mac behavior yet.

mac compile gate, first mac implementation PR:
- `cargo test --lib` for platform-neutral pure logic.
- `cargo test --test <platform-neutral integration subset>` for tests that do not require systemd or real providers.
- Systemd-specific files gated with `#[cfg(target_os = "linux")]` or runtime capability skip.

mac backend gate after process/scope implementation:
- Add kqueue watcher tests mirroring pidfd watcher tests.
- Add process-group orphan reap test mirroring `tests/orphan_reap.rs`.
- Add launchd service render/bootstrap pure tests before real launchd e2e.

Real provider mac job:
- Separate optional GitHub Actions job, not part of first mac compile gate.
- Install `tmux` on macOS runner, likely via Homebrew.
- Run non-destructive provider/mock tests first; real Codex/Claude tests stay opt-in because auth is external.

### 3.3 CI plan

Phase CI:
1. PR-1 extraction: Linux CI unchanged; full `cargo test` including integration tests.
2. mac compile PR: add GitHub Actions `macos-latest` job running `cargo test --lib` and selected platform-neutral integration tests. Do not add cargo-dist mac targets yet.
3. mac process/scope PRs: expand mac job to process watcher and process-group orphan tests.
4. release PR: only after mac CI is meaningful, add `aarch64-apple-darwin` and optionally `x86_64-apple-darwin` to cargo-dist targets.

## 4. PR Slicing

### PR-1: platform extraction, Linux only

Goal: introduce `src/platform/` traits and Linux implementations by moving existing code, with zero behavior change.

Scope:
- Move pidfd helpers into `platform::linux::process`, preserving public wrapper functions where needed.
- Move systemd command construction/runners into `platform::linux::scope` and `platform::linux::service`, preserving current public functions and command output.
- Move cgroup daemon identity into `platform::linux::identity`.
- Move `/proc` zombie/liveness helpers into `platform::linux::proc_info`.
- Add `#[cfg(target_os = "linux")]` only where needed to compile the Linux implementation.
- No macOS implementation beyond `compile_error!` or explicit stubs if unavoidable; prefer not compiling mac in PR-1 if that keeps review smaller.

Acceptance:
- Linux full `cargo test` passes.
- `cargo build --release --bin ah --bin ahd` passes.
- No behavior changes in command-rendering snapshots/assertions.

### PR-2: mac compile skeleton

Goal: make the crate compile on macOS with explicit unsupported stubs for risky subsystems.

Scope:
- Add mac `ProcessWatcher`/`ProcessReaper` skeleton for compile, not full lifecycle support.
- Mark unsupported service/scope operations with clear errors.
- Gate Linux-only integration tests.

Acceptance:
- Linux full suite still green.
- mac `cargo test --lib` compiles/runs platform-neutral tests.

### PR-3: mac process watcher

Goal: implement kqueue process exit detection.

Scope:
- kqueue `EVFILT_PROC` watcher.
- `ProcessIdentity` start-time fencing.
- Exit event maps to `exit_code=None` for non-child processes.

Acceptance:
- mac watcher tests for external death and stale pid/generation no-op.
- Linux pidfd tests still green.

### PR-4: mac ownership/cascade

Goal: implement process-group ownership tree and startup reconcile.

Scope:
- Spawn tmux/master/worker in owned groups.
- Durable DB ownership records.
- `killpg` cascade with per-pid fallback.
- Startup reconcile ordering with recovery windows before orphan cleanup.

Acceptance:
- mac orphan-reap test equivalent to `tests/orphan_reap.rs`.
- Recovery-window invariant tests pass on both platforms.

### PR-5: mac service supervisor

Goal: install/start ahd as a macOS user service.

Scope:
- LaunchAgent plist renderer.
- `launchctl bootstrap/bootout/kickstart`.
- Stale plist GC.
- Explicit no-op logs for Linux-only OOM/slice/linger behavior.

Acceptance:
- Pure plist render tests.
- Optional local/manual launchd dogfood.

### PR-6: mac release target

Goal: publish mac artifacts only after runtime support is real.

Scope:
- Add cargo-dist mac targets.
- Add mac CI release validation.

Acceptance:
- cargo-dist plan includes mac targets.
- mac CI green for selected tests.

## 5. PM Decision Points

1. Cascade ownership model: choose process groups + DB reconcile as MVP, or require launchd jobs per session/agent for stronger OS-managed ownership.
2. Orphan lifetime tolerance on macOS: accept bounded orphan lifetime until launchd restarts ahd and startup reconcile runs, or require a separate watchdog/helper for faster reap.
3. DB-only ownership records vs DB plus runtime marker files: DB-only is simpler; marker files improve recovery if DB state is missing/corrupt.
4. Sandbox degradation: approve macOS no-op for `BindReadOnlyPaths` and OOM controls with explicit warning, or require a mac sandbox mechanism before public mac support.
5. Exit-code semantics: preserve current `None` for non-child process exits, or invest in provider-specific child ownership to capture more exit data. This design recommends preserving current semantics.
6. launchd timing: implement persistent ahd service after process watcher/cascade, not before. PM should confirm order.
7. CI rollout: add mac compile CI before mac release artifacts. PM should confirm when to add cargo-dist mac targets.

## 6. Completion Boundary

This design phase stops here. No implementation should start until PM approves the abstraction boundary and the cascade ownership model.

Implementation must preserve:
- Linux full-suite behavior.
- pidfd non-child `exit_code=None` semantics.
- cascade-coord invariant A: failed/expired recovery eventually reaps worker/master/tmux ownership.
- cascade-coord invariant B: active recovery windows and healthy completed revival do not get falsely cascaded.
- Explicit platform logs for unsupported macOS substitutes rather than silent degradation.
