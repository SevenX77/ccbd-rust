# ah Control Plane Refactor — Design

Status: draft, master-authored, pending a3 adversarial review. gap-patched 2026-07-10 (operator acceptance round) — D1+D2 section gained queuing-reason observability + cancel driver/timeout takeover design notes from a live Gen-2 incident; see requirements.md's Existing Grounding for the incident detail. The two realign bugs the same incident surfaced are explicitly out of D4's pure-move scope — see `research/backlog-realign-bugs.md`.

## Design Thesis

The architecture assessment converged on a single root diagnosis across items D1/D2 (job has no state machine, F3=F2 hardcoded), D3 (four independent kill orderings), and D4 (spawn_realign misplaced, causing a dependency cycle): **the control plane has no single owner for any of its three core write surfaces** (job status, kill/teardown sequencing, realign lifecycle). This spec gives each surface exactly one owner, mirroring the same "single writer" discipline the perception arbiter spec applies to `agents.state`.

```text
                    [Perception Arbiter]  (ah-perception-arbiter spec)
                            |
                            | verdict events (agent turn ended, stalled, etc.)
                            v
                 [Job Verification Step]  (D2 — new)
                            |
                            | evidence-checked outcome
                            v
                  [Job State Machine Gate]  (D1 — single writer of jobs.status)
                            |
                            v
                      [jobs.status]

  [Kill/Teardown Unification]  (D3 — single writer of destructive actions,
   consumed by agent.rs / sessions.rs / orchestrator / master_watch call sites)

  [Realign]  (D4 — relocated to orchestration module, no longer in rpc)
```

## D1 + D2 Combined Design: Job State Machine with Physical/Business Split

These two requirements are designed together because D2's two-phase split only makes sense once D1's gate exists to receive its second-phase write.

