# ah Orchestration Reliability Requirements

Status: frozen implementation spec transcribed from `research/orchestration-reliability-design.md`.

This spec converts the converged orchestration-reliability design into acceptance criteria and TDD framing. It intentionally does not reopen design debate. It covers the reliability layer that turns `ahd` from a mostly reactive event handler into a reconciler that continuously compares desired DB state with host-observed process, I/O, UI, and filesystem truth.

## Scope

In scope:

- Destructive ownership gates for all reap, kill, and stop-class actions.
- Linux systemd scope cleanup in crash and runtime cleanup paths.
- macOS/Unix process-group spawning and cleanup fallback.
- Continuous active-agent reconcile loop.
- Classified job completion for evidence-gated and artifact-less jobs.
- T3 pane-diff corroboration before lifecycle writes.
- Filesystem evidence freshness gating.
- `agents.spawned_at` schema migration and anti-recycling checks.
- State-version CAS preservation for concurrent reconciler, hook, and pidfd writes.
- F5 telemetry emitted by the reconcile loop.

Out of scope:

- A hook-side durable outbox.
- Replacing the existing RPC hook fast path.
- Full implementation details for remote providers that cannot expose host-visible artifacts. Those must fail closed under the freshness rules below.

## Existing Grounding

- Current evidence denial checks only DB evidence flags in `src/db/state_machine.rs:1223-1245`.
- Current pidfd watcher observes only the root pidfd in `src/monitor/agent_watch.rs:33-105`.
- Current crash lifecycle entry points are `mark_agent_crashed_with_exit_sync` in `src/db/agents_lifecycle.rs:143-149` and async wrapper in `src/db/agents_lifecycle.rs:399-418`.
- Current runtime cleanup starts at `cleanup_agent_runtime_resources_with_policy` in `src/agent_io/registry.rs:106-185`.
- Linux worker commands are already launched through `systemd-run --user --scope --collect` with `--description=ccbd-agent-{agent_id}@{daemon_marker}` in `src/platform/linux/scope.rs:180-267`.
- Linux scope stopping exists via `SystemctlRunner::stop_unit` in `src/platform/linux/scope.rs:22-58`.
- macOS scope support is currently a stub: `is_own_ccbd_scope` returns false, `wrap_in_scope` returns the base command, and `detect_scope_policy` returns `ScopePolicy::None` in `src/platform/macos/scope.rs:27-80`.
- State-machine CAS exists in `src/db/state_machine.rs:933`, where the update increments `state_version` and filters by the prior version.
- `PROMPT_PENDING` and `STUCK` pane/UI paths currently write direct lifecycle states in `src/db/state_machine.rs:286` and `src/db/state_machine.rs:448`.
- Current startup reconcile is one-shot in `src/db/system.rs:526-562` and async entry points begin at `src/db/system.rs:1203-1228`.
- Sandbox state dirs and hashed cache homes are created via `src/sandbox/path.rs:7-20`, retained on successful spawn by `SandboxDirGuard::release` in `src/rpc/handlers/agent.rs:428`, and materialized under `.cache/ah/sandboxes` in `src/provider/home_layout.rs:1619-1633`.

## Requirement D1: Three-Layer Ownership Gate

Every destructive action against an agent runtime resource MUST pass all applicable ownership layers before it can reap, kill, stop, or remove process-tree resources.

Destructive actions include:

- `systemctl --user stop <scope>`.
- `kill(-pgid, SIGKILL)`.
- `kill(pid, SIGKILL)` when used as fallback after group/scope cleanup.
- Any cleanup path that tears down a tmux session/pane or agent sandbox because the agent is believed dead.

Fail-closed rule:

- If any D1 layer fails or cannot be proven, no destructive action is allowed.
- The implementation must leave the process/resource alone and emit exactly one warn-level, operator-visible log/event for that attempted destructive action.
- Rationale: orphans are visible and recoverable; a wrongful kill is irreversible.

### D1.1 Provenance Gate

The PR4 daemon-marker provenance model is a hard prerequisite. Main's current startup reconcile entry point (`src/db/system.rs:1203-1233`) does not yet carry provenance; implementation must merge PR4 first or introduce equivalent `DaemonMarkerProvenance { Explicit, Ambient }` plumbing before enabling this spec's stop-class logic. If daemon identity provenance is `Ambient`, all stop-class operations MUST be rejected before identity matching or kill execution.

Acceptance criteria:

