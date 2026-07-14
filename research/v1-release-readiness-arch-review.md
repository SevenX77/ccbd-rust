# ah (Agent Hypervisor) v1 Release-Readiness Architectural Health Review

**Lead Designer**: Antigravity  
**Date**: 2026-07-09  
**Status**: COMPLETE (Assessment Phase under Design SOP)

---

## 1. Executive Summary & Top Release-Impact Items

This review evaluates the architectural health of `ccbd-rust` as it is reshaped into **ah (Agent Hypervisor)**, a public orchestration substrate for external integrators (such as Graph Agent Studio, Copilot builders, and agentic IDEs). 

The approved v1 scope (defined in [.kiro/specs/ah-v1-public-release/design.md](file:///.kiro/specs/ah-v1-public-release/design.md)) focuses on splitting the rules kernel/scenario layer, auto-injecting rules, failing on unknown providers, and establishing pre-built binary releases. While these configurations are extracted, several runtime layers present critical release risks.

The top release-impact items have been audited, attributed, and classified. Of the P0 findings, the primary release-blockers for v1 ship are prioritized below:

1. **[P0#1] Broken Command Spawning on Windows (Execution Failure)**  
   * **Attribution**: **(b) Windows-native track** (covered by [.kiro/specs/ah-windows-native/m0-spec.md](file:///.kiro/specs/ah-windows-native/m0-spec.md)).
   * **Why it matters**: The Windows native wrapper ([src/platform/windows/scope.rs:154-168](file:///home/sevenx/coding/ccbd-rust/src/platform/windows/scope.rs#L154-L168)) appends raw `KEY=VALUE` environment variables directly preceding the command name in the execution array. Because Windows does not support Unix-like command-line environment prefixing, command execution fails immediately with an OS error, preventing any native Windows workers from launching.
   * **Remediation**: Re-route Windows command execution to set environment variables via Rust's native `Command::envs` API rather than prepending them to the argument list.

2. **[P0#2] Process Monitoring & Termination Stubbed on Windows & macOS (Zombie Process Leaks)**  
   * **Attribution**: **(b) Windows-native track** (partially covered by [.kiro/specs/ah-windows-native/m1-spec.md](file:///.kiro/specs/ah-windows-native/m1-spec.md) for Windows; macOS stubs are post-v1 debt).
   * **Why it matters**: Both `pidfd_open` and `pidfd_send_sigkill` return `EnvironmentNotSupported` on Windows ([src/platform/windows/process.rs:38-50](file:///home/sevenx/coding/ccbd-rust/src/platform/windows/process.rs#L38-L50)) and macOS ([src/platform/macos/process.rs:58-62](file:///home/sevenx/coding/ccbd-rust/src/platform/macos/process.rs#L58-L62)). Because process monitoring is stubbed, `ahd` cannot detect unexpected agent crashes on Windows. Furthermore, process group termination during restarts or session shutdowns fails silently ([src/db/system.rs:304-338](file:///home/sevenx/coding/ccbd-rust/src/db/system.rs#L304-L338)), leaking zombie process trees on the host system.
   * **Remediation**: Implement PGID-based reaping (`kill(-pgid, SIGKILL)`) on macOS and associate spawned Windows processes with native Job Objects for atomic termination.

3. **[P0#3] Gated STUCK State Dead-End for LogAndUi Providers (Perception Wedge)**  
   * **Attribution**: **(a) Orchestration-reliability spec** (governed by [research/orchestration-reliability-design.md](file:///home/sevenx/coding/ccbd-rust/research/orchestration-reliability-design.md) Section 3.2 "Classified Job Completion Model").
   * **Precise Failure Mode**: An agent stuck in the `STUCK` state is only allowed to resolve back to `IDLE` through log or hook completion events if `late_health_completion_stuck_allows_terminal` ([src/db/state_machine.rs:1142](file:///home/sevenx/coding/ccbd-rust/src/db/state_machine.rs#L1142)) returns `true`. This gate requires:
     1. The latest database entry in the `events` table for the agent must be a `state_change` event.
     2. The event payload's `reason` must exactly equal `"HEALTH_CHECK_STUCK"`.
     3. The payload's `signal_kinds` array must contain `"health:completion"`.
     4. The payload's `job_id` must match the active `dispatched_job_id`.
     If the agent is transitioned to `STUCK` by any other mechanism—such as a PTY marker timeout (`STUCK_TIMEOUT` in [src/db/state_machine.rs:448](file:///home/sevenx/coding/ccbd-rust/src/db/state_machine.rs#L448)) or a prompt-only diff mismatch—the gate returns `false`. All subsequent hook or log completion events are silently swallowed, keeping the agent permanently stuck and blocking the queue.
   * **Remediation**: Adjust the CAS update validation ([src/db/state_machine.rs:854-857](file:///home/sevenx/coding/ccbd-rust/src/db/state_machine.rs#L854-L857)) to allow transition from `STUCK` to `IDLE` upon receiving valid completion tokens from log or hook streams, regardless of how the agent entered the `STUCK` state.

4. **[P0#4] Non-Atomic SQLite Migrations Lack Transaction & Version Guards (Upgrade Fragility)**  
   * **Attribution**: **(c) Genuine NEW v1 release blocker** (not covered by existing tracks; directly impacts the Linux upgrade path for external integrators).
   * **Precise Failure Mode**: Rather than executing as a single atomic transaction, the schema initialization in [src/db/mod.rs:53-73](file:///home/sevenx/coding/ccbd-rust/src/db/mod.rs#L53-L73) executes `execute_batch(SCHEMA_DDL)` and then calls separate `migrate_*` functions sequentially (e.g. `migrate_sub_state`, `migrate_jobs_cancel_requested`). There is no transactional rollback wrapping the set of migrations. If a migration fails mid-run (e.g., table recreation fails or network/disk contention aborts execution), the schema is left in a partially migrated state. Because there is no schema version tracking table (like `schema_migrations`), on subsequent starts, the daemon attempts to run the migrations again, which can fail or crash due to structural discrepancies, permanently wedging the daemon.
   * **Remediation**: Wrap all migration functions inside a single, transaction-protected block, and introduce a `schema_migrations` metadata table to track applied changes.

---

## 2. Weakest Scope Areas

Based on our analysis, the two weakest areas of the architecture are:
1. **Cross-Platform Parity**: The codebase is fundamentally Linux-centric. The Windows and macOS implementations are thin compile-gate stubs that fail at runtime when executing process control or service supervision operations.
2. **Orchestration Liveness & State Integrity**: The reliance on asynchronous, non-atomic database reads and updates, combined with a lack of out-of-band process group tracking, leaves the state contract vulnerable to race conditions and "zombie" states (such as `PROMPT_PENDING` loops and false-positive `STUCK` blocks).

---

## 3. Comprehensive Architecture Review & Priorities

### 3.1. Orchestration & Dispatch

#### 3.1.1. Non-Atomic Dispatch Phase Races
* **Attribution**: **(a) Orchestration-reliability spec** (covered by [research/orchestration-reliability-design.md](file:///home/sevenx/coding/ccbd-rust/research/orchestration-reliability-design.md) Section 4 Mechanism 3).
* **Severity**: `should-fix-before-v1`
* **Evidence**: In [src/orchestrator/mod.rs:104-177](file:///home/sevenx/coding/ccbd-rust/src/orchestrator/mod.rs#L104-L177), the dispatch loop performs multiple asynchronous calls (`wait_for_dispatchable_idle`, `query_agent`, `run_dispatch_guard`) before executing the final `dispatch_queued_job` database write. 
* **Impact**: Concurrent client requests can race against the orchestrator, leading to double-dispatches or mismatched state records.
* **Remediation**: Wrap the dispatch sequence (from agent selection to job status write) inside a serializable database transaction or utilize an in-memory session lock to prevent overlapping dispatch tasks.

#### 3.1.2. Queue Block on `PROMPT_PENDING`
* **Attribution**: **(a) Orchestration-reliability spec** (covered by [research/orchestration-reliability-design.md](file:///home/sevenx/coding/ccbd-rust/research/orchestration-reliability-design.md) Section 3.2).
* **Severity**: `should-fix-before-v1`
* **Evidence**: In [src/db/jobs.rs:1866-1886](file:///home/sevenx/coding/ccbd-rust/src/db/jobs.rs#L1866-L1886), `claim_next_job_sync` explicitly skips agents whose state is `PROMPT_PENDING`.
* **Impact**: If a worker agent gets stuck in `PROMPT_PENDING` (due to an unhandled interactive prompt or a false positive), all subsequent jobs in its queue remain `QUEUED` indefinitely. There is no fallback timeout or error reporting to the caller, wedging the integrator's pipeline.
* **Remediation**: Introduce a `PROMPT_PENDING` expiration watchdog timer that auto-fails or requeues the active job after a configurable timeout (e.g. 5 minutes) and notifies the client.

---

### 3.2. State Contract & Database

#### 3.2.1. Global Database Connection Lock Contention
* **Attribution**: **(d) Post-v1 debt** (scalability optimization; does not break baseline function).
* **Severity**: `should-fix-before-v1`
* **Evidence**: The database wrapper in [src/db/mod.rs:74-77](file:///home/sevenx/coding/ccbd-rust/src/db/mod.rs#L74-L77) holds a single connection wrapped in a mutex `conn: Arc<Mutex<Connection>>`.
* **Impact**: Every read and write transaction across the entire daemon (RPC requests, tmux status sweeps, log parser writes) competes for a single global lock. SQLite blocks the tokio executor during disk I/O, causing high latency and lock timeouts under concurrent workloads.
* **Remediation**: Replace the single connection with a connection pool (e.g., using the `r2d2` or `sqlx` crate) and enable SQLite WAL (Write-Ahead Logging) mode to support concurrent readers alongside writers.

#### 3.2.2. Incomplete State Representation in `RuntimeSnapshot`
* **Attribution**: **(c) Genuine NEW v1 release blocker** (integrator visibility requirement).
* **Severity**: `should-fix-before-v1`
* **Evidence**: In [src/runtime_events.rs:71-83](file:///home/sevenx/coding/ccbd-rust/src/runtime_events.rs#L71-L83), `RuntimeSessionSnapshot` omits the `master_last_exit_reason` database column ([src/db/schema.rs:19](file:///home/sevenx/coding/ccbd-rust/src/db/schema.rs#L19)).
* **Impact**: External integrators cannot programmatically distinguish a session that shut down cleanly (`IDLE_MASTER_EXIT`) from one that crashed due to OOM or panic. This forces integrators to poll log files or guess session state.
* **Remediation**: Expose `master_last_exit_reason` and session lifecycle flags (like `cleanup_required`) in the `RuntimeSessionSnapshot` schema.

---

### 3.3. Agent Lifecycle & Recovery/Revive

#### 3.3.1. Unwired Orphan Scope Reconciler
* **Attribution**: **(a) Orchestration-reliability spec** (covered by [research/orchestration-reliability-design.md](file:///home/sevenx/coding/ccbd-rust/research/orchestration-reliability-design.md) Section 3.1).
* **Severity**: `should-fix-before-v1`
* **Evidence**: In [src/db/system.rs:577-615](file:///home/sevenx/coding/ccbd-rust/src/db/system.rs#L577-L615), `reconcile_orphan_scopes_with_runner_sync` exists but is never invoked by the production startup sequence in [src/bin/ahd.rs:84-101](file:///home/sevenx/coding/ccbd-rust/src/bin/ahd.rs#L84-L101).
* **Impact**: If `ahd` crashes or is killed, orphan systemd scopes containing old agent processes leak and are never swept upon daemon startup, leading to cumulative resource consumption.
* **Remediation**: Wire the sync orphan scope reconciliation call into `ahd`'s async startup sequence.

#### 3.3.2. Ephemeral Master Watcher State
* **Attribution**: **(a) Orchestration-reliability spec** (covered by [research/orchestration-reliability-design.md](file:///home/sevenx/coding/ccbd-rust/research/orchestration-reliability-design.md) Section 4 Mechanism 1 & 2).
* **Severity**: `should-fix-before-v1`
* **Evidence**: In [src/monitor/master_watch.rs:30-88](file:///home/sevenx/coding/ccbd-rust/src/monitor/master_watch.rs#L30-L88), the master watcher task is spawned in-memory and is not re-armed on daemon restart.
* **Impact**: If `ahd` restarts, all active master pidfd watches are lost. If the master subsequently crashes, `ah` will never detect the crash, leaving the session orphaned in the DB as `ACTIVE` and preventing worker cleanup.
* **Remediation**: Follow the pattern used for workers (Stage R in `agent_recovery_intents`) by storing the master watch intent in the DB and re-arming all active session watches during `ahd` startup.

---

### 3.4. Perception Layer

#### 3.4.1. Un-Corroborated T3 Pane Diff Prompt Detections
* **Attribution**: **(a) Orchestration-reliability spec** (covered by [research/orchestration-reliability-design.md](file:///home/sevenx/coding/ccbd-rust/research/orchestration-reliability-design.md) Section 3.3).
* **Severity**: `should-fix-before-v1`
* **Evidence**: The terminal diff monitor in [src/pane_diff/mod.rs:436-488](file:///home/sevenx/coding/ccbd-rust/src/pane_diff/mod.rs#L436-L488) transitions agents to `PROMPT_PENDING` solely based on regex matches on the tmux screen buffer.
* **Impact**: A worker printing text containing prompt-like patterns (e.g. logs containing `Confirm?`) gets falsely trapped in `PROMPT_PENDING`.
* **Remediation**: Corroborate T3 pane diffs with T0 process state (checking if the agent's PGID is actually blocked in a syscall read) before declaring `PROMPT_PENDING`.

---

### 3.5. CLI / UX Surface

#### 3.5.1. Session Status Hidden in `ah ps`
* **Attribution**: **(c) Genuine NEW v1 release blocker** (directly affects command-line users' ability to monitor state).
* **Severity**: `should-fix-before-v1`
* **Evidence**: In [src/cli/output.rs:8-15](file:///home/sevenx/coding/ccbd-rust/src/cli/output.rs#L8-L15) and [L40-47](file:///home/sevenx/coding/ccbd-rust/src/cli/output.rs#L40-L47), the table output struct ignores the `status` string returned by the `session.list` RPC call.
* **Impact**: Completed, failed, and active sessions look identical in the terminal table. Users cannot tell if a session is alive or dead without querying the raw JSON API.
* **Remediation**: Add a `STATUS` column to the `ah ps` CLI layout.

---

### 3.6. Cross-Platform Parity

#### 3.6.1. Invalid Command Formatting on Windows
* **Attribution**: **(b) Windows-native track** (covered by [.kiro/specs/ah-windows-native/m0-spec.md](file:///.kiro/specs/ah-windows-native/m0-spec.md)).
* **Severity**: `release-blocker`
* **Evidence**: In [src/platform/windows/scope.rs:154-168](file:///home/sevenx/coding/ccbd-rust/src/platform/windows/scope.rs#L154-L168), env vars are formatted as `KEY=VALUE` strings and appended directly into the executable's argv list.
* **Impact**: On Windows, the OS tries to run a command starting with the literal argument `KEY=VALUE` as the program file, failing immediately. **Windows support is broken for all command spawns**.
* **Remediation**: Modify the Windows command wrapper to pass environment variables using the Rust `Command::envs` API, rather than inline command line prefixing.

#### 3.6.2. macOS Service Supervisor Plist Placeholder
* **Attribution**: **(d) Post-v1 debt** (macOS integration is deferred from v1 scope).
* **Severity**: `release-blocker`
* **Evidence**: In [src/platform/macos/service.rs:89-92](file:///home/sevenx/coding/ccbd-rust/src/platform/macos/service.rs#L89-L92), `render_unit_file` returns a systemd-like unit text configuration string with a comment stating "plist rendering lands in PR-5".
* **Impact**: Launchd cannot parse systemd-style unit configurations, so service installation fails completely on macOS.
* **Remediation**: Implement a real plist XML generator in `src/platform/macos/service.rs`.

---

### 3.7. Security & Isolation Boundaries

#### 3.7.1. Missing Filesystem & Network Isolation
* **Attribution**: **(d) Post-v1 debt** (out of scope for the initial substrate release).
* **Severity**: `post-v1 debt`
* **Evidence**: The sandbox provider in [src/provider/home_layout.rs](file:///home/sevenx/coding/ccbd-rust/src/provider/home_layout.rs) isolates only the `$HOME` folder.
* **Impact**: Sandbox execution does not prevent agents from accessing host network ports or reading files outside the sandbox directory structure (e.g., traversing `../..` to read host keys). An untrusted rule or agent can easily compromise the developer's system.
* **Remediation**: Evaluate the use of lightweight containers or sandboxing wrappers (such as `bubblewrap` on Linux or `sandbox-exec` on macOS) to restrict filesystem access and network exposure.

---

### 3.8. Packaging, Install & Release Mechanics

#### 3.8.1. Dynamically Linked C Dependencies
* **Attribution**: **(c) Genuine NEW v1 release blocker** (directly impacts the cargo-dist release binary usability on clean machines).
* **Severity**: `should-fix-before-v1`
* **Evidence**: [Cargo.toml](file:///home/sevenx/coding/ccbd-rust/Cargo.toml) relies on dynamic linking for `rusqlite` and dynamic openSSL linking.
* **Impact**: Prebuilt binaries generated by cargo-dist will fail to execute on host machines that lack matching versions of `libsqlite3` or `libssl.so`.
* **Remediation**: Enable the `bundled` feature for `rusqlite` and statically link OpenSSL (using `openssl/vendored`) to produce zero-dependency, statically-linked binaries.

---

### 3.9. Test & CI Health

#### 3.9.1. Test-Suite Registry Collision Flakiness
* **Attribution**: **(d) Post-v1 debt** (impacts developer velocity, not end-user binary).
* **Severity**: `should-fix-before-v1`
* **Evidence**: As cataloged in [research/global-state-deflake-inventory.md](file:///home/sevenx/coding/ccbd-rust/research/global-state-deflake-inventory.md), parallel unit tests share mutable global static registries (e.g. `TMUX_PANE_MAP`, `LOG_MONITORS`).
* **Impact**: Running `cargo test` with multiple threads causes frequent random failures when parallel assertions conflict on agent IDs (like `a1` or `s1`). This slows down development velocity and causes false alarms in CI.
* **Remediation**: Namespace all test IDs dynamically using UUIDs and use async locks around tests that spawn mock tmux servers.

---

### 3.10. Assessment of the v1 Spec Surfaces

We evaluated the five core surfaces implemented for the v1 public release in [.kiro/specs/ah-v1-public-release/design.md](file:///.kiro/specs/ah-v1-public-release/design.md):

1. **Rules Split (Design 1) & Auto-Injection (Design 2)**:
   * *Status*: Sound, but presents minor **write-path atomicity risk**.
   * *Evidence*: In [src/provider/home_layout.rs:517-523](file:///home/sevenx/coding/ccbd-rust/src/provider/home_layout.rs#L517-L523), `write_builtin_rules` creates directories and writes rules directly to files (e.g., `.claude/CLAUDE.md`) using `fs::write`. If the write is interrupted, the rule file is left truncated.
   * *Mitigation*: The slot-mapping path validates agent IDs via `is_valid_agent_id` ([src/cli/config.rs:356](file:///home/sevenx/coding/ccbd-rust/src/cli/config.rs#L356)), preventing directory traversal via malicious agent IDs. Furthermore, worker home directories are isolated under `.cache/ah/sandboxes/<id>`, meaning parallel spawns do not collide on the same destination files.
2. **Provider Typo Loud Error (Design 3)**:
   * *Status*: Highly Sound.
   * *Evidence*: Config parsing in [src/cli/config.rs:200](file:///home/sevenx/coding/ccbd-rust/src/cli/config.rs#L200) uses `is_valid_provider` to ensure only registered providers (such as `claude`, `codex`, `antigravity`, or `bash`) can be specified. Typo checks fail loudly and prevent daemon launch.
3. **README and One-Line Install (Design 4) & cargo-dist Release (Design 5)**:
   * *Status*: At Risk.
   * *Evidence*: As detailed in Section 3.8.1, the lack of static compilation for OpenSSL and SQLite breaks the "one-line install" and pre-built binary promise on target developer systems that lack matching dynamic libraries.

---

## 4. v1 PRE-RELEASE MUST-FIX LIST

The following items are classified as **(c) Genuine NEW v1 release blockers** and must be resolved before the public release to ensure a robust upgrade and execution path for external integrators:

* **Robust Database Migrations (P0#4)**: Introduce atomic transactions around all sequential schema alteration steps in `src/db/mod.rs:init` and track migration execution history in a metadata table.
* **Expose Session Exit Reason in RuntimeSnapshot (3.2.2)**: Expose the `master_last_exit_reason` session column in the API's `RuntimeSessionSnapshot` payload.
* **Restore Session Status Column to CLI output (3.5.1)**: Reinstate the `status` column in the `ah ps` CLI layout to distinguish active vs finished sessions.
* **Statically Compiled Release Binaries (3.8.1)**: Statically compile `rusqlite` (bundled feature) and `openssl` (vendored feature) to avoid runtime dynamic library dependency errors on target hosts.
