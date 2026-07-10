# ah Perception Arbiter — Tasks (outline only, not scheduled)

Status: framing only. **Do not begin implementation from this file.** Scheduling is gated on module A/B closeout + 换血#2 per `research/orchestration-plan-2026-07-10.md` §五 node 3/4. This file exists so a3's adversarial review and eventual codex/implementer handoff have a shared task shape — it is not a sprint plan. gap-patched 2026-07-10 (operator acceptance round) — see the added Cross-Cutting item below (second pane-diff site).

Execution note for whenever this graduates to implementation: this spec is large enough that it should NOT be one PR. Suggested phase boundaries below map to independently-mergeable slices; do not implement phases out of order — each depends on the write-gate existing before producers can be migrated onto it.

## Phase 0: PoC (can run standalone, ahead of the rest)

- [ ] C8 sibling-cgroup PoC (requirements.md C8) — **revised 2026-07-10: host-side-only mechanism, see design.md Q3 correction**. The PoC must validate that `ahd`'s host-side spawn wrapper — not code running inside the sandbox — creates both sibling scopes and places spawned child PIDs into `workload.scope/cgroup.procs`. Do not build or test a version where the sandboxed CLI itself writes to any cgroup path (sandbox-escape risk, a3-flagged). If the PoC finds the host-side wrapper cannot observe child-process forks happening inside the sandbox, that's a valid PoC failure — escalate to design reconsideration, don't fall back to sandbox-internal writes.
  - Testability: CI-integration (real cgroup v2 + systemd-run required, not `--lib`).

## Phase 1: Write Gate + Event Channel (foundation, everything else depends on this)

- [ ] C1: `crate::db::perception::gate` module with the single state-write entry point. No producer migration yet — this phase only builds the gate and proves it compiles/works against a synthetic caller.
- [ ] C2: `perception_events` channel (schema decision — new table vs. typed `events` convention — must be made in design refinement before this task starts, not during it).
- [ ] CI grep/lint rule banning direct `UPDATE agents SET state` outside the gate module.

## Phase 2: Producer Migration (one PR per producer, do not batch)

- [ ] Migrate `marker/timer.rs:113` (BUSY 3h fallback) off direct `mark_agent_stuck` onto event emission.
- [ ] Migrate `provider/health_check.rs:133` (`escalate_health_stuck`) off direct `mark_agent_stuck` onto event emission. This is the migration that directly fixes the dual-reason `PANE_DIFF_STUCK`/`HEALTH_CHECK_STUCK` bug (requirements.md Existing Grounding) — treat this one's regression test as the module's headline acceptance evidence.
- [ ] Migrate hook-event and log-event completion paths (`db/state_machine.rs` `mark_agent_idle_hook_event*`, `mark_agent_idle_log_event*`) onto event emission.
- [ ] Delete now-dead direct-write call sites once all producers are migrated (do not leave both paths live "just in case" — that reintroduces the dual-writer bug this module exists to kill).

## Phase 3: Tri-State + Budgets + Epoch (arbiter logic itself)

- [ ] C3: tri-state verdict model, explicit `Unknown` emission on first-observation-without-judgment.
- [ ] C4: per-layer Unknown budgets (OS 30s / Log reuse `MAX_LOG_MONITOR_WAIT` / Hook 2s) wired into arbiter consumption loop.
- [ ] C5: epoch/generation versioning, stale-event rejection.
- [ ] C6: `Stalled` explicit-signal semantics + accurate `reason` attribution; regression coverage for genuine-liveness-dead still transitioning to Stalled.

## Phase 4: Hook Attribution (can start in parallel with Phase 3 once Phase 1 lands)

- [ ] **First sub-task, blocking the rest of this phase** (2026-07-10, a3-flagged gap): audit dispatch path for an existing per-attempt identifier; if none fit-for-purpose exists, mint and pin the exact env var name/format (e.g. `AH_JOB_ATTEMPT_COOKIE`) in writing before either side of C7 is implemented. Both the sandbox-side hook CLI and host-side consumer build against this pinned contract, not independent guesses.
- [ ] C7: outbox-pattern durable hook write + daemon-side consumption + cold-scan-on-restart recovery, keyed by the pinned attribution identifier above.

## Cross-Cutting, Not Phase-Bound

- [ ] Regression suite proving pane-text is never reintroduced as a completion/stall signal anywhere in the migrated producers (guard against silent regression of P0-1's already-settled decision).
- [ ] Dogfood ledger hook: once merged and live, this module's headline metric is "does `error_reason` on a stalled job ever again mismatch the layer that actually detected the stall" — operator to track post-merge, not a task here but flagged so it isn't lost at handoff.
- [ ] **Second pane-diff deletion target, out of this module's phases but not dropped** (added 2026-07-10, gap-patch; requirements.md Existing Grounding has the incident). The daemon's dispatch-readiness recheck (pre-send pane-diff gate) is a second surviving pane-content-inference site, same "no pane-content inference" family this module exists to kill, but implemented in the dispatch path rather than `agents.state` writes — so it's not one of this spec's phases. Whenever that subsystem is scheduled (design track, not this module), replace the raw pane-diff check with an explicit readiness signal (e.g. a last-injected-text watermark or an ack-based check) instead of comparing pane snapshots. Flagged here so it isn't re-discovered as a surprise later.
