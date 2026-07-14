# macOS Native Support Phase 1 Research

Status: inventory only. This document records Linux-only dependency points found by grep plus code reads. It does not propose implementation architecture beyond first-pass replacement candidates.

Method:
- Grep terms: `pidfd`, `P_PIDFD`, `SYS_pidfd_open`, `SYS_pidfd_send_signal`, `waitid`, `with_borrowed`, `master_process_is_alive`.
- Grep terms: `systemd-run`, `systemctl`, `loginctl`, `--scope`, `BindsTo`, `PartOf`, `OOMScoreAdjust`, `systemd/user`, `scope`, `slice`, `unit`.
- Grep terms: `/proc/`, `/proc/self`, `cgroup`, `oom_score_adj`.
- Grep terms: `std::os::unix`, `tokio::signal::unix`, `tokio::io::unix`, `nix::`, `libc::`, `SYS_`, `P_PIDFD`, `prctl`, `unshare`, `setns`, `memfd`, `inotify`, `epoll`, `CLONE_`, `SIGCHLD`, `#[cfg`, `cfg!`.

Counts below are table rows / grouped runtime-code sites, not individual test assertions:
- pidfd/process-watch sites: 30
- systemd/scope/service sites: 46
- `/proc` sites: 4 runtime groups
- other Unix/libc/nix/cfg sites: 17

## 1. pidfd and Process Supervision Inventory

