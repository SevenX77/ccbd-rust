# ah Job Events Runtime Stream Design

## A. Pinned Carrier Decision

Decision: add a focused `job_transitions` table. `job_events[]` is a projection of `job_transitions`, not of the existing `events` table.

Evidence:

- `ah pend` waits through `event.subscribe` with `event_kind:["job_state_change"]` and a `job_id` at `src/bin/ah.rs:1442` through `src/bin/ah.rs:1449`.
- `event.subscribe` has a special terminal-job path when `job_id` plus `job_state_change` is requested: `should_use_job_terminal_subscription` returns true at `src/rpc/handlers/events.rs:162` through `src/rpc/handlers/events.rs:167`, then `stream_event_subscribe` reads `JOB_UPDATES` and calls `event_frame_for_job` at `src/rpc/handlers/events.rs:93` through `src/rpc/handlers/events.rs:105`.
- `event_frame_for_job` queries the `jobs` row, requires terminal `status`, and synthesizes a JSON frame with `event_id: job.completed_at.unwrap_or(0)`, `kind: "job_state_change"`, and `state: job.status` at `src/rpc/handlers/events.rs:289` through `src/rpc/handlers/events.rs:310`. That `event_id` is not an `events.seq_id` row id.
- The live `EventFrame` carrier is broadcast-only: `EVENT_FRAMES` is a `tokio::sync::broadcast::Sender<EventFrame>` at `src/orchestrator/pubsub.rs:25` through `src/orchestrator/pubsub.rs:28`, `notify_event` only sends to that channel at `src/orchestrator/pubsub.rs:53` through `src/orchestrator/pubsub.rs:55`, and `subscribe_events` only subscribes to it at `src/orchestrator/pubsub.rs:57` through `src/orchestrator/pubsub.rs:59`.
- The durable `events` table exists with `seq_id INTEGER PRIMARY KEY AUTOINCREMENT`, `agent_id`, `request_id`, `event_type`, `payload`, and `created_at` at `src/db/schema.rs:124` through `src/db/schema.rs:131`; `query_events_backfill_sync` projects those persisted rows into `EventFrame` with `event_id: seq_id` at `src/db/events.rs:141` through `src/db/events.rs:177` and `src/db/events.rs:343` through `src/db/events.rs:372`.
- However, durable event backfill is not the `ah pend` job terminal path. `subscription_backfill` only runs when `since_seq_id` is supplied at `src/rpc/handlers/events.rs:169` through `src/rpc/handlers/events.rs:187`; ordinary `ah pend` supplies no `since_seq_id` at `src/bin/ah.rs:1445` through `src/bin/ah.rs:1449`.
- Existing job-state terminal frames are therefore synthesized from `jobs`, notified by `JOB_UPDATES`, and use `completed_at` as their id. They are not durably persisted `job_state_change` rows with a monotonic event id.

The existing `events` table should remain the provider/agent output and state-change event log. Job lifecycle edges need different semantics: old/new job status, non-status job updates, job-level ordering, and same-transaction insertion next to job status writes. A focused `job_transitions` table avoids overloading `events.event_type`/`payload` and provides a clean monotonic `job_event_id`.

## B. Runtime Schema v2

Bump `RuntimeSnapshot.schema_version` from `1` to `2`. Current active snapshots set `schema_version: 1` at `src/runtime_events.rs:239` through `src/runtime_events.rs:247`; inactive snapshots set it at `src/runtime_events.rs:121` through `src/runtime_events.rs:141`.

Keep every runtime stream line snapshot-shaped:

```json
{
  "schema_version": 2,
  "event": "snapshot",
  "sequence": 42,
  "reason": "job_changed",
  "jobs": [],
  "job_events": [],
  "job_event_cursor": 12345
}
```

Additive fields:

- `jobs`: bounded current job state. Include all non-terminal `QUEUED` and `DISPATCHED` jobs plus recent terminal jobs under a time/count cap. Exclude `prompt_text` and unbounded `reply_text`.
- `job_events`: durable transition/update rows included in this snapshot. A job-change snapshot should include the transition(s) that caused it when available; initial snapshots may include an empty array until `since_job_event_id` exists.
- `job_event_cursor`: highest `job_transitions.job_event_id` included or observed while building the snapshot. It is for de-duplication and future resume, not a replacement for stream `sequence`.

Recommended `jobs[]` object:

```json
{
  "job_id": "job_...",
  "agent_id": "a1",
  "request_id": "optional-client-key",
  "status": "DISPATCHED",
  "cancel_requested": false,
  "created_at": 1783478400,
  "dispatched_at": 1783478402,
  "completed_at": null,
  "error_reason": null,
  "requires_physical_evidence": false,
  "requires_test_evidence": false
}
```

