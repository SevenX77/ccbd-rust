# STUCK False-Positive: 300s log-monitor handoff + health_check `.or()` staleness shadow

Status: root-cause confirmed in main (code verified 2026-07-02). Design input for the orchestration-reliability hardening round. Distinct from the two already-fixed modes.

## Incident

a1's PR #84 re-review job ran **10m47s** of genuine work. ah left the job stuck `DISPATCHED` and `health_check` judged a1 `STUCK` (dead-end terminal, no self-heal) — PM-proxy had to read the verdict from the tmux pane and recover via cancel→kill→up. journalctl timestamps line up exactly (dispatch 22:36:13 → log-monitor gave up 22:41:13, i.e. exactly +300s; the remaining ~6 min of real work ran under the fallback layer).

The concurrent dispatch-ACK race also fired here but was NOT the cause of the freeze — it is a separate known issue (see `dispatch-ack-race.md`). The STUCK verdict came from the two bugs below.

## Root cause 1 — flat 300s log-monitor wait cap (`src/completion/monitor.rs:10`)

```rust
pub const MAX_LOG_MONITOR_WAIT: Duration = crate::pane_diff::DEFAULT_STUCK_THRESHOLD; // = 300s
```

The log-completion-signal monitor's maximum wait is hardwired to the stuck threshold. At 300s it unconditionally gives up and hands completion detection to the UI/pane fallback — regardless of whether the worker is still actively producing output. Any legitimately long task (code review, big test run) blows past 300s and loses its authoritative log-completion path.

**Fix direction:** the log-monitor wait must not be a flat 300s hard cap.
- Renew-on-progress: reset/extend the wait whenever new real output arrives (a busy worker should never time the monitor out), OR
- widen by task class, OR
- decouple `MAX_LOG_MONITOR_WAIT` from `DEFAULT_STUCK_THRESHOLD` so the two concerns tune independently.

## Root cause 2 — `.or()` lets a stale marker shadow live output (`src/provider/health_check.rs`)

```rust
let last_progress_ts = observation
    .last_marker_ts
    .or(observation.last_output_ts)      // BUG: Option::or — if last_marker_ts is Some(old), last_output_ts is ignored
    .unwrap_or(0)
    .max(observation.dispatched_at.unwrap_or(0));
```

`Option::or` returns `last_marker_ts` whenever it is `Some`, **never consulting `last_output_ts`** — so a single old marker timestamp permanently shadows the live pane-output clock. A worker actively generating output cannot refresh its own staleness window.

Worked example (wall-clock now = T0+330s, threshold 300s):
- `last_marker_ts = T0+30s` (stale early marker), `last_output_ts = T0+325s` (live output 5s ago).
- `.or()` → progress_ts = T0+30 → staleness = 300s ≥ threshold → **false STUCK**.
- `.max()` of both → progress_ts = T0+325 → staleness = 5s → healthy (correct).

**Fix direction:** take the newer of the two, not the first-present:
```rust
let last_progress_ts = observation.last_marker_ts.unwrap_or(0)
    .max(observation.last_output_ts.unwrap_or(0))
    .max(observation.dispatched_at.unwrap_or(0));
```
Keep the existing `.max(dispatched_at)` floor — that is the *separate* G0 false-STUCK fix (see code comment) for freshly re-dispatched idle agents; it must stay. Only the `.or()` → `.max()` change is new.

**Acceptance:** a worker producing pane output within the threshold window must never be judged STUCK, even if a stale marker timestamp predates the window. Add a regression test with (old marker_ts, fresh output_ts, now within threshold) → healthy; and the G0 case (both predate a fresh dispatch) still floors to dispatched_at → healthy.

## Distinctness from already-fixed modes

- **Antigravity premature-completion** (fixed, PR #84): turn-idle / deferred-background-work mistaken for done. Still holds, no regression.
- **G0 dispatch-time idle false-STUCK** (fixed earlier, the `.max(dispatched_at)` floor): freshly re-dispatched idle agent judged dead instantly. Still holds.
- **THIS (new, third mode):** long-running-but-alive worker judged STUCK because (1) the log monitor abandons at a flat 300s and (2) the fallback's `.or()` can't see live output. Neither prior fix covers it.

## Fold into the hardening round

Group with: `dispatch-ack-race.md`, `realign-atomicity.md`, and the `TEST_DENIAL_NUDGES` global-test-spy parallel-race debt. Common theme: STUCK must not be a silent dead-end — completion capture and staleness accounting must reflect real worker activity, and STUCK should trigger self-heal rather than require manual pane-read + cancel→kill→up.