- Given an ambient daemon marker and a scope whose description exactly matches a live marker, startup reconcile and continuous reconcile return read-only observations and perform zero stop/kill calls.
- Given an ambient marker and a `TMUX_PANE_MAP` entry matching the pid and socket, macOS/Unix cleanup still rejects process-group kill because provenance is not explicit.
- Given an ambient marker and exact-matching Linux non-scope registry/session/pid/socket ownership, tmux teardown and sandbox teardown still reject because provenance is not explicit.
- The rejection is observable through a structured event/log with reason `ownership_provenance_ambient`.

TDD RED -> GREEN:

- RED: add a unit test in `src/db/system.rs` using a fake `SystemctlRunner` where continuous reconcile observes an orphan scope with matching `@marker` but ambient provenance; assert no `stop_unit`.
- RED: add a unit test with ambient provenance and exact-matching `TMUX_PANE_MAP` pid/socket/agent; assert no `kill(-pgid)` through a fake kill sink.
- RED: add a unit test with ambient provenance and exact-matching Linux non-scope registry/session/pid/socket ownership; assert zero tmux teardown and zero sandbox teardown through fake cleanup sinks.
- GREEN: thread explicit `DaemonMarkerProvenance` through reconcile and cleanup APIs and short-circuit all stop-class actions when provenance is ambient.

Testability: `--lib` unit/mock.

### D1.2 Identity Match

After provenance passes, the target identity MUST match the daemon-owned runtime identity.

Linux:

- The target scope description MUST contain the exact `@{daemon_marker}` suffix.
- Known-agent-id fallback matching MUST NOT authorize scope stop.
- Linux non-scope destructive cleanup, including tmux pane/session teardown and sandbox teardown, MUST also prove registry/session/pid/socket ownership before acting.
- For Linux non-scope cleanup, exact scope marker alone is not enough because the target action is not scoped to a systemd unit.

macOS/Windows/Unix fallback:

- The target MUST have a `TMUX_PANE_MAP` entry in `src/agent_io/registry.rs`.
- The entry MUST match the requested `agent_id`.
- The entry MUST have `expected_pid == observed root pid`.
- The entry MUST have `socket_name == current daemon tmux socket_name`.

Acceptance criteria:

- Linux scope with `ccbd-agent-a1@foreign-marker` is never stopped by daemon marker `own-marker`, even if DB contains agent `a1`.
- Linux non-scope cleanup with matching marker but mismatched runtime registry entry does not kill tmux session/pane and does not remove sandbox state.
- macOS/Unix cleanup with matching `agent_id` but mismatched `expected_pid` rejects process-group kill.
- macOS/Unix cleanup with matching `agent_id` and pid but mismatched `socket_name` rejects process-group kill.

TDD RED -> GREEN:

- RED: add unit tests around a new ownership-check helper that feed matching and mismatching Linux scope descriptions and registry entries.
- RED: add a Linux non-scope cleanup test where marker matches but registry/session/pid/socket ownership is mismatched; assert no tmux teardown and no sandbox removal.
- GREEN: centralize ownership evaluation in a reusable helper consumed by reconcile and cleanup paths.

Testability: `--lib` unit/mock.

### D1.3 Anti-Recycling Check

After provenance and identity match pass, anti-recycling MUST prove that the OS process currently addressed by the pid is the same process that `ahd` spawned.

Linux:

- Read `/proc/<pid>` creation/start metadata using a platform helper.
- Compare it against the agent DB `spawned_at` timestamp captured at spawn time.
- If the process creation timestamp is earlier than, equal to an impossible timestamp, missing, or otherwise not provably the spawned process, abort the destructive action.

Non-Linux Unix:

- Use the best platform process birth/start time available.
- If process birth/start time is unavailable, process-group reaping MUST be disabled by default and emit `ownership_birthtime_unavailable`; it may not silently fall back to pid-only ownership.

Acceptance criteria:

- A recycled pid with a stale DB row is not killed even if `agent_id`, `expected_pid`, and `socket_name` match.
- A missing or unreadable birth/start time fails closed.
- A matching birth/start time allows cleanup only after D1.1 and D1.2 have already passed.
- A null or missing `agents.spawned_at` fails closed for every destructive action; there is no Linux exact-marker exception.

TDD RED -> GREEN:

- RED: add unit tests using injected process-birth metadata for equal, older, newer, missing, and matching cases.
- GREEN: add platform helpers and integrate them into the ownership gate.

Testability: `--lib` unit/mock; platform-specific real `/proc` behavior is CI-integration.

