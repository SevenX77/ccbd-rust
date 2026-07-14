# Unified State-Contract Implementation Specification

This specification defines the formal state contract for the Agent Hypervisor (`ah`), detailing the unified database schema version 2, session lifecycle subdivisions, resource cleanup signals, CLI visualization overhauls, configuration startup guards, and daemon-level startup resource reconciliation.

## Contract Principles

We establish these three hard boundaries as the formal state contract of the Agent Hypervisor system:

1. **`ah ps` is the HUMAN display surface**: This CLI view is lossy, stylized, and optimized for human readability. It must never be relied on for machine automation or state scraping.
2. **`ah events` / `RuntimeSnapshot` is the SOLE machine-authoritative surface**: All external automated tools, controllers (like Studio), and scrapers must consume either the live streaming events (`ah events --format json` / `runtime.subscribe`) or the one-shot state projection (`ah status --json` / `runtime.snapshot`).
3. **State transitions are written to the database ONLY by the daemon (`ahd`)**: The CLI binaries and external processes act purely as RPC clients. No external entity modifies SQLite state directly, ensuring strict transaction serializability.

---

## 1. Unified Runtime Schema v2

The `RuntimeSnapshot` schema version is bumped from `1` to `2`. 

### A. SQLite DDL Extensions

To support durable job state transition tracking, we incorporate the focused `job_transitions` table into the schema:

```sql
-- Pinned from .kiro/specs/ah-job-events/design.md
CREATE TABLE IF NOT EXISTS job_transitions (
    job_event_id INTEGER PRIMARY KEY AUTOINCREMENT,
    job_id TEXT NOT NULL REFERENCES jobs(id) ON DELETE CASCADE,
    agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    request_id TEXT,
    kind TEXT NOT NULL CHECK(kind IN ('job_transition', 'job_updated')),
    old_status TEXT,
    new_status TEXT,
    changed_json TEXT NOT NULL,
    reason TEXT NOT NULL,
    job_created_at INTEGER,
    dispatched_at INTEGER,
    completed_at INTEGER,
    cancel_requested INTEGER NOT NULL,
    error_reason TEXT,
    created_at INTEGER NOT NULL DEFAULT (unixepoch())
) STRICT;

CREATE INDEX IF NOT EXISTS idx_job_transitions_event_id ON job_transitions(job_event_id);
CREATE INDEX IF NOT EXISTS idx_job_transitions_job_event_id ON job_transitions(job_id, job_event_id);
CREATE INDEX IF NOT EXISTS idx_job_transitions_agent_event_id ON job_transitions(agent_id, job_event_id);
```

### B. Serialization Schema Definition

The full JSON schema structure of `RuntimeSnapshot` (v2) is as follows:

```json
{
  "schema_version": 2,
  "event": "snapshot",
  "sequence": 42,
  "reason": "job_changed",
  "runtime_state": "Active",
  "ahd_alive": true,
  "active": true,
  "ahd_has_inventory": true,
  "tmux_server_alive": true,
  "master_tmux_alive": true,
  "worker_tmux_alive": true,
  "worker_tmux_expected_count": 2,
  "sessions": [
    {
      "session_id": "sess_f3808d36",
      "project_id": "ccbd-rust",
      "path": "/home/sevenx/coding/ccbd-rust",
      "status": "CLOSED",
      "master_state": "IDLE",
      "master_tmux_session": "ahd-sess_f3808d36-master",
      "master_tmux_alive": false,
      "master_pane_id": "%0",
      "master_pid": null,
      "master_last_exit_reason": "IDLE_MASTER_EXIT",
      "db_tracked_agents": 1,
      "live_agents": 0,
      "cleanup_required": false,
      "safe_to_cleanup": true
    }
  ],
  "agents": [
    {
      "agent_id": "a1",
      "session_id": "sess_f3808d36",
      "provider": "codex",
      "state": "KILLED",
      "sub_state": null,
      "pid": null,
      "tmux_session": "ahd-sess_f3808d36-agent-a1",
      "tmux_alive": false
    }
  ],
  "jobs": [
    {
      "job_id": "job_123",
      "agent_id": "a1",
      "request_id": "client-token",
      "status": "COMPLETED",
      "cancel_requested": false,
      "created_at": 1783478400,
      "dispatched_at": 1783478402,
      "completed_at": 1783478410,
      "error_reason": null,
      "requires_physical_evidence": false,
      "requires_test_evidence": false
    }
  ],
  "job_events": [
    {
      "event_id": 1,
      "kind": "job_transition",
      "job_id": "job_123",
      "agent_id": "a1",
      "request_id": "client-token",
      "old_status": "DISPATCHED",
      "new_status": "COMPLETED",
      "changed": ["status", "completed_at"],
      "cancel_requested": false,
      "created_at": 1783478400,
      "dispatched_at": 1783478402,
      "completed_at": 1783478410,
      "error_reason": null,
      "reason": "idle_marker"
    }
  ],
  "job_event_cursor": 1
}
```

