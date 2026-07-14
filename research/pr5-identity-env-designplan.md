# PR5 Identity Env Design Plan

## Context

PR5 adds per-process ah identity environment variables to every ah-spawned master and worker process. These variables are not daemon identity. They must not be confused with PR4's daemon socket/state variables:

- daemon identity / routing: `CCB_SOCKET`, `AH_STATE_DIR`, `CCBD_STATE_DIR`
- process identity for PR5: `AH_AGENT_ID`, `AH_SESSION_ID`, `AH_ROLE`

PR4's harness scrub is exact today: `tests/common/mod.rs:9` defines `DAEMON_IDENTITY_ENV` as only `["CCB_SOCKET", "AH_STATE_DIR", "CCBD_STATE_DIR"]`, and `tests/common/mod.rs:12-17` / `tests/common/mod.rs:25-37` remove only those keys. That scrub must stay exact. It must not strip `AH_AGENT_ID`, `AH_SESSION_ID`, or `AH_ROLE`.

## Env Var Contract

Appendable spec text:

```md
### ah Process Identity Environment

Every process spawned by ah as a managed master or worker must receive explicit ah identity environment variables derived from the spawn request/session state, not from ambient process environment.

Worker processes:
- `AH_ROLE=worker`
- `AH_SESSION_ID=<session_id>`
- `AH_AGENT_ID=<agent_id>`

Master processes:
- `AH_ROLE=master`
- `AH_SESSION_ID=<session_id>`
- no `AH_AGENT_ID`

The guarantee applies to initial spawn, `session.realign` spawn, worker crash recovery respawn, master-revive worker reprovision, initial master spawn, master cutover spawn, and master revive spawn. These variables are ah-native process identity, not daemon socket/state identity. They do not replace `CCB_SOCKET` or `AH_STATE_DIR`.

Explicit spawn state wins. `AH_SESSION_ID` and `AH_AGENT_ID` must come from the RPC/session/recovery data structures already driving the spawn, never from inherited environment.
```

Non-goals:

- No `CLAUDE.md`, provider rule, or generated rules rewrite.
- No `CCB_` compatibility shim for `CCB_CALLER_ACTOR`.
- No daemon identity behavior change.

## Injection Loci

### Worker Initial Spawn

Primary code path:

- `src/rpc/handlers/agent.rs:87-88` reads explicit `session_id` and `agent_id` from `agent.spawn` params.
- `src/rpc/handlers/agent.rs:97-104` parses caller-provided `extra_env_vars`.
- `src/rpc/handlers/agent.rs:142-143` builds the worker spawn env via `build_agent_spawn_env_vars_for_hook_push`.
- `src/rpc/handlers/agent.rs:155-168` extends that env with provider home overrides such as `CLAUDE_CONFIG_DIR` / `CODEX_HOME`.
- `src/rpc/handlers/agent.rs:178-190` passes env to `systemd::wrap_command_with_recovery_and_sandbox_overrides`.
- `src/rpc/handlers/agent.rs:254-256` spawns the final tmux window command.

Recommended PR5 change:

- Extend or replace `build_agent_spawn_env_vars_for_hook_push` at `src/rpc/handlers/agent.rs:441-450` with an identity-aware helper, e.g. `build_agent_spawn_env_vars(state_dir, session_id, agent_id, extra_env_vars)`.
- It should insert:
  - `AH_ROLE=worker`
  - `AH_SESSION_ID=<session_id>`
  - `AH_AGENT_ID=<agent_id>`
  - existing deterministic `CCB_SOCKET=<state_dir>/ahd.sock`
- Identity keys should be inserted after caller `extra_env_vars`, so user config cannot spoof ah process identity.

Unit coverage:

- Existing test at `src/rpc/handlers/agent.rs:570-585` checks deterministic `CCB_SOCKET`. Extend it to assert worker identity vars and overwrite stale user-provided `AH_ROLE`, `AH_SESSION_ID`, `AH_AGENT_ID`.

### Provider Home Env Assembly

Provider-specific home env is assembled separately:

- `src/provider/home_layout.rs:142-193` dispatches provider home layout.
- `src/provider/home_layout.rs:244-247` returns `CLAUDE_CONFIG_DIR`.
- `src/provider/home_layout.rs:271-274` returns `CODEX_HOME`.
- `src/provider/home_layout.rs:327` returns no antigravity extra env today.

Recommended PR5 stance:

- Do not inject `AH_*` in provider home layout. These vars are not provider-home materialization artifacts and must apply even when `manifest.requires_home_materialization` is false or `unsafe_no_sandbox` skips sandbox home creation.
- Keep injection in the spawn env helpers before `wrap_command`.

### Low-Level Worker Command Wrapper

The wrapper currently carries env through:

- `src/sandbox/systemd.rs:54-75` forwards `extra_env_vars` to platform scope wrapping.
- `src/platform/linux/scope.rs:222-251` appends `command_with_env_prefix`.
- `src/platform/linux/scope.rs:340-363` converts collected env into an `env KEY=VALUE ...` prefix.
- `src/provider/manifest.rs:476-495` merges provider passthrough, provider injected env, and extra env.

