# Global State Deflake Inventory

Date: 2026-07-08
Branch: fix/deflake-completion-dispatch-tests

## Scope

Sweep command:

```sh
rg -n "(static|LazyLock|OnceLock|OnceCell|lazy_static!|thread_local!)" src tests --glob '*.rs'
```

All `--lib` unit tests run in one process. Any mutable process-global state keyed by short literals such as `a1`, `s1`, or `job_1` can cross-talk under `--test-threads > 1`.

## Inventory

| Item | Location | Mutable | Test-reachable | Key / shared surface | Collision mechanism | Risk | PR#84 verdict | Strategy |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| completion log monitor registry `LOG_MONITORS` | `src/completion/registry.rs:16` | Yes | Yes | `agent_id` | Parallel tests can register the same bare agent id; `contains/cancel/cursor_snapshot` then observes a sibling test. This caused `missing_or_unreadable_log_root_switches_to_ui_immediately` with `a1`. | Flaky | Pure test isolation issue. Product registry is intentionally global per daemon; unit tests shared one process. | Namespace test agent ids; add Drop cleanup guard for orchestrator tests that touch the registry. |
| agent IO registry `TMUX_PANE_MAP` | `src/agent_io/registry.rs:17` | Yes | Yes | `agent_id` | Parallel tests can replace/remove another test's runtime pane entry if they use the same agent id. | Flaky | Pure test isolation issue. Product daemon owns one live namespace; tests must not share ids. | Namespace direct `agent_io` test id; use orchestrator Drop cleanup guard for registered agents. |
| marker timer registry `MARKER_TIMER_REGISTRY` | `src/marker/registry.rs:8` | Yes | Yes | `agent_id` | Same-key timer registration cancels an older timer. | Flaky if bare ids are shared | Pure test isolation issue. | Existing direct registry test uses UUID; keep unique IDs. |
| parser registry `PARSER_REGISTRY` | `src/marker/parser_registry.rs:8` | Yes | Yes | `agent_id` | Parallel tests can overwrite/remove parser handles. | Flaky | Pure test isolation issue. | Orchestrator Drop cleanup guard removes parser entries; affected tests use unique agent IDs. |
| pidfd registry `PIDFD_REGISTRY` | `src/platform/linux/process.rs:12`, `src/platform/macos/process.rs:13`, `src/platform/windows/process.rs:35` | Yes | Yes | agent/master monitor key | Same key can replace a registered fd/handle. | Flaky if shared keys are used | Pure test isolation issue. | Existing agent-watch tests use UUID-style ids; master-watch keys are session/generation-specific. Keep namespaced keys. |
| macOS watch identities `WATCH_IDENTITIES` | `src/platform/macos/process.rs:16` | Yes | macOS tests | raw fd | Raw fd identity bookkeeping can cross-talk only if fd is reused before cleanup. | Low | Pure test/platform-test issue if seen. | Existing tests use owned fd lifetimes; no change. |
| job update broadcast `JOB_UPDATES` | `src/orchestrator/pubsub.rs:15` | Yes | Yes | broadcast stream | A subscriber that assumes "next message is mine" can receive another test's job id first. This caused the `job_hook_failed_pull_fallback` vs `job_log` flake. | Flaky | Pure test assertion issue. Product broadcast is intentionally global. | Filter for this test's job id with bounded timeout. |
| agent output broadcast `AGENT_OUTPUT` | `src/orchestrator/pubsub.rs:20` | Yes | Yes | broadcast stream | Same next-message cross-talk if tests consume without filtering. | Flaky if consumed naively | Pure test isolation issue. | Consumers should filter by their expected agent id. No current flake found in this sweep. |
| event frame broadcast `EVENT_FRAMES` | `src/orchestrator/pubsub.rs:25` | Yes | Yes | broadcast stream | Same next-message cross-talk if tests consume without filtering. | Flaky if consumed naively | Pure test isolation issue. | Consumers should filter by event fields. No current flake found in this sweep. |
| runtime update broadcast `RUNTIME_UPDATES` | `src/orchestrator/pubsub.rs:30` | Yes | Yes | broadcast stream | Same next-message cross-talk if tests consume without filtering. | Flaky if consumed naively | Pure test isolation issue. | Consumers should filter by reason/session where applicable. No current flake found in this sweep. |
| orchestrator notify `WAKER` | `src/orchestrator/mod.rs:30` | Yes | Yes | global `Notify` | Extra wakeups can make loops run sooner. They are level-less and not keyed. | Low | Pure test timing issue if a test assumes no external wake. Product tolerates extra wakeups. | Do not assert absence of wakes; no change. |
| dispatch test hook `BEFORE_DISPATCH_SEND_HOOK` | `src/orchestrator/mod.rs:41` | Yes | Test-only | singleton | Parallel tests can overwrite/clear the hook. This caused the `DISPATCHED` vs `QUEUED` flake. | Flaky | Pure test-injection issue; hook is `#[cfg(test)]`. | Serialize hook users with test-only async mutex. |
| dispatch hook test lock `BEFORE_DISPATCH_SEND_HOOK_TEST_LOCK` | `src/orchestrator/mod.rs:1447` | Yes | Test-only | singleton lock | Serialization aid only. | Low | Test-only mitigation. | Keep. |
| state-machine denial nudge list `TEST_DENIAL_NUDGES` | `src/db/state_machine.rs:1957` | Yes | Test-only | shared Vec | Tests clearing/reading the Vec can observe another test's nudges. | Flaky if used in parallel with overlapping assertions | Pure test-injection issue. | Existing tests clear/read around their assertions; if flake appears, serialize or filter by agent id. No current flake reproduced. |
| recovery replacement hook `REPLACE_KILLED_AGENT_AFTER_DELETE_HOOK` | `src/db/recovery.rs:15` | Yes | Test-only | singleton hook list via `OnceLock<Mutex<_>>` | Hook installed by one test could affect another if not removed. | Low/flaky if shared | Pure test-injection issue. | Existing helper uses scoped test hook; no change. |
| master failed-reap recorders | `src/monitor/master_watch.rs:1444` | Yes | Test-only | `session_id` | Same session id could mix recorder events. | Low | Pure test isolation issue. | Existing tests use specific session ids and guard removal; no change. |
| revive readiness probe overrides | `src/monitor/master_watch.rs:1644` | Yes | Test-only | `session_id` | Same session id could see sibling override. | Low | Pure test isolation issue. | Overrides are keyed by session id; no shared flake found. Keep namespaced session ids. |
| revive readiness ack overrides | `src/monitor/master_watch.rs:1649` | Yes | Test-only | `session_id` | Same session id could see sibling override. | Low | Pure test isolation issue. | Same as probe overrides. |
| transient unknown prompt map | `src/prompt_handler/integration.rs:23` | Yes | Yes | `agent_id` | Same agent id can accumulate prompt-stability observations across tests. | Flaky if bare ids overlap | Pure test isolation issue. Product map is per daemon namespace. | Existing prompt-handler tests mostly use isolated DB plus bare `a1`; no current flake reproduced. If touched, namespace agent ids or clear by agent. |
| master spawn locks | `src/master_revival.rs:12` | Yes | Yes | project/session-derived key | Lock map only serializes work; stale entries are lock objects, not behavioral state. | Low | Not a product defect. | No isolation needed beyond unique projects/sessions for tests. |
| session window locks | `src/rpc/handlers/sessions.rs:54` | Yes | Yes | `session_id` | Lock map serializes tmux window operations. Same id can serialize unrelated tests but should not corrupt state. | Low | Not a product defect. | No change. |
| tmux test servers in rpc handler unit tests | `src/rpc/handlers.rs:463`, `src/rpc/handlers.rs:521`, `src/rpc/handlers.rs:592`, `src/rpc/handlers.rs:679`, `src/rpc/handlers.rs:719`, `src/rpc/handlers.rs:936`, `src/rpc/handlers.rs:980`, `src/rpc/handlers.rs:1124`, `src/rpc/handlers.rs:1344` | Yes, external process state | Yes | tmux socket/server process | These tests spawn panes on test tmux servers and some call `kill-server`; under full parallel lib load, a sibling test can observe its test server disappearing during setup/cleanup. | Flaky | Pure test isolation issue. The product daemon does not run independent unit tests in one process while killing each other's test servers. | Added a test-only async mutex around tmux-spawning rpc handler tests. |
| provider manifests `MANIFESTS` | `src/provider/manifest.rs:335` | No effective mutation after init | Yes | immutable manifest map | Read-only after LazyLock init. | None | No defect. | No isolation needed. |
| regex/thread-local caches | `src/pane_diff/mod.rs:501`, `src/prompt_handler/matcher.rs:223`, `src/completion/parser.rs:55` | No behavioral mutation after init | Yes | compiled regex cache | Immutable compiled regex values. | None | No defect. | No isolation needed. |
| environment locks in integration test files | `tests/*` `ENV_LOCK`, `DEV_STATE_LOCK`, dogfood locks | Yes | Integration tests only | singleton mutex | Serialization helpers, not shared product state. | Low | Test-only serialization. | No change. |

