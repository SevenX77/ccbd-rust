# ah Orchestration Reliability TDD Tasks

Each task is framed as RED -> GREEN. Do not implement broad refactors outside the named behavior.

## Phase 0: Test Harness and Injection Points

- [ ] Add fake sensor and fake reaper traits for `reconcile_active_agents_once`.
  - RED: a compile-failing test sketches `FakeSensors`, `FakeReaper`, and `reconcile_active_agents_once`.
  - GREEN: add minimal interfaces with no production behavior change.
  - Testability: `--lib`.

- [ ] Add fake process birth/start metadata provider.
  - RED: tests can inject matching, stale, missing, and recycled process metadata.
  - GREEN: add platform abstraction for process birth/start lookup.
  - Testability: `--lib`; real `/proc` is CI-integration.

## Phase 1: D1 Provenance and Identity Gate

- [ ] Implement provenance gate for stop-class operations.
  - RED: ambient marker plus exact matching Linux scope produces zero `stop_unit` calls.
  - RED: ambient marker plus exact-matching `TMUX_PANE_MAP` pid/socket/agent produces zero `kill(-pgid)` calls through a fake kill sink.
  - RED: ambient marker plus exact-matching Linux non-scope registry/session/pid/socket produces zero tmux teardown and zero sandbox teardown through fake cleanup sinks.
  - GREEN: thread `DaemonMarkerProvenance` through reconcile and cleanup authorization.
  - Safety-critical: yes.
  - Testability: `--lib`.

- [ ] Implement exact Linux scope marker identity.
  - RED: foreign marker with overlapping agent id is rejected.
  - RED: known agent id without `@{daemon_marker}` is rejected.
  - GREEN: authorize Linux only on exact scope description marker.
  - Safety-critical: yes.
  - Testability: `--lib`.

- [ ] Implement registry identity for macOS/Windows/Unix fallback.
  - RED: matching agent id but mismatched `expected_pid` rejects kill.
  - RED: matching agent id and pid but mismatched `socket_name` rejects kill.
  - GREEN: check `TMUX_PANE_MAP` entry before fallback reaping.
  - Safety-critical: yes.
  - Testability: `--lib`.

## Phase 2: `agents.spawned_at` Migration

- [ ] Add additive DB schema migration.
  - RED: old DB fixture without `spawned_at` fails assertion that init adds nullable column.
  - GREEN: add migration and fresh schema column.
  - Testability: `--lib`.

- [ ] Persist `spawned_at` on new agent spawn.
  - RED: spawn unit test expects non-null UTC microsecond `spawned_at`.
  - GREEN: capture host `SystemTime` immediately after physical spawn and persist before dispatchability.
  - Testability: `--lib` for DB action; CI-integration for full spawn.

## Phase 3: D1 Anti-Recycling Gate

- [ ] Implement anti-recycling helper with injected metadata.
  - RED: recycled pid metadata rejects cleanup.
  - RED: missing process birth/start metadata rejects cleanup.
  - RED: matching metadata allows cleanup only after provenance and identity pass.
  - GREEN: add process birth/start comparison helper against `agents.spawned_at`.
  - Safety-critical: yes.
  - Testability: `--lib`; real platform metadata in CI-integration.

- [ ] Integrate anti-recycling with DB `spawned_at`.
  - RED: migrated historical row with `spawned_at = NULL` rejects Linux scope stop, process-group kill, pid fallback, tmux teardown, and sandbox teardown.
  - RED: non-null incompatible `spawned_at` rejects every destructive path.
  - GREEN: require `agents.spawned_at` proof in the D1 authorization helper.
  - Safety-critical: yes.
  - Testability: `--lib`.

## Phase 4: D2 Dual-Platform Reaping

- [ ] Wire Linux scope stop into runtime cleanup.
  - RED: fake `SystemctlRunner` records `stop_unit` during `cleanup_agent_runtime_resources_with_policy` for an owned scoped agent.
  - RED: fake runner proves `stop_unit` is not called when D1 rejects.
  - GREEN: add cleanup context/runner injection and stop owned scope before pid fallback.
  - Testability: `--lib`; systemd behavior CI-integration.

- [ ] Route pidfd crash cleanup through whole-tree cleanup.
  - RED: pidfd-dead path calls cleanup that attempts owned scope stop.
  - GREEN: connect `agent_watch` crash path to D1/D2 cleanup rather than reader-only cleanup.
  - Testability: `--lib` with fake cleanup sink; CI-integration for live pidfd/scope.

- [ ] Add Unix process-group spawn support.
  - RED: launcher unit test proves macOS/Unix fallback command installs `setpgid(0,0)` or equivalent.
  - GREEN: add platform launcher wrapper and registry pgid storage.
  - Testability: `--lib`; CI-integration on Unix/macOS.

- [ ] Add Unix process-group kill.
  - RED: fake kill sink receives `SIGKILL` for `-pgid` after D1 passes.
  - RED: fake kill sink receives no call when D1 rejects.
  - GREEN: implement group reaper.
  - Testability: `--lib`; CI-integration for real process trees.

## Phase 5: Continuous Reconcile Core

- [ ] Implement unit-testable `reconcile_active_agents_once`.
  - RED: one fake tick with dropped hook and fresh evidence completes a job through CAS.
  - GREEN: implement query, sensor collection, decision, CAS write, and report.
  - Testability: `--lib`.

- [ ] Schedule daemon reconcile loop.
  - RED: integration harness shows loop invokes `reconcile_once` on interval.
  - GREEN: spawn loop from `ahd` with shutdown-safe task lifecycle.
  - Testability: CI-integration.