| Site | Purpose | macOS candidate | Grade |
| --- | --- | --- | --- |
| `src/monitor/mod.rs:13` | Global `PIDFD_REGISTRY` keyed by monitor key; lets cleanup paths borrow an existing pidfd. | Replace registry value with platform process handle abstraction; on macOS store pid + kqueue registration metadata. | 需设计 |
| `src/monitor/mod.rs:17` | `pidfd_open(pid)` calls `libc::SYS_pidfd_open`; Linux-only acquisition of process fd. | kqueue `EVFILT_PROC` registration by pid for exit notification; no fd equivalent. | 需设计 |
| `src/monitor/mod.rs:44` | `pidfd_send_sigkill` calls `libc::SYS_pidfd_send_signal` with `SIGKILL`. | `kill(pid, SIGKILL)` or `killpg` with generation/process-group fencing. | 需设计 |
| `src/monitor/mod.rs:57` | `register_pidfd` inserts owned pidfd for later watchers/reapers. | Register platform watcher handle; macOS likely pid + kqueue watch token. | 需设计 |
| `src/monitor/mod.rs:64` | `remove_pidfd` drops pidfd registration by key. | Remove kqueue watch/registry entry. | 直换 |
| `src/monitor/mod.rs:71` | `pidfd_contains` checks if a process handle is registered. | Registry check can be platform neutral. | 直换 |
| `src/monitor/mod.rs:79` | `with_borrowed_pidfd` duplicates the fd and lends it to a callback. | No pidfd on macOS; callback should borrow platform process handle or pid with generation guard. | 需设计 |
| `src/monitor/mod.rs:101` | `list_pidfd_keys` introspects active pidfd keys. | Platform-neutral registry introspection. | 直换 |
| `src/monitor/agent_watch.rs:6` | Agent watcher wraps pidfd in Tokio `AsyncFd` for readiness on process exit. | kqueue `EVFILT_PROC NOTE_EXIT`; likely use `mio`/Tokio unix fd integration with a kqueue wrapper. | 需设计 |
| `src/monitor/agent_watch.rs:97` | `kill(pid, 0)` liveness fallback for agents. | POSIX `kill(pid, 0)` exists on macOS; must account for pid reuse. | 直换 |
| `src/monitor/agent_watch.rs:117` | Reads `/proc/{pid}/stat` to classify zombie state during liveness checks. | `sysctl KERN_PROC` or `libproc` process info. | 需设计 |
| `src/monitor/agent_watch.rs:124` | `waitid(P_PIDFD, ...)` reads child exit code from pidfd. | `waitpid` only for child pids; kqueue gives exit note but not the same pidfd wait semantics. | 需设计 |
| `src/monitor/agent_watch.rs:159` | `spawn_agent_pidfd_watch_task` waits for pidfd readability and routes agent exit. | kqueue process-exit watcher task. | 需设计 |
| `src/monitor/master_watch.rs:121` | Patrol loop uses `master_process_is_alive` for active masters. | Platform process liveness abstraction. | 直换 |
| `src/monitor/master_watch.rs:144` | `master_process_is_alive` is implemented as `pidfd_open(pid).is_ok()`. | `kill(pid, 0)` plus process-start-time/generation guard, or kqueue registration state. | 需设计 |
| `src/monitor/master_watch.rs:159` | `arm_or_route_master_watch` opens pidfd, registers it, and spawns master pidfd watch. | kqueue watch registration for master pid. | 需设计 |
| `src/monitor/master_watch.rs:417` | `spawn_master_pidfd_watch_task` uses `AsyncFd` readiness to detect master exit. | kqueue event loop or platform watcher task. | 需设计 |
| `src/monitor/master_watch.rs:760` | Revived replacement master gets a pidfd, registry entry, and watch task after pane pid detection. | Same kqueue watcher registration after tmux pane pid lookup. | 需设计 |
| `src/monitor/master_watch.rs:1340` | Failed revive cleanup borrows pidfd and sends SIGKILL, falling back to `kill(SIGKILL)`. | `kill(pid, SIGKILL)`/`killpg` with generation fence; no pidfd signal. | 需设计 |
| `src/monitor/master_watch.rs:1510` | Revive ACK readiness verifies `master_process_is_alive(expected_pid)`. | Same process liveness abstraction. | 直换 |
| `src/monitor/master_watch.rs:1562` | Probe readiness verifies stored pid/generation, process liveness, pane pid, and capture stability. | Same process liveness abstraction; tmux capture is portable if tmux is present. | 直换 |
| `src/db/system.rs:240` | Master-death worker cleanup stops scopes then uses pidfd SIGKILL fallback for agents. | Process-group reaper or per-agent kill fallback; must preserve anti-orphan semantics. | 需设计 |
| `src/db/system.rs:400` | Cascade kill path stops scopes then uses pidfd fallback for pre-scope agents. | Process-group cascade + per-pid fallback; pid reuse/generation fencing required. | 需设计 |
| `src/db/system.rs:1152` | Startup reconcile re-registers pidfd watches for alive agents. | Re-register kqueue watches for alive pids. | 需设计 |
| `src/rpc/handlers/agent.rs:250` | Agent spawn opens pidfd for provider process. | Register kqueue watcher after spawn pid. | 需设计 |
| `src/rpc/handlers/agent.rs:325` | Agent spawn registers pidfd and starts `spawn_agent_pidfd_watch_task`. | Platform watcher task. | 需设计 |
| `src/rpc/handlers/agent.rs:542` | Direct `kill(pid, SIGKILL)` for a known agent pid. | POSIX `kill` exists; should be fenced by process identity. | 直换 |
| `src/rpc/handlers/sessions.rs:432` | Cutover master watch opens pidfd and spawns master watcher. | kqueue process watcher. | 需设计 |
| `src/rpc/handlers/sessions.rs:462` | Local cutover `master_process_is_alive` uses `pidfd_open`. | Shared process liveness abstraction. | 需设计 |
| `src/rpc/handlers/sessions.rs:638` | Cutover readiness rejects if the new master is no longer alive. | Same process liveness abstraction. | 直换 |

Notes:
- The hard Linux-only primitives are `SYS_pidfd_open`, `SYS_pidfd_send_signal`, and `waitid(P_PIDFD)`.
- macOS can watch pids with kqueue `EVFILT_PROC`/`NOTE_EXIT`, but it does not provide a pidfd identity/token that naturally solves pid reuse. Any replacement needs explicit generation or process-start-time fencing.

## 2. systemd, Scope, Unit, and User Service Inventory

