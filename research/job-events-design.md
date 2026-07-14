# Job Events in the Runtime Stream

## Current Grounding

`RuntimeSnapshot` is the payload used by the runtime stream today. It has `schema_version`, `event`, `sequence`, `reason`, runtime health fields, and then only `sessions` plus `agents` collections; there is no job collection or job transition field in the struct as of `src/runtime_events.rs:49` through `src/runtime_events.rs:69`. The nested runtime collections are session snapshots at `src/runtime_events.rs:71` through `src/runtime_events.rs:83` and agent snapshots at `src/runtime_events.rs:85` through `src/runtime_events.rs:95`.

The current active snapshot builder queries sessions and agents only: `build_runtime_snapshot` obtains `(sessions, agents)` from `query_runtime_inventory` at `src/runtime_events.rs:144` through `src/runtime_events.rs:150`, fills session rows at `src/runtime_events.rs:170` through `src/runtime_events.rs:181`, fills agent rows at `src/runtime_events.rs:184` through `src/runtime_events.rs:214`, and emits a snapshot with `schema_version: 1` plus `event: "snapshot"` at `src/runtime_events.rs:239` through `src/runtime_events.rs:245`. The inventory query itself selects from `sessions` at `src/runtime_events.rs:316` through `src/runtime_events.rs:326` and from `agents` at `src/runtime_events.rs:350` through `src/runtime_events.rs:356`.

`runtime.subscribe` sends an initial snapshot, then suppresses unchanged snapshots by fingerprint. The stream starts with `sequence = 1` at `src/rpc/handlers/runtime.rs:25` through `src/rpc/handlers/runtime.rs:52`, then listens for `RUNTIME_UPDATES` and interval ticks, builds a candidate with `sequence + 1`, and only increments the sequence when the candidate fingerprint differs at `src/rpc/handlers/runtime.rs:54` through `src/rpc/handlers/runtime.rs:83`. Each emitted snapshot is serialized as one JSON line at `src/rpc/handlers/runtime.rs:87` through `src/rpc/handlers/runtime.rs:105`.

The CLI `ah events` command only accepts `--format json` at `src/bin/ah.rs:1310` through `src/bin/ah.rs:1315`, connects to `runtime.subscribe`, and streams daemon JSON lines directly to stdout at `src/bin/ah.rs:1317` through `src/bin/ah.rs:1343`. On daemon absence or loss, the CLI emits local inactive snapshots and reconnects at `src/bin/ah.rs:1345` through `src/bin/ah.rs:1399`.

The separate `event.subscribe` path already exists, but it is not the runtime stream. RPC dispatch supports both `event.subscribe` and `runtime.subscribe` as separate streaming methods at `src/rpc/mod.rs:64` through `src/rpc/mod.rs:75`, and the router lists both methods separately at `src/rpc/router.rs:38` through `src/rpc/router.rs:45`. `ah pend` currently waits through `event.subscribe` with `event_kind:["job_state_change"]` and a specific `job_id` at `src/bin/ah.rs:1442` through `src/bin/ah.rs:1465`.

The current job table has the status and lifecycle timestamps needed for a runtime job view: `id`, `agent_id`, `request_id`, `status`, `error_reason`, `created_at`, `dispatched_at`, `dispatched_at_seq_id`, `completed_at`, and `cancel_requested` are schema columns at `src/db/schema.rs:155` through `src/db/schema.rs:170`. The queue index covers `QUEUED` and `DISPATCHED` jobs at `src/db/schema.rs:172`, and idempotent submit is indexed by `(agent_id, request_id)` at `src/db/schema.rs:173`.

