# Recovery Reinsert Atomicity Design

## Problem Restatement

The master-revive worker reprovision path currently has an observable database gap for an interrupted job:

- `src/monitor/master_watch.rs:824-825` detects a KILLED worker and calls `crate::db::agents::delete_agent(...)`.
- `src/db/agents.rs:162-164` opens `db.conn()` and executes a bare `DELETE FROM agents WHERE id = ?`.
- `src/monitor/master_watch.rs:690-703` only calls `requeue_master_revive_interrupted_jobs_after_reprovision(...)` after reprovision returns.
- `src/monitor/master_watch.rs:977,995` opens another connection guard and calls recovery requeue.
- `src/db/recovery.rs:343-353` reinserts from the captured recovery intent through `insert_recovered_queued_job_sync`.
- `src/db/jobs.rs:72-99` executes a bare `INSERT INTO jobs (...)`.

Because `src/db/schema.rs:136-138` defines `jobs.agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE`, and `src/db/mod.rs:80-88` enables `PRAGMA foreign_keys = ON`, deleting the KILLED agent cascades through jobs before the later reinsert. During that committed interval, `src/db/jobs.rs:112-119` can return `Ok(None)` for the same job id.

The fix must only close this atomicity window. It must not redesign the larger semantics around master death, worker cleanup, anti-orphan cascading, or revive policy.

## Existing Transaction Surface

The codebase already uses rusqlite transactions through a mutable connection guard:

- `src/db/jobs.rs:147-151`: `let mut conn = db.conn(); let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;`
- `src/db/jobs.rs:222-224`, `src/db/jobs.rs:416-419`, and `src/db/jobs.rs:476-478` use the same pattern for state transitions.

The problematic path does not use that API today:

- Delete side: `src/db/agents.rs:162-164` takes `db.conn()` internally and performs a single autocommit `DELETE`.
- Reinsert side: `src/monitor/master_watch.rs:977` takes a separate `db.conn()` after reprovision; `src/db/recovery.rs:343-353` and `src/db/jobs.rs:72-99` insert the recovered job without a transaction.

The transaction API is sufficient, but the call chain must be changed. A transaction cannot just be wrapped around the current functions because the current order crosses an async reprovision boundary:

- `src/monitor/master_watch.rs:824-827` deletes the KILLED row, then awaits `spawn_realign_agent(...)`.
- `src/rpc/handlers/realign.rs:316-338` awaits `handle_agent_spawn_with_recovery(...)`.
- `src/rpc/handlers/agent.rs:242-250` inserts the replacement agent through async `insert_agent(...)`.
- `src/rpc/handlers/realign.rs:339-367` then updates config hash, persists spawn spec, and writes events.

Holding a rusqlite transaction or the `Db` mutex guard across that async spawn path would be the wrong boundary: it mixes external tmux/process side effects with a DB critical section and still leaves the replacement agent insert inside a separate helper.

## Recommended Scheme: Atomic DB Replacement Helper

Recommended approach: keep physical reprovision outside the DB transaction, but make the database replacement step for a KILLED worker atomic.

High-level flow:

1. Capture recovery intents before reprovision as today: `src/monitor/master_watch.rs:757-763`.
2. For each stored worker, pass its captured intent into the worker reprovision path instead of requeueing all intents afterward.
3. Spawn the physical worker first while the old KILLED agent and old job row remain committed and visible.
4. Once physical spawn data is available, run one synchronous DB helper that:
   - starts `TransactionBehavior::Immediate`;
   - confirms the existing agent is still KILLED;
   - deletes the old agent, triggering cascade inside the uncommitted transaction;
   - inserts the replacement agent row with the same id;
   - persists the replacement spawn spec/config hash;
   - reinserts the interrupted job with the captured job id;
   - commits.
5. After commit, write non-critical events that describe the new spawn. Old events/evidence are intentionally part of the old agent lifecycle and do not need to be atomically recreated.

The essential property is SQLite isolation: readers using other connections see the old committed state until the transaction commits, then see the replacement state. They never observe the uncommitted cascade-deleted intermediate state.

Boundary sketch:

```rust
// src/monitor/master_watch.rs
let captured_by_agent = collect_master_revive_recovery_intents_before_reprovision(...)?;
for stored in stored_specs {
    let intent = captured_by_agent.get(stored.spec.agent_id.as_str());
    revive_reprovision_one_worker(&ctx, session_id, &agent, stored.config_hash.as_str(), intent).await?;
}
// The post-reprovision bulk requeue at master_watch.rs:699-703 should no longer reinsert
// jobs for intents handled by the atomic replacement path.
```