| Site | Purpose | macOS candidate | Grade |
| --- | --- | --- | --- |
| `src/sandbox/mod.rs:33` | Detects whether sandbox enforcement is required by checking `systemd-run`, `INVOCATION_ID`, and unsafe bypass. | Platform sandbox capability abstraction; macOS likely no systemd path. | 需设计 |
| `src/sandbox/mod.rs:62` | Hard errors when `systemd-run`, `XDG_RUNTIME_DIR`, or user manager probe is missing. | macOS-specific supervisor/process-group availability check. | 需设计 |
| `src/sandbox/mod.rs:100` | Probes `systemctl --user is-system-running`. | launchd/session probe or no-op depending architecture. | 需设计 |
| `src/sandbox/systemd.rs:13` | Wraps worker provider command in `systemd-run --user --scope --collect`. | `setsid`/`setpgid` process group wrapper, launchd job, or internal supervisor. | 需设计 |
| `src/sandbox/systemd.rs:41` | Assigns workers to project slice `ccb-<project>-ccbd-agents.slice`. | No slice equivalent; process group/session tagging only. | 无对应 |
| `src/sandbox/systemd.rs:72` | Recovery worker wrapper adds systemd scope plus bind overrides. | Process group wrapper; bind mount read-only overlays have no direct macOS equivalent. | 需设计 |
| `src/sandbox/systemd.rs:104` | Wraps master provider command in systemd-run. | Same process group/supervisor abstraction. | 需设计 |
| `src/sandbox/systemd.rs:142` | Adds `--property=BindsTo=<unit>` and `--property=PartOf=<unit>` for daemon-cascaded teardown. | Core anti-orphan replacement: process-group tree, launchd dependency, or userspace supervisor. | 需设计 |
| `src/sandbox/systemd.rs:150` | Adds `BindReadOnlyPaths=` properties for read-only binds. | macOS sandbox-exec is deprecated; no direct systemd bind property. | 无对应 |
| `src/sandbox/systemd.rs:159` | Derives systemd slice names. | No direct macOS equivalent. | 无对应 |
| `src/sandbox/systemd.rs:248` | Master command shell writes `/proc/self/oom_score_adj`. | macOS has no Linux OOM score; skip or alternative priority/resource policy. | 无对应 |
| `src/tmux/scope.rs:1` | Defines `ScopePolicy::Systemd` vs `None` for tmux server process. | Platform-neutral scope policy enum with macOS process group/launchd variant. | 需设计 |
| `src/tmux/scope.rs:21` | Wraps tmux server in `systemd-run --user --scope --unit=ahd-tmux-* --slice=ahd-agents.slice`. | Start tmux in dedicated process group/session and track it. | 需设计 |
| `src/tmux/scope.rs:40` | Adds `BindsTo`/`PartOf` to tie tmux server to ahd daemon unit. | Process-group cascade or launchd relationship. | 需设计 |
| `src/tmux/scope.rs:55` | Detects scope policy from current daemon unit and `systemd-run --user --scope -- true`. | macOS capability detection for process groups/supervisor. | 需设计 |
| `src/tmux/session.rs:19` | Tmux server construction auto-detects scope policy. | Platform-aware policy detection. | 需设计 |
| `src/systemd_unit.rs:1` | Reads cgroup to detect current daemon service unit. | No cgroups on macOS; use env/launchd label/daemon registry. | 需设计 |
| `src/systemd_unit.rs:16` | Defines daemon unit recognition as `ahd.service` or `ah-*.service`. | Platform-neutral daemon identity marker needed. | 需设计 |
| `src/cli/service_unit.rs:22` | Derives persistent user service unit name `ah-<hash>.service`. | launchd label/plist name or supervisor config name. | 需设计 |
| `src/cli/service_unit.rs:39` | Renders persistent systemd user unit with `Restart=on-failure`, `OOMScoreAdjust`, env, install target. | launchd plist with `KeepAlive`, env, program args; no OOMScore. | 需设计 |
| `src/cli/service_unit.rs:83` | Resolves systemd user unit dir under XDG/HOME. | `~/Library/LaunchAgents` for launchd, if launchd is chosen. | 需设计 |
| `src/cli/service_bootstrap.rs:12` | `SystemctlRunner` executes `systemctl --user`. | launchctl runner or platform service runner. | 需设计 |
| `src/cli/service_bootstrap.rs:59` | User manager availability checks `XDG_RUNTIME_DIR` and `systemctl --user is-system-running`. | launchd availability or no-op; macOS user launchd is always present in login context. | 需设计 |
| `src/cli/service_bootstrap.rs:95` | Persistent ahd install flow: write unit, `daemon-reload`, `reset-failed`, `enable`, `restart`. | Write launchd plist, `launchctl bootstrap/bootout/kickstart`; semantics differ. | 需设计 |
| `src/cli/service_bootstrap.rs:159` | Stale `ah-*.service` GC disables `--now` and deletes unit file. | Stale LaunchAgent GC with `launchctl bootout` and plist delete. | 需设计 |
| `src/cli/service_bootstrap.rs:271` | Migrates legacy transient `ahd.service` using `systemctl show ... Transient` and `stop`. | No direct legacy equivalent unless prior macOS service exists. | 无对应 |
| `src/cli/service_bootstrap.rs:402` | Uses `loginctl show-user ... Linger` for boot-before-login note. | macOS launchd has different login/boot agents; no linger. | 无对应 |
| `src/cli/start.rs:30` | Builds transient `systemd-run --user --unit=ahd.service` bootstrap fallback. | launchd bootstrap/kickstart or direct spawn fallback. | 需设计 |
| `src/cli/start.rs:74` | Best-effort `systemctl --user reset-failed` for a unit. | launchctl has different failed-state handling. | 需设计 |
| `src/bin/ah.rs:537` | `ah start` derives persistent systemd unit and bootstraps persistent service. | launchd/supervisor backend. | 需设计 |
| `src/bin/ah.rs:581` | Fallback transient ahd systemd-run bootstrap. | Direct spawn or launchd transient job. | 需设计 |
| `src/bin/ah.rs:633` | Detects nested systemd environment via cgroup markers to avoid recursion. | Platform daemon identity/parent marker. | 需设计 |
| `src/bin/ahd.rs:34` | Warns if ahd is not under systemd and sandbox is enabled. | Platform supervisor warning. | 需设计 |
| `src/bin/ahd.rs:52` | Tmux server is created with detected daemon unit for BindsTo propagation. | Platform daemon identity propagation. | 需设计 |
| `src/bin/ahd.rs:247` | On shutdown, stops systemd anchor units via `systemctl --user stop`. | Stop process groups or launchd jobs. | 需设计 |
| `src/db/system.rs:258` | Worker cleanup lists systemd scopes and stops matching agent scopes. | Process-group registry/reaper or launchd job listing. | 需设计 |
| `src/db/system.rs:436` | Cascade path lists and stops agent scopes before pid fallback. | Core anti-orphan cascade replacement. | 需设计 |
| `src/db/system.rs:557` | `RealSystemctlRunner` lists `systemctl --user list-units --type=scope` and stops units. | launchctl/process registry runner. | 需设计 |
| `src/db/system.rs:621` | Startup reconcile stops orphan systemd scopes. | Startup reconcile for orphan process groups/launchd jobs. | 需设计 |
| `src/db/system.rs:835` | Parses `systemctl list-units` scope output. | Replace with platform runner output parser or internal registry. | 需设计 |
| `src/monitor/session_watch.rs:163` | Checks anchor unit active via `systemctl is-active`. | Process group/session anchor liveness or launchd job state. | 需设计 |
| `src/rpc/handlers/sessions.rs:163` | Master cutover starts new master in `systemd-run --user --scope --unit=<cutover>`. | Process-group/launchd-scoped cutover launch. | 需设计 |
| `src/rpc/handlers/sessions.rs:192` | Rollback cutover stops the cutover systemd unit. | Stop process group/job for cutover candidate. | 需设计 |
| `src/cli/doctor.rs:34` | Doctor requires `systemd-run` along with tmux. | macOS doctor should check platform backend instead. | 需设计 |
| `src/error.rs:244` | Test/error shape assumes `"systemd-run missing"`. | Platform-specific validation error. | 直换 |
| `src/ahd_test_helper.rs:39` | Test helper detects/overrides scope policy and binds tmux scope. | Test-only backend abstraction. | 需设计 |

