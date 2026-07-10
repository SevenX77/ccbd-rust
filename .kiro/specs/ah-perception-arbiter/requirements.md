# ah Perception Arbiter — Requirements

Status: converged after a3 adversarial review (2026-07-10) — see `research/a3-adversarial-review-of-c-d-specs-2026-07-10.md` for the review and this file/design.md's inline "Correction"/"Pinned"/"Resolved" notes for what changed in response (Q1 internal db/ boundary, Q2 hook-timeout/late-completion reconciliation, Q3 host-side-only cgroup PID placement, Q4 attribution-key pinning). **Not yet cleared for implementation** — awaiting operator/user sign-off per the review-and-freeze process; tasks.md is outline-only, no scheduling, gated on module A/B closeout + 换血#2. gap-patched 2026-07-10 (operator acceptance round) — see Existing Grounding / Scope below for the added dispatch-readiness pane-diff cross-reference.

Source material (do not re-litigate; cite instead of re-deriving):
- `research/perception-final-convergence-2026-07-09.md` — deep-research-validated design (107-agent adversarial verification, 23/25 claims confirmed). This is the authority for *what's industry-standard vs. custom*.
- `research/perception-divergence-a3-round2-2026-07-10.md` — a3's independent (blind) mechanism proposals for the four must-answer questions. Concrete mechanisms below are adapted from this report; a3 gets citation credit, not veto — master owns the final call and is accountable for it.
- `research/architecture-assessment-converged-2026-07-09.md` §一.3/11/12 — the structural finding this module fixes: 6 completion inferers with no arbitration, CAS-first-come-first-served as the only coordination, STUCK accept-gate asymmetry.

## Scope

