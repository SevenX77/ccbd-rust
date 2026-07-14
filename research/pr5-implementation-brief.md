# PR5 Implementation Brief: ah Process Identity Env

## Scope

PR5 injects explicit per-process identity environment variables into every process ah spawns as a managed worker or master. These variables are distinct from PR4 daemon identity and routing variables.

Worker contract:

- `AH_ROLE=worker`
- `AH_SESSION_ID=<session_id>`
- `AH_AGENT_ID=<agent_id>`

Master contract:

- `AH_ROLE=master`
- `AH_SESSION_ID=<session_id>`
- no `AH_AGENT_ID`

Values must be sourced from explicit spawn/session state already driving the spawn. They must not be read from inherited environment. Caller-provided `extra_env` / `extra_env_vars` must not be able to spoof these identity keys.

Non-goals:

- No `CLAUDE.md`, provider rules, or generated rules rewrite.
- No `CCB_` compatibility shim for `CCB_CALLER_ACTOR`.
- No schema or migration. This is env-only.

## PR4 Alignment Check

Authoritative post-PR4 source: `git show origin/main:tests/common/mod.rs`.

- `tests/common/mod.rs:9-14` defines `DAEMON_IDENTITY_ENV` as `CCB_SOCKET`, `AH_STATE_DIR`, `CCBD_STATE_DIR`, and `XDG_STATE_HOME`.
- `tests/common/mod.rs:17-26` removes only those daemon identity keys from spawned test daemon/helper commands unless the test explicitly set them.
- `tests/common/mod.rs:255-279` asserts explicit test isolation values for those daemon identity keys are preserved.

Result: the PR4 scrub set does not include `AH_AGENT_ID`, `AH_SESSION_ID`, or `AH_ROLE`. PR5 process identity vars will not be accidentally stripped by the merged harness scrub.

## Injection Loci

All line numbers below are from `git show origin/main:<path> | nl -ba`.

### 1. Worker Initial Spawn

File: `src/rpc/handlers/agent.rs`

Relevant lines:

- `81-88`: `handle_agent_spawn_with_db_action` reads explicit `session_id` and `agent_id` from the RPC params.
- `97-104`: parses caller-provided `extra_env_vars`.
- `142-143`: builds worker env with `build_agent_spawn_env_vars_for_hook_push(&ctx.state_dir, extra_env_vars)`.
- `155-168`: extends env with provider home overrides such as `CLAUDE_CONFIG_DIR` / `CODEX_HOME`.
- `178-190`: passes the final env map to `systemd::wrap_command_with_recovery_and_sandbox_overrides`.
- `241-255`: ensures the tmux session and spawns the final worker pane.
- `441-450`: `build_agent_spawn_env_vars_for_hook_push` currently injects deterministic `CCB_SOCKET` only.
- `567-587`: existing unit test covers deterministic worker `CCB_SOCKET` injection.

PR5 change:

- Replace or extend `build_agent_spawn_env_vars_for_hook_push` into an identity-aware worker env helper, for example:
  - `build_agent_spawn_env_vars(state_dir, session_id, agent_id, extra_env_vars)`.
- Insert/overwrite after caller `extra_env_vars`:
  - `AH_ROLE=worker`
  - `AH_SESSION_ID=<session_id>`
  - `AH_AGENT_ID=<agent_id>`
  - existing `CCB_SOCKET=<state_dir>/ahd.sock`
- Update call site at `agent.rs:142-143` to pass explicit `session_id` and `agent_id`.

Completeness:

- This single worker helper covers initial `agent.spawn`.
- It also covers realign, recovery, and master-revive worker reprovision because those paths all funnel into `handle_agent_spawn_with_db_action`.

### 2. Session Realign Worker Spawn

File: `src/rpc/handlers/realign.rs`

Relevant lines:

- `375-383`: `spawn_realign_agent` receives explicit `ctx`, `session_id`, `RealignAgentParams`, expected config hash, and recovery flags.
- `393-411`: calls `handle_agent_spawn_with_db_action` with explicit `session_id`, `agent.agent_id`, provider, and `agent.env` as `extra_env_vars`.

PR5 change:

- No separate injection here if worker identity is centralized in `handle_agent_spawn_with_db_action` / the worker env helper.
- Add/adjust tests only to ensure this path remains covered by the shared worker helper.

### 3. Worker Crash Recovery Respawn

File: `src/orchestrator/mod.rs`

Relevant lines:

- `479-482`: `run_recovery_once` delegates to `run_recovery_once_with_respawn`.
- `484-486`: `RecoveryRespawnFn` signature carries explicit `ctx`, `session_id`, `RealignAgentParams`, and expected hash.
- `488-496`: `spawn_realign_agent_for_recovery` calls `spawn_realign_agent(ctx, session_id, agent, expected_hash, true, true, None)`.
- `499-525`: recovery loop selects crashed agents and uses the respawn function.

PR5 change:

- No separate injection if worker identity is centralized in the worker env helper.
- The test plan should include a recovery-labeled unit test so a future refactor does not bypass the helper.

### 4. Master-Revive Worker Reprovision

File: `src/monitor/master_watch.rs`

Relevant lines:

- `2025-2031`: revive flow calls `revive_reprovision_one_worker`.
- `2053-2070`: `revive_reprovision_one_worker` calls `spawn_realign_agent(ctx, session_id, agent, expected_hash, false, true, captured_intent)`.

PR5 change:

- No separate injection if worker identity is centralized in the worker env helper.
- The test plan should include a master-revive-reprovision-labeled unit test or mock assertion that this path still reaches identity-bearing worker spawn env.

### 5. Initial Master Spawn

File: `src/rpc/handlers/sessions.rs`

Relevant lines:

- `375-386`: `handle_session_spawn_master_pane` reads explicit `session_id`, `cmd`, and caller `extra_env`.
- `424-454`: `prepare_master_pane_plan` loads the explicit session row and starts `master_env_vars` from `params.extra_env`.
- `465-482`: provider master home materialization extends `master_env_vars`.
- `494-500`: `spawn_master_pane_inner` prepares the plan and spawns it.
- `502-514`: `spawn_prepared_master_pane` builds the master command via `systemd::master_command_with_env`.
- `523-530`: spawns the final master pane.
- `554-606`: records master runtime and arms revival watch.

PR5 change:

- Add a master env helper in `sessions.rs`, for example:
  - `build_master_spawn_env_vars(session_id, extra_env)`.
- In `prepare_master_pane_plan`, replace `let mut master_env_vars = params.extra_env.clone();` at `sessions.rs:454` with the helper.
- Helper behavior:
  - start from caller `extra_env`
  - remove any caller-supplied `AH_AGENT_ID`
  - insert/overwrite `AH_ROLE=master`
  - insert/overwrite `AH_SESSION_ID=<params.session_id>`
- Keep provider home env extension after this. Current provider home env (`CLAUDE_CONFIG_DIR`, `CODEX_HOME`, or empty) does not set PR5 identity keys.

### 6. Master Cutover Spawn

File: `src/rpc/handlers/sessions.rs`

Relevant lines:

- `1001-1013`: cutover builds `extra_env` with `AH_STATE_DIR`, `CCB_SOCKET`, `AH_CUTOVER_ID`, `AH_MASTER_HANDOFF`, and legacy/internal `AH_MASTER_ROLE=managed`.
- `1014-1028`: creates `SpawnMasterPaneParams` with explicit `session_id` and that `extra_env`.
- `1029-1035`: calls `prepare_master_pane_plan`, so ordinary master env helper can cover this master spawn.
- `1051-1071`: provisions declared workers through `spawn_realign_agent`.
- `1080-1081`: spawns the managed master from the prepared plan.
- `1123-1125`: arms master revive watch after ACTIVE.

PR5 change:

- If `prepare_master_pane_plan` owns master identity insertion, cutover master spawn is covered automatically.
- Preserve existing `AH_MASTER_ROLE=managed` unless a separate cleanup explicitly removes it. PR5's primary role identity is `AH_ROLE=master`; legacy `AH_MASTER_ROLE` is not the new contract.
- Declared workers at `sessions.rs:1070-1071` are covered by the centralized worker helper.

### 7. Master Revive Spawn

File: `src/monitor/master_watch.rs`

Relevant lines:

- `768-772`: loads explicit session row and derives master cwd/session names.
- `773-779`: directly assembles `master_env_vars` with `AH_STATE_DIR`, `CCB_SOCKET`, and legacy/internal `AH_MASTER_ROLE=managed`.
- `780-785`: optionally inserts `AH_REDISPATCH_MARKER`.
- `786-801`: inserts `HOME` and `CLAUDE_CONFIG_DIR` for sandboxed revive.
- `803-812`: builds final command via `systemd::master_command_with_env` or `shell_command_with_env_prefix`.
- `814-823`: spawns the revived master pane.
- `830-848`: records revived master runtime or kills stale orphan pane.
- `4520-4595`: existing unit-style env capture test prints and asserts revived master env for `AH_STATE_DIR`, `CCB_SOCKET`, `AH_MASTER_ROLE`, `AH_REDISPATCH_MARKER`, `HOME`, and `CLAUDE_CONFIG_DIR`.