Notes:
- `BindsTo`/`PartOf` is the anti-orphan backbone: ahd death cascades to tmux/master/worker scopes. macOS has no direct equivalent. This is the main design risk.
- `systemd-run --scope` is used for both lifecycle containment and observability/reconciliation. A macOS port needs a replacement for both, not only process launch.

## 3. `/proc` Filesystem Inventory

| Site | Purpose | macOS candidate | Grade |
| --- | --- | --- | --- |
| `src/monitor/agent_watch.rs:117` | Reads `/proc/{pid}/stat` to detect zombie process state (`Z`). | `sysctl KERN_PROC_PID`, `libproc`, or `kinfo_proc` state fields. | 需设计 |
| `src/sandbox/systemd.rs:251` | Master shell writes `500` to `/proc/self/oom_score_adj`; tests assert presence at `src/sandbox/systemd.rs:435`, `:454`, `:472`, `:501`. | No macOS OOM score; omit or model as unsupported resource policy. | 无对应 |
| `src/systemd_unit.rs:2` | Reads `/proc/self/cgroup` to derive current systemd daemon unit. | No cgroups; use env marker, launchd label, or daemon registry. | 需设计 |
| `src/bin/ah.rs:538` and `src/bin/ah.rs:635` | Reads `/proc/self/cgroup` for persistent-unit recursion guard and nested-environment detection. | Same platform daemon identity marker as above. | 需设计 |