- [ ] Preserve startup reconcile behavior.
  - RED: existing startup reconcile tests still pass while continuous reconcile is disabled in unit harness.
  - GREEN: keep one-shot startup reconcile and add continuous loop separately.
  - Testability: `--lib`.

## Phase 6: D3 Classified Job Completion

- [ ] Add job completion classifier.
  - RED: evidence-required jobs classify as `EvidenceGated`; review/design/Q&A classify as `ArtifactLess`.
  - GREEN: implement classifier from job flags and request metadata.
  - Testability: `--lib`.

- [ ] Implement evidence-gated scanner completion.
  - RED: fresh scanner evidence completes a physical-evidence job without hook.
  - RED: stale scanner evidence does not complete the job.
  - GREEN: integrate T1 scanner with state-machine completion.
  - Testability: `--lib`; real git/test scans CI-integration.

- [ ] Implement artifact-less optimistic completion.
  - RED: artifact-less job with end-turn completes without physical evidence.
  - GREEN: allow end-turn/done fast path for artifact-less class only.
  - Testability: `--lib`.

- [ ] Implement artifact-less quiet watchdog.
  - RED: artifact-less quiet-without-declaration transitions to `STUCK`, emits alert, and does not auto-fail job.
  - GREEN: add watchdog decision and operator alert event.
  - Testability: `--lib`.

## Phase 7: D4 T3 Corroboration

- [ ] Make T3 pane hints inert without T0/T2 concurrence.
  - RED: pane prompt + T0 busy + T2 quiet does not write `PROMPT_PENDING`.
  - RED: pane prompt + T0 idle + T2 recent output does not write `PROMPT_PENDING`.
  - RED: pane prompt-only completion + T0 busy does not write `STUCK`.
  - RED: pane prompt-only completion + T0 idle + T2 recent output does not write `STUCK`.
  - GREEN: gate T3 lifecycle writes through sensor fusion.
  - Safety-critical: yes.
  - Testability: `--lib`.

- [ ] Allow T3 lifecycle write only when corroborated.
  - RED: pane prompt + T0 idle + T2 quiet writes `PROMPT_PENDING` through CAS.
  - RED: pane prompt-only + T0 idle + T2 quiet writes `STUCK` through CAS.
  - GREEN: implement corroborated T3 decisions.
  - Safety-critical: yes.
  - Testability: `--lib`.

## Phase 8: FS Scanner Freshness

- [ ] Implement strict timestamp evaluator.
  - RED: `mtime == dispatched_at` rejects evidence.
  - RED: `mtime < dispatched_at` rejects evidence.
  - RED: `mtime > dispatched_at` accepts evidence when no skew applies.
  - GREEN: implement `T_evidence > T_dispatch`.
  - Safety-critical: yes.
  - Testability: `--lib`.

- [ ] Handle same-second precision race.
  - RED: whole-second mtime equal to dispatch rejects without later persisted DB `inserted_at`.
  - RED: stale pre-dispatch artifact scanned after dispatch is not counted fresh.
  - RED: persisted DB `inserted_at` strictly after dispatch plus changed content hash accepts.
  - GREEN: incorporate persisted DB transaction timestamp and content baseline; do not use volatile scanner observation time as freshness authority.
  - Safety-critical: yes.
  - Testability: `--lib`.

- [ ] Handle clock-domain skew.
  - RED: remote/virtual evidence inside `epsilon_drift` rejects.
  - RED: remote/virtual evidence after `dispatch + epsilon_drift` accepts.
  - GREEN: add clock-domain metadata and default 1s drift window.
  - Safety-critical: yes.
  - Testability: `--lib`.

## Phase 9: CAS Concurrency Contract

- [ ] Add CAS conflict regression.
  - RED: hook completes a job after reconcile read but before reconcile write; reconcile affects zero rows and emits conflict telemetry.
  - GREEN: all reconciler writes use state-version CAS helpers.
  - Testability: `--lib`.

- [ ] Audit new writes.
  - RED: static/unit assertion or code review checklist catches non-CAS reconciler state updates.
  - GREEN: route writes through centralized helper.
  - Testability: review plus `--lib` helper tests.

## Phase 10: F5 Telemetry

- [ ] Emit reconcile decision telemetry.
  - RED: fake reconcile tick produces telemetry containing T0/T1/T2/T3, D1, D2, and CAS fields.
  - GREEN: add telemetry report structs and runtime event plumbing.
  - Testability: `--lib`.

- [ ] Expose telemetry via runtime machine surface.
  - RED: runtime snapshot/event stream includes reconcile telemetry; `ah ps` is not required.
  - GREEN: extend runtime events/snapshot schema.
  - Testability: `--lib`; CI-integration for stream.

## Phase 11: CI-Only Integration Gates

- [ ] Linux whole-tree reap integration.
  - RED: worker spawns a child that outlives root pid; root exits; reconcile stops scope and child is gone.
  - GREEN: scope stop wired and verified.
  - Testability: CI-integration only.

- [ ] macOS/Unix group reap integration.
  - RED: worker process group child survives root without implementation.
  - GREEN: process-group kill removes descendants after D1 passes.
  - Testability: CI-integration only.

- [ ] Dropped hook recovery integration.
  - RED: suppress `agent.notify`; fresh filesystem evidence still completes evidence-gated job.
  - GREEN: continuous reconciler scanner handles completion.
  - Testability: CI-integration.

- [ ] Ghost pane text integration.
  - RED: inject stale prompt/banner into pane while T0 busy or T2 active; lifecycle state remains unchanged.
  - GREEN: D4 corroboration enforced.
  - Testability: CI-integration.