Today, job status writes are spread across multiple DB helpers and state-machine paths. Job insertion writes `QUEUED` at `src/db/jobs.rs:49` through `src/db/jobs.rs:69`; recovered insertion also writes `QUEUED` at `src/db/jobs.rs:72` through `src/db/jobs.rs:109`; claiming a job updates `QUEUED` to `DISPATCHED` at `src/db/jobs.rs:166` through `src/db/jobs.rs:214`; completion updates `DISPATCHED` to `COMPLETED` at `src/db/jobs.rs:365` through `src/db/jobs.rs:383`; queued cancellation updates `QUEUED` to `CANCELLED` at `src/db/jobs.rs:386` through `src/db/jobs.rs:399`; cancel request sets `cancel_requested = 1` for `DISPATCHED` without changing `status` at `src/db/jobs.rs:402` through `src/db/jobs.rs:411`; idle settlement can update a requested `DISPATCHED` job to `CANCELLED` at `src/db/jobs.rs:431` through `src/db/jobs.rs:452`; direct dispatched cancellation updates `DISPATCHED` to `CANCELLED` at `src/db/jobs.rs:455` through `src/db/jobs.rs:464`; failure updates `QUEUED` or `DISPATCHED` to `FAILED` at `src/db/jobs.rs:467` through `src/db/jobs.rs:485`; recovery requeue updates `DISPATCHED` to `QUEUED` at `src/db/jobs.rs:488` through `src/db/jobs.rs:547`; pre-send requeue starts its `DISPATCHED` to `QUEUED` update path at `src/db/jobs.rs:549` through `src/db/jobs.rs:570`; and bulk agent failure updates `DISPATCHED` jobs for one agent to `FAILED` at `src/db/jobs.rs:627` through `src/db/jobs.rs:636`.

Some job completions are caused by state-machine paths rather than direct RPC job helpers. Idle marker matching completes or cancels the dispatched job at `src/db/state_machine.rs:720` through `src/db/state_machine.rs:737`; hook events do the same at `src/db/state_machine.rs:931` through `src/db/state_machine.rs:945`; log events do the same at `src/db/state_machine.rs:1094` through `src/db/state_machine.rs:1105`; STUCK handling updates an agent to `STUCK` and the affected dispatched job to `FAILED` at `src/db/state_machine.rs:1439` through `src/db/state_machine.rs:1456`; and evidence-denial handling cancels or fails an affected dispatched job at `src/db/state_machine.rs:1578` through `src/db/state_machine.rs:1590`.

Recovery can also transition jobs. Captured interrupted job metadata is persisted in recovery intent records at `src/db/recovery.rs:157` through `src/db/recovery.rs:240`. Revive recovery can update a captured failed job back to `QUEUED` at `src/db/recovery.rs:335` through `src/db/recovery.rs:378`, or reinsert a missing captured job as recovered `QUEUED` work at `src/db/recovery.rs:380` through `src/db/recovery.rs:405`. Replacement and requeue happen in one recovery transaction at `src/db/recovery.rs:414` through `src/db/recovery.rs:475`. Master revive also requeues captured interrupted jobs at `src/monitor/master_watch.rs:2206` through `src/monitor/master_watch.rs:2232`.

The current pubsub layer has separate runtime and job update channels. `RUNTIME_UPDATES` carries only `RuntimeSnapshotReason` values at `src/orchestrator/pubsub.rs:30` through `src/orchestrator/pubsub.rs:35`, while `JOB_UPDATES` carries only a job id string at `src/orchestrator/pubsub.rs:15` through `src/orchestrator/pubsub.rs:18` and `src/orchestrator/pubsub.rs:37` through `src/orchestrator/pubsub.rs:43`. Existing async wrappers notify `JOB_UPDATES` after several successful changes at `src/db/jobs.rs:1010` through `src/db/jobs.rs:1137` and `src/db/jobs.rs:1150` through `src/db/jobs.rs:1168`, but cancel request itself does not notify because `request_dispatched_job_cancel` only returns the DB result at `src/db/jobs.rs:1041` through `src/db/jobs.rs:1046`.

## 1. Carrier Trade-Offs

### Option A: Add `jobs[]` to every `RuntimeSnapshot`