Recommended `job_events[]` object:

```json
{
  "event_id": 12345,
  "kind": "job_transition",
  "job_id": "job_...",
  "agent_id": "a1",
  "request_id": "optional-client-key",
  "old_status": "DISPATCHED",
  "new_status": "COMPLETED",
  "changed": ["status", "reply_text", "completed_at"],
  "cancel_requested": false,
  "created_at": 1783478400,
  "dispatched_at": 1783478402,
  "completed_at": 1783478410,
  "error_reason": null,
  "reason": "idle_marker"
}
```

For cancel requests, use `kind: "job_updated"` and `changed: ["cancel_requested"]`; do not fake an `old_status == new_status` transition.

Proposed `job_transitions` schema:

```sql
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

## C. Emission Placement and Coverage

Introduce one transaction-aware helper:

```text
record_job_transition_conn_sync(conn, job_id, old_status, new_status, kind, reason, changed_fields)
```

Requirements:

- The helper inserts into `job_transitions` using the same `Connection` or transaction as the job mutation.
- For status changes, call it only after the `UPDATE`/`INSERT` is known to have affected one or more rows.
- For single-row CAS updates, no changed row means no transition row.
- For bulk updates, collect the affected job ids before the update inside the same transaction, then insert one transition row per affected job after the update succeeds.
- The helper must read the post-mutation job row to populate `agent_id`, `request_id`, timestamps, `cancel_requested`, and `error_reason`.
- After commit, the async wrapper or caller must notify both existing `JOB_UPDATES` compatibility paths and `RUNTIME_UPDATES` with `RuntimeSnapshotReason::JobChanged`.

Full mutation coverage:

- Submit: `null -> QUEUED` for `insert_job_sync` at `src/db/jobs.rs:49` through `src/db/jobs.rs:69` and recovered insertion at `src/db/jobs.rs:72` through `src/db/jobs.rs:109`.
- Dispatch: `QUEUED -> DISPATCHED` from `claim_next_job_sync` at `src/db/jobs.rs:166` through `src/db/jobs.rs:230`.
- Complete: `DISPATCHED -> COMPLETED` from `mark_job_completed_conn_sync` at `src/db/jobs.rs:374` through `src/db/jobs.rs:383`.
- Cancel queued: `QUEUED -> CANCELLED` from `mark_queued_job_cancelled_conn_sync` at `src/db/jobs.rs:391` through `src/db/jobs.rs:399`.
- Cancel requested: `job_updated` with `changed:["cancel_requested"]` from `request_dispatched_job_cancel_sync`, which currently only sets `cancel_requested = 1` for `DISPATCHED` at `src/db/jobs.rs:402` through `src/db/jobs.rs:411`.
- Cancel settled: `DISPATCHED -> CANCELLED` from `mark_dispatched_job_cancelled_if_agent_idle_sync` at `src/db/jobs.rs:431` through `src/db/jobs.rs:452` and `mark_job_cancelled_conn_sync` at `src/db/jobs.rs:455` through `src/db/jobs.rs:464`.
- Fail: `QUEUED|DISPATCHED -> FAILED` from `mark_job_failed_conn_sync` at `src/db/jobs.rs:476` through `src/db/jobs.rs:485`.
- Requeue: `DISPATCHED -> QUEUED` from `requeue_recovered_dispatch_io_failure_sync` at `src/db/jobs.rs:488` through `src/db/jobs.rs:547` and `requeue_dispatched_job_before_send_sync` at `src/db/jobs.rs:549` through `src/db/jobs.rs:570`.
- Bulk fail: one transition per affected job for `mark_dispatched_jobs_failed_for_agent_conn_sync` at `src/db/jobs.rs:627` through `src/db/jobs.rs:636`. Today the async wrapper queries only one affected dispatched job before the update at `src/db/jobs.rs:1150` through `src/db/jobs.rs:1168`; the transition writer must not rely on that one-job assumption.
- State-machine completion: ensure the shared connection-level completion/cancellation helpers used by idle marker, hook, and log paths emit transitions. Those paths call `mark_job_cancelled_conn_sync` or `mark_job_completed_conn_sync` at `src/db/state_machine.rs:731` through `src/db/state_machine.rs:737`, `src/db/state_machine.rs:938` through `src/db/state_machine.rs:945`, and `src/db/state_machine.rs:1099` through `src/db/state_machine.rs:1105`.
- STUCK: `DISPATCHED -> FAILED` from the STUCK path that updates agents at `src/db/state_machine.rs:1437` through `src/db/state_machine.rs:1442` and updates the affected job to `FAILED` at `src/db/state_machine.rs:1444` through `src/db/state_machine.rs:1456`.
- Evidence-denial/unknown path: cancel or fail the affected dispatched job when evidence handling transitions the agent to `UNKNOWN` at `src/db/state_machine.rs:1572` through `src/db/state_machine.rs:1589`.
- Recovery: requeue or recovered-create events from `requeue_interrupted_job_from_captured_intent_sync` at `src/db/recovery.rs:335` through `src/db/recovery.rs:405`. Replacement and requeue are one transaction at `src/db/recovery.rs:414` through `src/db/recovery.rs:476`, so the transition row belongs in that transaction. Master revive also reaches the same recovery helper at `src/monitor/master_watch.rs:2206` through `src/monitor/master_watch.rs:2231`.

## D. Runtime Notification Wiring

Add `RuntimeSnapshotReason::JobChanged`. Current reason variants are `initial`, `inventory_changed`, `tmux_changed`, `agent_changed`, `shutdown`, `daemon_absent`, and `daemon_lost` at `src/runtime_events.rs:9` through `src/runtime_events.rs:19`.

Runtime subscribers currently suppress unchanged snapshots by fingerprint: the subscription initializes `last_fingerprint` at `src/rpc/handlers/runtime.rs:40` through `src/rpc/handlers/runtime.rs:52`, receives `RUNTIME_UPDATES` or interval ticks at `src/rpc/handlers/runtime.rs:54` through `src/rpc/handlers/runtime.rs:66`, then emits only when the candidate fingerprint differs at `src/rpc/handlers/runtime.rs:67` through `src/rpc/handlers/runtime.rs:83`. The fingerprint removes only `sequence` and `reason` at `src/runtime_events.rs:261` through `src/runtime_events.rs:270`.

Requirements:

- Extend `RuntimeSnapshot` with `jobs`, `job_events`, and `job_event_cursor`, and ensure those fields are included in `runtime_snapshot_fingerprint`.
- Add a regression test where a job-only change changes the fingerprint and causes `runtime.subscribe` to emit; without this, `RuntimeSnapshotReason::JobChanged` alone is insufficient because `reason` is removed from the fingerprint.
- Notify `RUNTIME_UPDATES` after job transition commit with `RuntimeSnapshotReason::JobChanged`.
- Keep existing `notify_job_update` calls for `job.wait`/`ah pend` compatibility.
- Fix the current cancel-request notification gap: `request_dispatched_job_cancel` at `src/db/jobs.rs:1041` through `src/db/jobs.rs:1046` currently returns the DB result without `notify_job_update`; the new wrapper must notify `JOB_UPDATES` and `RUNTIME_UPDATES` when changes > 0.

## E. jq Consumer Contract

`ah events --format json` remains line-delimited JSON. Every line is still a snapshot. Consumers should treat `schema_version >= 2` as supporting optional `.jobs[]`, `.job_events[]`, and `.job_event_cursor`.

Wait for one job terminal edge:

```bash
ah events --format json |
  jq -rc --arg job "$JOB_ID" '
    select(.schema_version >= 2)
    | .job_events[]?
    | select(.job_id == $job)
    | select(.new_status == "COMPLETED" or .new_status == "FAILED" or .new_status == "CANCELLED" or .new_status == "KILLED")
  ' |
  head -n 1