## 4. Other libc/nix/Unix Calls and cfg Status

### Linux-only or likely macOS-impacting calls

| Site | Purpose | macOS candidate | Grade |
| --- | --- | --- | --- |
| `src/monitor/agent_watch.rs:125` | Uses `libc::siginfo_t` with pidfd `waitid`. | Replace as part of kqueue/watch abstraction. | 需设计 |
| `src/monitor/agent_watch.rs:129` | Calls `libc::waitid(libc::P_PIDFD, ...)`; `P_PIDFD` is Linux-only. | kqueue event plus optional `waitpid` only for child processes. | 需设计 |
| `src/monitor/mod.rs:20` | Calls `libc::syscall(libc::SYS_pidfd_open, ...)`. | kqueue watcher registration. | 需设计 |
| `src/monitor/mod.rs:46` | Calls `libc::syscall(libc::SYS_pidfd_send_signal, ...)`. | `kill`/`killpg` with identity fencing. | 需设计 |
| `src/sandbox/systemd.rs:251` | Writes Linux `/proc/self/oom_score_adj`. | No equivalent. | 无对应 |

### Unix-portable calls that still need an abstraction review

| Site | Purpose | macOS candidate | Grade |
| --- | --- | --- | --- |
| `src/monitor/agent_watch.rs:6`, `src/monitor/master_watch.rs:37`, `src/agent_io/reader.rs:7` | Tokio `AsyncFd` for pidfds and FIFOs. | Works for fds on Unix, but pidfd use must be replaced; FIFOs likely remain. | 需设计 |
| `src/monitor/agent_watch.rs:97`, `src/db/system.rs:1086` | `kill(pid, 0)` liveness probe. | POSIX; add pid-reuse fencing if used for authority. | 直换 |
| `src/monitor/master_watch.rs:1361`, `src/rpc/handlers/agent.rs:542`, `src/rpc/handlers/system.rs:14` | Direct `kill(SIGKILL/SIGTERM)`. | POSIX; prefer process-group kill for cascade. | 直换 |
| `src/bin/ahd.rs:179`, `src/db/system.rs:1310`, `src/bin/ah.rs:989`, `src/cli/doctor.rs:217`, `src/prompt_handler/integration.rs:843`, `src/tmux/mod.rs:67` | Uses `libc::geteuid()` to locate `/tmp/tmux-<uid>` sockets. | POSIX `geteuid` exists; tmux socket layout likely same on macOS if tmux uses default. | 直换 |
| `src/db/system.rs:19`, `src/db/system.rs:1062`, `src/rpc/handlers/agent.rs:31`, `src/rpc/handlers/agent.rs:195`, `src/tmux/mod.rs:52`, `src/tmux/mod.rs:339` | `OpenOptionsExt::custom_flags(libc::O_NONBLOCK)` for FIFO/nonblocking reads. | POSIX `O_NONBLOCK` exists on macOS. | 直换 |
| `src/rpc/handlers/agent.rs:27`, `src/rpc/handlers/agent.rs:182`, `src/tmux/mod.rs:50`, `src/tmux/mod.rs:160`, `src/tmux/mod.rs:335` | `nix::unistd::mkfifo` with `Mode` for agent/tmux FIFO creation. | POSIX FIFO exists. | 直换 |
| `src/prompt_handler/kb.rs:6`, `src/prompt_handler/kb.rs:244` | `nix::fcntl::flock` for file locking. | `flock` exists on macOS; verify nix feature/platform support. | 直换 |
| `src/bin/ahd.rs:16` | Tokio Unix signal handling (`SIGTERM`, etc.). | Tokio supports unix signals on macOS; signal set may need review. | 直换 |
| `src/rpc/mod.rs:5`, `src/cli/rpc_client.rs:6`, `src/bin/ah.rs:503`, `src/bin/ah.rs:569` | Unix domain sockets for JSON-RPC. | Unix sockets work on macOS. | 直换 |
| `src/rpc/handlers.rs:64`, `src/rpc/mod.rs:30`, `src/provider/home_layout.rs:9` | Unix permissions (`PermissionsExt`) for socket/home files. | Available on macOS; mode semantics mostly portable. | 直换 |
| `src/provider/home_layout.rs:504`, `src/provider/home_layout.rs:1213`, `src/provider/home_layout.rs:1237`, `src/provider/home_layout.rs:1326`, `src/cli/service_unit.rs:199` | Unix symlink creation in home materialization/tests. | Available on macOS. | 直换 |
| `src/bin/ah.rs:1006` | `std::os::unix::process::CommandExt::exec` for attach handoff. | Available on macOS. | 直换 |