This is the simplest mental model for consumers. Every `ah events --format json` line remains a snapshot, and a consumer can derive the latest known job state from `.jobs[]` without subscribing to a second RPC method. It also recovers well from missed broadcasts because the next snapshot reflects database truth.

The cost is payload growth and weaker transition fidelity. If `jobs[]` contains all historical jobs, the runtime stream grows without bound. If it contains only live jobs, terminal transitions can disappear before a consumer observes them. If it contains bounded recent terminal jobs, it is good for state convergence but still does not explicitly encode `old_status -> new_status` edges.

Use this only as the state carrier, not as the only transition carrier.

Recommended `jobs[]` content:

- Include all non-terminal jobs: `QUEUED` and `DISPATCHED`.
- Include bounded recent terminal jobs: `COMPLETED`, `FAILED`, `CANCELLED`, and `KILLED` if that status is ever introduced in the persisted job domain.
- Bound by time and count, for example terminal jobs completed in the last 10 minutes plus a hard cap of the latest 100 terminal jobs per workspace.
- Exclude full `prompt_text` and unbounded `reply_text` from snapshots. Include `job_id`, `agent_id`, `request_id`, `status`, `cancel_requested`, `created_at`, `dispatched_at`, `completed_at`, `error_reason`, and a compact `terminal_summary` only if needed.

### Option B: Add discrete job-transition frames

Discrete frames are the most faithful carrier for consumers that need to wait on state edges. A frame can represent one transition with `job_id`, `agent_id`, `old_status`, `new_status`, `cancel_requested`, timestamps, `error_reason`, and an ordered event id. This avoids polling and avoids scanning a full snapshot for every status edge.

The compatibility cost is high if `runtime.subscribe` starts emitting a second top-level JSON shape. Current `ah events` streams raw JSON lines from `runtime.subscribe` at `src/bin/ah.rs:1317` through `src/bin/ah.rs:1343`, and current runtime frames are snapshots serialized from `RuntimeSnapshot` at `src/rpc/handlers/runtime.rs:87` through `src/rpc/handlers/runtime.rs:105`. A new top-level frame type would force every consumer to branch on frame shape.

Use discrete transition records, but carry them additively inside snapshot-shaped runtime frames unless the RPC API is explicitly version-negotiated.

### Option C: Hybrid: `jobs[]` state plus `job_events[]` transition delta

This is the recommended design.

Keep `event: "snapshot"` and keep the current top-level snapshot shape. Add optional fields:

```json
{
  "schema_version": 2,
  "event": "snapshot",
  "sequence": 42,
  "reason": "job_changed",
  "jobs": [],
  "job_events": []
}
```

`jobs[]` is the bounded state view. `job_events[]` is the transition delta that caused this emission, usually one element but allowed to contain multiple transitions if the daemon coalesces updates before building the snapshot. This preserves convergence after missed broadcasts and preserves edge fidelity for consumers replacing `pend` polling.

Recommended `job_events[]` record:

```json
{
  "event_id": 12345,
  "job_id": "job_...",
  "agent_id": "a1",
  "request_id": "optional-client-key",
  "old_status": "DISPATCHED",
  "new_status": "COMPLETED",
  "cancel_requested": false,
  "created_at": "2026-07-08T00:00:00Z",
  "dispatched_at": "2026-07-08T00:00:02Z",
  "completed_at": "2026-07-08T00:00:10Z",
  "error_reason": null,
  "reason": "idle_marker"
}
```

For non-status mutations such as `cancel_requested = 1`, emit a `job_events[]` record with `kind: "job_updated"` and `changed: ["cancel_requested"]`, or use a sibling field named `job_updates[]`. Do not fake `old_status == new_status` as a transition unless the contract explicitly calls it an update. The current cancel-request write changes `cancel_requested` without changing `status` at `src/db/jobs.rs:402` through `src/db/jobs.rs:411`, so a distinct update event is more honest.

## 2. Emission Placement and Miss/Duplicate Analysis

