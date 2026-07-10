# ah Perception Arbiter — Design

Status: draft, master-authored, pending a3 adversarial review. This design answers the four must-answer questions from `research/perception-final-convergence-2026-07-09.md` §四 explicitly — none are left open past this document. Where a3's round-2 divergence report proposed a mechanism, it's cited and adopted or explicitly modified with a reason; master owns the final call. gap-patched 2026-07-10 (operator acceptance round) — see Open Items below for the added dispatch-readiness pane-diff cross-reference; full incident detail lives in requirements.md's Existing Grounding.

## Design Thesis

Today, six completion inferers each get to write `agents.state` directly, coordinated only by "DB CAS first-come-first-served + scattered yield-ifs" (`architecture-assessment-converged-2026-07-09.md` §一.3). This produces exactly the bug class this module exists to kill: contradictory audit trails for the same transition (see requirements.md Existing Grounding — `PANE_DIFF_STUCK` vs `HEALTH_CHECK_STUCK` written for the same event), and STUCK states reached by timeout-inference instead of explicit signal.

The fix, per K8s-conditions precedent (deep-research confirmed, final-convergence §1.1): monitors become **read-only signal producers**. Exactly one consumer — the arbiter — holds write authority over `agents.state`. Producers and arbiter communicate only through an append-only event log.

```text
[marker/timer.rs]  [health_check.rs]  [hook handler]  [log monitor]
        |                  |                 |               |
        +------------------+-----------------+---------------+
                            | (perception_events, tri-state, epoch-tagged)
                            v
                  [Perception Arbiter]  <-- single writer of agents.state
                            |
                            v
                     [agents.state]
                            |
                            | (Stalled/Idle/etc. verdict events)
                            v
             [Orchestrator — consumes verdicts, drives Job state (module D)]
```

