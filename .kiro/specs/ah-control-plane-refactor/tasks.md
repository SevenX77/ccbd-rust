# ah Control Plane Refactor — Tasks (outline only, not scheduled)

Status: framing only. **Do not begin implementation from this file.** Gated on module A/B closeout + 换血#2, and on `ah-perception-arbiter` Phase 1 (write gate) landing first for D2/D7's sake — see dependency note below.

## Dependency Note

D2 (physical/business split) and D7 (collaboration boundary) assume `ah-perception-arbiter`'s Phase 1 (write gate + event channel) already exists, since D2's Phase 1 write becomes a perception-arbiter-gated producer call. **Sequence this spec's D1/D3/D4/D6 work independently of the arbiter (no dependency)**, but hold D2/D7 until the arbiter's Phase 1 lands. Do not implement D2 against the current direct-write `mark_agent_idle_*` functions and then re-migrate later — that's double work.

## Phase 1: Job State Machine Gate (D1) — independent, can start anytime

- [ ] `JobStatus` enum + transition table + `transit_job_state` gate function.
- [ ] Migrate all current scattered `UPDATE jobs SET status` call sites (re-grep at implementation time, do not trust the Existing Grounding count blindly) onto the gate.
- [ ] CI grep rule banning direct `UPDATE jobs SET status` outside the gate.

## Phase 2: Kill/Teardown Unification (D3) — independent, sequenced after module B's B2

- [ ] Confirm module B's B2 (tmux cleanup fallback) has merged; rebase onto it rather than reimplementing.
- [ ] Unified teardown function with canonical order (authorize -> kill -> tmux teardown -> sandbox cleanup -> event).
- [ ] Migrate all four call sites (`agent.rs`, `sessions.rs`, `orchestrator/mod.rs`, `master_watch.rs`->`system.rs`) onto it.

## Phase 3: `spawn_realign_agent` Relocation (D4) — independent

- [ ] Move function out of `rpc/handlers/realign.rs` into an orchestration-owned module.
- [ ] Verify orchestrator/master_watch no longer import `rpc` for this path (module-graph / `cargo check` sanity).
- [ ] Re-point any RPC-entry-point caller to the relocated function.

## Phase 4: F3/F2 Decoupling (D2) — depends on `ah-perception-arbiter` Phase 1 — revised 2026-07-10 (VERIFYING state added, see design.md)

- [ ] Add `VERIFYING` and `FAILED_VERIFICATION` to the `agents.state` vocabulary; audit and update all dispatch-candidate queries to exclude `VERIFYING` (not just the obvious one — architecture assessment notes dispatch logic is scattered, check for more than one query).
- [ ] Convert `mark_agent_idle_*` family to transition the agent to `VERIFYING` (through the perception arbiter's gate, not directly) and emit `JobExecutionFinished`-class events, instead of writing `jobs.status` or `agents.state = 'IDLE'` directly (coordinate with arbiter's producer migration — this may literally be the same commit as one of the arbiter's Phase 2 producer migrations, not a separate one).
- [ ] Orchestrator-owned Phase-2 consumer: evidence check against the still-`VERIFYING` (untouched) workspace + `transit_job_state` call + `VERIFYING -> IDLE` (success) or `VERIFYING -> FAILED_VERIFICATION` (failure, workspace preserved) transition.
- [ ] `Failed -> Completed` guarded reconciliation transition (late-evidence case, modeled on the existing `late_health_completion_stuck_allows_terminal` accept-gate).
- [ ] Explicit operator/master recovery path out of `FAILED_VERIFICATION` (no auto-transition back to `IDLE`).
- [ ] Crash-recovery re-discovery pass for unconsumed `JobExecutionFinished` events AND agents stuck in `VERIFYING` past a reasonable bound, on startup reconcile.
- [ ] Verify the two existing evidence-gate guardrails (`is_mutating` static labeling, 2-nudge-then-escalate cap, per `perception-final-convergence-2026-07-09.md` §2.4) are still exercised through the relocated Phase-2 call path, not silently dropped by the relocation.

## Phase 5: `master_watch.rs` Decomposition (D6) — independent, low-risk-first ordering

- [ ] Step 1: extract ~3300 inline test lines to sibling test module, zero behavior change.
- [ ] Step 2: extract provider-specific knowledge (Claude transcript parsing, `CLAUDE_CONFIG_DIR`) to a provider module.
- [ ] Step 3 (name only, not required to close this round): revival/cutover pipeline extraction, tracked as follow-on.

## Not Scheduled This Round

- [ ] D5 (`db/` domain/application/repository split) — directional target named in design.md, no tasks here. Revisit after Phases 1-5 land, since D1-D4/D6 will otherwise move code in directions that could make D5 harder if done blind to the target boundary.

## Cross-Cutting

- [ ] D7 collaboration-boundary integration test spanning both specs' fakes, once both Phase 1s (arbiter's write gate, this spec's D1 gate) exist.
- [ ] Explicit reconciliation checkpoint between whoever implements `ah-perception-arbiter` C2/C6 and whoever implements this spec's D2/D7, to agree on the actual verdict-event shape before either side hardcodes assumptions about it (design.md flags this as unresolved-by-design, not an oversight).