## Requirement D2: Dual-Platform Process-Tree Reaping

The system MUST reap the whole worker process tree, not just the root pid.

### D2.1 Linux Scope Stop in Crash/Cleanup Path

Linux worker commands already run inside systemd scopes assembled in `src/platform/linux/scope.rs:180-267`. Crash cleanup MUST wire `SystemctlRunner::stop_unit` into `cleanup_agent_runtime_resources_with_policy` and crash lifecycle paths before pid-only fallback.

Acceptance criteria:

- When a root pidfd death is confirmed in `src/monitor/agent_watch.rs:68-105`, cleanup attempts to stop the owned Linux scope if D1 passes.
- If scope stop succeeds, cleanup does not depend on root-pid `SIGKILL` to reap descendants.
- If scope stop fails, pidfd or tmux cleanup may run as fallback, but telemetry records `scope_stop_failed`.

TDD RED -> GREEN:

- RED: unit test `cleanup_agent_runtime_resources_with_policy_stops_owned_scope_before_pid_fallback` with a fake runner.
- GREEN: add runner injection or cleanup context that can call `stop_unit` after D1 passes.

Testability: `--lib` unit/mock for call ordering; CI-integration for actual systemd cgroup behavior.

### D2.2 macOS/Unix Process Group Reaping

macOS has no systemd scope in `src/platform/macos/scope.rs:27-80`. Unix fallback MUST create a dedicated process group for each agent and kill that group after D1 passes.

Spawn requirements:

- At agent spawn, call `setpgid(0, 0)` through `CommandExt::pre_exec` or a safe equivalent.
- On Rust versions/platforms that expose `Command::process_group(0)`, that API may be used instead.
- Persist the pgid in runtime registry and, where needed, DB state.

Cleanup requirements:

- Kill with negative process-group id: `kill(-pgid, SIGKILL)`.
- Never kill a group without D1 provenance, identity, and anti-recycling passing.
- Do not add Linux process-group kill as the primary path for scoped agents; Linux primary path is scope stop.

Acceptance criteria:

- A spawned macOS/Unix fallback worker records a non-null pgid different from unrelated agent pgids.
- Cleanup sends `SIGKILL` to `-pgid`, not just `pid`.
- A mismatched or recycled pid refuses group kill.

TDD RED -> GREEN:

- RED: unit test command construction/launcher abstraction proves a pgid setup hook is installed for non-Linux Unix fallback.
- RED: unit test fake `kill` sink receives `-pgid` only after D1 passes.
- GREEN: implement process-group launcher and cleanup reaper.

Testability: `--lib` unit/mock; CI-integration for real process groups on macOS/Unix.

## Requirement D3: Classified Job Completion

F3 job completion MUST be classified by job type. End-turn alone is not sufficient for evidence-gated jobs, and artifact-less jobs must not be auto-failed because no artifact exists.

### D3.1 Evidence-Gated Jobs

Jobs with `requires_physical_evidence` or `requires_test_evidence` MUST complete only when hypervisor-side scanners confirm fresh evidence.

Acceptance criteria:

- DB evidence flags alone are insufficient unless they pass the freshness gate in Requirement FS.
- Hypervisor-side scanner can synthesize local evidence from host-visible filesystem state and test artifacts.
- A dropped `agent.notify` hook does not prevent completion if fresh evidence is observed.
- Stale evidence never completes the current job.

TDD RED -> GREEN:

- RED: unit test where `requires_physical_evidence = true`, hook is absent, and scanner observes fresh diff/test metadata after dispatch; job completes.
- RED: unit test where scanner observes only stale pre-dispatch evidence; job remains `BUSY`.
- GREEN: implement scanner and completion integration.

Testability: `--lib` unit/mock for scanner inputs; CI-integration for real git/test artifact scans.

### D3.2 Artifact-Less Jobs

Artifact-less jobs include review, design, Q&A, and other tasks that intentionally produce no file or test artifact.

Acceptance criteria:

- End-turn or explicit done may optimistically complete artifact-less jobs.
- If output becomes quiet without declaration, the watchdog transitions the agent to `STUCK`, emits an operator alert, and leaves the job requiring human/operator resolution.
- The watchdog MUST NOT auto-fail artifact-less jobs solely because they are quiet.

TDD RED -> GREEN:

- RED: unit test an artifact-less job receives end-turn and no evidence requirement; it completes.
- RED: unit test an artifact-less job becomes quiet without end-turn/done; it transitions to `STUCK` with alert event and job is not auto-failed.
- GREEN: implement job classifier and watchdog behavior.