---

## 2. Session Lifecycle Subdivision

### A. Concrete Design Decision
We introduce the terminal status `"CLOSED"` to represent sessions that exited normally.
- A watched active master that exits cleanly with no active workers/jobs (`IdleNoWork`) is marked `status = 'CLOSED'` (distinguishing it from the error-state `status = 'FAILED'`).
- The recovery window phase for this master will still transition to `FAILED` or `COMPLETED` depending on recovery timeout bounds, but the user-facing session status becomes `CLOSED`.
- We expose `master_last_exit_reason` inside `RuntimeSessionSnapshot` as an `Option<String>` so that external consumers can inspect the precise exit signal (e.g. `IDLE_MASTER_EXIT`, `OOM_OR_CRASH`, `MASTER_REVIVE_WINDOW_EXPIRED`).

### B. Exact Code Loci
- **Status Writer**: [src/monitor/master_watch.rs:1952-1973](file:///home/sevenx/coding/ccbd-rust/src/monitor/master_watch.rs#L1952-L1973)  
  Update `mark_session_failed_after_idle_master_death` (rename to `mark_session_closed_after_idle_master_death`):
  ```rust
  // Change inside SQL UPDATE statement:
  SET status = 'CLOSED',
  ```
- **Terminal Status Definition**: [src/rpc/handlers/sessions.rs:214-219](file:///home/sevenx/coding/ccbd-rust/src/rpc/handlers/sessions.rs#L214-L219)  
  Update `is_terminal_session_status` to include `"CLOSED"`:
  ```rust
  fn is_terminal_session_status(status: &str) -> bool {
      matches!(status, "KILLED" | "FAILED" | "CLOSED")
  }
  ```
- **Cascade Check**: [src/db/system.rs:399](file:///home/sevenx/coding/ccbd-rust/src/db/system.rs#L399)  
  Update `cascade_kill_session_agents` to treat `"CLOSED"` as terminal, allowing cleanup:
  ```rust
  if !matches!(status.as_deref(), Some("KILLED" | "FAILED" | "CLOSED")) {
      return Ok(0);
  }
  ```
- **Snapshot Inventory Loader**: [src/runtime_events.rs:71-83](file:///home/sevenx/coding/ccbd-rust/src/runtime_events.rs#L71-L83) & [:316-340](file:///home/sevenx/coding/ccbd-rust/src/runtime_events.rs#L316-L340)  
  Add `master_last_exit_reason` to `InventorySession` and `RuntimeSessionSnapshot`. Update the SQL query in `query_runtime_inventory_sync` to select `sessions.master_last_exit_reason` and group by it.

### C. Acceptance Criteria
1. **Unit Test**: In `src/rpc/handlers/sessions.rs`, add:
   ```rust
   assert!(is_terminal_session_status("CLOSED"));
   ```
2. **E2E Integration Test**: Start a session with `ah start`, let the master exit with no active work. Verify via `runtime.snapshot` RPC that the session status is `"CLOSED"`, the `master_last_exit_reason` is `"IDLE_MASTER_EXIT"`, and that cascade cleanup proceeds without error.

### D. Migration / Compatibility
- **Database Backfill**: At startup, we run a migration function `migrate_sessions_failed_idle_to_closed` inside [src/db/mod.rs](file:///home/sevenx/coding/ccbd-rust/src/db/mod.rs) executing:
  ```sql
  UPDATE sessions
  SET status = 'CLOSED'
  WHERE status = 'FAILED' AND master_last_exit_reason = 'IDLE_MASTER_EXIT';
  ```
- **Rationale**: Backfilling historical rows ensures a single source of truth for historical metrics and prevents external dashboard scrapers from needing complex conditional logic to interpret old run records.
- **Consumer Compatibility**: Legacy consumers that only recognize `"FAILED"` and `"KILLED"` should be updated to treat `"CLOSED"` as terminal.

---

## 3. Cleanup Signal Fields

### A. Concrete Design Decision
We introduce `cleanup_required` and `safe_to_cleanup` to [RuntimeSessionSnapshot](file:///home/sevenx/coding/ccbd-rust/src/runtime_events.rs#L71-L83). These fields inform external controllers whether a session has lingering active resource leaks, and when it is safe to invoke `session.kill` (or external sweep).

### B. Exact Computation Formulas
- **`cleanup_required: bool`**: Indicates if terminal status has been set, but system resources are still allocated.
  $$\text{cleanup\_required} = \text{status} \in \{\text{"KILLED"}, \text{"FAILED"}, \text{"CLOSED"}\} \ \wedge \\ 
  ( \text{master\_tmux\_alive} \lor \text{master\_pid.is\_some()} \lor \text{any\_agent\_tmux\_alive} \lor \text{any\_agent\_pid.is\_some()} \lor \text{any\_agent\_state} \notin \{\text{"CRASHED"}, \text{"KILLED"}\} )$$
  
- **`safe_to_cleanup: bool`**: Indicates if it is safe to tear down resources (e.g. recovery window deferral period is over).
  $$\text{safe\_to\_cleanup} = \text{status} \in \{\text{"KILLED"}, \text{"FAILED"}, \text{"CLOSED"}\} \ \wedge \\
  ( \text{no recovery window exists for this session in DB} \lor \text{recovery\_window.phase} \in \{\text{"COMPLETED"}, \text{"FAILED"}, \text{"FUSED"}\} \lor \\
  \text{now} > \text{recovery\_window.defer\_until} \lor \text{recovery\_window.active\_work} = 0 )$$

### C. Exact Code Loci
- **Struct Addition**: [src/runtime_events.rs:71-83](file:///home/sevenx/coding/ccbd-rust/src/runtime_events.rs#L71-L83)  
  Add `pub cleanup_required: bool` and `pub safe_to_cleanup: bool` to `RuntimeSessionSnapshot`.
- **Builder Logic**: [src/runtime_events.rs:170-181](file:///home/sevenx/coding/ccbd-rust/src/runtime_events.rs#L170-L181)  
  Compute these values for each session using the formulas above during `build_runtime_snapshot`. The recovery window parameters are loaded by joining `master_recovery_windows` in the database.

### D. Acceptance Criteria
- A session in status `"ACTIVE"` must report `cleanup_required = false` and `safe_to_cleanup = false`.
- When a master crashes and transitions to recovery, during the active recovery window deferral (`now <= defer_until`), the session must report `safe_to_cleanup = false`.
- Once the recovery window completes or expires, it must transition to `safe_to_cleanup = true`.

---

## 4. `ah ps` Overhaul & active_agents Semantics

### A. `ah ps` Human View
- **Status Column**: We add a `status` column to the CLI human `tabled` output.
- **Default Filtering**: By default, running `ah ps` lists only active sessions (i.e. sessions where `!is_terminal_session_status(&status)`).
- **All Flag**: We add a `--all` argument to the `Ps` subcommand. If passed, it requests all sessions, including terminal ones (`CLOSED`/`FAILED`/`KILLED`).

### B. `active_agents` Semantics
- **Rename/Relabel**: In the DB layer, RPC structures, and CLI rows, `active_agents` is renamed to `db_tracked_agents` to clearly state it is the DB count of agents not in `CRASHED` or `KILLED`.
- **Live Agents**: We add `live_agents` (the sum of `tmux_alive` count for all agents in the session) to `RuntimeSessionSnapshot` and CLI rows, giving immediate visibility into live container/process health.
  
```rust
// Population logic in build_runtime_snapshot:
session_snapshot.live_agents = agent_snapshots.iter()
    .filter(|a| a.session_id == session.id && a.tmux_alive)
    .count() as i64;
```

### C. Structured Exit: `ah status --json`
- **Command Selection**: We implement `ah status --json` as the dedicated command to print a one-shot `RuntimeSnapshot` projection in JSON format and exit.
- **Rejected Alternative (`ah ps --json`)**:
  - **Rationale**: Overloading `ah ps` violates the core design contract that `ah ps` is purely a human display surface. If we add `--json` to `ah ps`, we risk developers building automation scripts against a table representation that may change. Keeping the CLI structure separated reinforces Principle 1 (human visual) and Principle 2 (machine-authoritative snapshots via `ah status` or `ah events`).

### D. Exact Code Loci
- **CLI Options**: [src/bin/ah.rs:60](file:///home/sevenx/coding/ccbd-rust/src/bin/ah.rs#L60)  
  Modify `Ps` and add `Status`:
  ```rust
  Ps {
      #[arg(long)]
      all: bool,
  },
  Status {
      #[arg(long)]
      json: bool,
  },
  ```
- **CLI Handler**: [src/bin/ah.rs:1138](file:///home/sevenx/coding/ccbd-rust/src/bin/ah.rs#L1138)  
  Implement `cmd_status` and update `cmd_ps` to accept `all: bool` and pass it to RPC.
- **RPC Handler**: [src/rpc/handlers/sessions.rs:1161](file:///home/sevenx/coding/ccbd-rust/src/rpc/handlers/sessions.rs#L1161)  
  Update `handle_session_list` to parse `all` parameter. If false, filter out terminal sessions:
  ```rust
  let show_all = params.get("all").and_then(Value::as_bool).unwrap_or(false);
  let sessions = list_session_summaries(ctx.db.clone()).await?
      .into_iter()
      .filter(|s| show_all || !is_terminal_session_status(&s.status))
      .collect::<Vec<_>>();
  ```
- **CLI Output Columns**: [src/cli/output.rs:8-15](file:///home/sevenx/coding/ccbd-rust/src/cli/output.rs#L8-15) & [:40-47](file:///home/sevenx/coding/ccbd-rust/src/cli/output.rs#L40-47)  
  Update `SessionRow` and `session_row` mapping to include `status`, `db_tracked_agents` (renamed), and `live_agents`.

---

## 5. Bare `ah start` Guard

### A. Concrete Design Decision
We prevent "bare start pollution" by resolving and validating the configuration file *before* checking or starting the daemon process.
- If config discovery or validation fails, `ah` exits with an error immediately without starting the daemon or touching any socket.
- If a config file is discoverable (e.g. `ah.toml` in CWD or parent folders), the command proceeds silently without interactive confirmation.
- **Rationale**: Automation scripts, hook executions, and CI workflows depend on a non-interactive setup. Prompting the developer for confirmation when `ah.toml` is present degrades developer ergonomics and breaks scripting, whereas failing silently or prompting on config errors is unnecessary since we exit with a non-zero exit code.

### B. Exact Code Loci
- **CLI Commands**: [src/bin/ah.rs:1176-1193](file:///home/sevenx/coding/ccbd-rust/src/bin/ah.rs#L1176-L1193)  
  Reorder operations in `cmd_start`:
  ```rust
  async fn cmd_start(
      client: &UnixRpcClient,
      config: Option<PathBuf>,
      wait: bool,
  ) -> Result<(), CliError> {
      let cwd = std::env::current_dir()?;
      // 1. First resolve the config path
      let config_path = match config.as_ref() {
          Some(path) => path.clone(),
          None => crate::cli::config::find_config(&cwd)?,
      };
      // 2. Validate config contents
      let _loaded = crate::cli::config::load_project_config(&config_path)?;
      
      // 3. Ensure the daemon is running only after config checks pass
      ensure_daemon_running(client.socket())?;
      
      let summary = start_from_options(
          client,
          StartOptions {
              config_path: Some(config_path),
              cwd,
              wait,
          },
      )
      .await?;
      print_start_summary(&summary);
      Ok(())
  }
  ```

---

## 6. Dead-Pane Reconcile Wiring

### A. Concrete Design Decision
We **WIRE** `reconcile_orphan_scopes_with_runner_sync` into the async production startup path.
- **Rationale**: When `ahd` crashes or reloads, user systemd scopes for legacy or active sessions can drift. Running orphan scope reclamation ensures we reap dead worker agent scopes, closing the dead-pane reclamation gap.
- **Risk Mitigation**: The function checks `active_session_and_agent_refs_sync` to query active database sessions, preventing the termination of live scopes. We also pass the dry-run configuration based on the `CCBD_RECONCILE_DRY_RUN` environment variable to ensure developers can safely inspect actions.

### B. Exact Code Loci
- **Startup Integration**: [src/db/system.rs:1215-1235](file:///home/sevenx/coding/ccbd-rust/src/db/system.rs#L1215-L1235)  
  Integrate inside `reconcile_startup_with_tmux_socket`:
  ```rust
  pub async fn reconcile_startup_with_tmux_socket(
      db: Db,
      state_dir: PathBuf,
      current_socket_name: Option<String>,
  ) -> Result<usize, CcbdError> {
      spawn_db("system::reconcile_startup", move || {
          let socket_name = current_socket_name
              .clone()
              .unwrap_or_else(|| crate::tmux::compute_socket_name(&state_dir));
          
          let agents_count = reconcile_active_agents_to_crashed_sync(&db, Some(&state_dir), Some(&socket_name))?;
          let recovery_count = reconcile_master_recovery_windows_with_runner_sync(&db, unixepoch(), &socket_name, &RealSystemctlRunner)?;
          
          // WIRING OF ORPHAN RECONCILE:
          let dry_run = reconcile_orphan_scopes_dry_run_enabled();
          let orphan_count = reconcile_orphan_scopes_with_runner_sync(&db, &RealSystemctlRunner, &socket_name, dry_run)?;
          
          sweep_stale_tmux_sockets_sync(current_socket_name.as_deref())?;
          Ok(agents_count + recovery_count + orphan_count)
      })
      .await
  }
  ```

---

## Proposed PR Implementation Series

To execute this design systematically, we structure the work into four independent, green-able PRs:

### PR 1: Database Migration & Schema V2 Infrastructure
- **Focus**: SQL schema adjustments and serialization.
- **Changes**:
  - Add `job_transitions` SQLite table in [src/db/schema.rs](file:///home/sevenx/coding/ccbd-rust/src/db/schema.rs).
  - Add the `migrate_sessions_failed_idle_to_closed` SQL backfill migration in [src/db/mod.rs](file:///home/sevenx/coding/ccbd-rust/src/db/mod.rs).
  - Define schema version 2 structs (`jobs[]`, `job_events[]`, `job_event_cursor`) and update `RuntimeSessionSnapshot` with `db_tracked_agents`, `live_agents`, `master_last_exit_reason`, `cleanup_required`, and `safe_to_cleanup`.
- **Green Verification**: Compilation passes; snapshot serialization tests succeed.

### PR 2: CLOSED Status & State Transitions Wiring
- **Focus**: Lifecycle state logic.
- **Changes**:
  - Rename and modify `mark_session_failed_after_idle_master_death` to set status `"CLOSED"` in [src/monitor/master_watch.rs](file:///home/sevenx/coding/ccbd-rust/src/monitor/master_watch.rs).
  - Update `is_terminal_session_status` and `cascade_kill_session_agents` in [src/rpc/handlers/sessions.rs](file:///home/sevenx/coding/ccbd-rust/src/rpc/handlers/sessions.rs) and [src/db/system.rs](file:///home/sevenx/coding/ccbd-rust/src/db/system.rs).
  - Wire `record_job_transition_conn_sync` to write transitions during job status changes.
- **Green Verification**: Master watch test suites and recovery cascade test suites verify clean close transitions.

### PR 3: CLI Status Command & human `ps` Overhaul
- **Focus**: CLI parsing and tables.
- **Changes**:
  - Add `ah status --json` CLI command and map it to `runtime.snapshot`.
  - Add `status` and `live_agents` columns to `SessionRow` in [src/cli/output.rs](file:///home/sevenx/coding/ccbd-rust/src/cli/output.rs).
  - Add `--all` filtering logic in `ah ps` and `session.list` RPC.
- **Green Verification**: Integration tests checking `ah ps` parsing and JSON schema output.

### PR 4: Startup Config Guards & Orphan Scope Wiring
- **Focus**: Resource safety.
- **Changes**:
  - Reorder `cmd_start` in [src/bin/ah.rs](file:///home/sevenx/coding/ccbd-rust/src/bin/ah.rs).
  - Wire `reconcile_orphan_scopes_with_runner_sync` in [src/db/system.rs](file:///home/sevenx/coding/ccbd-rust/src/db/system.rs).
- **Green Verification**: Startup reconcile test suites verify orphan reaping logs and daemon initialization bounds.

---

## Consolidated Acceptance Criteria

All tests must execute within the following cargo constraint framework:
```bash
CARGO_BUILD_JOBS=1 --test-threads=1 CCB_TEST_SKIP_REAL_PROVIDER=1 cargo test <test_name>
```

### Unit Tests
- `test_is_terminal_session_status_closed`: Asserts that `CLOSED` is recognized as a terminal session status.
- `test_failed_idle_to_closed_migration`: Asserts that database startup backfills historical FAILED sessions with the `IDLE_MASTER_EXIT` reason to CLOSED.
- `test_cleanup_required_safe_to_cleanup`: Mock session snapshot inputs asserting appropriate boolean transitions under recovery windows.
- `test_config_discovery_guard`: Validates that a bare startup without `ah.toml` fails before spawning the socket.

### E2E / Integration Tests
- `test_status_json_schema_v2`: Invokes `ah status --json`, validating that the payload parses as schema version 2 containing job and lifecycle fields.
- `test_ps_filtering_by_default`: Executes `ah ps` and asserts that closed sessions are excluded, and that `ah ps --all` lists them with a visual status column.
- `test_startup_orphan_reap`: Creates a dummy unit cgroup marker, runs daemon startup reconcile, and verifies that the systemctl runner receives a stop call for the orphan unit.