This module ends at `agents.state`. It does not decide `jobs.status` — that's module D's `ah-control-plane-refactor` spec, and the boundary between them is itself must-answer question territory covered in that spec (job-state-machine's "collaboration with perception arbiter" section), not here. Keep them decoupled: this arbiter emits verdict events; module D's control plane is one specific *consumer* of those events, not a co-owner of this module's write path.

## Must-Answer Question 1: Single-Write Entry Point — Hard Constraint Form

**Decision: two-layer defense, not three.** a3's round-2 report (§1) proposes three layers — compile-time visibility gating, CI static scanning, and a SQLite `BEFORE UPDATE` trigger with session-variable authorization. Adopting all three as originally proposed is over-engineering for this codebase's actual risk profile:

- The compile-time visibility wrapper (`ReadConnection`/`MutConnection`, private `state_write_gate` module) is adopted **as the primary mechanism** — Rust module privacy is a real, enforced boundary as long as `rusqlite::Connection` is never re-exported through a public path. This is the cheapest layer that gives the strongest guarantee for the common-mistake case (someone writes a new `UPDATE agents SET state` call without thinking).
- The CI static-scan layer (grep or Semgrep rule banning `UPDATE agents SET state` / `UPDATE agents SET status` outside the gate module) is adopted as a **cheap backstop**, not the primary mechanism — a3 is right that visibility alone doesn't stop `format!()`-constructed SQL or `unsafe` transmutation, but those are deliberate-bypass scenarios, not the failure mode this module is actually defending against (accidental reintroduction of a direct write during a future refactor). A grep-based CI check is sufficient for that; do not build a Semgrep pipeline for a threat model this codebase doesn't have (no untrusted internal contributors).
- **The DB-trigger layer is rejected.** a3's own failure-mode analysis undercuts it: the "authorized session token" trick only works if no other code path can read/reuse the same DB connection with the token still set, which in a single `Arc<Mutex<Connection>>` process (confirmed architecture, `architecture-assessment-converged-2026-07-09.md` §二 "SQLite 多头写入" revision) is a real risk, not a hypothetical — the gate module already prevents this at compile time more cheaply, and the trigger overhead/complexity buys nothing on top of that in this specific architecture. Reconsider only if a future multi-process writer model is adopted (currently out of scope, out of roadmap).
- a3's "table-splitting" alternative (append-only `agent_state_journal`, current state = latest row) is **not adopted for `agents.state` directly** but its underlying idea — append-only writes, current state derived — is exactly what `perception_events` (Q2 below) already gives us at the event layer. Splitting the `agents` table itself is unnecessary duplication of that pattern; reject as redundant, not as wrong.

Implementation locus: new private module `crate::db::perception::gate` (name TBD at implementation time) exposes exactly one function that mutates `agents.state`.

**Correction (2026-07-10, a3 adversarial review, `research/a3-adversarial-review-of-c-d-specs-2026-07-10.md` §二.Q1/§三.1) — internal `db/` boundary was left leaky in the first draft.** The original wording ("`src/db/mod.rs` does not re-export raw `Connection::execute` to callers **outside** `db::`") only blocks callers outside the `db` module tree. a3 correctly points out that `db::jobs`, `db::recovery`, and `db::state_machine` are all *inside* `db::`, already hold native connection access for their own tables, and the gate as originally worded does nothing to stop one of them from writing a stray `UPDATE agents SET state` directly — which is precisely the accidental-reintroduction failure mode this requirement exists to prevent. **Fix**: the gate's privacy boundary must be drawn at `crate::db::perception::gate` itself, not at `crate::db`'s outer edge — meaning raw connection access sufficient to write `agents.state`/`agents.status`-shaped columns must not be freely available to sibling `db::` submodules either. Concretely: the `agents` table's write-capable connection handle is a private type owned by the gate module; `db::jobs`, `db::recovery`, etc. get whatever narrower connection/handle type they need for their own tables (jobs, events, recovery bookkeeping) without it also granting `agents` write access. This is more invasive than the first draft implied — implementer should expect to touch connection-handle types used across multiple `db::` submodules, not just add one new private module alongside the existing ones unchanged.

On the DB-trigger rejection specifically: a3's rebuttal (§三.1) — that a `Drop`-guard-scoped authorization token, set at transaction start and cleared at transaction end within the single `Mutex`-guarded connection, cannot be read/reused by concurrent code because nothing else can hold the lock during that window — is a sound RAII argument and **is accepted as a valid defense-in-depth mechanism**, not rejected. It is still not made a *requirement* here: given the corrected internal gate boundary above already closes the specific hole a3 found (sibling `db::` submodules no longer have ambient `agents`-write capability at all, trigger or no trigger), the trigger's marginal value shrinks to "defense against `unsafe`/`transmute`-level deliberate bypass," which was already conceded out of scope in the first draft (not this codebase's threat model). Implementer may still add it cheaply as extra depth; it is optional, not blocking, and the reasoning for "optional" is now the corrected gate boundary, not the original (partially wrong) rebuttal to a3's RAII point.

CI check lives alongside existing lint/check tooling (implementer's choice of grep script vs. existing clippy config — do not introduce a new CI tool dependency for this); the CI rule itself should also be updated to flag `agents`-shaped writes found anywhere under `db::`, not just outside it, matching the corrected boundary.

## Must-Answer Question 2: Per-Signal-Class Unknown Budgets

**Decision: three layers, values below.** a3's round-2 report (§2) proposed OS=30s / Log=15min / Hook=2s. Two of three are adopted as-is; one is corrected against existing code:

| Layer | Budget | Rationale | Correction vs. a3 draft |
|---|---|---|---|
| OS/process liveness | **30s**, 5s retry cadence | DBus/`/proc` queries are normally sub-millisecond; 30s absorbs transient scheduler/DBus stalls under load without masking real death. Adopted as proposed — no existing constant to reconcile against. | none |
| Log/completion quietness | **900s** (reuse `MAX_LOG_MONITOR_WAIT`, `src/completion/monitor.rs:10`) | a3 proposed a fresh 15-minute (900s) value independently, which happens to already be the live constant (raised from 300s by `a7c9d34`, confirmed on HEAD by a4's G1 spot-check 2026-07-10). **Do not introduce a second 900s constant** — this layer's budget must literally be the existing `MAX_LOG_MONITOR_WAIT`, imported, not restated. If the arbiter needs a different value later, that's a deliberate change to the one constant, not a fork. | reuse existing constant, don't restate |
| Hook delivery | **2s** post-process-exit grace window | a3's reasoning holds: OS layer can observe process exit before the hook's socket/file write completes; 2s absorbs that ordering gap without masking genuinely missing hooks. This value is coupled to the attribution mechanism in Q4 — see that section for why 2s and the outbox design are consistent, not independently guessed. | none, but see Q4 cross-reference |

Failure modes both a3 and this design flag as real, not hypothetical: system clock skew in virtualized hosts breaking `Instant`-based budgets (mitigate: use monotonic clock source consistently, do not mix `SystemTime` and `Instant` across the budget-tracking code); `ahd` itself stalling under SQLite lock contention causing the 2s hook-grace window to fire spuriously (mitigate: the arbiter's own tick latency should be part of its self-observed health telemetry — if arbiter tick latency exceeds ~500ms routinely, that's a signal worth surfacing, not silently absorbed into false hook-timeout verdicts).

**Correction (2026-07-10, a3 adversarial review, §二.Q2) — the 2s hook budget must not be a one-way "guillotine" into job-terminal `Failed`.** a3 correctly identified that if the 2s expiry directly drove `jobs.status -> Failed` and the job state machine (`ah-control-plane-refactor` D1) has no `Failed -> Completed` transition, a hook that arrives late (via C7's cold-scan-on-restart recovery, or simply queued behind `ahd` lock contention) can never resurrect a job that was prematurely failed — regressing the existing `late_health_completion_stuck_allows_terminal` accept-gate behavior this whole module is supposed to preserve. **Fix, two parts, both required:**

1. **The 2s hook-budget expiry produces a perception-layer verdict** (e.g. `AgentUnexpectedExit` or equivalent — exact name is an implementation decision), **not a direct job-state write**. This verdict feeds into `ah-control-plane-refactor`'s D2 Phase 2 evidence-check consumer as one input among others (the same consumer that already evaluates `requires_physical_evidence`), rather than being sufficient on its own to terminally fail the job. Phase 2 is the sole place that decides `Completed` vs `Failed` (D1's gate), and by design it runs *after* the physical turn, giving a genuinely late-but-real hook (arriving via cold-scan before Phase 2 actually evaluates) a chance to be seen as legitimate evidence rather than being raced against a premature terminal write.
2. **`ah-control-plane-refactor`'s D1 transition table gets one narrow addition**: `Failed -> Completed`, gated specifically by an explicit late-evidence-reconciliation event (not a general-purpose reopen-anything transition) — this is the direct structural analog of the existing `late_health_completion_stuck_allows_terminal` accept-gate for `STUCK`, preserved rather than silently dropped by the new state machine. See `ah-control-plane-refactor/design.md`'s D1 section for the transition-table update and the guard condition.

Consistency check both specs' implementers must observe at kickoff: the 2s hook budget (this document) and the late-evidence-reconciliation window (`ah-control-plane-refactor` D1) are describing the same physical race from two sides — if Phase 2 evaluates and commits `Failed` *before* a hook that's merely running late (not actually lost) arrives, the reconciliation transition is the safety net; it should not be treated as the *expected* common path. If reconciliation fires often in practice, that's a signal the 2s budget or Phase 2's evaluation timing needs tuning, not that the reconciliation path is working as designed.

## Must-Answer Question 3: Parent/Child Cgroup Delegation — Corrected to Sibling Layout

**a3's round-2 report correctly identifies that the brief's original framing ("parent cgroup holds agent CLI, child cgroup holds spawned processes") is physically impossible under cgroup v2's no-internal-processes ("leaf") rule** — a cgroup with a controller enabled cannot contain both processes and child cgroups simultaneously. This is accepted without further debate; it's a kernel constraint, not a design preference.

**Decision: sibling-scope layout**, per a3 §3:

```text
              [ah-agent-<id>.slice]
                    |
      +-------------+-------------+
      |                           |
[ah-agent-<id>-cli.scope]   [ah-agent-<id>-workload.scope]
(agent CLI process only)    (all spawned shells/subprocesses,
                              Delegate=yes)
```

Liveness reads `workload.scope/cgroup.events`'s `populated` field: `1` = something besides the agent CLI is still running, `0` = clean. This is the OS-layer's C8 PoC target, not yet integrated into the arbiter pending PoC results (per requirements.md C8 — a failed PoC blocks graduation, this is a real gate not a formality).

Known risk carried forward from a3's failure-mode analysis, not resolved by this design (deliberately deferred to PoC evidence): `systemd-run` DBus overhead under rapid spawn bursts, and the ESRCH race when a child process dies between fork and its PID being written to `workload.scope/cgroup.procs`. Fallback for platforms without cgroup v2 delegation (macOS, some container hosts) is PGID-tree tracking, explicitly named as a degraded mode with a known escape risk (`setsid()`/`setpgid()` detachment) — not designed further here; out of scope per requirements.md.

**Additional risk surfaced by a3's adversarial review (§二.Q3), corrected here rather than merely noted**: the PoC's original framing risked implying the *sandboxed agent CLI itself* would write its spawned children's PIDs into `workload.scope/cgroup.procs`. a3 correctly flags this as a potential sandbox-escape vector — a sandboxed process with write access to any part of `/sys/fs/cgroup` (even a scope nominally delegated to it) is a process with a lever against host resource control, and most sandbox models mask or read-only-mount `/sys/fs/cgroup` precisely to prevent this. **Corrected design, adopting a3's own alternative (§七.2) as the required approach, not merely a fallback**: the `workload.scope` is created and PID-populated **entirely host-side, by `ahd`**, never by code running inside the sandbox. Concretely — `ahd` creates both sibling scopes at agent-spawn time (before handing control to the sandboxed CLI), and when the sandboxed CLI's wrapper (the same host-side spawn path that already launches the CLI inside its `cli.scope` today) forks a child process, the **host-side spawn wrapper** — not the sandboxed CLI — is what places that child into `workload.scope/cgroup.procs`, because the wrapper is the one entity that sees the fork happen on the host side of the sandbox boundary in the first place (it's already invoking `systemd-run`/equivalent for the top-level CLI process; the same host-side code path is the natural place to also handle its children). This removes the requirement for sandbox-internal cgroup filesystem write access entirely — C8's PoC must validate *this* host-side-only mechanism, not the sandbox-internal-write version the first draft implied. If the PoC finds that the host-side wrapper cannot observe/intercept child-process forks inside the sandbox at all (a real possibility a3's own alternative flags as unresolved — "ahd 无法感知沙箱内 Shell 的 fork 时序"), that is itself a valid PoC failure outcome per requirements.md C8's existing "failed PoC blocks graduation" clause — do not paper over it with a sandbox-write compromise if the host-side approach doesn't pan out; escalate to design reconsideration instead.

## Must-Answer Question 4: Hook Attribution Race

**Decision: adopt a3's outbox-pattern mechanism (§4)** — durable local write before any network/socket attempt, daemon-side inotify/scan consumption, at-least-once delivery across daemon restart:

```text
[Agent sandbox]
  ah agent notify  --writes-->  {agent_home}/outbox/{event_id}.tmp
                                        | rename (atomic)
                                        v
                                 {agent_home}/outbox/{event_id}.json
                                        |
                                        | ahd: inotify + cold-scan-on-restart
                                        v
                                 perception_events insert
                                 (attributed by job_cookie, not "current active job")
```

**Corrected 2026-07-10 after a3 adversarial review** (§二.Q4/§三.2) — the first draft of this section deferred the attribution-key decision ("reuse whatever exists") without confirming a fit-for-purpose identifier is actually injected today. a3 checked and found only `jobs.id` is plausibly available, which alone cannot distinguish two dispatch attempts of the same job (a fast redispatch's stale, late-arriving hook would misattribute to the new attempt) — so "reuse, don't mint" as originally worded was not a safe instruction, and leaving the exact variable name unpinned risked the sandbox-side hook CLI and host-side consumer being built against incompatible assumptions.

**Resolved**: the attribution key is a per-dispatch-attempt identifier, not just a per-job one. If dispatch doesn't already inject something that serves this purpose, one must be minted — the effort saved by "not inventing a new cookie" was never the point; the point was not inventing an *unpinned, ambiguous* one. Whoever implements C7 pins the exact env var name and value format (e.g. `AH_JOB_ATTEMPT_COOKIE = "{job_id}:{dispatch_seq}"` or equivalent — exact format is theirs to choose, but it must be chosen and written down as the first step of implementation, not discovered independently by two different implementers of the two sides of this contract).

Consistency with Q2's 2s hook budget: the outbox pattern makes the *write* durable immediately (hook process can die right after `rename()` and the report survives) — the 2s budget in Q2 is not "how long we wait for the outbox write," it's "how long we wait for `ahd` to *notice and process* an outbox file that's already durably written," covering inotify latency + daemon tick cadence, not disk I/O. If the two ever need different values, that's a sign the outbox consumption loop's cadence assumption was wrong, not that the budget itself needs a second number.

Failure modes carried forward from a3's analysis: sandbox write permission restrictions blocking outbox writes entirely (must fail loud, not silently drop — if the sandbox model can't guarantee outbox write access, that's an escalation to the sandbox design, not a perception-arbiter problem to route around); disk-full (`ENOSPC`) on the outbox path — a3's suggested in-memory ring-buffer last-resort is noted but not adopted as a committed requirement here; treat disk-full as an operational alert case (dogfood ledger data point) rather than over-engineering a fallback for a failure mode that should be caught by existing disk-space monitoring (per master handoff §六.6, disk-full is already a known independent failure family with its own runbook).

## Relationship to Module D (Control Plane / Job State Machine)

This arbiter's write authority stops at `agents.state` and perception-verdict events. It does not read or write `jobs.status`. The control-plane spec (`ah-control-plane-refactor`) defines how its job state machine *consumes* this arbiter's verdict events — that boundary is designed in that spec, cross-referenced here so neither spec silently assumes ownership of the seam. See `ah-control-plane-refactor/design.md` §"Perception Arbiter Collaboration Boundary".

## Open Items Explicitly Deferred to Implementation (not must-answer, don't block spec freeze)

- Exact `perception_events` schema (column list/types) — sketch in requirements.md C2 is directional, not final DDL. **Note**: the verdict-event shape consumed by `ah-control-plane-refactor`'s D7 is now pinned at a minimum-viable level (see that spec's design.md D7 section) — this module's actual `perception_events` row schema should be able to produce that shape, though it need not be byte-identical to it (the row schema may carry more internal detail than the cross-module event does).
- Whether `Stalled`-reason string values are a Rust enum or free-form `&str` — pick whichever the existing `events` payload convention already uses, don't introduce a new pattern for this one field.
- Retention/reap policy for `perception_events` (a3 flagged event-table bloat as a real risk in §7 failure modes) — reuse whatever the existing `events` table reap policy already does if one exists; if none exists, that's a pre-existing gap this module inherits, not one it's required to solve.
- **Second pane-diff site, cross-referenced not designed here** (added 2026-07-10, gap-patch): the daemon's dispatch-readiness recheck (pre-send pane-diff gate, distinct subsystem from this module's `agents.state` write path) is a second surviving pane-content-inference mechanism — incident detail in requirements.md's Existing Grounding, deletion tracked in tasks.md's Cross-Cutting section. Not designed here because it isn't this module's write authority; noted so the "no pane-content inference" precedent this module establishes doesn't quietly stop at this module's own boundary.