Emit from the database mutation boundary, not from orchestrator control flow. The persisted job state can change in normal job helpers, state-machine completion paths, STUCK handling, recovery requeue, and master-revive code. Orchestrator-only emission would miss state-machine completions at `src/db/state_machine.rs:720` through `src/db/state_machine.rs:737`, STUCK job failure at `src/db/state_machine.rs:1439` through `src/db/state_machine.rs:1456`, and recovery requeue at `src/db/recovery.rs:335` through `src/db/recovery.rs:405`.

The implementation should introduce one transaction-aware helper, conceptually:

```text
record_job_transition_conn_sync(conn, job_id, old_status, new_status, source, changed_fields)
```

Every status-changing helper should call it in the same transaction as the job update, after the update is known to have affected rows. For single-row CAS updates, emit only when the update row count is greater than zero. That matches the existing async notification pattern, where wrappers notify `JOB_UPDATES` only after `changes > 0` at `src/db/jobs.rs:1020` through `src/db/jobs.rs:1024`, `src/db/jobs.rs:1033` through `src/db/jobs.rs:1038`, `src/db/jobs.rs:1058` through `src/db/jobs.rs:1063`, `src/db/jobs.rs:1076` through `src/db/jobs.rs:1080`, `src/db/jobs.rs:1104` through `src/db/jobs.rs:1109`, and `src/db/jobs.rs:1132` through `src/db/jobs.rs:1137`.

Prefer a durable transition log over broadcast-only delivery. A broadcast after commit can be lost if the daemon crashes between commit and notify; a durable row inserted in the same transaction can be replayed on the next snapshot or by `since_event_id`. The project already has an event stream shape with `event_id`, `kind`, `agent_id`, `job_id`, `state`, `ts_unix_micro`, and `payload` at `src/orchestrator/pubsub.rs:4` through `src/orchestrator/pubsub.rs:13`, so the new job transition log can either reuse the durable events table if it can represent all job updates cleanly, or add a focused `job_transitions` table. The important requirement is same-transaction persistence with the job row.

Concrete mutation coverage:

- Submit: emit creation `null -> QUEUED` for `insert_job_sync` at `src/db/jobs.rs:49` through `src/db/jobs.rs:69` and recovered insertion at `src/db/jobs.rs:72` through `src/db/jobs.rs:109`.
- Dispatch: emit `QUEUED -> DISPATCHED` from `claim_next_job_sync` at `src/db/jobs.rs:166` through `src/db/jobs.rs:214`.
- Complete: emit `DISPATCHED -> COMPLETED` from `mark_job_completed_conn_sync` at `src/db/jobs.rs:365` through `src/db/jobs.rs:383`.
- Cancel queued: emit `QUEUED -> CANCELLED` from `mark_queued_job_cancelled_conn_sync` at `src/db/jobs.rs:386` through `src/db/jobs.rs:399`.
- Cancel requested: emit a job update for `cancel_requested` from `request_dispatched_job_cancel_sync` at `src/db/jobs.rs:402` through `src/db/jobs.rs:411`.
- Cancel settled: emit `DISPATCHED -> CANCELLED` from `mark_dispatched_job_cancelled_if_agent_idle_sync` at `src/db/jobs.rs:431` through `src/db/jobs.rs:452` and `mark_job_cancelled_conn_sync` at `src/db/jobs.rs:455` through `src/db/jobs.rs:464`.
- Fail: emit `QUEUED|DISPATCHED -> FAILED` from `mark_job_failed_conn_sync` at `src/db/jobs.rs:467` through `src/db/jobs.rs:485`.
- Requeue: emit `DISPATCHED -> QUEUED` from `requeue_recovered_dispatch_io_failure_sync` at `src/db/jobs.rs:488` through `src/db/jobs.rs:547` and from `requeue_dispatched_job_before_send_sync` at `src/db/jobs.rs:549` through `src/db/jobs.rs:570`.
- Bulk fail: collect all affected job ids and emit one transition per job for `mark_dispatched_jobs_failed_for_agent_conn_sync` at `src/db/jobs.rs:627` through `src/db/jobs.rs:636`. The current wrapper only queries one affected dispatched job before the update at `src/db/jobs.rs:1150` through `src/db/jobs.rs:1168`, which is enough for today if an agent can only own one dispatched job, but a transition design should not encode that assumption in the log writer.
- State-machine completion: ensure the shared helper is used by the connection-level completion and cancellation calls reached from idle-marker, hook, and log paths at `src/db/state_machine.rs:720` through `src/db/state_machine.rs:737`, `src/db/state_machine.rs:931` through `src/db/state_machine.rs:945`, and `src/db/state_machine.rs:1094` through `src/db/state_machine.rs:1105`.
- STUCK: emit `DISPATCHED -> FAILED` from the STUCK path at `src/db/state_machine.rs:1439` through `src/db/state_machine.rs:1456`.
- Recovery: emit requeue or recovered-create events from `requeue_interrupted_job_from_captured_intent_sync` at `src/db/recovery.rs:335` through `src/db/recovery.rs:405`; because replacement and requeue are currently part of one transaction at `src/db/recovery.rs:414` through `src/db/recovery.rs:475`, the transition row should be committed in that transaction.

