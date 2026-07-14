# Orchestration-Reliability Implementation Phasing (Rollout Plan)

> Master-authored, 2026-07-09. Turns the frozen spec (`.kiro/specs/ah-orchestration-reliability/` design.md + tasks.md; north star `research/perception-layer-first-principles.md`) into a dependency-ordered, audited rollout. This is a **post-v1.4.0** workstream (not release-blocking). Implementation = **a1-antigravity** (per-task TDD), every task through **a4-claude audit**; codex frozen until 7/11.

## The hard safety invariant (non-negotiable, drives the whole order)
**No stop-class / reap operation may exist in an enabled code path before the D1 ownership gate is implemented and green.** This is the direct lesson of the 6 live-stack kills. Concretely:
1. D1 (provenance + identity + anti-recycling) lands and is `--lib`-proven BEFORE any D2 reaper is wired.
2. The continuous reconcile loop (Phase 5) is authored **disabled / read-only** and is only enabled (Phase 11) after D1+D2 are green. No PR may leave a reaper reachable without the full D1 gate in front of it.
3. Every reconciler state write goes through the state-version CAS helper from its first appearance (Phase 5 onward), not retrofitted at Phase 9.

## Rollout sequence (phase → PR unit → dependency / risk / audit focus)

| PR | Phases | Depends on | Risk | a4 audit focus |
|----|--------|-----------|------|----------------|
| **P-0** Harness | Phase 0 (fake sensors/reaper traits, process birth/start metadata provider) | PR4 (merged) | low — no behavior change | that the fakes don't leak into production paths; `--lib` only |
| **P-1** D1 gate core ⚠️ | Phase 1 (provenance gate + exact Linux marker identity + registry identity) | P-0 | **safety-critical** | each RED truly enforces "ambient/foreign → ZERO stop/kill/teardown"; test-parallel-safety (no unserialized global env); no reaper enabled yet |
| **P-2** spawned_at | Phase 2 (additive migration + persist on spawn) | P-1 | med (schema) | migration is additive/idempotent; captured before dispatchability; ties to the P0#4 DB-migration-atomicity concern — coordinate |
| **P-3** D1 anti-recycling ⚠️ | Phase 3 (helper + DB `spawned_at` integration) | P-2 (needs the column) | **safety-critical** | fail-closed: NULL/incompatible `spawned_at` → refuse EVERY destructive path; the 3-layer gate is now complete |
| **P-4** D2 reaping ⚠️ | Phase 4 (Linux scope-stop into cleanup; pidfd crash → whole-tree; Unix setpgid + group kill) | **P-1..P-3 complete** (invariant) | **safety-critical** | prove reaper is unreachable when D1 rejects; this is the first PR that can actually stop things — heaviest audit |
| **P-5** Reconcile core | Phase 5 (`reconcile_active_agents_once`, loop **kept disabled**) + CAS helper from Phase 9 | P-4 | med | loop not enabled in production; all writes via CAS; startup reconcile preserved |
| **P-6** D3 completion | Phase 6 (classifier; evidence-gated scanner; artifact-less optimistic + quiet watchdog) | P-5 | med | freshness dependency (P-8) — scanner completion must not ship before Phase-8 freshness or it can false-complete; sequence P-8 before enabling evidence-gated auto-complete |
| **P-7** D4 corroboration ⚠️ | Phase 7 (T3 inert w/o T0+T2; corroborated writes only) | P-5 | **safety-critical** | every ghost-text RED enforced; a T3 hint alone can never write PROMPT_PENDING/STUCK |
| **P-8** FS freshness ⚠️ | Phase 8 (strict `T_evidence > T_dispatch`; same-second → DB `inserted_at`; clock-skew ε_drift) | P-5; gates P-6's evidence path | **safety-critical** (premature-completion guard) | stale-pre-dispatch-scanned-late → NOT fresh; volatile scanner time never authoritative |
| **P-9** CAS contract | Phase 9 (conflict regression + non-CAS write audit) | P-5 | low-med | catches any non-CAS reconciler write |
| **P-10** Telemetry | Phase 10 (reconcile decision telemetry; runtime surface) | P-5 | low | rides existing runtime-events spine; `ah ps` not required |
| **P-11** Enable + CI gates | Phase 11 (enable the loop; Linux/macOS reap, dropped-hook, ghost-pane integration) | **all above green** | high (goes live) | the isolated-runner integration gate; this is where the loop is first enabled in a real stack |

## Sequencing notes
- **Recommended P-6/P-8 swap:** implement **P-8 (freshness) before enabling P-6's evidence-gated auto-completion** — evidence-gated completion without the freshness guard reintroduces the premature-completion class. Author P-6's classifier/watchdog freely, but gate the auto-complete on P-8.
- **CAS is not "Phase 9 later"** — the helper must exist when the reconciler first writes (P-5). Phase 9 is the *audit + regression* that nothing bypasses it.
- **P-2 ↔ P0#4:** the `spawned_at` migration touches `db/mod.rs` migrations — coordinate with the v1 P0#4 DB-migration-atomicity fix (transaction envelope + `schema_migrations`) so they land coherently, not as competing migration rewrites.
- **`--lib` vs CI-integration:** ~everything through P-10 is `--lib`-testable (fakes/mocks) → safe on the dev box under the iron rule. Phase 11 is CI-integration-only (real daemons/scopes) → the isolated GitHub runner, never the live stack.

## Suggested entry point
Start with **P-0** (pure harness, zero behavior change) as a1-antigravity's first *real* orchestration-reliability PR after the pilot converges — low risk, establishes the fake-sensor/reaper injection seams the rest of the plan tests against, and lets the a1→a4 audit loop settle on something safe before the safety-critical P-1.

## Open coordination items for the operator
- Confirm this is post-v1 (v1.4.0 already shipped) — no rush; correctness over speed.
- P-2/P0#4 migration coordination (who lands the migration-atomicity substrate first).
- a1-antigravity throughput: given the false-completion/background-task trait, expect one PR at a time with a4 audit between — the plan is deliberately small-increment for that reason.
