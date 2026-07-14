# state-contract PR3 & PR4 Grounded Design and Test Plans

This document provides the grounded implementation designs and test plans for PR3 (CLI status/ps overhaul and active_agents alias) and PR4 (bare-start guard and startup orphan-scope reconciliation). These designs are based on main branch code following the merge of PR1.

---

## PR3 — CLI: `ah status --json` + `ah ps` Overhaul + `active_agents` Alias

### 1. `ah status --json`
- **Objective**: Provide a one-shot JSON dump of the state projection (`RuntimeSnapshot` v2) and exit immediately.
- **RPC Mapping**: The daemon already implements a one-shot `runtime.snapshot` RPC in [src/rpc/router.rs:108](file:///home/sevenx/coding/ccbd-rust/src/rpc/router.rs#L108) which maps to `handle_runtime_snapshot` in [src/rpc/handlers/runtime.rs:10](file:///home/sevenx/coding/ccbd-rust/src/rpc/handlers/runtime.rs#L10). This RPC builds a single snapshot with `RuntimeSnapshotReason::Initial` and returns it as a JSON payload. Therefore, no new daemon RPC handlers are required.
- **CLI Wiring Loci**:
  - **Subcommand Definition**: In [src/bin/ah.rs:54](file:///home/sevenx/coding/ccbd-rust/src/bin/ah.rs#L54), append the `Status` variant to the `Cmd` enum:
    ```rust
    /// Print a single runtime snapshot projection as JSON and exit.
    Status {
        /// Format output as JSON
        #[arg(long, default_value_t = true)]
        json: bool,
    },
    ```
  - **CLI Handler**: In [src/bin/ah.rs](file:///home/sevenx/coding/ccbd-rust/src/bin/ah.rs), define `cmd_status`:
    ```rust
    async fn cmd_status(client: &UnixRpcClient, json: bool) -> Result<(), CliError> {
        let result = client.call("runtime.snapshot", serde_json::json!({})).await?;
        if json {
            println!("{}", serde_json::to_string_pretty(&result)?);
        } else {
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        Ok(())
    }
    ```
  - **Dispatch**: In [src/bin/ah.rs](file:///home/sevenx/coding/ccbd-rust/src/bin/ah.rs) inside the match statement for `cli.cmd`:
    ```rust
    Some(Cmd::Status { json }) => cmd_status(&client, json).await,
    ```

### 2. `ah ps` Overhaul
- **Objective**: Display session `status` and `live_agents` in the human-facing CLI table, filtering out terminal sessions by default unless `--all` is supplied.
- **CLI Subcommand update**: In [src/bin/ah.rs:60](file:///home/sevenx/coding/ccbd-rust/src/bin/ah.rs#L60), update the `Ps` variant:
    ```rust
    /// List sessions, agents, and pending evidence.
    Ps {
        /// Show all sessions, including terminal ones
        #[arg(long)]
        all: bool,
    },
    ```
- **CLI Handler**: In [src/bin/ah.rs:1138](file:///home/sevenx/coding/ccbd-rust/src/bin/ah.rs#L1138), update `cmd_ps(client, all)` to forward the `all` parameter:
  ```rust
  let sessions = client.call("session.list", serde_json::json!({"all": all})).await?;
  ```
- **Daemon Filtering Locus**: In [src/rpc/handlers/sessions.rs:1161](file:///home/sevenx/coding/ccbd-rust/src/rpc/handlers/sessions.rs#L1161), update `handle_session_list` to parse the `"all"` argument. If false, filter out sessions where `is_terminal_session_status(&session.status)` is true.
  - **Rationale**: Performing the filter at the RPC handler level rather than in the CLI client ensures that business logic remains centralized inside the daemon, keeping socket payload sizes minimal for standard queries.
- **Output Column Loci**:
  - In [src/cli/output.rs:8-15](file:///home/sevenx/coding/ccbd-rust/src/cli/output.rs#L8-15), update `SessionRow`:
    ```rust
    #[derive(Tabled)]
    pub struct SessionRow {
        pub session_id: String,
        pub project_id: String,
        pub path: String,
        pub status: String,
        pub master_state: String,
        pub db_tracked_agents: String,
        pub live_agents: String,
    }
    ```
  - In [src/cli/output.rs:40-47](file:///home/sevenx/coding/ccbd-rust/src/cli/output.rs#L40-47), update `session_row` mapping:
    ```rust
    pub fn session_row(session: &Value) -> SessionRow {
        SessionRow {
            session_id: string_field(session, "id"),
            project_id: string_field(session, "project_id"),
            path: string_field(session, "absolute_path"),
            status: string_field(session, "status"),
            master_state: string_field(session, "master_state"),
            db_tracked_agents: option_i64_field(session, "db_tracked_agents"),
            live_agents: option_i64_field(session, "live_agents"),
        }
    }
    ```

### 3. `active_agents` Compatibility
- **Objective**: Prevent breaking unversioned external consumers of the `session.list` RPC by preserving `active_agents` as a deprecated field alias while introducing `db_tracked_agents` and `live_agents`.
- **SessionSummary Locus**: In [src/db/sessions.rs:8-17](file:///home/sevenx/coding/ccbd-rust/src/db/sessions.rs#L8-17), rename the struct field to `db_tracked_agents` and add `active_agents` as a deprecated alias:
  ```rust
  #[derive(Debug, Clone, PartialEq, Eq)]
  pub struct SessionSummary {
      pub id: String,
      pub project_id: String,
      pub absolute_path: String,
      pub status: String,
      pub master_state: String,
      pub master_pane_id: Option<String>,
      #[deprecated(since = "2.2.0", note = "Use db_tracked_agents instead")]
      pub active_agents: i64,
      pub db_tracked_agents: i64,
      pub created_at: i64,
  }
  ```
  Map both fields to the same SQL sum result inside `list_session_summaries_sync` (formerly `active_agents`).
- **RPC Locus**: In [src/rpc/handlers/sessions.rs:1161](file:///home/sevenx/coding/ccbd-rust/src/rpc/handlers/sessions.rs#L1161) `handle_session_list`, serialize both `active_agents` and `db_tracked_agents` to the JSON map.
- **live_agents computation**: Compute the live tmux session counts asynchronously in `handle_session_list` before returning:
  1. Retrieve active agents (not in `CRASHED` or `KILLED` states) for all listed sessions.
  2. Perform parallel check calls against the tmux server:
     ```rust
     let mut live_checks = Vec::new();
     for agent in &active_agents {
         let ts = ctx.tmux_server.clone();
         let session_id = agent.session_id.clone();
         let tmux_session = agent_session_name(&agent.id);
         live_checks.push(async move {
             let alive = ts.session_exists(tmux_session).await.unwrap_or(false);
             (session_id, alive)
         });
     }
     let results = futures::future::join_all(live_checks).await;
     ```
  3. Aggregate the `alive` status count per `session_id` and inject as `"live_agents"` in each session's JSON payload.
  - **Rationale**: Isolating the async `tmux_server` calls inside the async RPC handler keeps the synchronous rusqlite thread pool (`list_session_summaries_sync`) clean and free of async runtime dependencies.

### 4. PR3 Test Plan
1. **RPC Schema test**: Write a unit test `test_session_list_carries_alias_and_live_fields` verifying that `session.list` response contains `active_agents`, `db_tracked_agents`, and `live_agents` (with mock tmux states).
2. **CLI default filtering test**: Test `cmd_ps` with `all = false` and verify that terminal status sessions are filtered out. Test with `all = true` and assert that terminal sessions are displayed in the tabled output with their status shown.
3. **One-shot status test**: Run `ah status --json` with a running session and verify the stdout contains a valid JSON string mapping exactly to `RuntimeSnapshot` schema version 2.

---

## PR4 — Bare-Start Guard + Orphan-Scope Reconcile Wiring

### 1. Bare `ah start` Guard
- **Objective**: Prevent starting the daemon during a missing-config or invalid-config start command to avoid empty socket directory pollution.
- **CLI Wiring Loci**: In [src/bin/ah.rs:1176-1193](file:///home/sevenx/coding/ccbd-rust/src/bin/ah.rs#L1176-L1193), modify `cmd_start`:
  ```rust
  async fn cmd_start(
      client: &UnixRpcClient,
      config: Option<PathBuf>,
      wait: bool,
  ) -> Result<(), CliError> {
      let cwd = std::env::current_dir()?;
      // 1. Resolve configuration path
      let config_path = match config.as_ref() {
          Some(path) => path.clone(),
          None => crate::cli::config::find_config(&cwd)?,
      };
      // 2. Validate configuration contents (fails fast if empty or invalid)
      let _loaded = crate::cli::config::load_project_config(&config_path)?;
      
      // 3. Ensure the daemon runs only after configuration succeeds
      ensure_daemon_running(client.socket())?;
      
      // 4. Start the project, passing the already-resolved config path to avoid double walk
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
  - **Interactive behavior**: Since config discovery walking upward from CWD is silent and non-interactive by default, `ah start` will silently proceed when a config is discoverable, which is ideal for scripting and automation.

### 2. Orphan-Scope Reconcile Wiring
- **Objective**: Wire orphan systemd scope cleanup during startup reconciliation to ensure dead worker process resources are reaped, while protecting active scopes from accidental termination (remedying ccb Bug-Y).
- **Wiring Locus**: In [src/db/system.rs:1215-1235](file:///home/sevenx/coding/ccbd-rust/src/db/system.rs#L1215-L1235) `reconcile_startup_with_tmux_socket`, invoke `reconcile_orphan_scopes_with_runner_sync` inside the `spawn_db` sync closure:
  ```rust
  // WIRING OF ORPHAN RECONCILE:
  let dry_run = reconcile_orphan_scopes_dry_run_enabled();
  let orphan_count = reconcile_orphan_scopes_with_runner_sync(
      &db,
      &RealSystemctlRunner,
      &socket_name,
      dry_run,
  )?;
  ```
  - **Ordering**: This call is placed *after* `reconcile_active_agents_to_crashed_sync` and `reconcile_master_recovery_windows_with_runner_sync` have finished updating states, but *before* `sweep_stale_tmux_sockets_sync`. This ensures the active session/agent references loaded from the database are accurate before running scope reaping.

### 3. MANDATORY Regression Test Design (ccb Bug-Y Prevention)
To ensure startup reconciliation never stops live agent scopes during daemon restart, we implement a targeted unit test `test_reconcile_startup_retains_live_agent_scope_and_reaps_orphan` in [src/db/system.rs](file:///home/sevenx/coding/ccbd-rust/src/db/system.rs).

#### Test Structure and Implementation
```rust
#[test]
fn test_reconcile_startup_retains_live_agent_scope_and_reaps_orphan() {
    with_test_db_handle(|db| {
        let conn = db.conn();
        let state_dir = tempfile::TempDir::new().unwrap();
        let daemon_marker = crate::tmux::compute_socket_name(state_dir.path());
        
        // 1. Database Setup:
        // Set up one ACTIVE session and one ACTIVE agent
        insert_session_sync(
            &conn, 
            "sess_active", 
            "proj_1", 
            state_dir.path().to_str().unwrap()
        ).unwrap();
        
        insert_agent_sync(
            &conn, 
            "agent_live", 
            "sess_active", 
            "codex", 
            "IDLE", 
            Some(1234)
        ).unwrap();

        // 2. Mock Systemd Scopes Setup:
        // - Unit A matches the active agent in the active session -> must survive
        // - Unit B belongs to the same daemon generation but does not match any DB active refs -> orphan
        let live_scope = ScopeUnit {
            unit: "run-live-agent.scope".to_string(),
            description: format!("ccbd-agent-agent_live@{}", daemon_marker),
        };
        let orphan_scope = ScopeUnit {
            unit: "run-orphan-agent.scope".to_string(),
            description: format!("ccbd-agent-agent_orphan@{}", daemon_marker),
        };

        let runner = RecordingSystemctl::new(
            vec![live_scope, orphan_scope],
            Rc::new(RefCell::new(Vec::new())),
        );

        // 3. Execution:
        // Run the startup reconciliation sync runner
        reconcile_startup_sync_with_state_dir_and_runner(
            db,
            Some(state_dir.path()),
            &runner,
            unixepoch(),
            false, // dry-run disabled, perform actual stops
        )
        .unwrap();

        // 4. Assertions:
        let stopped_units = runner.events.borrow();
        
        // Assert the orphan scope was successfully reaped
        assert!(
            stopped_units.contains(&"stop:run-orphan-agent.scope".to_string()),
            "Orphan scope should have been stopped, but wasn't. Stopped events: {:?}",
            stopped_units
        );

        // Assert the active scope was NOT reaped
        assert!(
            !stopped_units.contains(&"stop:run-live-agent.scope".to_string()),
            "Live agent scope was stopped by mistake! Stopped events: {:?}",
            stopped_units
        );
    });
}
```

### 4. PR4 Test Plan
1. **Config Discovery Test**: Write CLI tests invoking a bare `ah start` inside a temporary directory with no `ah.toml` file. Assert that the command returns a non-zero exit code, displays a config missing error, and that `ensure_daemon_running` is never executed (i.e. no daemon socket or processes exist).
2. **Interactive validation**: Assert that running `ah start` inside a directory containing a valid `ah.toml` executes silently and successfully without blocking or waiting for user confirmation.
3. **Integrated Startup Reconcile Test**: Run the wired async startup task `reconcile_startup_with_tmux_socket` with mock DB states, asserting it computes the sum of crashed, recovery window re-armed, and reaped orphan scopes correctly.
