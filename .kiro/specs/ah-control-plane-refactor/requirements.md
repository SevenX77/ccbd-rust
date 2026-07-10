# ah Control Plane Refactor — Requirements

Status: converged after a3 adversarial review (2026-07-10) — see `research/a3-adversarial-review-of-c-d-specs-2026-07-10.md` and this file/design.md's inline "Revised"/"Pinned"/"Clarified" notes (D2's `VERIFYING` state fix for the read-after-write race, D1's `Failed -> Completed` reconciliation transition, D7's pinned minimum event shape, evidence-gate guardrail preservation). **Not yet cleared for implementation** — awaiting operator/user sign-off; tasks.md is outline-only, gated on module A/B closeout + 换血#2. gap-patched 2026-07-10 (operator acceptance round) — D1 gained two acceptance criteria (queuing-reason observability, cancel driver/timeout takeover) from a live Gen-2 incident; see Existing Grounding below. The two realign bugs the same incident surfaced (non-atomic swap, respawn misattribution) are explicitly NOT folded into D4 — tracked independently in `research/backlog-realign-bugs.md`.

Source material:
- `research/architecture-assessment-converged-2026-07-09.md` §一 (items 1, 2, 4, 5, 6, 7, 9) and §三 (action list items 1, 3, 5) — the structural findings this module fixes, all dual-verified (both operator and a3 independently reached them or one verified the other's claim against live code).
- `research/perception-divergence-a3-round2-2026-07-10.md` §5-7 — a3's job-state-machine mechanism proposal and the perception/control-plane collaboration boundary.

## Scope

In scope:
- D1. Job state machine: single write authority, explicit transition table, replacing ~11 scattered `UPDATE jobs SET status` call sites.
- D2. F3/F2 decoupling: separating "agent turn ended" (physical) from "job completed" (business/verified) into two phases instead of one dual-purpose transaction.
- D3. Kill/teardown unification: collapsing four independently-ordered kill/teardown implementations into one.
- D4. `spawn_realign_agent` relocation out of the RPC layer, breaking the orchestrator↔monitor↔rpc dependency cycle.
- D5. `db/` module boundary correction (domain logic currently living in what should be a pure data layer) — long-term item, scoped as directional guidance only, not a task list, in this pass.
- D6. `master_watch.rs` decomposition (2245 production lines + 3300 inline test lines, 13 stated responsibilities).

Out of scope:
- Perception/completion signal semantics — that's `ah-perception-arbiter`. This spec consumes that module's verdict events; it does not redefine what a verdict means.
- Per-worker credential isolation — separate spec `ah-per-worker-credentials`.
- Any behavior change to what counts as "done" for a job (evidence-gate rules, `requires_physical_evidence`) — this spec only relocates *where* and *how* the decision is written, not the decision logic itself. **Clarified 2026-07-10 after a3 review** (§四.3): this out-of-scope line must not be read as license to silently drop the two guardrails `perception-final-convergence-2026-07-09.md` §2.4 already established for the physical-evidence gate (`is_mutating` static task-dispatch labeling; a 2-nudge-then-escalate-to-human cap preventing infinite retry loops on read-only-mislabeled or permission-denied tasks). Those guardrails belong to the *existing* evidence-gate mechanism, which this spec does not redesign — but D2's Phase 2 consumer is a new *caller* of that mechanism, and its implementer must confirm both guardrails are still exercised through the relocated call path, not silently dropped because the call site moved. This is a verification checklist item for D2's implementation, not a design decision this spec needs to make.

## Existing Grounding

- ~11 scattered `UPDATE jobs SET status` sites confirmed live on current HEAD (spot-checked 2026-07-10): `src/db/jobs.rs:277,332,414,525,559,664,711,925` plus additional sites in `src/db/recovery.rs` and `src/db/state_machine.rs` cited in the architecture assessment. Exact count may have drifted slightly since the assessment was written (7/9) — implementer must re-grep at implementation time, do not trust either count blindly.
- F3=F2 hardcoding confirmed: `mark_agent_idle_matched_conn_inner` (`src/db/state_machine.rs`, function starts ~727) and sibling functions `mark_agent_idle_log_event_sync`/`mark_agent_idle_hook_event_sync`/`mark_agent_idle_hook_event_at_version_sync` mutate `agents.state` to `IDLE` and `jobs.status` to `COMPLETED` inside the same transaction. This is the deepest structural root of the "false completion" bug family (architecture-assessment §一.2) — an agent's turn ending is being treated as proof the job is done, with no independent verification step in between.
- Kill/teardown confirmed scattered across four call sites with inconsistent ordering (architecture-assessment §一.5): `agent.rs:275-300`, `sessions.rs:134-165`, `orchestrator/mod.rs:562/577`, `master_watch.rs:1029` → `system.rs:381`. This is named as the structural root of both the C2 teardown-escape bug (module B, already dispatched) and wrongful-kill-of-live-agent risk.
- `spawn_realign_agent` lives at `src/rpc/handlers/realign.rs:375` (confirmed on HEAD). Architecture assessment (§一.6) states orchestrator and monitor both depend on `rpc`, forming a three-layer cycle when `spawn_realign_agent`'s natural home should be orchestration, not transport.
- `master_watch.rs` size/responsibility count is as stated in the architecture assessment; not independently re-measured for this spec pass — implementer should re-run `wc -l` at implementation time since the file has likely grown since 2026-07-09.
- **Queuing-reason observability gap (added 2026-07-10, gap-patch, incident-confirmed).** When an agent has an in-flight `DISPATCHED` job that never reaches a terminal state (e.g. because the implementing tool's turn ended without emitting any completion signal — the "semantic false-completion" failure family, architecture-assessment §一.2/§一.3, now hardened into a hard-blocking form), a newly `ah ask`'d job for the same agent silently queues behind it with zero daemon-side log line explaining why. Operator/master incident, 2026-07-10 night: 5+ minutes of `ah ps`/`ah logs` giving no diagnostic signal before the stale occupant job was identified by hand via raw job-state-history inspection; `ah cancel` against it did not resolve the queue either (see next bullet). D1's gate is the natural place to close this — see Requirement D1 below.
- **Cancel-without-a-driver gap (added 2026-07-10, gap-patch, incident-confirmed).** `ah cancel <job_id>` against the stale `DISPATCHED` job above returned `CANCEL_REQUESTED` but the job stayed logically live — repeating `STOPPED_UNDECLARED_ALERT`/`UI_RECAPTURE_ALERT` state churn in the daemon's own event log — for the remainder of the session. Cancellation depended entirely on the (dead/unresponsive) agent-side process eventually acknowledging it, with no timeout or orchestrator-side forced takeover. Actual resolution required an operator-level `ah kill` on both occupant agents plus a daemon `REALIGN` restart (which itself surfaced two more bugs — see `research/backlog-realign-bugs.md`) — cancel alone never cleared it. The existing transition table already allows `DISPATCHED -> CANCELLED`; it does not say *who* drives that write when the agent side never cooperates. See Requirement D1 below.

## Requirement D1: Job State Machine — Single Write Authority

Replace all scattered `UPDATE jobs SET status` call sites with one transition-table-gated entry point, analogous in spirit to the perception arbiter's write gate (`ah-perception-arbiter` C1) but for job business state, not agent physical state.

Acceptance criteria:
- A `JobStatus` enum (`Queued`, `Dispatched`, `Completed`, `Cancelled`, `Failed`) with an explicit valid-transition table gates every status change. Illegal transitions (e.g. `Completed -> Dispatched`) are rejected at the gate, not merely discouraged by convention.
- The transition table includes one narrow, guarded exception: `Failed -> Completed`, gated by late-evidence reconciliation (added 2026-07-10 after a3 adversarial review found the original table, with no recovery path out of `Failed`, would regress the existing late-completion accept-gate once combined with the perception arbiter's hook-timeout verdict — see `ah-control-plane-refactor/design.md` D1 and `ah-perception-arbiter/design.md` Q2 for the full mechanism). This is not a general reopen-any-failed-job path; the guard condition is specific and must not be loosened without a corresponding spec update.
- Every current call site enumerated in Existing Grounding above is migrated to call the gate function; no direct `UPDATE jobs SET status` remains outside `db::jobs`'s internal gate implementation.
- A database-level `CHECK`/trigger backstop on `jobs.status` transitions is **optional, not required** — see design.md's cost/benefit call on this (same reasoning as the perception arbiter's rejection of a DB-trigger layer: the compile-time gate plus CI grep is judged sufficient for this codebase's actual risk profile; implementer may add the DB trigger anyway as defense-in-depth if the marginal cost is proven low, but it is not a blocking requirement).
- **Queuing-reason observability (added 2026-07-10, gap-patch — see Existing Grounding).** When a job cannot be dispatched immediately because its target agent has an in-flight, non-terminal job occupying its slot (or is otherwise not yet ready), that reason must be observable at the moment the decision is made — either a structured log line, or a queryable field/state surfaced through `ah ps`/`ah events` (exact mechanism is implementer's choice). "Nothing observable until the caller gives up waiting" is not acceptable — this was a real 5+-minute silent stall with zero diagnostic signal in production the night before this gap-patch was written.
- **Cancel driver and timeout takeover (added 2026-07-10, gap-patch — see Existing Grounding).** The gate must specify who is authorized to drive a `DISPATCHED -> CANCELLED` transition when the agent-side process never acknowledges a cancel request (dead, hung, or unresponsive agent, or one stuck in the semantic-false-completion failure family). At minimum: a cancel request unacknowledged past a bounded timeout must be forcibly completed by the orchestrator/daemon side, not left indefinitely pending on agent cooperation that may never arrive. Exact timeout value and the precise mechanism (fully automatic daemon-side forced transition vs. requiring an explicit operator `ah kill` first) are implementer decisions — but "cancel can hang forever with no takeover path" is the specific gap this closes.

TDD RED -> GREEN: RED = a test asserting a known scattered call site (e.g. `jobs.rs:525`'s direct `COMPLETED` write) is gone / redirected through the gate; GREEN = gate function performs the same write with transition validation.

Testability: `--lib`.

## Requirement D2: F3/F2 Decoupling — Physical Turn-End vs. Business Completion

**Revised 2026-07-10 after a3 adversarial review** (`research/a3-adversarial-review-of-c-d-specs-2026-07-10.md` §四.1/四.2/六.2) found two real bugs in the first draft: (a) a read-after-write hazard — an agent marked `IDLE` immediately in Phase 1 could be redispatched into the same physical workspace before Phase 2 read that workspace's evidence, corrupting it; (b) a cross-spec contradiction — this requirement's first draft described `mark_agent_idle_*` writing `agents.state` "in its own transaction," which read as a second write authority alongside the perception arbiter's single-writer gate (`ah-perception-arbiter` C1). Both are fixed by the same correction: see design.md's revised D1+D2 section for the mechanism (a new `VERIFYING` intermediate state, not `IDLE`, gates dispatch eligibility until business verification completes).

An agent's turn ending (physical) and a job being verified complete (business) become two separably-timed events instead of one hardcoded transaction — but the agent is not dispatcher-eligible again until *both* have happened.

Acceptance criteria:
- The function(s) currently named `mark_agent_idle_*` no longer write `jobs.status = 'COMPLETED'` inside their own transaction, and no longer write `agents.state` directly at all — they call through the perception arbiter's single-writer gate (`ah-perception-arbiter` C1) to transition the agent to `VERIFYING`, and emit a `JobExecutionFinished`-shaped event (exact event shape is a joint decision with `ah-perception-arbiter`, see D7) carrying the job's physical-completion outcome. This is a perception-arbiter-gated producer call, not an independent write path — restating this explicitly because the first draft's looser wording caused a3 to (correctly) flag it as contradicting C1.
- The dispatcher's agent-selection query(ies) exclude `VERIFYING` the same way they already exclude `BUSY`/`STUCK`/`WAITING_FOR_ACK`/`SPAWNING`. Implementer must audit all dispatch-candidate query call sites, not assume there is exactly one.
- A separate consumer (the orchestrator's job-verification step) reads the `JobExecutionFinished` event, performs whatever evidence check the job's `requires_physical_evidence` flag (or equivalent) demands **against the still-untouched workspace**, and only then calls the D1 gate to transition `jobs.status` to `Completed` or `Failed`, followed by transitioning the agent `VERIFYING -> IDLE` (success) or `VERIFYING -> FAILED_VERIFICATION` (failure — workspace deliberately preserved for operator triage, not auto-recycled).
- `VERIFYING` and `FAILED_VERIFICATION` are first-class `agents.state` values, visible in `ah ps`/`ah events` the same way any other state is — this satisfies the "observable intermediate state" requirement directly, no separate flag/marker needed.
- Crash recovery: if the daemon restarts between the physical-turn-end event being written and the verification step consuming it, the job is not permanently stuck `Dispatched`, and the agent is not permanently stuck `VERIFYING` — startup reconcile must re-discover and re-drive unconsumed `JobExecutionFinished`-class events. This is not optional hardening; a3's own failure-mode analysis (round-2 §6, and reaffirmed in the adversarial review) names this exact gap.
- An agent in `FAILED_VERIFICATION` requires an explicit operator/master recovery action to return to service — it must not auto-transition back to `IDLE`, or the workspace-preservation guarantee (the whole point of not reusing `IDLE` immediately) is defeated on the very next dispatch cycle.

Testability: `--lib` for the state-transition logic and crash-recovery re-discovery; full evidence-check behavior may require CI-integration if it touches real filesystem/git-diff scanning.

## Requirement D3: Kill/Teardown Unification

Collapse the four independently-ordered kill/teardown call sites into one shared implementation with one canonical ordering (ownership-gate check → process/scope kill → tmux teardown → sandbox cleanup → event emission, or whatever canonical order design.md settles on — the point of this requirement is that there is exactly one order, decided once).

Acceptance criteria:
- `agent.rs:275-300`, `sessions.rs:134-165`, `orchestrator/mod.rs:562/577`, and the `master_watch.rs:1029` → `system.rs:381` path all route through the same underlying teardown function after this change; none independently re-implements ordering or step selection.
- The existing ownership-gate discipline (`ah-orchestration-reliability` spec's D1 three-layer gate, already implemented/in-flight per that spec) is preserved and centralized here, not weakened or bypassed by the unification — this requirement must not regress an already-shipped safety property.
- Module B's tmux-cleanup-fallback fix (already dispatched separately, `research/modB-workorder-draft.md` B2) and this requirement's unification target the same code region (`agent_io/registry.rs`) — design.md must state explicitly whether D3 supersedes/absorbs B2 or the two are sequenced (B2 first as the smaller machine-fix, D3 later as the structural unification). Do not let both land independently and conflict.

Testability: `--lib` with fake kill/teardown sinks per call site, asserting identical step sequence regardless of entry point.

## Requirement D4: `spawn_realign_agent` Relocation

Move `spawn_realign_agent` (`src/rpc/handlers/realign.rs:375`) out of the RPC handler layer into an orchestration-owned module, breaking the orchestrator↔monitor↔rpc dependency cycle named in the architecture assessment.

Acceptance criteria:
- `orchestrator/mod.rs` and `master_watch.rs` no longer depend on `rpc` for realign functionality after this change (verify via `cargo` dependency/module-graph check, not just visual inspection — a simple `cargo check` after deleting the old import path is sufficient proof).
- RPC-layer callers of realign (if any external RPC surface actually needs to trigger it) call into the relocated orchestration module instead of the reverse.
- No behavior change to realign semantics — this is a pure relocation, not a redesign of what realign does. Any accompanying cleanup (e.g. removing dead code found during the move) is fine but must not be conflated with a semantic change in the same commit.

Testability: `--lib`, plus a `cargo check`-level module-graph sanity check.

## Requirement D5: `db/` Module Boundary Correction (directional only)

`db/` currently carries ~70% domain/orchestration logic per the architecture assessment (`system.rs`: 12 SQL statements vs. 37 `systemctl` side-effecting calls; 60+ active `pubsub` call sites originating from what should be a data layer). This requirement is **directional guidance for a future pass, not a task list for this implementation round** — do not schedule work against it in tasks.md beyond "name the target boundary."

Acceptance criteria (directional, not gating):
- design.md states the target boundary (state machine → `domain`, reconcile/recovery → `application`, raw SQL → `repository`, or an equivalent split) so that when this work is eventually scheduled, there's a named target instead of an ad-hoc reshuffle.
- No code movement is required to satisfy this requirement in the current spec round — it exists so D1-D4/D6 don't inadvertently move code in a direction that makes the eventual D5 split harder.

Testability: N/A (documentation-only requirement for this round).

## Requirement D6: `master_watch.rs` Decomposition

Split the 2245-line, 13-responsibility `master_watch.rs` god-file, starting with extracting its ~3300 lines of inline tests before touching production code ordering (per the architecture assessment's suggested sequencing: "先移测试再拆 revival 流水线").

Acceptance criteria:
- Inline tests are extracted to a dedicated test module/file first, as an isolated, low-risk, behavior-preserving commit, before any production-code splitting begins.
- Provider-specific knowledge currently leaked into `master_watch.rs` (Claude transcript format parsing, `CLAUDE_CONFIG_DIR` semantics) is relocated to a provider-specific module as part of the split — monitoring code should not need to know Claude-specific config-directory semantics to do its job.
- The revival/cutover pipeline (named as one of the 13 responsibilities) is a natural first extraction target after tests move, per the architecture assessment's ordering — design.md should name the extraction order for the remaining responsibilities, but full decomposition into N files is not required to close this requirement; a demonstrated first extraction (tests + one responsibility) is sufficient to consider D6 "started correctly," with the rest tracked as follow-on work.

Testability: `--lib` (test extraction is self-verifying: the same tests must still pass from their new location).

## Requirement D7: Perception Arbiter Collaboration Boundary

Define, once, how this module's job state machine consumes verdict events from `ah-perception-arbiter` — so neither spec silently assumes ownership of the seam between "agent physical state" and "job business state."

Acceptance criteria:
- Job state machine (D1) never writes `agents.state`; perception arbiter never writes `jobs.status`. This is a hard boundary, not a convention — see design.md for the enforcement mechanism.
- The event shape the job state machine consumes from the perception arbiter (verdict events: agent finished a turn, agent stalled, etc.) is named explicitly in design.md, cross-referenced against `ah-perception-arbiter/design.md`'s event model so the two specs don't independently invent incompatible event shapes.

Testability: `--lib`, integration-style test spanning both modules' fakes (fake perception event in, assert job state machine reacts correctly without touching `agents.state` itself).
