# Realign Atomicity Reliability Spec

## Scope

This spec covers `ah up` and `session.realign` behavior when reprovisioning existing agents. It targets the observed "one agent disappears during realign, second ah up restores it" failure.

## Evidence

- `src/cli/up.rs:29-56` sends one `session.realign` request containing all configured agents.
- `src/rpc/handlers/realign.rs:193-199` snapshots current running agents once at the start.
- `src/rpc/handlers/realign.rs:199-298` then iterates requested agents sequentially.
- `src/rpc/handlers/realign.rs:225-228` deletes a `CRASHED` agent before spawning its replacement.
- `src/rpc/handlers/realign.rs:275-282` marks a drifted agent killed, deletes the agent row, then spawns the replacement.
- `src/rpc/handlers/realign.rs:300-334` handles orphan reporting after the requested-agent loop.
- `src/rpc/handlers/realign.rs:371-379` only uses `AgentSpawnDbAction::ReplaceKilledAndRequeue` when `captured_intent.is_some()`.
- `src/rpc/handlers/agent.rs:299-318` default spawn inserts a new row after physical tmux/process setup.
- `src/db/recovery.rs:420-469` already has an atomic delete+insert transaction for killed-agent replacement, but it is recovery-specific and requires the old row to be `KILLED`.

## Root Cause

`session.realign` is not atomic across the topology. For drifted or crashed agents it commits destructive state first (`mark_agent_killed`, `delete_agent`) and only then performs physical spawn plus DB insert. If any spawn step fails, times out, is interrupted, or the process dies between delete and insert, the agent row is gone. `ah up` has no all-or-nothing transaction around the multi-agent topology, so the result can be a partially applied topology with `active_agents` lower than expected.

The "a4 always disappears" pattern is consistent with deterministic iteration order, not with a4-specific semantics. `ah up` builds the `agents` array from config order, and realign processes it sequentially. If the failure budget/timeout is reached near the end of the loop, the last drifted or reprovisioned slot is the one most likely to be left between delete and spawn. In a four-agent config that makes `a4` the recurring victim. If map ordering is changed upstream, the victim may change.

## Best Fix

Make realign topology updates all-or-nothing at the DB level and avoid deleting an agent row before a replacement is durably ready.

### A. Replace Delete-Then-Spawn With Two-Phase Replacement

For each agent that needs replacement:

1. Spawn the physical replacement first, but do not publish it as the active agent row yet.
2. Once pane, pid, fifo, reader, pidfd, parser, and init probe registration are ready, atomically swap DB state:
   - old row becomes `KILLED` or is archived,
   - new row is inserted with the same `agent_id`,
   - config hash and spawn spec are persisted,
   - recovery backoff is cleared,
   - event rows are inserted.
3. If spawn fails before the DB swap, clean up the new physical resources and leave the old row untouched.

This can be implemented by generalizing the existing recovery primitive in `src/db/recovery.rs:412-469` into a realign replacement primitive that does not require `captured_intent`, or by adding a new `AgentSpawnDbAction::ReplaceExistingRealign`.

The replacement primitive should accept:

- `expected_old_state_version` from the initial running-agent snapshot.
- `old_allowed_states`, e.g. `IDLE`, `CRASHED`, and `BUSY` only when `force=true`.
- `replacement_state="SPAWNING"`.
- `config_hash`.
- `AgentSpawnSpec`.

It should fail without DB side effects if the old row state/version changed while realign was spawning.

### B. Add a Session-Scoped Realign Lock

Wrap `handle_session_realign` in a session-level async lock:

```text
realign_lock(session_id).lock().await
```

This prevents two `ah up` calls from interleaving and invalidating each other's initial topology snapshot. Master realign already has a master-specific lock; agent topology needs its own session-scoped lock.

### C. Track Replacement Plans Before Mutating

Before any destructive action:

- Compute expected hashes for every requested agent.
- Build a `RealignPlan` containing `NoChange`, `New`, `Replace`, `SkipBusy`, and `Orphan`.
- Validate duplicate requested ids and fail the request before mutation.
- Validate that all replacement candidates have spawn specs and provider manifests resolvable.

Then execute the plan. If validation fails, return an error and leave topology unchanged.

### D. Define Failure Semantics

Realign cannot be a single SQL transaction across physical tmux/process creation, so "atomic" here means:

- Before DB swap: failure leaves the old agent row and old active topology intact.
- During DB swap: one SQL transaction updates old/new row, config hash, spawn spec, and events together.
- After DB swap: if post-registration init probe fails later, normal lifecycle/recovery handles it; the topology still has the expected agent row.

For new agents, if physical spawn succeeds but DB insert fails, cleanup already exists in `handle_agent_spawn_with_db_action`; keep that behavior.

For replacement agents, the default should be old-row preservation on spawn failure. Do not delete first.

## Edge Cases

- `BUSY` without `--force`: keep current skip behavior and emit `drift_skipped`.
- `BUSY` with `--force`: replacement is allowed, but the swap must also resolve any current `DISPATCHED` job according to the dispatch reliability spec. At minimum it must fail/requeue the job before old row removal.
- `CRASHED`: replacement can skip preserving a live old process, but the DB row still must not disappear if replacement spawn fails.
- Orphans: audit-only must remain non-destructive. Force cleanup can remain destructive because removing extra agents is the explicit operation, but it should not run before requested replacements are durably complete.

## Task Breakdown

1. Add session realign lock.
   - Suggested test: `session_realign_serializes_concurrent_topology_updates`.
   - Run two concurrent realigns against the same session and assert no duplicate delete/insert interleaving and final agent count equals requested count.

2. Introduce `RealignPlan`.
   - Suggested test: `session_realign_rejects_duplicate_agent_ids_before_mutation`.
   - Suggested test: `session_realign_plan_validation_failure_leaves_topology_unchanged`.

3. Add atomic replacement DB action.
   - Suggested test: `realign_replace_existing_is_db_atomic`.
   - Use a test hook equivalent to `set_replace_killed_agent_after_delete_test_hook` to force an error inside the transaction; assert old row remains or transaction rolls back.
   - Suggested test: `realign_spawn_failure_preserves_old_agent_row`.
   - Inject a spawn failure after physical setup begins but before DB swap; assert the original agent row, config hash, pid, and active count are unchanged.

4. Rewire drift and crashed replacement paths.
   - Replace `delete_agent(...); spawn_realign_agent(...)` at `src/rpc/handlers/realign.rs:225-228` and `src/rpc/handlers/realign.rs:275-282`.
   - Use the new replacement action and pass expected old state/version from the initial snapshot.
   - Suggested test: `session_realign_crashed_spawn_failure_does_not_drop_agent`.
   - Suggested test: `session_realign_drift_spawn_failure_does_not_drop_last_agent`.

5. Add active count regression for the a4 incident.
   - Suggested test: `ah_up_realign_four_agents_failure_on_last_preserves_active_count`.
   - Seed four active agents `a1..a4`, force drift on all or on `a4`, inject replacement failure for `a4`, run `session.realign`, and assert active agent count is still four and `a4` row still exists.
   - Then remove the injected failure and rerun realign; assert `a4` is replaced successfully without needing manual cleanup.

## Acceptance Gates

- `cargo test --test pr4e_up_fingerprint`
- `cargo test --test ah_full_e2e_drift -- --include-ignored --test-threads=1`
- `cargo test --test ah_full_e2e_realign_extra -- --include-ignored --test-threads=1`
- New tests named above pass.
- Manual dogfood acceptance: repeated `ah up` during drift/reprovision never reduces active agent count below configured count unless the request explicitly force-removes orphans.