No matches were found for `prctl`, `unshare`, `setns`, `memfd`, `inotify`, `epoll`, `CLONE_`, or `SIGCHLD` outside the grep terms listed above.

### Conditional compilation status

Observed OS/platform cfg:
- `src/rpc/mod.rs:28`: `#[cfg(unix)]` only for setting socket file permissions.
- `src/provider/home_layout.rs:502`, `:1211`, `:1235`, `:1324`: `#[cfg(unix)]` symlink paths, with `#[cfg(not(unix))]` fallbacks at `:1239` and `:1329`.
- Many `#[cfg(test)]` / `#[cfg(not(test))]` hooks exist in monitor/db/orchestrator modules.

Conclusion: there is currently no `#[cfg(target_os = "linux")]` / `#[cfg(target_os = "macos")]` portability boundary around process supervision, systemd, cgroup, or `/proc` behavior. The existing production code largely assumes Linux+systemd and Unix sockets.

## 5. Build and Release Surface

| Site | Current state | macOS implication | Grade |
| --- | --- | --- | --- |
| `Cargo.toml:28` | Direct dependency `libc = "0.2"`. | `libc` supports macOS, but Linux constants/syscalls used in code will not compile on macOS without cfg/abstraction. | 需设计 |
| `Cargo.toml:38` | Direct dependency `nix = { version = "0.28", features = ["fs"] }`. | Used for FIFO and flock; likely portable, but pidfd code does not use nix. | 直换 |
| `Cargo.toml:58` | `[workspace.metadata.dist]` cargo-dist config. | Already single-target Linux in current checkout. | 直换 |
| `Cargo.toml:69` | `targets = ["x86_64-unknown-linux-gnu"]`. | To publish macOS, add `x86_64-apple-darwin` and/or `aarch64-apple-darwin` after code compiles and tests have macOS backend. | 需设计 |
| `Cargo.toml:76` | `[profile.dist]` inherits release with thin LTO. | Portable if builds compile. | 直换 |
| `.github/workflows/release.yml:90` | cargo-dist dynamic artifact matrix generated from `dist plan`. | macOS runners would be introduced by cargo-dist when targets are added. | 直换 |
| `.github/workflows/release.yml:139` | Workflow has platform-specific dependency install step generated by cargo-dist. | Must verify generated macOS runner has tmux/system deps if release tests/build require them. | 需设计 |