## Strategy Adopted

| Risk family | Adopted change |
| --- | --- |
| Global registries keyed by `agent_id` | Renamed straggler bare IDs in orchestrator and agent_io tests to test-specific IDs. Added `AgentGlobalCleanup` Drop guard in orchestrator tests to cancel/remove completion, agent_io, and parser entries on both normal and panic paths. |
| Broadcast cross-talk | Kept the prior `JOB_UPDATES` fix and renamed the completion monitor test IDs: receive until the expected job id appears, with a timeout. |
| Singleton test hook | Kept the prior test-only async mutex around `BEFORE_DISPATCH_SEND_HOOK` users. |
| External tmux test server lifecycle | Added a test-only async mutex around rpc handler unit tests that spawn tmux-backed masters/agents or kill their test server. |
| Low-risk lock maps / immutable caches | Documented only; no code change. |

## Tests Re-namespaced

- `completion::monitor::tests::monitor_wakes_orchestrator_and_notifies_job_update_on_complete`
  - `a_log` -> `completion_monitor_job_update`
  - `job_log` -> `job_completion_monitor_job_update`
- `agent_io::tests::shutdown_reader_with_stale_session_does_not_cleanup_recycled_live_entry`
  - `a1` -> `agent_io_stale_shutdown_live`
  - session/fifo names made test-specific
- `orchestrator::tests::dispatch_guard_handled_or_error_refuses_before_job_claim`
  - `a1`/`job_1` -> `orchestrator_guard_refuse`/`job_orchestrator_guard_refuse`
- `orchestrator::tests::monitor_registers_baseline_before_send`
  - `a1` -> `orchestrator_monitor_baseline`
- `orchestrator::tests::dispatch_guard_capture_error_keeps_job_queued_before_log_monitor`
  - `job_1` -> `job_orchestrator_guard_capture_err`
- `orchestrator::tests::missing_or_unreadable_log_root_switches_to_ui_immediately`
  - `a1` -> `orchestrator_missing_log_root`
- Other orchestrator tests that already had unique IDs now use `AgentGlobalCleanup` for panic-path cleanup.

## PR#84 Gate

All reproduced failures in this sweep are pure test isolation defects:

- Product registries/broadcasts are intentionally process-global within one daemon.
- The observed flakes arise because unrelated `--lib` tests share a single test process while using bare test IDs or singleton test hooks.
- No product concurrency defect was identified. No product logic was changed.