PR5 change:

- Add explicit process identity to the direct `master_env_vars` map at `master_watch.rs:773-779`:
  - `AH_ROLE=master`
  - `AH_SESSION_ID=<session_id>`
  - ensure `AH_AGENT_ID` is absent
- Keep existing daemon/cutover/revive vars.
- Extend the existing env capture test at `master_watch.rs:4520-4595` to print/assert `AH_ROLE`, `AH_SESSION_ID`, and empty/absent `AH_AGENT_ID`.

## Env Propagation Notes

The low-level wrappers already preserve explicit env maps:

- `src/sandbox/systemd.rs:54-75`: worker env map is forwarded into platform scope wrapping.
- `src/sandbox/systemd.rs:87-100`: master env map is forwarded into platform master command wrapping.
- `src/platform/linux/scope.rs:214-236` and `263-291`: worker wrapper passes `extra_env_vars` through.
- `src/platform/linux/scope.rs:304-329`: master wrapper passes `extra_env_vars` through.
- macOS and Windows stubs have equivalent env-prefix paths (`src/platform/macos/scope.rs`, `src/platform/windows/scope.rs`), so helper-level injection is cross-platform.

PR5 should not inject identity in `src/provider/home_layout.rs`. Provider home layout currently returns provider-home variables (`CLAUDE_CONFIG_DIR`, `CODEX_HOME`, or empty) at `home_layout.rs:246`, `273`, and `327`. Process identity must apply even when home materialization is skipped, so it belongs in spawn env helpers before wrapping.

## Test Plan

Local PR5 validation should stay in `--lib` unit/mock tests. Runtime integration belongs in CI.

### Unit: Worker Initial Spawn Env Helper

File: `src/rpc/handlers/agent.rs`

RED:

- Extend or add a test near `hook_push_worker_spawn_env_injects_deterministic_ccb_socket`.
- Call the new worker env helper with `session_id="s1"`, `agent_id="a1"`, and spoofed caller values:
  - `AH_ROLE=master`
  - `AH_SESSION_ID=wrong`
  - `AH_AGENT_ID=wrong`
  - stale `CCB_SOCKET`
  - `USER_FLAG=1`
- Before implementation, the helper lacks the new identity keys or preserves spoofed values.

GREEN:

- Assert:
  - `AH_ROLE=worker`
  - `AH_SESSION_ID=s1`
  - `AH_AGENT_ID=a1`
  - deterministic `CCB_SOCKET=<state_dir>/ahd.sock`
  - unrelated `USER_FLAG=1` preserved

### Unit: Worker Realign/Recovery Coverage

Files: `src/rpc/handlers/realign.rs`, `src/orchestrator/mod.rs`, and/or `src/monitor/master_watch.rs`

RED:

- Add a narrowly named test proving a recovery or reprovision caller receives the same identity-bearing worker env, not a separate identity-less map.
- Best implementation option: expose the worker env helper as `pub(crate)` and test it from the existing recovery/reprovision module test context, or add a small mock around the respawn function input if that is already locally available.

GREEN:

- Test name should explicitly include recovery/reprovision, for example:
  - `worker_recovery_respawn_env_contains_process_identity`
  - `master_revive_worker_reprovision_uses_worker_identity_env`
- Assert worker triple from explicit `session_id` and `agent_id`.

### Unit: Initial Master Spawn Env Helper

File: `src/rpc/handlers/sessions.rs`

RED:

- Add tests for `build_master_spawn_env_vars`.
- Use spoofed caller env:
  - `AH_ROLE=worker`
  - `AH_SESSION_ID=wrong`
  - `AH_AGENT_ID=a1`
  - `USER_FLAG=1`
- Before implementation, `prepare_master_pane_plan` starts directly from caller env and no master helper exists.

GREEN:

- Assert:
  - `AH_ROLE=master`
  - `AH_SESSION_ID=<explicit session_id>`
  - `AH_AGENT_ID` absent
  - unrelated `USER_FLAG=1` preserved

### Unit: Master Cutover Env

Files: `src/rpc/handlers/sessions.rs`, optionally `src/sandbox/systemd.rs`

RED:

- Extend existing cutover env coverage to include PR5 identity.
- Current `src/sandbox/systemd.rs:377-407` checks `AH_STATE_DIR`, `CCB_SOCKET`, `AH_MASTER_ROLE`, `AH_CUTOVER_ID`, and `AH_MASTER_HANDOFF`; it does not check `AH_ROLE` or `AH_SESSION_ID`.