```

Read current queued/dispatched work:

```bash
ah events --format json |
  jq -rc '
    select(.schema_version >= 2)
    | .jobs[]?
    | select(.status == "QUEUED" or .status == "DISPATCHED")
  '
```

Track all terminal transitions:

```bash
ah events --format json |
  jq -rc '
    select(.schema_version >= 2)
    | .job_events[]?
    | select(.new_status == "COMPLETED" or .new_status == "FAILED" or .new_status == "CANCELLED" or .new_status == "KILLED")
    | {event_id, job_id, agent_id, request_id, status: .new_status, error_reason, completed_at}
  '
```

Consumer rules:

- Use `.job_events[]` for edges.
- Use `.jobs[]` for convergence after reconnect or daemon restart.
- De-duplicate `.job_events[]` by `event_id`.
- Treat `reason` as advisory; initial and recovery snapshots may contain useful job state even when `reason != "job_changed"`.
- Treat unknown job statuses as non-terminal unless the consumer explicitly knows them.

Reconnect caveat: until `runtime.subscribe` accepts `since_job_event_id`, a reconnecting consumer can miss a terminal edge if that edge has scrolled past both the bounded `jobs[]` terminal window and the immediate `job_events[]` delta. The convergence story is `jobs[]` for current state and recent terminal state, `job_events[]` for edges observed while connected, and future `since_job_event_id` for exact replay.

## F. Compatibility and Reason Strategy

Compatibility requirements:

- Do not introduce a second top-level runtime frame shape. `runtime.subscribe` serializes `RuntimeSnapshot` directly at `src/rpc/handlers/runtime.rs:87` through `src/rpc/handlers/runtime.rs:105`, and `ah events` streams daemon JSON lines directly at `src/bin/ah.rs:1310` through `src/bin/ah.rs:1343`.
- Keep `event: "snapshot"`.
- Use additive fields and `schema_version: 2`.
- Keep `sequence` as a per-subscription sequence. It starts at `1` for each runtime subscription at `src/rpc/handlers/runtime.rs:40` through `src/rpc/handlers/runtime.rs:52`; it is not a durable cursor.
- Use `job_events[].event_id` and `job_event_cursor` as durable job event ids.
- Emit `reason: "job_changed"` for job-driven updates in v2. External consumers must treat `reason` as extensible. If a transitional release needs strict v1 enum safety, job-driven snapshots may temporarily use `inventory_changed`, but the long-term v2 value is `job_changed`.

## G. Acceptance Criteria

Concrete tests:

- Carrier decision regression: a terminal `job_state_change` produced for `ah pend` is not assumed to be an `events.seq_id`; new `job_events[].event_id` comes from `job_transitions.job_event_id` and is monotonic across job transitions.
- Schema v2 snapshot: active and inactive runtime snapshots serialize with `schema_version: 2` and include `jobs`, `job_events`, and `job_event_cursor` fields.
- Runtime fingerprint: changing only `jobs[]` or only `job_events[]` changes `runtime_snapshot_fingerprint`; changing only `sequence` or `reason` still does not.
- Runtime subscribe gate: a `RuntimeSnapshotReason::JobChanged` notification after a job-only DB mutation emits a new runtime snapshot instead of being suppressed by the fingerprint gate.
- Submit and dispatch: inserting a new job records `null -> QUEUED`; claiming it records `QUEUED -> DISPATCHED`.
- Completion: state-machine idle-marker, hook, and log completion paths record `DISPATCHED -> COMPLETED` or `DISPATCHED -> CANCELLED`.
- Cancel queued: queued cancellation records `QUEUED -> CANCELLED` and appears in `job_events[]`.
- Cancel requested: `request_dispatched_job_cancel` records `kind:"job_updated"`, `changed:["cancel_requested"]`, notifies `JOB_UPDATES`, notifies runtime `job_changed`, and appears in `job_events[]`.
- Cancel settled: cancel settlement records `DISPATCHED -> CANCELLED`.
- Fail: direct fail records `QUEUED|DISPATCHED -> FAILED`.
- Requeue: recovered dispatch I/O failure and pre-send requeue record `DISPATCHED -> QUEUED`.
- Bulk fail: failing dispatched jobs for an agent records one transition per affected job.
- STUCK: STUCK handling records the affected job `DISPATCHED -> FAILED`.
- Evidence-denial/unknown path: cancel-requested dispatched jobs record cancellation and non-cancel-requested dispatched jobs record failure.
- Recovery: revive recovery records requeue or recovered-create transition rows inside the same transaction as replacement/requeue.
- jq contract: sample jq filters work against serialized v2 snapshots with empty arrays, non-empty `jobs[]`, terminal `job_events[]`, and cancel-request `job_updated` records.
- Reconnect limitation: a test or doc assertion verifies there is no `since_job_event_id` parameter yet and documents that exact edge replay after reconnect is not guaranteed.

The relevant command to run after implementation is the focused `cargo test` set covering `runtime_events`, `rpc::handlers::runtime`, `db::jobs`, `db::state_machine`, and `db::recovery`, plus any new integration tests for `ah events --format json`. This spec job does not run cargo.

## H. Task Breakdown

T1. Add `job_transitions` schema, indexes, Rust row types, insert/query helpers, and migration/version handling if this project has explicit DB migrations beyond `schema.rs`.

T2. Implement `record_job_transition_conn_sync` and a query helper for bounded `jobs[]`, recent `job_events[]`, and `job_event_cursor`.

T3. Wire all normal job helpers: submit, recovered insert, dispatch, complete, queued cancel, cancel request, cancel settlement, fail, requeue, and bulk fail.

T4. Wire state-machine and recovery paths: idle-marker, hook, log, STUCK, evidence-denial/unknown, revive recovery, and master-revive recovery.

T5. Add `RuntimeSnapshotReason::JobChanged`, schema v2 fields, runtime inventory/job queries, and fingerprint inclusion.

T6. Add post-commit notification wiring for `RUNTIME_UPDATES` and preserve or repair `JOB_UPDATES`, including the current cancel-request gap.

T7. Add serialization and jq-contract tests for `jobs[]`, `job_events[]`, `job_event_cursor`, terminal transitions, and `job_updated` cancel-request records.

T8. Add regression tests for runtime fingerprint suppression and the reconnect limitation until `since_job_event_id` exists.
