# Dispatch ACK Race Reliability Spec

## Scope

This spec covers the dispatch path that can create a queued job, transition an agent into active dispatch state, send the prompt into tmux, and wait for ACK/idle evidence. It is intentionally separate from PR #84 and does not change provider semantics.

## Evidence

- `src/orchestrator/mod.rs:101` only enumerates agents currently in `IDLE` for dispatch.
- `src/orchestrator/mod.rs:154` calls `dispatch_queued_job`, which uses `IDLE -> WAITING_FOR_ACK`.
- `src/orchestrator/mod.rs:160-199` disables idle scan, captures baseline, starts log monitoring, then sends text to the pane.
- `src/orchestrator/mod.rs:164-187` already has one pre-send readiness recheck that can requeue a `DISPATCHED` job before the prompt is sent.
- `src/orchestrator/mod.rs:201-225` compensates tmux send failure by marking the job `FAILED` and the agent `STUCK`.
- `src/rpc/handlers/ack.rs:61-73` attempts `WAITING_FOR_ACK -> BUSY` after the ACK stability window; on wrong state it only logs `failed to mark agent BUSY after ACK stability window`.
- `src/rpc/handlers/ack.rs:160-170` has the same one-shot transition attempt for ACK visual diff.
- `src/provider/health_check.rs:134-170` marks active agents `STUCK` after stale health, then emits a stuck event.
- `src/db/state_machine.rs:1357-1426` implements `mark_agent_stuck`; it changes only agent state and emits a state event.
- `src/db/jobs.rs:614-618` has a helper to fail current `DISPATCHED` jobs for an agent, but `mark_agent_stuck` and health escalation do not call it.

## Root Cause

Dispatch has two narrow race windows:

1. The scheduler only sees an agent if it is `IDLE` at the start of a tick. If a user/PM dispatches immediately after the previous task appears complete but before all ACK/marker/log tasks have converged, `dispatch_job_to_agent_sync` rejects with `AgentWrongState { current_state: BUSY }`. That failure is safe inside the DB transaction, but the caller has no bounded wait/retry policy for this transitional state.
2. After a job is marked `DISPATCHED`, ACK confirmation is best-effort. `spawn_new_capture_seed` sets `busy_marked = true` even when the state transition to `BUSY` fails. A failed ACK transition is logged and the task continues until timeout or stale health, but there is no durable "ACK failed, requeue/fail job" action. When health later marks `STUCK`, the current `DISPATCHED` job remains open, so completion waits forever.

The incident symptom "job is DISPATCHED, prompt never reached pane, old output in capture-pane, then STUCK" is therefore a liveness bug across dispatch DB state, pane send/ACK, and health escalation. `STUCK` is terminal for scheduling because `run_once` dispatches only `IDLE` agents and recovery only scans `CRASHED`.

## Best Fix

Implement a bounded dispatch readiness gate and make `STUCK` non-dead-end for jobs.

### A. Bounded IDLE Wait Before Dispatch

Add a helper near the orchestrator dispatch path:

```text
wait_for_dispatchable_idle(ctx, agent_id, max_wait=2s, poll=50ms) -> DispatchReadiness
```

Behavior:

- If state is `IDLE`, return `Ready`.
- If state is `BUSY` or `WAITING_FOR_ACK` and there is no `DISPATCHED` job, poll until `IDLE` or deadline. This covers lagging marker/ACK cleanup.
- If state is `BUSY` or `WAITING_FOR_ACK` with a `DISPATCHED` job, do not dispatch a new job.
- If state is `PROMPT_PENDING`, `UNKNOWN`, `STUCK`, `CRASHED`, `KILLED`, or missing, do not dispatch.
- On deadline, leave the queued job `QUEUED`, emit `dispatch_deferred` with `reason="target_not_idle"`, and wake the orchestrator with bounded backoff. Do not spin continuously.

This should be used before `dispatch_queued_job(ctx, &agent.id)` and again immediately before send, replacing the current one-off assumption that the agent is still usable after the first guard.

### B. Make ACK Transition Attempts State-Aware

Replace the one-shot `WAITING_FOR_ACK -> BUSY` calls in `spawn_new_capture_seed` with a helper:

```text
ack_mark_busy_or_resolve(db, agent_id, reason) -> AckBusyOutcome
```

Behavior:

- `WAITING_FOR_ACK -> BUSY` succeeds: return `MarkedBusy`.
- Current state is already `BUSY`: return `AlreadyBusy`; this is not an error.
- Current state is `IDLE`: return `AlreadyIdle`; cancel ACK seed and wake orchestrator because completion won the race.
- Current state is `PROMPT_PENDING`: return `PromptPending`; cancel ACK seed.
- Current state is `STUCK`, `CRASHED`, `KILLED`, or missing: return `Terminal`; cancel ACK seed.
- Any CAS conflict gets retried up to a small count, e.g. 5 polls at 50ms. After that, emit `ack_busy_deferred` and return without setting `busy_marked = true`.