Testability: `--lib` unit/mock.

## Requirement D4: T3 Corroboration Before Lifecycle Writes

Pane-diff/UI text is a hint, not lifecycle authority. A T3 hint MUST be inert unless T0 and T2 concur.

T3 lifecycle writes include:

- `PROMPT_PENDING` transitions currently represented by `src/db/state_machine.rs:286`.
- `STUCK` transitions currently represented by pane/UI recapture paths such as `src/db/state_machine.rs:448`.

Required corroboration:

- T0 says the process tree is alive but idle/blocking, not actively consuming CPU or gone.
- T2 says stdout/FIFO/log output is quiet for the configured stable tick count.
- T3 identifies an interactive prompt or prompt-only stuck condition.

Acceptance criteria:

- T3 prompt text with active T0 process work does not write `PROMPT_PENDING`.
- T3 prompt text with recent T2 output does not write `PROMPT_PENDING`.
- T3 prompt-only reply with active T0 or recent T2 output does not write `STUCK`.
- Only when T0 idle and T2 quiet concur can T3 write state through CAS.

TDD RED -> GREEN:

- RED: unit test pane ghost text plus T0 busy plus T2 quiet does not transition.
- RED: unit test pane ghost text plus T0 idle plus T2 recent output does not transition.
- RED: unit test pane prompt-only STUCK hint plus T0 idle plus T2 recent output does not transition.
- RED: unit test pane prompt plus T0 idle plus T2 quiet transitions through CAS.
- GREEN: route T3 decisions through the sensor fusion engine and remove direct uncorroborated lifecycle writes.

Testability: `--lib` unit/mock.

## Requirement FS: Filesystem Scanner Freshness

Evidence counts toward completion only if its authoritative timestamp is strictly after the current job dispatch timestamp.

### FS.1 Clock Source

- `jobs.dispatched_at` is the authoritative dispatch timestamp for the current job.
- New `agents.spawned_at` uses host `SystemTime` captured immediately after successful spawn.
- Evidence timestamps must be normalized to UTC microsecond epoch.
- Local timezone timestamps are forbidden.

### FS.2 Authoritative Evidence Timestamp

For each scanner evidence item, compute:

- Host-observed file `mtime` for the artifact, normalized to UTC microseconds.
- Persisted DB evidence `inserted_at` transaction timestamp, normalized to UTC microseconds.
- Effective evidence time is authoritative only when backed by persisted DB transaction time (`inserted_at`) or by an unambiguous host file `mtime` strictly after dispatch. If there is any same-precision ambiguity, use the persisted DB `inserted_at`; a volatile scanner observation timestamp is not authoritative.
- A content-hash or baseline comparison is a separate proof of post-dispatch content change. It can support freshness only when paired with an authoritative timestamp; it cannot make a stale pre-dispatch artifact fresh merely because the scanner observed it later.

### FS.3 Strict Freshness Rule

The rule is strict greater-than:

```text
T_evidence > T_dispatch
```

If remote or virtualized clock domains are involved, apply a configured safety skew:

```text
T_evidence > T_dispatch + epsilon_drift
```

Default `epsilon_drift` is 1 second unless provider configuration proves shared host clock and subsecond precision.

### FS.4 Same-Second Race

If filesystem precision is whole-second or unknown and `mtime` lands in the same second as dispatch, the evidence is stale unless the persisted DB evidence `inserted_at` transaction timestamp is strictly later than dispatch plus any skew window and content/baseline comparison independently proves post-dispatch change.

Acceptance criteria:

- Evidence with `mtime == dispatched_at` is ignored.
- Evidence with `mtime < dispatched_at` is ignored.
- Evidence with `mtime > dispatched_at` and no skew conflict counts.
- Evidence with same-second `mtime` but persisted DB `inserted_at` strictly after dispatch counts only when the artifact content/hash also differs from the pre-dispatch baseline.
- An unchanged pre-dispatch artifact scanned after dispatch is not fresh, even if the scan itself happens later.
- Evidence from a remote clock-domain provider counts only after `epsilon_drift`.

TDD RED -> GREEN:

- RED: unit test exact equality rejects evidence.
- RED: unit test same-second mtime without later persisted DB `inserted_at` rejects evidence.
- RED: unit test unchanged pre-dispatch artifact scanned after dispatch is not counted fresh.
- RED: unit test persisted DB `inserted_at` strictly after dispatch plus changed content accepts evidence.
- RED: unit test remote skew requires `dispatch + epsilon_drift`.
- GREEN: implement freshness evaluator and integrate it into evidence-gated completion.