```rust
// New sync helper, likely in src/db/recovery.rs or src/db/agents.rs.
pub(crate) fn replace_killed_agent_and_requeue_job_sync(
    db: &Db,
    session_id: &str,
    spec: &AgentSpawnSpec,
    config_hash: &str,
    pid: i64,
    captured_intent: Option<&AgentRecoveryIntent>,
) -> Result<usize, CcbdError> {
    let mut conn = db.conn();
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;

    let state: Option<String> = tx.query_row(
        "SELECT state FROM agents WHERE id = ?",
        params![spec.agent_id],
        |row| row.get(0),
    ).optional()?;

    if state.as_deref() == Some("KILLED") {
        tx.execute("DELETE FROM agents WHERE id = ?", params![spec.agent_id])?;
    }

    crate::db::agents::insert_agent_conn_sync(
        &tx,
        &spec.agent_id,
        session_id,
        &spec.provider,
        "SPAWNING",
        Some(pid),
    )?;
    crate::db::agents::update_agent_config_hash_conn_sync(&tx, &spec.agent_id, config_hash)?;
    crate::db::recovery::persist_agent_spawn_spec_sync(&tx, spec, config_hash)?;

    let requeued = match captured_intent {
        Some(intent) => crate::db::recovery::requeue_interrupted_job_from_captured_intent_sync(
            &tx,
            intent,
        )?,
        None => 0,
    };

    tx.commit()?;
    Ok(requeued)
}
```

The helper needs connection-oriented variants for operations that currently take `Db` and open their own connection:

- `delete_agent_sync` at `src/db/agents.rs:162-164` should either gain a `delete_agent_conn_sync(&Connection, ...)` sibling or be bypassed by the helper's direct `DELETE`.
- `insert_agent_sync` already accepts `&Connection` at `src/db/agents.rs:7-18`; a transaction can be passed through rusqlite's deref behavior.
- `update_agent_config_hash_sync` already accepts `&Connection` at `src/db/agents.rs:66-75`.
- `persist_agent_spawn_spec_sync` already accepts `&Connection` at `src/db/recovery.rs:363-380`.
- `requeue_interrupted_job_from_captured_intent_sync` accepts `&Connection` at `src/db/recovery.rs:284-287`.

This changes the reprovision/requeue ordering: the job reinsert happens inside each worker's DB replacement transaction, not in the later bulk pass at `src/monitor/master_watch.rs:699-703`. The later bulk function should either be removed for this path or skip intents marked as handled by the atomic helper.

## Five-Table Cascade Semantics

Deleting an agent currently cascades to five direct child tables:

- `agent_spawn_specs`: `src/db/schema.rs:73-74`.
- `agent_recovery_intents`: `src/db/schema.rs:82-83`.
- `events`: `src/db/schema.rs:105-107`.
- `evidence`: `src/db/schema.rs:118-120`.
- `jobs`: `src/db/schema.rs:136-138`.

There is also an indirect cascade from `jobs` to `evidence` through `src/db/schema.rs:126`.

Per-table treatment:

- `agent_spawn_specs`: must be atomically recreated. The reprovision path depends on the replacement worker having its current spawn spec. `src/rpc/handlers/realign.rs:371-387` already persists the realign snapshot after success; this should move into the atomic DB helper for this path.
- `agent_recovery_intents`: does not need to survive as a row. It is captured in memory before reprovision at `src/monitor/master_watch.rs:757-763`, and `src/db/recovery.rs:201-282` materializes the interrupted job payload into `AgentRecoveryIntent`. The captured value is the source of truth for the atomic job reinsert. Recreating the intent row is unnecessary after the job is requeued.
- `events`: old events describe the killed agent incarnation. Existing cascade semantics already discard them when replacing an agent with the same id. This design does not change that policy. New spawn events can be inserted after commit; they are useful audit data but not required to make `query_job` stable.
- `evidence`: evidence rows are tied to both `agent_id` and, sometimes, `job_id`. The direct agent cascade and the indirect job cascade mean old evidence is already discarded today. This design keeps that behavior. Recreating evidence is outside scope because the captured recovery intent only contains the job payload fields needed to requeue.
- `jobs`: must be atomically recreated when a captured interrupted job exists. It is the only child table whose temporary absence is the target bug. Reinsert uses the same id through `src/db/recovery.rs:343-353` and `src/db/jobs.rs:72-99`.

