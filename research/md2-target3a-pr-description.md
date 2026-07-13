# MD2 target3 PR-A: master_watch decision lift + named saga

## MD3 index statement

Read index entries: `monitor`, `master_revival`, `master_cutover`.

Changed index entries:
- `monitor`: updated responsibility to state that `master_watch` keeps master-revival saga ordering while delegating DB decision/fence helpers to `master_revival`; updated `master_watch.rs` line count to 5700.
- `master_revival`: updated responsibility/key symbols for moved DB decision/fence adapters; updated line count to 1236.
- `master_cutover`: read for boundary/in-flight semantics; no change.

## Scope

PR-A only:
- Moved DB decision/fence adapters into `master_revival` and replaced same-location `master_watch` expressions with named calls.
- Kept execution logic in `src/monitor/master_watch.rs`; no `master_reaper` module added.
- Refactored the two master-revival flows into ordered same-file helper calls so the saga order remains locally visible.
- Added session-scoped readiness-timeout test override and removed `AH_MASTER_REVIVE_READINESS_TIMEOUT_SECS` process-env mutation from readiness tests.

Known/out of scope, intentionally unchanged:
- Dead symbol `master_watch::monitor_key` remains at `src/monitor/master_watch.rs:496`.
- Layer inversion `monitor -> rpc::handlers::{RealignAgentParams, spawn_realign_agent}` remains at `src/monitor/master_watch.rs:28`.
- Per-revive `ClaudeGatewayService::new()` construction remains at `src/monitor/master_watch.rs:416` and `src/monitor/master_watch.rs:2004`.

## Regression map

### stale-inflight misclassification

Before: death classification stayed in `master_revival::classify_master_death`, with cutover in-flight and pid/generation stale checks at `src/master_revival.rs:69`. Runtime fences were inline in `master_watch`.

After: classification still lives at `src/master_revival.rs:69`. Runtime fences are now named `master_revival` helpers at `src/master_revival.rs:151` and `src/master_revival.rs:174`, called from readiness and failed-reap paths at `src/monitor/master_watch.rs:1729`, `src/monitor/master_watch.rs:1786`, and `src/monitor/master_watch.rs:1444`. Stale-inflight test was hardened to assert worker pid replacement rather than tmux pane-id monotonicity at `src/monitor/master_watch.rs:4293`.

Why unchanged: same SQL predicates and same call sites in the saga; only ownership/name changed.

### restart does not reinstall probes

Before: startup rearm and MASTER_VERIFYING resume stayed in `master_watch`.

After: startup rearm path remains in `master_watch`; verifying-window generation read is now `master_revival::master_recovery_verifying_window_expected_generation` at `src/master_revival.rs:223`. Readiness resume still runs through `resume_master_recovery_readiness` at `src/monitor/master_watch.rs:1164`.

Why unchanged: probe arming/resume chain was not moved out of `master_watch`.

### cascade beats revive

Before: non-terminal worker cleanup happened before revive; readiness/worker failures used fail-window -> cascade -> generation-fenced reap.

After: saga keeps cleanup first at `src/monitor/master_watch.rs:596`, marks workers reaped at `src/monitor/master_watch.rs:613`, and centralizes failure ordering in `fail_readiness_then_cascade_and_reap` at `src/monitor/master_watch.rs:1314`.

Why unchanged: the helper preserves the exact order: fail readiness, cascade, then best-effort fenced reap.

### failed-revive orphan shell

Before: finalize-stale killed orphan pane; failure paths called generation-fenced reap.

After: finalize-stale kill remains at `src/monitor/master_watch.rs:666`. Top-level catch still calls claimed-generation reap at `src/monitor/master_watch.rs:563`. Readiness failures call the shared fail/cascade/reap helper at `src/monitor/master_watch.rs:1192`, `src/monitor/master_watch.rs:1225`, and `src/monitor/master_watch.rs:1287`. Reap fence is now `master_revival::master_runtime_generation_matches`, called at `src/monitor/master_watch.rs:1444`.

Why unchanged: all failure exits still reach the same best-effort reap chain; only the fence predicate moved to `master_revival`.

## Verification

RED:
- `timeout 300 env CARGO_BUILD_JOBS=1 cargo test master_runtime_fences_are_generation_scoped -- --test-threads=1 --exact`
- Failed with missing `master_revival` functions: `master_runtime_matches`, `master_runtime_generation_matches`, `mark_session_closed_after_idle_master_death`.

GREEN:
- `timeout 300 env CARGO_BUILD_JOBS=1 cargo check`
- `timeout 300 env CARGO_BUILD_JOBS=1 cargo test --lib master_revival::tests::master_runtime_fences_are_generation_scoped -- --test-threads=1 --exact`
- `timeout 300 env CARGO_BUILD_JOBS=1 cargo test --lib master_revival::tests::idle_master_death_close_is_a_revival_decision_transition -- --test-threads=1 --exact`
- `timeout 300 env CARGO_BUILD_JOBS=1 cargo test --lib monitor::master_watch::tests`
- `timeout 300 env CARGO_BUILD_JOBS=1 cargo test --lib master_revival::tests`

Parallel proof:
- Default parallel loop: 20/20 passes for `monitor::master_watch::tests` plus `master_revival::tests`.
- High pressure loop: 20/20 passes for both filters with `--test-threads=16`.

Full closeout:
- `timeout 1800 env CCB_TEST_SKIP_REAL_PROVIDER=1 CARGO_BUILD_JOBS=1 cargo test --workspace -- --test-threads=1`
- Passed.