Testability: `--lib` unit/mock; CI-integration for filesystem precision behavior.

## Requirement DB: `agents.spawned_at` Migration

The agents table MUST gain an additive `spawned_at` column required by D1 anti-recycling.

Schema:

```sql
ALTER TABLE agents ADD COLUMN spawned_at INTEGER;
```

Semantics:

- UTC microsecond epoch.
- Captured using host `SystemTime` immediately after physical process spawn and before the agent is exposed as dispatchable.
- Existing rows migrate with `NULL`.
- Cleanup ownership for rows with `NULL spawned_at` fails closed for every destructive cleanup path, including Linux scope stop, process-group kill, pid fallback, tmux teardown, and sandbox teardown.

Acceptance criteria:

- Fresh DB has `agents.spawned_at`.
- Migrated DB has `agents.spawned_at` with `NULL` for historical rows.
- New agent spawn persists non-null `spawned_at`.
- Anti-recycling refuses all destructive cleanup for null `spawned_at`; no exact-marker or platform exception is allowed.

TDD RED -> GREEN:

- RED: migration test old DB lacks column, init adds nullable `spawned_at`.
- RED: spawn unit test asserts inserted agent row has non-null `spawned_at`.
- GREEN: implement migration and spawn write.

Testability: `--lib` unit/mock.

## Requirement R: Continuous Reconcile Core

`ahd` MUST run a continuous reconcile loop in addition to startup reconcile.

Behavior:

- Periodic interval defaults to 5 seconds.
- Targets agents in `SPAWNING`, `WAITING_FOR_ACK`, `BUSY`, `PROMPT_PENDING`, `STUCK`, and recoverable `CRASHED` states as appropriate.
- Gathers T0 process tree, T2 output quietness, T1 filesystem evidence, T3 pane hints, and F5 telemetry.
- Writes through state-machine CAS only.
- Runs stop-class cleanup only through D1 and D2.
- Treats `agent.notify` as a fast-path optimization; lost hooks are recovered by reconcile.

Acceptance criteria:

- Dropped hook plus fresh evidence completes evidence-gated job within one reconcile interval.
- Dropped hook plus artifact-less quiet output triggers `STUCK` alert but not auto-fail.
- Dead process tree is reconciled to `CRASHED` or cleanup according to ownership and recovery policy.
- Reconcile emits structured metrics/events for every state decision and every rejected destructive action.

TDD RED -> GREEN:

- RED: unit test one reconcile tick with fake sensors drives expected state transition through CAS.
- RED: unit test CAS conflict causes no stale overwrite and emits conflict telemetry.
- GREEN: implement `reconcile_active_agents_once` and schedule loop in `ahd`.

Testability: `--lib` unit/mock for `reconcile_once`; CI-integration for daemon loop.

## Requirement CAS: State-Version Concurrency Contract

Every state write made by the new reconciler MUST use existing state-version CAS style.

Acceptance criteria:

- Reconciler reads `state_version` before deciding.
- Reconciler writes with `WHERE id = ? AND state_version = ?`.
- If `changes == 0`, no event representing a successful transition is emitted.
- CAS conflict telemetry records `agent_id`, attempted transition, observed old version, and reason.

TDD RED -> GREEN:

- RED: unit test hook completes a job between reconcile read and write; reconcile write affects zero rows and does not overwrite hook result.
- GREEN: implement CAS-aware state write helpers or reuse existing helpers.

Testability: `--lib` unit/mock.

## Requirement F5: Telemetry

The reconcile loop MUST collect and publish F5 resource telemetry as first-class runtime data.

Minimum telemetry:

- Agent process-tree status: root pid, pgid/scope id, descendant count where available.
- Token/context indicators exposed by provider logs or transcripts.
- Output quiet ticks and last output timestamp.
- Evidence scanner status and freshness rejection reason.
- Ownership gate outcome for every attempted destructive action.
- Reconcile tick duration, CAS conflicts, and action counts.

Acceptance criteria:

- `runtime.snapshot` or runtime event stream exposes enough telemetry for operators to distinguish busy, quiet, prompt-pending, stuck, and ownership-rejected states.
- Telemetry is emitted for successful and rejected cleanup attempts.
- Telemetry does not require parsing `ah ps`.

TDD RED -> GREEN:

- RED: unit test `RuntimeSnapshot` or reconcile event includes F5 fields after a fake reconcile tick.
- GREEN: add telemetry structs and runtime event plumbing.

Testability: `--lib` unit/mock; CI-integration for end-to-end snapshot stream.

## Addenda Captured Post-Freeze (2026-07-12)

Requirement AGY below was captured after the original freeze, transcribed from operator field diagnosis on 2026-07-12. Unlike the requirements above (transcribed from a converged design), it is newly captured and its design has NOT yet converged. It is recorded here in `requirements.md` — not in a side-named document — so operator and master track it through `research/REQUIREMENT-LEDGER.md`. Design/TDD detail is filled as it converges.

AGY is a **reliability defect** (a completion-classification bug, sibling to Requirement D3), so it belongs in this spec. The dynamic-topology / single-agent-lifecycle-control requirement (R-DYN-1) that arrived in the same user message is a **new control capability, not a reliability defect** — it lives in its own spec `.kiro/specs/ah-agent-lifecycle-control/requirements.md`, not here.

## Requirement AGY: Completion-Detector Correctness for Artifact-Less Turns

Source: operator field verification 2026-07-12 (pane %6), overturning the earlier "o1 hung, must `ah up`" conclusion. This symptom has recurred many times across generations (user: "我发现了好多次").

The yield-and-wait completion detector (introduced by PR #122 to prevent false "yield-and-wait" completion) MUST NOT hallucinate a pending background command on a turn that has no actual background command, and MUST NOT indefinitely defer completion or auto-inject "wait for the background command" nudges on artifact-less (design/markdown/Q&A) turns.

Observed failure (verbatim field trace):

- agy process fully alive (answers "ping - are you idle?" -> "Yes"), had self-reported completion in the pane ("The design-divergence task is fully completed... This round is concluded"), sitting at an idle composer.
- ahd completion detector judged `Deferred` (refused completion) and auto-injected `> The job is still open. Wait for the background command to finish, then report the final test result. Do not stop at 'waiting for cargo test'.` into the pane.
- agy was pushed back into `Working...` with nothing to do (the task was pure-markdown design divergence — no cargo test, no background command) -> re-concludes -> re-nudged -> false-BUSY loop.
- Field-proven resolution: poke one real message so agy runs one turn + Esc -> agent state cleanly returns `IDLE/Matched` (confirmed by both `ah ps` and `ah status`). `ah up` is NOT required and is the wrong tool.

This is the root-cause layer of the false-BUSY / false-death symptom family; prior records captured only the symptom and the master-side `ah ask --wait` deadlock it triggers.

### AGY.1 No Hallucinated Pending on Artifact-Less Turns

The detector MUST distinguish "a real background command (e.g. a cargo test) is still running" from "an artifact-less turn has concluded with no background command". Only the former may defer completion.

Acceptance criteria:

- Given an artifact-less/design turn that ends with no spawned background process and no `requires_test_evidence`, the detector completes (or hands to the artifact-less watchdog per D3.2) and does NOT emit a "wait for background command" nudge.
- Given a turn that did spawn a still-running background command, deferral remains in effect until that command's completion is observed.
- The detector never injects a completion nudge into the pane based purely on the absence of a declared "done" when no background command exists.

### AGY.2 Bounded Recovery from Wrongly-Deferred Completion

The system MUST provide a bounded recovery from a wrongly-deferred completion state that does not require whole-session `ah up`.

Acceptance criteria:

- A single-turn poke or an equivalent lightweight nudge transitions a wrongly-`Deferred` artifact-less agent back to `IDLE`.
- Fixing AGY.1 at the source is preferred; `ah agent restart` (Requirement DT) is the heavier fallback, not the primary recovery.

TDD RED -> GREEN:

- RED: unit test — a concluded artifact-less turn with no background-command handle asserts the detector returns complete/needs-watchdog, NOT `Deferred`, and emits zero pane nudges.
- RED: unit test — a turn with a live background-command handle asserts `Deferred` persists until the handle completes.
- GREEN: thread an explicit "outstanding background command" signal into the yield-and-wait detector; gate deferral on it.

Testability: `--lib` unit/mock; CI-integration for real agy transcript replay.

<!-- Requirement DT (single-agent lifecycle control + dynamic topology, R-DYN-1) was moved OUT of this spec on 2026-07-12: it is a new control capability, not a reliability defect. See .kiro/specs/ah-agent-lifecycle-control/requirements.md. -->