GREEN:

- Assert the cutover master command/env contains:
  - `AH_ROLE=master`
  - `AH_SESSION_ID=<new cutover session_id>`
  - no `AH_AGENT_ID`
- Preserve existing `AH_MASTER_ROLE=managed` unless deliberately removed in a later cleanup.

### Unit: Master Revive Env

File: `src/monitor/master_watch.rs`

RED:

- Extend the existing env capture test at `master_watch.rs:4520-4595`.
- Update the printed env command to include `$AH_ROLE`, `$AH_SESSION_ID`, and `${AH_AGENT_ID:-}`.
- Before implementation, `AH_ROLE` and `AH_SESSION_ID` are empty.

GREEN:

- Assert:
  - `AH_ROLE=master`
  - `AH_SESSION_ID=<revived session_id>`
  - `AH_AGENT_ID` empty
  - existing assertions for `AH_STATE_DIR`, `CCB_SOCKET`, `AH_MASTER_ROLE`, `AH_REDISPATCH_MARKER`, `HOME`, and `CLAUDE_CONFIG_DIR` still pass

### Optional Unit: Wrapper Preservation

File: `src/sandbox/systemd.rs`

RED:

- Pass an env map containing worker or master identity into `wrap_command_with_recovery_and_sandbox_overrides` or `master_command_with_env`.
- Assert the final argv includes identity `KEY=VALUE` entries.

GREEN:

- Confirms lower-level wrappers preserve `AH_*` keys for systemd and shell-prefix paths.
- This is optional if helper tests are strong, because wrappers already preserve arbitrary env maps.

### CI-Only Integration

Runtime tests should run only in CI because they spawn real daemon/tmux/systemd paths.

- Worker initial spawn: spawn a worker whose command prints `AH_ROLE`, `AH_SESSION_ID`, and `AH_AGENT_ID`; assert `worker`, current session, current agent.
- Initial master spawn: spawn a master command that prints `AH_ROLE`, `AH_SESSION_ID`, and `${AH_AGENT_ID:-}`; assert `master`, current session, and empty agent ID.
- Master cutover: verify the managed cutover master carries the same master identity contract.
- Master revive: kill/revive master in CI and assert revived process still has `AH_ROLE=master` and same `AH_SESSION_ID`.
- Worker recovery or master-revive worker reprovision: assert respawned/reprovisioned worker carries `AH_ROLE=worker`, same `AH_SESSION_ID`, and its `AH_AGENT_ID`.

## Implementation Order

1. Add worker identity env helper in `src/rpc/handlers/agent.rs`.
   - Touch `build_agent_spawn_env_vars_for_hook_push` and its call site at `agent.rs:142-143`.
   - Add/extend unit tests at `agent.rs:567-587`.

2. Confirm worker indirect paths require no separate injection.
   - `src/rpc/handlers/realign.rs:393-411`
   - `src/orchestrator/mod.rs:488-496`
   - `src/monitor/master_watch.rs:2053-2070`
   - Add recovery/reprovision-named unit coverage so the centralization is explicit.

3. Add master env helper in `src/rpc/handlers/sessions.rs`.
   - Touch `prepare_master_pane_plan` at `sessions.rs:454`.
   - Ensure helper removes `AH_AGENT_ID` and overwrites `AH_ROLE` / `AH_SESSION_ID`.
   - Add unit tests for spoof overwrite and absent agent ID.

4. Let master cutover flow through the master helper.
   - Existing cutover env assembly at `sessions.rs:1001-1013` can keep daemon/cutover vars.
   - Add coverage that `prepare_master_pane_plan` or final command includes `AH_ROLE=master` and `AH_SESSION_ID`.

5. Add direct master revive identity env.
   - Touch `src/monitor/master_watch.rs:773-779`.
   - Extend env capture test at `master_watch.rs:4520-4595`.

6. Run local allowed validation in the implementation worktree.
   - `cargo test --lib -- --test-threads=1`
   - `cargo check --all-targets`
   - No local integration/mvp/full-suite if the iron rule is still in force.

## Files To Touch

Expected source/test files:

- `src/rpc/handlers/agent.rs`
- `src/rpc/handlers/sessions.rs`
- `src/monitor/master_watch.rs`
- Possibly `src/orchestrator/mod.rs` or `src/rpc/handlers/realign.rs` only for targeted unit coverage names/assertions
- Optionally `src/sandbox/systemd.rs` for wrapper-preservation unit tests

Not expected:

- No DB schema files.
- No migrations.
- No provider rule files.
- No test harness daemon-identity scrub changes unless tests reveal an accidental interaction.