**Transition table** (base adopted from a3's round-2 §5 proposal; one narrow addition below, made necessary by the Q2 hook-budget correction):

```text
QUEUED -> DISPATCHED | CANCELLED | FAILED
DISPATCHED -> COMPLETED | FAILED | CANCELLED
FAILED -> COMPLETED   -- narrow, guarded: see below
```

**`FAILED -> COMPLETED` guard condition (added 2026-07-10, closes a gap a3's adversarial review found, `ah-perception-arbiter/design.md` Q2 correction)**: this transition is not general-purpose "reopen any failed job." It is gated by a specific late-evidence-reconciliation event — a hook or other authoritative completion evidence arriving *after* the job was already marked `Failed*`, where that evidence's timestamp/epoch predates or is contemporaneous with the original run (not a new, unrelated run being misattributed — C7's attribution mechanism must already have ruled that out before this transition is even considered). This is the direct structural analog of the existing `late_health_completion_stuck_allows_terminal` accept-gate for `STUCK` — that precedent already exists in the codebase for exactly this "late-arriving legitimate signal shouldn't be trapped by an earlier terminal-looking state" problem, and this transition generalizes it to the job layer rather than reinventing a different mechanism. Implementer should model this transition's guard directly on the existing accept-gate's logic where practical, not design a new pattern from scratch.

**Gate function**: `db::jobs::transit_job_state(tx, job_id, expected_from, to, reason)` — takes an explicit expected prior state (CAS-style, matching the existing `state_version` pattern already used for `agents`) so a stale caller can't silently stomp a state another writer already moved past.

**Queuing-reason observability (added 2026-07-10, gap-patch, Gen-2 incident).** The gate function's caller in the dispatch path — wherever "is this agent eligible to receive a new job right now" is decided — must emit a structured signal (log line, event row, or a field the `ah ps`/`ah events` surface already reads; implementer's choice of mechanism) whenever a dispatch attempt is deferred because the target agent has an in-flight, non-terminal job. Tonight's incident (operator report, 2026-07-10) had zero daemon-side signal for 5+ minutes while a new job silently queued behind a stale `DISPATCHED` job that never reached a terminal state (a job whose implementing agent's turn ended without the daemon ever recording a completion signal — see D1's `Failed -> Completed` reconciliation discussion above for the general "late/missing signal" problem family this is part of). This does not require new infrastructure beyond what D1 already needs for the transition gate — it's an acceptance requirement on the gate's observability, not a new subsystem.

**Cancel driver + timeout takeover (added 2026-07-10, gap-patch, Gen-2 incident).** `DISPATCHED -> CANCELLED` is a legal transition in the table above, but the table alone doesn't say who writes it. Decision: the orchestrator/daemon side is the authoritative driver of this transition. An agent acknowledging and cleanly exiting its own cancelled job is the fast/common path, but it is not the *only* path the gate must support. If an `ah cancel` request against a `DISPATCHED` job goes unacknowledged past a bounded timeout, the orchestrator must be able to forcibly drive the `DISPATCHED -> CANCELLED` transition itself through the same gate function, rather than leaving the job logically stuck pending agent cooperation that may never arrive (dead process, hung process, or a semantic-false-completion turn that never emits any signal at all). Tonight's incident is the concrete failure mode this closes: `ah cancel` returned `CANCEL_REQUESTED` but the underlying job stayed live (repeating `STOPPED_UNDECLARED_ALERT`/`UI_RECAPTURE_ALERT` churn in the daemon's own event log) for the rest of the session — actual resolution needed an operator-level `ah kill` on the occupant agent(s) plus a daemon `REALIGN` restart, not the cancel gate itself. Exact timeout value and whether the forced takeover is fully automatic vs. requires an explicit prior `ah kill` are implementer decisions; "cancel can hang forever with no orchestrator-side takeover path" is the specific gap this closes, not a prescription for exactly how fast the takeover must fire.

**DB trigger backstop — not required.** Same reasoning as the perception arbiter's Q1 (see `ah-perception-arbiter/design.md`): this process is a single `Arc<Mutex<Connection>>`, so the compile-time gate (module privacy, no raw `execute` re-export outside `db::jobs`) plus a CI grep rule already closes the realistic accidental-reintroduction risk. A DB trigger is allowed as optional defense-in-depth if an implementer judges the marginal cost low, but do not block the spec on it.

**Two-phase completion** (D2) — **revised 2026-07-10 after a3 adversarial review found a real bug in the first draft; correction adopted, not just noted**:

The first draft of this design had the agent go straight to `IDLE` in Phase 1, before Phase 2's evidence check ran. a3's review (`research/a3-adversarial-review-of-c-d-specs-2026-07-10.md` §四.1) correctly identified this as a **read-after-write hazard**: `IDLE` is the dispatcher's eligibility signal, so a freshly-`IDLE` agent can be handed Job B into the *same physical workspace* (sandbox, tmux pane) before Phase 2 has finished reading Job A's evidence (git diff, file mtimes, pane transcript) from that same workspace. Job B's writes corrupt Job A's evidence out from under the verifier. This is a real bug, not a hypothetical — it's exactly the kind of physical-layer race this whole spec exists to eliminate, and the original draft reintroduced a version of it. **Adopted fix, a3's proposed mechanism (§七.3), essentially verbatim:**

1. **Phase 1 (physical)**: what is currently `mark_agent_idle_matched_conn_inner` and its siblings transition the agent to a **new intermediate state `VERIFYING`** (not `IDLE`) via the perception arbiter's gate — this function itself becomes a perception-layer producer, see `ah-perception-arbiter` D1/D2 — and emit a `JobExecutionFinished { job_id, epoch, outcome_hint }` event. It does **not** touch `jobs.status`, and it does **not** make the agent dispatcher-eligible yet.
2. **`VERIFYING` is excluded from dispatch eligibility**: the orchestrator's "find an available agent" query must not select `VERIFYING` agents, the same way it already excludes `BUSY`/`STUCK`/`WAITING_FOR_ACK`/`SPAWNING`. This is a small, mechanical addition to whatever query currently checks `state = 'IDLE'` for dispatch candidacy — implementer must audit all such call sites (there is more than one dispatch-candidate query in the codebase per the architecture assessment's general finding of scattered logic; do not assume there's exactly one).
3. **Phase 2 (business)**: a new orchestrator-owned consumer reads `JobExecutionFinished` events, runs whatever evidence check the job requires (existing `requires_physical_evidence`-gated logic, relocated but not redesigned) **against the still-untouched workspace**, and either:
   - **Success**: calls `transit_job_state` to move the job to `Completed`, then transitions the agent `VERIFYING -> IDLE` (now genuinely dispatcher-eligible, workspace state no longer matters).
   - **Failure**: calls `transit_job_state` to move the job to `Failed`, then transitions the agent to a new terminal-ish state `FAILED_VERIFICATION` (not `IDLE`) — this **deliberately preserves the workspace** (pane, sandbox files) for operator in-situ debugging, per a3's §四.2 finding that immediate `IDLE`-and-reuse also destroys the ability to triage *why* an evidence check failed. An operator/master action (equivalent to today's manual recovery/requeue path) is required to move an agent out of `FAILED_VERIFICATION` — do not auto-recycle it back to `IDLE`, that would silently reintroduce the same destroyed-evidence problem for the next debugging attempt.

This makes `VERIFYING` and `FAILED_VERIFICATION` the answer to the original draft's "observability of the intermediate state" question — they're first-class `agents.state` values, not a hidden marker, so `ah ps`/`ah events` show them the same way any other agent state shows today. No separate `pending_verification` flag is needed; the state value itself carries the meaning.

**Consequence for dispatch throughput**: an agent is now unavailable for new work for the duration of Phase 2's evidence check, not just Phase 1's physical run. This is the correct tradeoff — the alternative (a3's rejected original draft) trades a small throughput loss for a correctness bug. If Phase 2 checks turn out to be slow enough that this throughput loss matters in practice, that's a performance-tuning problem for Phase 2's implementation (make evidence checks fast), not a reason to reopen this correctness decision.

**Crash recovery**: startup reconcile (existing pattern already used for other recovery scans, e.g. the orphan-scope reconciliation in `ah-orchestration-reliability`) gains a pass that finds jobs with an unconsumed `JobExecutionFinished` event (agent already `IDLE`, job still `DISPATCHED`, no terminal state reached) and re-drives Phase 2 for them. This directly closes the gap a3's round-2 report flagged in its own D2-equivalent design (§6 failure modes, "daemon crash between event write and consumption").

## D3 Design: Kill/Teardown Unification

**Canonical order** (adapted from the existing ownership-gate discipline in `ah-orchestration-reliability`'s D1, which this requirement must not weaken):

```text
authorize_destructive_action(...)  -- existing gate, reused not reinvented
    -> Rejected: stop here, emit warn-level event, do nothing further
    -> Authorized:
        1. process/scope kill (systemd stop_unit on Linux, kill(-pgid) fallback elsewhere)
        2. tmux session/pane teardown
        3. sandbox state cleanup
        4. event emission (state_change / teardown_complete)
```

All four current call sites (`agent.rs:275-300`, `sessions.rs:134-165`, `orchestrator/mod.rs:562/577`, `master_watch.rs:1029`→`system.rs:381`) become thin callers of one function implementing this order. This function lives in a new or existing shared module — `agent_io::registry` is the natural home since it already owns the tmux-cleanup surface (module B's B2 fix targets this same file).

**Sequencing vs. module B's B2 (tmux cleanup fallback)**: B2 is the smaller, already-dispatched machine-fix (add a fallback when `kill_*_if_owned` fails because `expected_pid` is already dead). **D3 absorbs B2's fix as part of its unified implementation, sequenced after B2 lands** — do not implement B2 and D3 in parallel against the same file; B2 merges first (it's already in a dispatched worktree with its own RED tests), D3's unification is designed to build on top of B2's fallback logic, not duplicate or race it. If D3's implementer finds B2 hasn't merged yet when D3 starts, D3 should rebase onto B2's branch/PR rather than reimplementing the fallback independently.

## D4 Design: `spawn_realign_agent` Relocation

Move `spawn_realign_agent` from `src/rpc/handlers/realign.rs:375` into an orchestration-owned module (suggested: `orchestrator::realign` or co-located with existing orchestrator lifecycle code — exact module path is an implementation naming call, not fixed here). This breaks the cycle described in the architecture assessment: today `orchestrator` and `master_watch` (monitor) both import from `rpc` to reach realign, while `rpc` handlers conceptually belong "above" orchestration in the dependency graph (transport layer calling into domain logic, not the reverse). After relocation, `rpc`'s realign *handler* (the actual RPC entry point, if external callers need one) calls into the relocated orchestration function, restoring a one-directional dependency edge.

This is a pure move — implementer should resist the temptation to "improve" realign semantics in the same commit; if problems are found during the move, file them as follow-ups, not silent scope creep into this requirement.

**Confirmed 2026-07-10 (operator acceptance round, gap-patch): two real realign-semantics bugs surfaced by a live daemon restart the night before this pass** — a non-atomic old-session-delete/new-session-create swap that can drop an agent entirely if the create step fails or is skipped mid-sequence, and a respawn session-naming bug that created one agent's tmux session under a different (adjacent/stale-slot) agent's name. Both are real and both stay **out of D4's scope by design**, per the pure-move framing above — they are semantics bugs, not the location-of-code problem D4 fixes. They are tracked independently in `research/backlog-realign-bugs.md` so "D4 doesn't own semantics" doesn't quietly become "nobody owns this."

## D5 Design (directional only): `db/` Target Boundary

Named here so a future pass has a target, not because this round schedules the work:

```text
db/
  domain/       -- pure state machine logic: valid transitions, CAS semantics
                   (this is where D1's transit_job_state and the perception
                   arbiter's gate conceptually belong, long-term)
  application/  -- reconcile loops, recovery scans, orchestration-adjacent
                   logic currently mixed into system.rs/recovery.rs
  repository/   -- raw SQL, schema, connection management only
```

`system.rs`'s 37 `systemctl` side-effecting calls are the clearest signal this file is doing `application`-layer work under a `repository`-layer name — they're the natural first extraction target whenever D5 is scheduled, but that scheduling is explicitly out of this round.

## D6 Design: `master_watch.rs` Decomposition

**Step 1 (this round's actual deliverable)**: extract the ~3300 inline test lines to a sibling test module, verified by running the moved tests and confirming zero behavior change. This is deliberately the *first* and lowest-risk step — per the architecture assessment's own sequencing advice, moving tests first means the production-code split that follows has a working safety net that isn't itself entangled with the file being split.

**Step 2 (name the target, implement opportunistically)**: extract provider-specific knowledge (Claude transcript parsing, `CLAUDE_CONFIG_DIR` semantics) into a provider-specific module — this is monitoring code reaching into provider internals it shouldn't need to know about, and is a clean, self-contained extraction.

**Step 3 (name the target only)**: the revival/cutover pipeline is the architecture assessment's suggested next extraction after tests+provider-knowledge move. Full 13-responsibility decomposition is not required to close D6 for this spec round — a working Step 1 + Step 2, with Step 3 named as the next follow-on, is sufficient.

**Acknowledged limitation (a3 review, §四.4)**: a3 correctly points out that Step 3 — the revival/cutover pipeline — is where most of `master_watch.rs`'s actual complexity and race-condition history lives, and naming it as a follow-on rather than implementing it this round means the god-file's sharpest edges are not actually resolved by this spec round. **This is accepted as a deliberate scope decision, not corrected**: Steps 1-2 are low-risk, quick, and unblock everything else (a safety net exists before anyone touches the risky code); Step 3 is a substantial redesign in its own right (revival/cutover logic has its own race conditions, per the architecture assessment) and deserves its own focused pass with its own RED tests, not a rushed inclusion here for the sake of a complete-looking checkbox. If this tradeoff is wrong — if Step 3's risk is judged too costly to defer — that's a call for whoever schedules this spec's implementation to make explicitly (e.g., promote Step 3 to a required task in this round), not something design.md silently resolves either way.

## D7 Design: Perception Arbiter Collaboration Boundary

**Hard rule, stated once so both specs can cite it instead of re-deriving it**: `agents.state` has exactly one writer (the perception arbiter's gate, `ah-perception-arbiter` C1). `jobs.status` has exactly one writer (this spec's D1 gate). Neither module's code may call the other's write gate directly — the only channel between them is the event log (`perception_events` producing verdict events; this module's `JobExecutionFinished`-class events are themselves perception-arbiter-adjacent since they originate from what is, after D2, an arbiter-owned producer function).

**Event shape consumed by this module from the perception arbiter — pinned 2026-07-10 after a3 adversarial review** (`research/a3-adversarial-review-of-c-d-specs-2026-07-10.md` §六.1) found "leave the shape to implementers to reconcile later" was not a safe deferral: without a concrete shape, neither side can write a fake/mock to test against, and independent guessing risks exactly the "空中碰撞" (mid-air collision) integration failure a3 describes. **Minimum pinned shape, both specs' implementers build against this and may add fields but must not remove or retype these**:

```rust
struct PerceptionVerdictEvent {
    agent_id: String,
    epoch: i64,           // matches ah-perception-arbiter C5's epoch/generation value
    job_id: Option<String>,  // the job this verdict pertains to, if any (None for
                              // agent-level-only verdicts with no active job)
    verdict: PerceptionVerdict,
    detail: serde_json::Value,  // free-form, layer-specific detail (unstructured
                                 // on purpose — this is where per-layer specifics
                                 // like signal_kinds or dead_layers go without
                                 // forcing a shared schema on layer-specific detail)
}

enum PerceptionVerdict {
    TurnFinished { outcome_hint: TurnOutcomeHint },  // -> D2 Phase 1 trigger
    Stalled { reason: String },                       // -> C6, accurate reason string
    UnexpectedExit,                                    // -> Q2's hook-budget-expiry verdict
    Unknown,                                            // -> C3's explicit-Unknown emission
}

enum TurnOutcomeHint {
    LooksSuccessful,
    LooksFailed,
}
```

This is a minimum viable contract, not final DDL/API — field names, exact enum variants, and serialization format (this may end up as a DB row shape rather than a literal Rust struct passed in-process, depending on how `perception_events` vs. this module's consumption loop are actually wired) are implementation-detail refinements either side may propose. The requirement this pinning satisfies is narrower and non-negotiable: **both sides have a written, shared minimum to build fakes against before either writes production code**, so integration is a refinement conversation, not a from-scratch reconciliation after both are half-built.