## Alternative: Detach Then Reattach Job

Detach-then-reattach would mean:

1. Set the interrupted job's `agent_id` to `NULL` or another non-cascading owner.
2. Delete the KILLED agent.
3. Reprovision the replacement agent.
4. Set the job's `agent_id` back to the replacement agent id.

The current schema does not allow this directly. `src/db/schema.rs:138` declares `agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE`, so `jobs.agent_id = NULL` is invalid. Using a sentinel agent would preserve the NOT NULL constraint but would create false ownership and would still require careful FK/cascade semantics. Making `jobs.agent_id` nullable would require a schema migration and would expose a new domain state: a queued job without an agent.

Pros:

- The job row can remain present across the operation even if physical spawn happens between detach and reattach.
- It avoids deleting/reinserting the job row itself.

Cons:

- Requires schema migration from NOT NULL to nullable or introduces a sentinel owner.
- Forces all job query/dispatch paths to handle agentless jobs.
- Does not address `agent_spawn_specs`, `agent_recovery_intents`, `events`, or `evidence` semantics.
- Broadens product behavior beyond the narrow atomicity window.

Conclusion: detach-then-reattach is not recommended for this scope. It is larger and changes the job ownership model. The single DB replacement transaction keeps the existing schema and lifecycle semantics.

## State Version and CAS

`jobs` has no `state_version` or other optimistic concurrency column in `src/db/schema.rs:136-150`, and `src/db/jobs.rs:30-46` maps no version field into `Job`. Reinsert does not need to preserve a job CAS token because none exists.

`agents` does have `state_version` at `src/db/schema.rs:58`, and recovery CAS logic uses agent versions elsewhere, for example `src/db/recovery.rs:468-480`. The recommended helper replaces a KILLED agent row with a new incarnation using the same id. That is consistent with the current delete-and-insert behavior, where the new row receives default `state_version = 1`. This design should not attempt to preserve the old KILLED row's `state_version`; doing so would change agent lifecycle semantics and is outside scope.

If the implementation wants an extra guard, the atomic helper can read and require `state = 'KILLED'` immediately before delete. It should not add a new CAS contract unless another caller already supplies the expected version.

## Test Strategy

The red test should assert the observable contract: while master-revive worker reprovision is in progress, concurrent `query_job(job_id)` never returns `None` for the interrupted job. It should not depend on load or sleep timing alone.

Recommended shape:

- Add a test-only hook or channel around the atomic helper boundary, not in product behavior. The hook should pause after the uncommitted delete and before the uncommitted reinsert inside the transaction.
- From a second connection, call `query_job_sync` or the async `query_job` during that pause.
- Assert the second connection sees `Some(job)` while the transaction is still open. In WAL mode with FK enabled, it should see the pre-transaction committed job row.
- Release the hook, let the transaction commit, then assert `query_job(job_id)` is still `Some(job)` and has the requeued fields expected from `insert_recovered_queued_job_sync`: status `QUEUED`, same id, same request id/prompt, recovered error marker.

This is not load-sensitive because the test controls the exact internal pause point. A second integration-style test can cover the full master-revive path, but the atomicity regression should live at the DB helper level where the interleaving is deterministic.

The existing comments around `src/monitor/master_watch.rs:1601-1604` already describe the observed delete-reinsert window; the new test should replace that expectation with the invariant that the job is continuously visible.

## Risks

- Refactoring the spawn path can accidentally duplicate agent insertion. The new realign/recovery path must ensure `src/rpc/handlers/agent.rs:242-250` does not also insert the same agent outside the atomic helper for this scenario.
- Moving spawn spec persistence into the atomic helper must preserve the existing behavior from `src/rpc/handlers/realign.rs:371-387`.
- Event ordering changes slightly if old events are deleted in the transaction and new `agent_spawned` is inserted after commit. That is acceptable because old events were already cascaded away by the delete; the fix does not promise event preservation.
- If a physical spawn succeeds but the DB transaction fails, cleanup must mirror the current insert failure cleanup path in `src/rpc/handlers/agent.rs:251-253`: tear down the spawned pane/fifo/process resources and leave the old committed KILLED agent/job visible.

## Scope Statement

This proposal only closes the committed database window where a recovered interrupted job disappears between agent cascade delete and same-id job reinsert. It does not change the policy that replacing a KILLED agent deletes old events/evidence/recovery-intent rows, and it does not redesign master death propagation, anti-orphan cleanup, or broader revive semantics.