Recommended PR5 stance:

- Keep `AH_*` in the explicit `extra_env_vars` map, so it works for both systemd and unsafe/no-sandbox paths through the existing wrapper.
- Add wrapper-level unit tests only if helpful to prove the final argv contains the identity env for both `env_state(false)` and unsafe/no-sandbox. The higher value tests are on the env assembly helper.

### `session.realign` Worker Spawn

`session.realign` is a worker spawn path and must not be missed:

- `src/rpc/handlers/realign.rs:375-383` defines `spawn_realign_agent`.
- `src/rpc/handlers/realign.rs:393-411` converts `RealignAgentParams` into a call to `handle_agent_spawn_with_db_action`.
- `src/rpc/handlers/realign.rs:395-398` passes explicit `session_id`, `agent_id`, provider, and `extra_env_vars`.

Recommended PR5 change:

- No separate injection here if worker identity is injected inside `handle_agent_spawn_with_db_action` using explicit `session_id` and `agent_id`.
- Add a unit test for `spawn_realign_agent` only if the existing env helper test is not considered enough; otherwise unit-test the helper and the recovery callers.

### Worker Crash Recovery Respawn

Crash recovery routes through `spawn_realign_agent`:

- `src/orchestrator/mod.rs:479-482` calls `run_recovery_once_with_respawn`.
- `src/orchestrator/mod.rs:488-496` defines `spawn_realign_agent_for_recovery`, which calls `spawn_realign_agent(ctx, session_id, agent, expected_hash, true, true, None)`.
- `src/orchestrator/mod.rs:499-510` iterates crashed agents.

Recommended PR5 change:

- No separate injection if `handle_agent_spawn_with_db_action` owns worker identity insertion.
- Unit test the recovery respawn function or its mockable call chain to assert the respawned worker env contains `AH_ROLE=worker`, `AH_SESSION_ID`, and `AH_AGENT_ID`.

### Master-Revive Worker Reprovision

Master revive may kill/reprovision active workers. This is a distinct path and must not produce identity-less workers:

- `src/monitor/master_watch.rs:2053-2070` defines `revive_reprovision_one_worker`.
- `src/monitor/master_watch.rs:2060-2068` calls `spawn_realign_agent(ctx, session_id, agent, expected_hash, false, true, captured_intent)`.

Recommended PR5 change:

- No separate injection if `handle_agent_spawn_with_db_action` owns worker identity insertion.
- Add/extend a `--lib` unit test in `master_watch` around worker reprovision env capture. Current tests already inspect master revive env at `src/monitor/master_watch.rs:4563`; a similar mock around `spawn_realign_agent` would cover the worker reprovision gap.

### Initial Master Spawn

Primary code path:

- `src/rpc/handlers/sessions.rs:375-400` handles `session.spawn_master_pane`.
- `src/rpc/handlers/sessions.rs:386` parses caller-provided extra env.
- `src/rpc/handlers/sessions.rs:424-491` prepares `MasterPanePlan`.
- `src/rpc/handlers/sessions.rs:454` starts `master_env_vars` from caller extra env.
- `src/rpc/handlers/sessions.rs:472-482` extends master env with provider home overrides.
- `src/rpc/handlers/sessions.rs:502-514` builds the master command with `systemd::master_command_with_env`.
- `src/rpc/handlers/sessions.rs:523-530` spawns the master pane.

Recommended PR5 change:

- Add a small helper in `sessions.rs`, e.g. `build_master_spawn_env_vars(session_id, extra_env)`, and call it before provider home overrides.
- It should insert:
  - `AH_ROLE=master`
  - `AH_SESSION_ID=<session_id>`
  - no `AH_AGENT_ID`
- It should overwrite stale/spoofed caller `AH_*` keys.

Unit coverage:

- Unit-test the helper for ordinary master spawn: stale `AH_ROLE`, `AH_SESSION_ID`, `AH_AGENT_ID` in caller env become `AH_ROLE=master`, explicit session ID, and no agent ID.
- Unit-test `systemd::master_command_with_env` final argv if desired, using existing tests around `src/sandbox/systemd.rs:352-402`.

### Master Cutover Spawn

Master cutover builds a separate master env:

- `src/rpc/handlers/sessions.rs:1001-1013` builds `extra_env` with `AH_STATE_DIR`, `CCB_SOCKET`, `AH_CUTOVER_ID`, `AH_MASTER_HANDOFF`, and current `AH_MASTER_ROLE=managed`.
- `src/rpc/handlers/sessions.rs:1014-1028` passes that env into `SpawnMasterPaneParams`.
- `src/rpc/handlers/sessions.rs:1051-1071` provisions declared workers before spawning the managed master.
- `src/rpc/handlers/sessions.rs:1080-1081` spawns the master via the prepared plan.

Recommended PR5 change:

- Replace or supplement current `AH_MASTER_ROLE=managed` with the new contract `AH_ROLE=master`.
- Preserve `AH_MASTER_ROLE=managed` only if existing hook/cutover behavior still depends on it. The PR5 contract should not use it as the primary role identity.
- Ensure `AH_SESSION_ID=<new cutover session_id>` is inserted from the explicit cutover-created session ID.
- Declared worker provisioning at `src/rpc/handlers/sessions.rs:1070` should get worker identity automatically through `spawn_realign_agent` / `handle_agent_spawn_with_db_action`.

### Master Revive Spawn

Master revive bypasses `sessions.rs` spawn planning and assembles env directly:

- `src/monitor/master_watch.rs:768-772` loads explicit session data and master session name.
- `src/monitor/master_watch.rs:773-779` builds `master_env_vars` with `AH_STATE_DIR`, `CCB_SOCKET`, and current `AH_MASTER_ROLE=managed`.
- `src/monitor/master_watch.rs:780-784` optionally adds `AH_REDISPATCH_MARKER`.
- `src/monitor/master_watch.rs:786-801` adds `HOME` and `CLAUDE_CONFIG_DIR` for sandboxed revive.
- `src/monitor/master_watch.rs:803-812` builds the final command.
- `src/monitor/master_watch.rs:814-823` spawns the revived master window.
- `src/monitor/master_watch.rs:871-882` arms the next master watcher for the revived process.

Recommended PR5 change:

- Add `AH_ROLE=master` and `AH_SESSION_ID=<session_id>` to `master_env_vars` here.
- Do not add `AH_AGENT_ID`.
- Keep `AH_MASTER_ROLE=managed` only as a legacy/internal marker if current hooks/tests depend on it.

Unit coverage:

- Existing master revive env capture test at `src/monitor/master_watch.rs:4563` prints `$AH_STATE_DIR`, `$CCB_SOCKET`, `$AH_MASTER_ROLE`, `$AH_REDISPATCH_MARKER`, `$HOME`, `$CLAUDE_CONFIG_DIR`.
- Extend that test to print/assert `$AH_ROLE` and `$AH_SESSION_ID`, and assert `$AH_AGENT_ID` is empty.

## Tests

Local PR5 tests should stay `--lib` unit/mock tests. Integration and true daemon/systemd tests belong in CI.

Recommended `--lib` tests:

1. Worker env helper:
   - target: `src/rpc/handlers/agent.rs`
   - extend `hook_push_worker_spawn_env_injects_deterministic_ccb_socket`
   - assert `AH_ROLE=worker`, `AH_SESSION_ID=s1`, `AH_AGENT_ID=a1`
   - assert caller-provided spoofed values are overwritten

2. Master env helper:
   - target: `src/rpc/handlers/sessions.rs`
   - new helper test for `build_master_spawn_env_vars`
   - assert `AH_ROLE=master`, `AH_SESSION_ID=s1`, and no `AH_AGENT_ID`
   - assert caller-provided spoofed values are overwritten/removed

3. Master revive env:
   - target: `src/monitor/master_watch.rs`
   - extend the existing env capture style around `src/monitor/master_watch.rs:4563`
   - assert revived master gets `AH_ROLE=master`, `AH_SESSION_ID`, no `AH_AGENT_ID`

4. Worker recovery/reprovision:
   - target: `src/orchestrator/mod.rs` and/or `src/monitor/master_watch.rs`
   - either mock `spawn_realign_agent` call input or assert the shared worker env helper covers recovery callers
   - quality bar: explicit test name should mention recovery or reprovision so future refactors do not drop the coverage

5. Wrapper argv smoke:
   - target: `src/sandbox/systemd.rs` or platform scope tests
   - pass a map containing the three worker identity vars through `wrap_command_with_recovery_and_sandbox_overrides`
   - assert final argv includes `env AH_AGENT_ID=... AH_ROLE=worker AH_SESSION_ID=...`
   - optional if helper-level tests are strong, but useful to prove both systemd and unsafe/no-sandbox wrappers preserve the vars

CI-only integration:

- A small managed worker script can print `AH_AGENT_ID`, `AH_SESSION_ID`, `AH_ROLE`.
- A managed master script can print `AH_SESSION_ID`, `AH_ROLE`, and verify `AH_AGENT_ID` is absent.
- A master revive scenario should verify the revived master still has the same `AH_SESSION_ID`.
- A worker recovery or master-revive worker reprovision scenario should verify the respawned worker still has the correct triple.

## Design Notes

- Prefer central helpers over injecting at every call site:
  - worker: centralize in `handle_agent_spawn_with_db_action` / env helper because initial spawn, `session.realign`, orchestrator recovery, and master-revive worker reprovision all funnel through it.
  - master: centralize ordinary spawn in `prepare_master_pane_plan`, but explicitly handle master revive because it assembles env directly in `master_watch.rs`.
- Insert `AH_*` after caller-supplied `extra_env` so caller config cannot spoof ah identity.
- Source `AH_SESSION_ID` from explicit spawn/session parameters, consistent with PR4's explicit daemon identity principle.
- Keep PR4 daemon env scrub exact. Do not add PR5 identity vars to `DAEMON_IDENTITY_ENV`.