Miss analysis:

- Broadcast lost after commit: if the daemon crashes after a job row commits but before notify, a broadcast-only design misses the edge. The hybrid design recovers state through `jobs[]`, and a durable transition row recovers the exact edge.
- Broadcast before rollback: if an emission happens before transaction commit and the transaction rolls back, consumers can observe a false edge. Same-transaction durable rows plus post-commit notification avoid this.
- Recovery requeue during revive: if recovery updates or reinserts a job while the runtime subscriber is disconnected, the next initial snapshot includes the bounded job state, and durable transition rows can populate `job_events[]` for clients using a future `since_event_id`.
- STUCK and evidence-denial paths: a design that emits only from RPC job handlers misses these because they mutate jobs inside state-machine code. DB-boundary emission catches them.

Duplicate analysis:

- Retried CAS updates should not duplicate transitions because each helper emits only when an update affected rows.
- A subscriber reconnect can see the same terminal job in `jobs[]` again. Consumers must treat `jobs[]` as state, not as an edge log.
- A subscriber can see the same durable `job_events[]` item again after reconnect or backfill. Include a monotonic `event_id`; consumers should de-duplicate by `event_id` or by `(job_id, old_status, new_status, event_id)`.
- Snapshot `sequence` is stream-local today, because it starts at `1` when `runtime.subscribe` starts at `src/rpc/handlers/runtime.rs:25` through `src/rpc/handlers/runtime.rs:52`. Do not use `sequence` as a durable job-event id.

Runtime notification wiring:

- Add `RuntimeSnapshotReason::JobChanged`.
- Make job transition persistence trigger `notify_runtime_changed(RuntimeSnapshotReason::JobChanged)` after commit, in addition to the existing `JOB_UPDATES` notification path for `job.wait` compatibility.
- Extend `query_runtime_inventory` or a sibling query to load bounded `jobs[]` and recent transition rows. Keep the runtime fingerprint including `jobs[]` and `job_events[]` so job-only changes are not suppressed by the current fingerprint gate at `src/rpc/handlers/runtime.rs:54` through `src/rpc/handlers/runtime.rs:83`.

## 3. jq Consumer Contract

The contract for `ah events --format json` should stay line-delimited JSON. Every line is still a snapshot object. Consumers should treat `schema_version >= 2` as supporting `.jobs[]` and `.job_events[]`, both optional arrays.

Waiting for one job to reach a terminal status:

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

Reconstructing current non-terminal work after subscribing:

```bash
ah events --format json |
  jq -rc '
    select(.schema_version >= 2)
    | .jobs[]?
    | select(.status == "QUEUED" or .status == "DISPATCHED")
  '
```