Dependency notes:
- Direct dependencies do not include an obviously Linux-only crate by name.
- Transitive crates may include platform-specific support packages (for example `linux-raw-sys` or Windows target crates), but the active compile blocker is current source code references to Linux-only `libc` constants/syscalls and Linux/systemd runtime commands.
- CI macOS enablement is not only a cargo-dist target-list change. The code needs platform boundaries first for pidfd/systemd/`/proc` and tests need macOS-safe paths.

## 6. macOS Replacement Candidate Summary

| Linux-only subsystem | macOS candidate | Grade | Risk / difficulty |
| --- | --- | --- | --- |
| pidfd process exit watch (`pidfd_open`, `AsyncFd`, pidfd registry) | kqueue `EVFILT_PROC` with `NOTE_EXIT`, wrapped behind process watcher trait | 需设计 | macOS has no pidfd identity token; pid reuse and generation fencing must be explicit. |
| pidfd signal (`pidfd_send_signal`) | `kill(pid, sig)` / `killpg` | 需设计 | Direct pid kill is portable but less authoritative; process-group kill must avoid killing unrelated reused pids. |
| `waitid(P_PIDFD)` exit-code query | kqueue exit event plus `waitpid` only for child processes | 需设计 | Existing code handles non-child agents; macOS replacement may lose exit code for non-child processes. |
| systemd scopes for worker/master/tmux containment | process groups via `setsid`/`setpgid`, or userspace supervisor tree | 需设计 | This is the core anti-orphan cascade requirement; must replace BindsTo/PartOf semantics. |
| systemd `BindsTo`/`PartOf` daemon cascading | process-group ownership tree, launchd relationships, or supervisor-enforced cascade | 需设计 | Hardest point: ahd crash/restart must not leak worker/master/tmux children, and recovery windows must not kill healthy revived sessions. |
| persistent ahd user service (`ah-*.service`, `systemctl enable/restart`) | launchd LaunchAgent plist with `KeepAlive` / `launchctl bootstrap/kickstart` | 需设计 | launchd semantics differ from systemd user manager; migration/GC/linger equivalents need separate policy. |
| transient `systemd-run ahd.service` fallback | direct spawn or launchd transient job | 需设计 | Fallback must avoid double ahd and preserve lifecycle guarantees. |
| startup orphan scope reconcile | process registry + process-group sweep, or launchd job query | 需设计 | Needs durable ownership markers comparable to scope unit descriptions. |
| cgroup daemon-unit detection (`/proc/self/cgroup`) | environment marker, launchd label, or daemon registry | 需设计 | Required for recursion guard and nested detection; no cgroup equivalent. |
| `/proc/{pid}/stat` zombie check | `sysctl KERN_PROC` / `libproc` | 需设计 | Possible but needs platform-specific parser and state mapping. |
| `/proc/self/oom_score_adj` and systemd `OOMScoreAdjust` | none direct; maybe nice/priority/resource policy | 无对应 | Linux OOM adjustment has no macOS equivalent; likely disable on macOS. |
| `systemctl is-active` session anchor | process-group/session anchor liveness | 需设计 | Anchor active/inactive drives cascade decisions; must be authoritative. |
| Unix domain sockets | native Unix sockets | 直换 | Should work; path length and permission behavior need macOS testing. |
| FIFO (`mkfifo`, `O_NONBLOCK`, `AsyncFd`) | POSIX FIFO and nonblocking open | 直换 | Likely portable; Tokio readiness behavior should be tested on macOS. |
| tmux socket path `/tmp/tmux-<uid>` | same default tmux convention, verify on macOS | 直换 | Depends on Homebrew/system tmux defaults. |
| file locks (`flock`) | macOS `flock` | 直换 | Verify nix crate platform support. |
| symlinks and Unix permissions | macOS Unix fs APIs | 直换 | Mostly portable. |

Highest-risk design area: anti-orphan cascading. Today it is the composition of systemd scopes (`BindsTo`/`PartOf`), startup orphan-scope reconciliation, and pidfd-based kill fallback. macOS needs an equally authoritative ownership and reaping model, likely spanning process groups, kqueue watchers, durable ownership records, and startup reconcile. A simple `kill(pid)` substitution is not enough to preserve the current worker/master/tmux cascade guarantees.
