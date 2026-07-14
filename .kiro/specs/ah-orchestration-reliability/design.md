# ah Orchestration Reliability Design

Status: implementable design spec transcribed from the frozen research design.

## Design Thesis

`ahd` must become a declarative reconciler that continuously converges DB intent with host-observed reality. Hooks remain useful fast-path notifications, but orchestration correctness must not depend on hook delivery, pane text alone, or LLM obedience to an imperative completion protocol.

The core model is sensor fusion:

- T0: host process tree truth, via systemd scope/cgroup on Linux and process group on macOS/Unix fallback.
- T1: host-side filesystem and test evidence scanners.
- T2: durable stdout/log quietness and progress.
- T3: pane text hints for interactive prompts and UI-only fallback, inert unless corroborated.
- F5: telemetry gathered on every reconcile tick.

## Architecture

```text
ahd
  runtime DB
  continuous reconcile loop
    active-agent query
    sensor fusion snapshot
      T0 process tree probe
      T1 filesystem evidence scanner
      T2 stdout/log quietness probe
      T3 pane scanner hint
      F5 telemetry collector
    CAS state write
    D1 ownership gate
    D2 process-tree reaper
```

The reconcile loop complements, not replaces, existing startup reconcile in `src/db/system.rs:526-562` and async startup entry points in `src/db/system.rs:1203-1228`.

## D1 Ownership Gate Design

Hard prerequisite: PR4's explicit daemon marker provenance model is required before this spec can be implemented safely. Main's current startup reconcile entry point (`src/db/system.rs:1203-1233`) does not yet carry a provenance parameter. The implementation sequence must either merge PR4 first or introduce the same `DaemonMarkerProvenance { Explicit, Ambient }` plumbing as the first D1 change. Until provenance is present, stop-class reconcile/reap work must remain disabled.

Introduce a single ownership decision primitive:

```text
authorize_destructive_action(target, daemon_identity, db_agent, runtime_registry, process_birth)
    -> Authorized | Rejected(reason)
```

Inputs:

- `DaemonMarker { value, provenance }`.
- Agent DB row with `id`, `session_id`, `pid`, `spawned_at`, `state_version`.
- Runtime registry entry from `TMUX_PANE_MAP` where available.
- Linux scope description where available.
- Process birth/start metadata where available.

Decision order:

1. Provenance: reject when provenance is `Ambient`.
2. Identity: Linux exact `@{daemon_marker}` scope marker for scope stop; registry/session/pid/socket ownership for Linux non-scope cleanup and all macOS/Windows/Unix fallback cleanup.
3. Anti-recycling: process birth/start metadata must be compatible with DB `spawned_at`.

No caller may bypass this helper for stop-class operations. If any layer fails or cannot be proven, the destructive action must fail closed: do not reap, stop, kill, tear down tmux, or remove sandbox state. Emit one warn-level operator-visible log/event with the rejection reason. The rationale is safety-critical: an orphan is visible and recoverable; a wrongful kill is irreversible.

Implementation loci:

- PR4 provenance hooks must be threaded through `src/db/system.rs` reconcile APIs.
- Linux identity consumes scope descriptions currently parsed in `src/platform/linux/scope.rs`.
- Registry identity consumes `src/agent_io/registry.rs:111-125`.
- `spawned_at` is added in DB schema and written by spawn path in `src/rpc/handlers/agent.rs`.

## D2 Reaper Design

### Linux

Linux already wraps workers in `systemd-run --user --scope --collect` in `src/platform/linux/scope.rs:180-267`. Wire scope stopping into runtime cleanup:

```text
cleanup_agent_runtime_resources_with_policy
  lookup registry entry
  authorize_destructive_action
  if Linux scoped:
      stop_unit(scope_unit)
  else:
      fallback cleanup
```

Scope stop uses `SystemctlRunner::stop_unit` in `src/platform/linux/scope.rs:22-58`.

The crash path in `src/monitor/agent_watch.rs:68-105` must eventually flow through this cleanup. The current `cleanup` function only shuts down reader state and removes monitor registration; it is not a process-tree reaper.

### macOS/Unix Fallback

macOS currently has no scope implementation in `src/platform/macos/scope.rs:27-80`. Add a Unix process-group launcher:

- Spawn each agent in its own process group using `setpgid(0, 0)` through `pre_exec`, or `Command::process_group(0)` where available.
- Store pgid in runtime registry and any DB structure required by reconnect/reconcile.
- After D1 passes, cleanup kills `-pgid` with `SIGKILL`.

The existing `ScopeHandle.process_group_id` field in `src/platform/mod.rs:42` can represent this but is not currently populated by any spawn code.

## D3 Classified Completion Design

Add a job classifier:

```text
JobCompletionClass =
  EvidenceGated { physical: bool, test: bool }
  ArtifactLess
```

Evidence-gated jobs:

- Use T1 scanner and FS freshness rules.
- May auto-complete without hook delivery when evidence is fresh.
- Never complete from end-turn alone.

Artifact-less jobs:

- May complete from end-turn or explicit done.
- If quiet without declaration, transition to `STUCK` and alert.
- Never auto-fail solely due to quiet-without-declaration.

The existing `evidence_denial_for_job` in `src/db/state_machine.rs:1223-1245` is not enough because it checks DB evidence flags, not host filesystem facts.

## D4 T3 Corroboration Design

Route all pane-diff lifecycle hints through the sensor fusion engine before state writes.

Current direct write loci:

- `PROMPT_PENDING`: `src/db/state_machine.rs:286`.
- `STUCK` prompt-only UI recapture: `src/db/state_machine.rs:448`.

Required new flow:

```text
pane_diff_hint
  + T0 process tree status
  + T2 quiet ticks
  -> fused lifecycle decision
  -> CAS write
```

Uncorroborated T3 hints are recorded as telemetry only and cannot write lifecycle state.

## FS Scanner Freshness Design

Represent scanner evidence as:

```text
ScannedEvidence {
  kind,
  path_or_source,
  content_hash,
  host_mtime_us,
  scanner_observed_at_us,
  clock_domain,
}
```

Decision:

```text
fresh if authoritative_time_us > dispatched_at_us + epsilon_drift_us
```

Rules:

- Strict `>` only.
- UTC microsecond epoch only.
- Same-second precision is ambiguous unless a persisted DB evidence transaction timestamp (`inserted_at`) is strictly post-dispatch and a content-hash/baseline check independently proves the artifact changed after dispatch.
- Merely observing a stale pre-dispatch artifact after dispatch never makes it fresh.
- Remote/virtual clock domains use a default 1s drift window unless a provider proves shared host clock.

## `spawned_at` Migration Design

Add nullable `agents.spawned_at INTEGER`.

Migration:

- Additive migration in `src/db/mod.rs`.
- Fresh schema includes the column.
- Existing rows get `NULL`.

Write:

- Capture host `SystemTime` immediately after physical spawn succeeds and before the row becomes dispatchable.
- Store UTC microsecond epoch.

Read:

- D1 anti-recycling refuses every destructive cleanup action when `spawned_at` is null, missing, incompatible, or otherwise cannot prove process identity.
- There is no Linux exact-marker exception for null `spawned_at`. Exact marker identity is only one layer; provenance, identity, and anti-recycling must all pass before scope stop or any other destructive cleanup.

## Continuous Reconcile Core Design

Add:

```text
reconcile_active_agents_once(ctx, sensors, reaper, now) -> ReconcileReport
reconcile_active_agents_loop(ctx, interval=5s)
```

`reconcile_once` must be unit-testable with injected sensors and fake reapers. The daemon loop is integration-tested.

Tick steps:

1. Query non-terminal active/recoverable agents and their dispatched jobs.
2. Read `state_version`.
3. Collect T0/T1/T2/T3/F5.
4. Classify completion.
5. Apply D4 corroboration for T3 hints.
6. Apply D1 before any destructive cleanup.
7. Write state through CAS.
8. Emit telemetry.

## CAS Design

Preserve the pattern in `src/db/state_machine.rs:933`:

```sql
UPDATE agents
SET state = ?, state_version = state_version + 1, updated_at = unixepoch()
WHERE id = ? AND state_version = ?;
```

Every new reconciler state write must go through a CAS helper or an existing state-machine function that already uses CAS. A zero-row update is a conflict, not success.

## F5 Telemetry Design

Add reconcile telemetry to runtime events/snapshots:

- `reconcile_tick_id`
- `agent_id`
- `state_version_observed`
- T0 status: scope id, pgid, root pid, process tree status
- T1 status: evidence found, stale reason, freshness timestamps
- T2 status: last output timestamp, quiet ticks
- T3 status: hint kind, corroborated bool
- D1 outcome: authorized or rejection reason
- D2 outcome: scope stopped, pgid killed, fallback used
- CAS outcome: success/conflict
- resource metrics: token/context/quota if available from provider logs

This telemetry is machine-facing; external automation must consume runtime events/snapshot, not `ah ps`.

## Platform Notes

Linux:

- Primary whole-tree cleanup is systemd scope stop.
- Pidfd remains useful for root process death detection but is not the whole-tree reaper.

macOS:

- Scope stubs remain no-op until process-group fallback is implemented.
- Process-group cleanup is mandatory for parity with Linux.

Windows:

- The D1 registry identity rules apply.
- Whole-tree cleanup should eventually map to Job Objects. Until then, destructive cleanup must fail closed rather than pid-only kill descendants.

## Sandbox Home Note

Sandbox homes are persistent during agent lifetime because successful spawn releases the guard in `src/rpc/handlers/agent.rs:428`. Normal cleanup removes the hashed cache home via `src/db/system.rs:887-906`; recovery-eligible crash cleanup can preserve home via `remove_agent_sandbox_dir_preserving_home_sync` in `src/db/system.rs:908-923` and `crashed_cleanup_policy` in `src/db/agents_lifecycle.rs:369-375`.

The scanner must therefore treat sandbox-side journals as useful hints but not the only durable source of truth. Host-side scanner evidence remains authoritative for completion.