In scope:
- A single-write arbitration layer for `agents.state`: monitors become read-only signal producers, one consumer owns all state mutation.
- `perception_events` table as the sole communication channel between signal producers and the arbiter.
- Tri-state signal model (True/False/**Unknown**) with per-signal-class Unknown budgets.
- `Stalled`-as-explicit-signal semantics (replacing today's timeout-inferred STUCK).
- Epoch/generation versioning so stale signals can't override fresh ones.
- Hook-report attribution mechanism that survives the reporting process dying before delivery.
- Sibling-cgroup PoC for child-process liveness (small, isolated spike — not the full arbiter).

Out of scope (explicitly deferred, do not fold in):
- Job state machine (module D, separate spec `ah-control-plane-refactor`).
- Pane-text as a completion signal of any kind (already killed — P0-1, PR #127). This module must not resurrect it in any form, including "corroboration."
- Voting/weighted-consensus across signal sources — refuted by research (final-convergence §1.2), not on the table.
- Active PTY probing (`echo $?`, ANSI DSR `\x1b[6n`) — refuted (final-convergence §1.4/2.4), violates the "never send keys to a busy agent" operating rule.
- macOS/Windows cgroup-equivalent liveness — out of scope for this spec; PGID-tracking fallback is noted as the known degraded mode, not designed here.
- **Dispatch-readiness pane-diff recheck (daemon send-gate) — added 2026-07-10, gap-patch.** The daemon's pre-send readiness recheck (pane-diff gate that refuses to inject a new prompt if the pane's content changed since the last snapshot) is pane-content-based inference of the same family this module kills, but it lives in the dispatch path, not in `agents.state` completion writes — see Existing Grounding below for the incident that surfaced it. It is explicitly not scheduled as a phase of this spec; flagged here only so the second surviving pane-diff site is not lost once this module's "no pane-content inference" precedent is established.

## Existing Grounding

- `mark_agent_stuck_outcome_sync` (`src/db/state_machine.rs`, ~1430-1560) unconditionally writes `reason = "PANE_DIFF_STUCK"` regardless of caller. Two production callers: `src/marker/timer.rs:113` (3h BUSY-marker-staleness fallback) and `src/provider/health_check.rs:133` (`escalate_health_stuck`, when `dead_layers` contains `"completion"`).
- `escalate_health_stuck` (`src/provider/health_check.rs:78-190`) separately writes its own `state_change` event with `reason: "HEALTH_CHECK_STUCK"` (line ~164) *after* calling `mark_agent_stuck`. **This produces two contradictory audit records for one transition** — the job's `error_reason` column says `PANE_DIFF_STUCK`, the accompanying event says `HEALTH_CHECK_STUCK`. This is the most likely root cause of the live incident where a3's stuck job was tagged `PANE_DIFF_STUCK` when the actual detector was a health-check completion-staleness judgment (operator-witnessed, 2026-07-10 night). Requirement C6 below fixes this directly and should be treated as the sharpest acceptance signal for this whole module.
- `health_check_observe` (`src/provider/health_check.rs:37-75`) already computes `dead_layers: Vec<String>` across three layers (`tmux`, `predicate`, `completion`) — this is a proto-arbiter and the closest existing analog to "signal fusion," but it fuses into a single boolean-ish decision inline rather than emitting per-layer tri-state events to a real arbiter.
- `CompletionSignalKind` enum was collapsed to a single `LogOnly` variant in P0-1 (`rework(p01): delete UiOnly CompletionSignalKind variant`, 36ab84e) — every provider is now log/hook-only for completion; pane is alert-only. This module's tri-state signal classes should map onto what P0-1 already established, not reopen it.
- No `perception_events` table exists yet; `events` table exists generically (used for `state_change`, `job_transition`, alerts) — C2 below must decide whether `perception_events` is a new table or a typed view/convention over the existing `events` table with a `layer` column. Default to the latter unless a concrete reason forces a new table (see design.md).
- **Second surviving pane-diff inference site (added 2026-07-10, gap-patch, incident-confirmed).** The daemon's dispatch-readiness recheck — a pre-send gate that diffs the target pane's current content against its last-known snapshot and refuses to dispatch if `changed=1` — is a second, independent pane-content-inference mechanism outside this module's completion-write scope. Incident, 2026-07-10 night (operator-witnessed): a claude gatekeeper agent had leftover CLI suggestion-chip text sitting in its input line; the readiness recheck saw this as `changed=1` against its snapshot and permanently refused to dispatch, silently re-queuing an audit job for 5+ minutes with zero daemon-side log signal explaining the refusal (workaround at the time: `/clear` the agent's pane to reset the diff baseline, then redispatch). This is the second pane-diff path surviving #126/P0-1's completion-signal cleanup (PR #127) — same anti-pattern (deciding behavior from raw pane text), different subsystem (daemon dispatch gate, not `agents.state` completion inference). Per the blanket "pane-lifecycle-inference must die" decision this module embodies, it is a deletion/replacement target too — see the Out of scope bullet below for why it is not scheduled as a phase of *this* spec, and `ah-control-plane-refactor/requirements.md`'s D1 gap-patch notes for the sibling observability/cancel-driver gaps the same incident surfaced.

## Requirement C1: Single-Write Arbitration Entry Point

All signal producers (health check, marker timer, hook handler, log monitor) MUST stop calling any function that mutates `agents.state` directly. They MUST instead emit a structured perception event; exactly one function (`transit_agent_perception_state` or equivalent name, TBD in design) may write `agents.state`.

Acceptance criteria:
- Given any of the four current callers of `mark_agent_stuck`, `mark_agent_idle_*`, or equivalent, after this change none of them execute an `UPDATE agents SET state = ...` in their own call frame — they call an event-emission function instead.
- Given a perception event is inserted, only the arbiter's consumption loop is observed calling the state-write function (verified via an instrumented fake in tests, not via source grep alone — grep is necessary but not sufficient, per a3's finding that visibility/type gating alone doesn't stop a determined bypass).
- A CI-checkable static rule (grep-based at minimum, AST-based if a Semgrep/clippy-lint budget is approved separately) flags any new `UPDATE agents SET state` outside the gate module. This is advisory/blocking-in-CI, not a compile-time guarantee — do not oversell it as such in the implementation.

TDD RED -> GREEN: RED = a test asserting a known-current direct-write caller (e.g. `mark_agent_stuck` from `timer.rs`) no longer exists / no longer writes state directly; GREEN = caller emits an event instead and arbiter consumption produces the same eventual state.

Testability: `--lib`.

## Requirement C2: `perception_events` Channel

Signal producers emit typed events carrying: `agent_id`, `layer` (`os` | `log` | `hook`), `verdict` (`True` | `False` | `Unknown`), `epoch` (see C5), `observed_at`, `detail` (free-form JSON). The arbiter is the only consumer that acts on them to write `agents.state`; all other consumers (audit UI, dogfood ledger tooling) are read-only.

Acceptance criteria:
- Every current direct-state-mutation call site converts to inserting exactly one `perception_events` row before this requirement is considered met for that call site (there are at minimum: `marker/timer.rs:113`, `provider/health_check.rs:133`, and the log-monitor/hook-event paths in `db/state_machine.rs`).
- Events are append-only; no `UPDATE`/`DELETE` on `perception_events` from producer code (reap/retention jobs are exempt and out of scope for producer-code review).
- Design.md must state explicitly whether this is a new table or a typed convention over `events` — do not leave this open past design.md.

Testability: `--lib`.

## Requirement C3: Tri-State + Unknown-as-Explicit-Signal

Every perception layer's verdict is one of `True` / `False` / `Unknown` — never silently omitted, never collapsed to `False` by absence. First reconcile after a layer starts observing an agent MUST write an explicit `Unknown` if it cannot yet judge, not skip emission.

Acceptance criteria:
- A layer that has never observed enough signal to judge (e.g. hook layer before first hook fires) emits an `Unknown` event, not zero events.
- Downstream arbiter logic never treats "no event received this tick" as equivalent to `False` — it treats an *absent expected event past budget* (see C4) as the trigger for an authoritative downgrade, not as an implicit verdict.

Testability: `--lib`.

## Requirement C4: Per-Signal-Class Unknown Budgets

Each layer's transition from `Unknown` to an authoritative verdict on absence is time-bounded, and the bound differs by layer because the physical semantics differ (per `perception-final-convergence-2026-07-09.md` §1.4, watchdog-absence-is-authoritative-failure precedent).

Design must pin exact budget values for at minimum three layers — **OS/process liveness**, **log/pane-adjacent quietness** (must reconcile with the existing `MAX_LOG_MONITOR_WAIT` = 900s constant, `src/completion/monitor.rs:10` — do not introduce a second, conflicting timeout for the same physical signal without an explicit migration note), and **hook delivery** (must reconcile with the attribution mechanism in C6 — the hook budget and the attribution-race window are the same physical timing problem viewed from two sides, design.md must show they're consistent, not two independently-guessed numbers).

Acceptance criteria:
- Each budget has a written rationale tied to the signal's physical delivery characteristics (not a round number picked for aesthetics).
- Exceeding budget produces an explicit authoritative verdict (not a silent state change) — the resulting `agents.state` transition must be traceable to a specific perception event with `verdict != Unknown`.
- The log-layer budget is either the existing 900s constant (reused, not reinvented) or design.md carries an explicit justification for diverging from it.

Testability: `--lib` for budget-expiry logic; real-clock integration is CI-only.

## Requirement C5: Epoch/Generation Versioning

Every state judgment carries the generation/epoch it was computed against (analogous to Kubernetes `observedGeneration`). A perception event computed against a stale generation must not be allowed to overwrite a judgment made against a newer one.

Acceptance criteria:
- Given two perception events for the same agent with epochs N and N+1 arriving out of order (N+1 processed first), processing the stale N event afterward does not regress `agents.state` or overwrite the N+1-derived state.
- CAS on `state_version` (already exists, `src/db/state_machine.rs:933`) is the natural implementation vehicle — design.md must state whether epoch is a new column or reuses `state_version` directly, and why.

Testability: `--lib`.

## Requirement C6: `Stalled` as an Explicit, Correctly-Attributed Signal

Replace the `STUCK` state's current form (a terminal-feeling state reached only by timeout inference, with a hardcoded and frequently-wrong `reason` string) with an explicit `Stalled` verdict that is (a) set by name from the layer that actually detected it, not defaulted by a shared function, and (b) not a dead end — must support the accept-gate's late-completion recovery path that already exists (`late_health_completion_stuck_allows_terminal`, `state_machine.rs` ~1176-1207).

Acceptance criteria:
- The `reason` string persisted on a job/agent stall is always accurate to the layer that triggered it (no more `PANE_DIFF_STUCK` written for a health-check-detected stall, or vice versa). This directly fixes the dual-contradictory-event bug described in Existing Grounding above.
- `timer.rs`'s 3h BUSY-marker-staleness fallback and `health_check.rs`'s completion-layer-staleness path are **not required to collapse into one code path** — design.md must decide whether they remain distinct triggers with distinct, accurate reasons, or whether one is subsumed by the arbiter's `log`-layer budget (C4) and should be deleted outright. Do not preserve both paths unexamined just because both currently exist.
- Liveness-genuine stalls (OS/process layer dead, not just completion-layer stale) still produce a real `Stalled` state transition — this module reduces false stalls and mislabeled reasons, it does not eliminate the ability to detect real ones.

Testability: `--lib`, plus a regression test asserting genuine liveness-dead scenarios still transition to Stalled (do not let this requirement silently become "never mark stuck again").

## Requirement C7: Hook-Report Attribution Survives Sender Death

A hook report (`ah agent notify`) must be attributable to the correct job/agent/epoch even if the reporting process is killed (by teardown or otherwise) immediately after emitting it, before any socket round-trip completes.

Acceptance criteria:
- A hook payload is durably recorded (file, or equivalent durability) before the reporting process's own exit can race the daemon's read of it.
- The daemon's consumption of a hook report does not rely on "currently active job for this agent" as its sole attribution key — it must carry an explicit, tamper-resistant job/epoch identifier set at dispatch time, so a report arriving after a fast redispatch cannot be misattributed to the new job.
- **Pinned 2026-07-10 after a3 adversarial review** (`research/a3-adversarial-review-of-c-d-specs-2026-07-10.md` §二.Q4/§三.2) found the first draft's "reuse an existing dispatch identifier, implementer to confirm which" was not actually verified to have a fit-for-purpose candidate, and left the wire-format keying unpinned across two independently-developed sides (sandbox-side hook CLI, host-side consumer) — a real integration-collision risk, not a hypothetical. **Resolved**: implementer MUST first check whether the dispatch path already injects an identifier that uniquely distinguishes *this specific dispatch attempt* (not just the job, since a redispatch/retry of the same job needs a distinguishable identifier from its predecessor attempt — `jobs.id` alone is insufficient for this per a3's analysis and must not be used alone). If no such per-attempt identifier already exists in the injected environment, **implementer MUST mint one** (a concatenation or pairing of `jobs.id` with a per-dispatch sequence number/timestamp/`state_version`-at-dispatch-time is sufficient) and it is a new, explicitly named environment variable — do not leave the variable name itself undecided past design.md's refinement; whoever implements C7 pins the exact name/format as the first sub-task, before either the sandbox-side hook CLI or the host-side consumer is written, specifically so both sides build against one agreed contract instead of independently guessing.
- At-least-once delivery survives a daemon restart: a report written before restart and not yet consumed is picked up on daemon startup, not lost.

Testability: `--lib` for attribution-key logic; process-kill-timing race is CI-integration (cannot be reliably forced in `--lib` unit tests).

## Requirement C8: Sibling-Cgroup Child-Liveness PoC

Deliver a small, isolated proof-of-concept (not full arbiter integration) demonstrating child-process liveness detection via a sibling-scope cgroup layout, validating the layout is physically viable before it's designed into the arbiter's OS-layer signal.

Acceptance criteria:
- PoC creates a parent transient slice with two sibling scopes (CLI scope, workload scope) per the mechanism in `perception-divergence-a3-round2-2026-07-10.md` §3 (parent/child containment layout is invalid under cgroup v2's no-internal-processes/leaf rule — sibling layout is the only viable form, this is settled, do not re-relitigate the layout choice itself in implementation).
- PoC demonstrates `workload.scope/cgroup.events` `populated` field correctly reads 0 when a spawned child process (e.g. `sleep 5 &`) exits, and 1 while it's alive — measured directly, not asserted from documentation.
- PoC reports on the two known risk points from the divergence report: `systemd-run` DBus overhead under rapid successive spawns, and the write-race when a child dies before its PID is written into `workload.scope/cgroup.procs` (ESRCH handling).
- PoC output feeds back into design.md as a design update, not treated as a rubber stamp — if the PoC finds the sibling layout impractical (DBus latency too high, delegation permissions unavailable in the ah sandbox model, etc.), that's a valid PoC outcome and blocks C8 from graduating into the arbiter design until resolved.

Testability: CI-integration only (requires real cgroup v2 + systemd-run); not `--lib`.