Do not set `busy_marked = true` after a failed transition. That masks the failed ACK and allows the seed task to exit with no durable resolution.

### C. Resolve Jobs When STUCK Is Reached

Change `mark_agent_stuck` or the immediate callers to close or requeue the current job under the same DB decision:

- For `WAITING_FOR_ACK` stuck before confirmed send/ACK, prefer requeue if the prompt may not have reached the pane. Use a new helper similar to `requeue_dispatched_job_before_send_sync`, but with source `ack_stuck_before_busy`.
- For `BUSY` stuck after confirmed send, fail the current job with `error_reason="HEALTH_CHECK_STUCK"` or the explicit stuck reason. This prevents `job.wait` from hanging.
- Emit an event carrying `job_resolution: "REQUEUED" | "FAILED" | "NONE"`.

The safer first implementation is:

- `WAITING_FOR_ACK -> STUCK`: requeue the current `DISPATCHED` job and restore the agent to `IDLE` only if pane capture shows no meaningful diff from baseline or the ACK failure reason is pre-send infrastructure (`pane_unregistered_during_ack`, `reader_unregistered_during_ack`, `tmux_capture_failed_during_ack`). Otherwise mark job `FAILED`.
- `BUSY -> STUCK`: mark current `DISPATCHED` job `FAILED`.

This prevents phantom `DISPATCHED` jobs while avoiding duplicate prompt sends when the pane may already have accepted the prompt.

### D. Optional Later Hardening: STUCK Revive

Do not make STUCK recovery the primary fix for this incident. It is useful follow-up, but auto-reviving STUCK without resolving the in-flight job risks duplicate prompts. If implemented:

- Revive only `STUCK` agents that have no `DISPATCHED` job, or whose job was explicitly failed/requeued by the stuck transition.
- Reuse the CRASHED recovery snapshot path and existing backoff columns.
- Cap retries with the existing recovery backoff.

## Non-Goals

- No infinite retry loop.
- No new dispatch from `BUSY`.
- No blind re-send when pane evidence indicates the prompt might already have been submitted.
- No change to user-visible job ids unless the existing job is requeued.

## Task Breakdown

1. Add dispatch readiness helper.
   - Suggested test: `orchestrator_dispatch_waits_for_transient_busy_then_sends`.
   - Seed an agent that starts `BUSY` with no dispatched job, flips to `IDLE` within the bounded wait, and assert the queued job is dispatched once.
   - Suggested test: `orchestrator_dispatch_defers_busy_with_inflight_job`.
   - Seed `BUSY` plus a `DISPATCHED` job and a second queued job; assert the second job remains `QUEUED` and a `dispatch_deferred` event is emitted.

2. Fix ACK busy marking.
   - Suggested test: `ack_stability_treats_already_busy_as_success`.
   - Force the state to `BUSY` before the ACK stability transition and assert no warn/error path marks the ACK failed.
   - Suggested test: `ack_stability_does_not_set_busy_marked_after_wrong_state`.
   - Force a non-active state and assert the seed task exits without masking the transition failure.

3. Close or requeue jobs on STUCK.
   - Suggested test: `mark_agent_stuck_from_waiting_requeues_unacknowledged_job`.
   - Seed `WAITING_FOR_ACK` plus `DISPATCHED`; invoke the new pre-ACK stuck helper; assert job returns `QUEUED` and agent returns `IDLE` only for safe pre-send reasons.
   - Suggested test: `mark_agent_stuck_from_busy_fails_dispatched_job`.
   - Seed `BUSY` plus `DISPATCHED`; invoke health stuck; assert job becomes `FAILED` and `job.wait` receives a terminal response.

4. Add an integration regression for the incident.
   - Suggested test: `dispatch_ack_race_no_phantom_dispatched_job`.
   - Use the existing `BEFORE_DISPATCH_SEND_HOOK` test hook to perturb state between DB dispatch and send/ACK.
   - Acceptance: after the race, there is no `DISPATCHED` job whose prompt was never sent; the job is either `QUEUED` for retry or `FAILED` with a reason.

## Acceptance Gates

- `cargo test --test dispatch_atomicity`
- `cargo test --test ack_fallback_lifecycle`
- `cargo test --test ah_full_e2e_main -- --include-ignored --test-threads=1`
- New tests named above pass.
- Manual dogfood acceptance: dispatching to an agent immediately after prior completion may defer briefly, but must not leave a permanent `DISPATCHED` job with unchanged pane content.