Replacing pend-style polling for any terminal transition:

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
- Treat `reason` as advisory. Consumers should not depend on receiving only `reason == "job_changed"` because initial and recovery snapshots can also contain useful `jobs[]`.
- Treat unknown job statuses as non-terminal unless the consumer explicitly knows otherwise.

## 4. Schema Version and Compatibility Strategy

Bump `RuntimeSnapshot.schema_version` from `1` to `2` when adding job fields. Current active snapshots set `schema_version: 1` at `src/runtime_events.rs:239` through `src/runtime_events.rs:245`, and inactive snapshots also set `schema_version: 1` at `src/runtime_events.rs:121` through `src/runtime_events.rs:141`.

Keep the top-level `event: "snapshot"` value. A second top-level frame shape would be more disruptive than additive fields because `runtime.subscribe` currently serializes `RuntimeSnapshot` objects directly at `src/rpc/handlers/runtime.rs:87` through `src/rpc/handlers/runtime.rs:105`, and `ah events` streams those lines directly at `src/bin/ah.rs:1317` through `src/bin/ah.rs:1343`.

Additive fields:

- `jobs`: optional array, present in schema v2 snapshots. Empty when no jobs match the bounded inclusion rule.
- `job_events`: optional array, present in schema v2 snapshots. Empty except on job-change snapshots or backfill snapshots.
- `job_event_cursor`: optional highest durable job event id included in the frame.

Reason compatibility:

- Add `job_changed` to `RuntimeSnapshotReason`, whose current serde values are `initial`, `inventory_changed`, `tmux_changed`, `agent_changed`, `shutdown`, `daemon_absent`, and `daemon_lost` at `src/runtime_events.rs:9` through `src/runtime_events.rs:19`.
- Document `reason` as an extensible string. Strict external enum parsers should gate on `schema_version` and either tolerate unknown reasons or ignore frames they cannot classify.
- If preserving strict v1 enum consumers is more important than a precise reason value, emit job-driven snapshots with `reason: "inventory_changed"` during one transitional release and introduce `job_changed` only with a documented v2 cutover. The cleaner long-term API is `job_changed`.

Cursor compatibility:

- `sequence` remains per-subscription stream sequence and is not a durable resume token; it starts from `1` for a new runtime subscription at `src/rpc/handlers/runtime.rs:25` through `src/rpc/handlers/runtime.rs:52`.
- Use durable `job_events[].event_id` and `job_event_cursor` for replay or de-duplication.
- A future `runtime.subscribe` request may accept `since_job_event_id`; until then, reconnecting consumers should use `jobs[]` for convergence and de-duplicate terminal transition records if the daemon includes recent `job_events[]`.

Provider neutrality:

- The job schema is keyed by `agent_id` and request metadata at `src/db/schema.rs:155` through `src/db/schema.rs:170`; the runtime job carrier should not branch on Claude, Codex, Antigravity, or other provider-specific concepts.
- Provider-specific runtime behavior should remain behind the existing agent/session fields. Job events should describe persisted job lifecycle only.

## Recommendation Summary

Use the hybrid carrier: keep `runtime.subscribe` as snapshot-shaped JSON lines, bump `schema_version` to `2`, add bounded `jobs[]` for convergent state, add `job_events[]` for exact job transitions, and add `reason: "job_changed"` as the long-term reason value.

Implement emission at the transaction-aware DB mutation boundary with durable transition rows, then notify `RUNTIME_UPDATES` after commit. This catches normal dispatch/completion, cancel settlement, STUCK failure, recovery requeue, recovered insertion, and bulk failure paths without depending on orchestrator-specific control flow.

For consumers, `.job_events[]` replaces pend-style polling for terminal edges, while `.jobs[]` provides reconnect and crash-recovery convergence. The stream `sequence` remains a per-connection ordering aid, not a durable replay cursor.
