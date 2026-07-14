# ah Completion Protocol — Tasks (skeleton / outline only, not scheduled)

Status: **framing only, g2-authored.** **Do not begin implementation from this file.** Scheduling is gated on operator亲验 of the merged `design.md` (STAGE-PLAN §四.5); g1-m1/g2-m1 stay IDLE until then. This is a **skeleton** — task groups at module/mechanism granularity, **not** a function-level decomposition. Its job: give the eventual codex/agy implementer handoff a shared task shape, carry every cross-review open item forward (nothing silently dropped), and bind each group to a dogfood node so 代码闭环≠实证闭环 is enforced from day one.

Conventions used below:
- **[Track A]** / **[Track B]** — lineage (which design draft owns the mechanism). **[Joint]** — a shared cross-track contract both lanes claim; **both run acceptance independently** at freeze.
- **[Req: …]** — the requirement in `requirements.md` this discharges.
- **[Dogfood: DF-x]** — the empirical closure node on the Gen-4 open window (`research/dogfood-ledger-2026-07-10.md`; efficacy verdicts via `research/gen-efficacy-reports.md`). Every task group carries one — no gaps.

Dogfood nodes referenced (Gen-4):
- **DF-A1** (R1/A1): `kill -9 ahd` mid-flight then relaunch → outbox cold-scan replays, event stream has no hole; a `job_done` written just before the kill applies **exactly once** post-restart.
- **DF-1** (R2/A2): live agent parks a background task and ends its turn → job stays DISPATCHED, no COMPLETED.
- **DF-2** (R2/A2): `STOPPED_UNDECLARED_ALERT` exactly-once per genuine stop-without-declare; no 1887-in-48h flood.
- **DF-3** (R2/A2): sampling audit — every COMPLETED job's `reply_text` is the declared result, never a brief fragment (kills #33).
- **DF-4** (R3/A3): inject banner/ghost text into a live pane → job lifecycle unchanged, dispatch still fires (kills ah#17).
- **DF-5** (证据闸门): mutating job, empty diff → denied + nudged; 3rd try released + escalated; read-only job never gated.
- **DF-6** (R2/§5): agy 假BUSY watchdog budget catches 哑火 without guillotining long legitimate reasoning turns.
- **DF-A4** (G4/A4): manually delete an agent's Stop hook config (per provider shape) → next startup alarms loudly + auto-repairs + re-check passes before dispatch.

Execution note for whenever this graduates: this spec is too large for one PR. The group boundaries below map to independently-mergeable slices. **Do not implement out of order** — R2/G4 depend on R1's transport existing; 证据闸门 depends on R2's declaration gate-point existing; R3 is design-only until R1/R2 are stable.

---

## Group R1 — Hook Delivery Reliability (foundation; everything else depends on it) · [Track A]

- [ ] **R1-T1** Journal-first outbox write in `ah agent notify` — `.tmp`+`fsync`+`rename()`+dir-`fsync`; durability before any RPC. Pin the invariant **"exit 0 ⇔ durable record exists"**; journal-commit failure ⇒ loud non-zero exit, RPC failure ⇒ exit-0-safe. [Req: CP-R1.1] [Track A] [Dogfood: DF-A1]
- [ ] **R1-T2** Cold-scan replay before serving + error-book quarantine for un-applyable records; `event_id`-ordered replay; reserved-prefix (`selfcheck:`) exempt from ordering. [Req: CP-R1.3] [Track A] [Dogfood: DF-A1]
- [ ] **R1-T3** ACK = durable DB commit; reap-after-commit; sender never blocks. Pin **one** `.tmp`→`.json` naming convention across all three drafts (writer glob == scanner glob — Track A review F-7). [Req: CP-R1.4] [Track A] [Dogfood: DF-A1]
- [ ] **R1-T4** *(open item, Track A review F-6)* Reconcile the G4 `selfcheck:` `event_id` with R1-T2's ordering + dedup ledger: document reserved-prefix ordering-exemption and whether a selfcheck record takes a ledger row (it should, else a crash-surviving selfcheck re-runs the no-op). [Req: CP-R1.3] [Track A] [Dogfood: DF-A4]

## Group R1↔R2/R1↔TrackB — Shared Transport Dedup Ledger · [Joint]

- [ ] **JC-1 (JOINT CONTRACT — both lanes run acceptance)** Single transport-level dedup ledger `outbox_consumed(event_id TEXT PRIMARY KEY)`, `INSERT … ON CONFLICT DO NOTHING` at the consume boundary **before** the `kind` fork, dedup+effect in one tx. Must cover **both** the F2 `events` path (Track A) *and* the F3 `job_transitions` path (`apply_job_done_declaration_sync`, Track B). State the real schema delta — **not** "reuse the `events` table's `event_id`" (that column doesn't exist; folds in Track A review F-5). [Req: CP-R1.2] [Joint — Track A owns transport, Track B routes through it] [Dogfood: DF-A1 (replay no double-apply) + DF-1 (declaration applied once)]
  - *Acceptance run by g1 (Track A): a replayed `events`-path record dedups. Acceptance run by g2 (Track B): a replayed `job_done` cannot double-apply to `job_transitions`. Both must pass at freeze.*

---

## Group R2 — Explicit Completion Protocol · [Track B]

- [ ] **R2-T1** Dispatch injects `AH_JOB_ID` + reused arbiter-Q4 `AH_JOB_ATTEMPT_COOKIE`; `ah job done`/`ah job fail` CLI writing the outbox wire form; CLI refuses id-mismatch loudly; stale-cookie declaration rejected+logged (not applied). [Req: CP-R2.1] [Track B] [Dogfood: DF-1]
- [ ] **R2-T2** *(load-bearing refactor)* Sever F3 from F2: strip `mark_job_completed_conn_sync` from all three idle paths (`state_machine.rs:501`/`:716`/`:885`); add `apply_job_done_declaration_sync` as the sole `DISPATCHED→COMPLETED` writer (`job_fail`→`FAILED`); `reason=explicit_done|explicit_fail`, `reply_source=protocol`. [Req: CP-R2.2] [Track B] [Dogfood: DF-1]
- [ ] **R2-T3** *(net-new — see Verified Baseline)* Build the Claude Stop-hook enforcement layer: `{"decision":"block","reason":…}` when undeclared & block-count<N; at ≥N allow + emit `STOPPED_UNDECLARED`; hook never writes state. **This capability does not exist today** — do not implement against an "already enforced" assumption. [Req: CP-R2.3] [Track B] [Dogfood: DF-1, DF-2]
- [ ] **R2-T4** Demote detectors to a single reconcile watchdog: `STOPPED_UNDECLARED_ALERT` edge-triggered + deduped by `(agent, job, dispatch_epoch)`; retire `classify_terminality`'s `Terminal`-for-completion role (→ F2 + watchdog-hint only). Anti-flood is the explicit target. [Req: CP-R2.4] [Track B] [Dogfood: DF-2, DF-6]
- [ ] **R2-T5** Reply-payload attribution: reply rides inside the declaration (`reply_source=protocol`); **delete** `collect_reply_for_dispatched_job_sync` + the `"screen"` source. [Req: CP-R2.5] [Track B] [Dogfood: DF-3, DF-4]
  - [ ] **R2-T5a** *(open item, g1 NB-3)* Fix the §6.3 deletion anchor list to enumerate **all three** `collect_reply` sites (`:398` matched, `:650` hook, `:819` log) + **both** `"screen"` defaults (`:702`, `:871`); note `:398`'s entanglement with `is_prompt_only_reply`/`classify_terminality` (they fall away with the declaration model — say so, don't leave implicit). Prevents a live scrape surviving in the log path if the removal PR follows the table literally. [Req: CP-R2.5] [Track B] [Dogfood: DF-3]
- [ ] **R2-T6** *(open item, g1 NB-1)* Confirm the outbox has no per-record size bound for an inlined `reply_text` (a `--reply-file` verdict can be multi-KB); if it does, carry the reply as a content-addressed file-ref instead of inline. Resolve in the merged design before coding. [Req: CP-R2.1/CP-R2.5] [Track B ↔ Track A seam] [Dogfood: DF-3]
- [ ] **R2-T7** *(open item, g1 NB-2 — net-new stateful component, NOT a tuning knob)* Specify the durable, dispatch-epoch-scoped, **reset-on-redispatch** integer counter backing both the Stop-hook block-count (R2-T3) and the evidence deny-count (EG-T2): where it lives, its keying, its reset semantics. `stop_hook_active` (bool) and `has_completion_deferred_event` (hash-dedup) do **not** provide it. [Req: CP-R2.3, CP-EG.2] [Track B] [Dogfood: DF-2, DF-5]
- [ ] **R2-T8** *(open item, Track B §7)* Finalize N (block count, seeded N=2) and per-provider watchdog budgets (agy floor = `MAX_LOG_MONITOR_WAIT`=900s, `monitor.rs:10`, reused) against live data — tuning values, not blind hardcode. [Req: CP-R2.3, CP-R2.4] [Track B] [Dogfood: DF-2, DF-6]

---

## Group G4 — Control-Path Self-Check · [Track A]

- [ ] **G4-T1** Three-tier startup check: light (blocking-with-repair, per-agent dispatch-readiness, **not** ahd boot) / medium (non-blocking synthetic round-trip) / deep (`ah doctor`, dead-letter census + wiring matrix). [Req: CP-G4.1] [Track A] [Dogfood: DF-A4]
- [ ] **G4-T2** Drift/delete detection scoped to ah-owned entries only (`is_ah_owned_hook_item`), **not** whole-config `compute_config_hash` — dissolves the benign-edit permanent-block contradiction (Track A rev-2 F-3). [Req: CP-G4.2] [Track A] [Dogfood: DF-A4]
- [ ] **G4-T4** Loud degradation + idempotent merge-preserving auto-repair + mandatory re-check-before-dispatch-eligible. [Req: CP-G4.4] [Track A] [Dogfood: DF-A4]

## Group G4↔R2/G4↔TrackB — Multi-Provider Self-Check Coverage · [Joint]

- [ ] **JC-2 (JOINT CONTRACT — both lanes run acceptance)** G4 enumerates all three config shapes as first-class targets: claude `settings.json` `hooks.Stop[]`, agy `hooks.json` (`ah-completion-push`, matcher `""`), codex `hooks.json` (`hooks`, matcher `"*"`), **plus codex `features.hooks=true`** gate (`:1163`); the synthetic round-trip exercises **each** provider's own installed hook, not the claude shape as stand-in (Track A rev-2 F-4). Track B §5.7's MA-2 enforcement hook depends on this — a per-provider silent completion loss is indistinguishable from 假完成. [Req: CP-G4.3] [Joint — Track A builds, Track B depends] [Dogfood: DF-A4 (run per provider shape)]
  - *Acceptance run by g1 (Track A): delete each provider's ah-owned Stop entry → each is detected+repaired. Acceptance run by g2 (Track B): the MA-2 claude block hook's presence is asserted by this check, and a drifted codex/agy hook is caught. Both must pass at freeze.*

---

## Group 证据闸门 — Physical-Evidence Gate · [Track B]

- [ ] **EG-T1** Deny-only job-level admission gate on the declaration: static `requires_physical_evidence` (`schema.rs:168`) → read-only never gated; edge-triggered at declaration; scoped to the agent's own sandbox worktree; no T-level, never completes, never enters the arbiter FSM. [Req: CP-EG.1] [Track B] [Dogfood: DF-5]
- [ ] **EG-T2** Bounded interception → 3rd declaration released as `COMPLETED_EVIDENCE_WAIVED` + mandatory `EVIDENCE_GATE_ESCALATION`; deny-count is the R2-T7 durable reset-on-redispatch counter (not a knob). [Req: CP-EG.2] [Track B] [Dogfood: DF-5]
- [ ] **EG-T3** *(open item, g1 NB-4a — highest build-the-wrong-thing risk)* Reconcile §4's live-`git diff` with the existing evidence-**event** model (`has_job_evidence_sync` over `mtime_changed`/`diff_generated`/`test_passed`, `state_machine.rs:1004-1027`): decide *produce-events* vs *replace-model*; do not leave two mechanisms unreconciled. Address the untouched `requires_test_evidence` dimension (`:1019-1025`). Resolve before coding. [Req: CP-EG.3] [Track B] [Dogfood: DF-5]
- [ ] **EG-T4** *(open item, g1 NB-4c)* Count untracked-but-non-ignored new files as evidence (`git status --porcelain` minus ignored, or `git add -A && git diff --cached`) so new-module/new-test jobs aren't spuriously denied → 3rd-try-waived. [Req: CP-EG.3] [Track B] [Dogfood: DF-5]
- [ ] **EG-T5** *(open item, g1 NB-4b)* Name mis-tag-as-read-only (`requires_physical_evidence=0` on a mutating job) as an accepted residual **or** add a cheap detector (completed read-only job with a dirty worktree = telemetry, not a gate). [Req: CP-EG.3] [Track B] [Dogfood: DF-5]
- [ ] **EG-T6** *(open item, g1 §7-h + convergence C5/git)* Harden the 产物轨 against git-plumbing failure modes — index-lock contention, detached HEAD, amended/squashed commits, submodules, git-not-on-PATH — **and** the Windows-native platform-conditional degradation. Include untracked-new-files (EG-T4) as a first-class failure mode, not Windows-only. [Req: CP-EG.3] [Track B] [Dogfood: DF-5]
- [ ] **EG-T7** *(open item, Track B §7 / g1 §7-c)* `COMPLETED_EVIDENCE_WAIVED`: flag/`reason` on COMPLETED (recommended — low blast radius) vs a distinct terminal status (ripples into every `status=='COMPLETED'` consumer). Either way escalation stays mandatory. [Req: CP-EG.2] [Track B] [Dogfood: DF-5]

---

## Group R3 — Pane-Lifecycle-Inference Removal · [Track B] · **DESIGN ONLY THIS STAGE, NOT IN IMPLEMENTATION SCHEDULE**

> **Iron rule (north-star §四 / STAGE-PLAN):** R3 removal PRs are **not** scheduled until R1/R2 are stable — "拆早了没有替代信号." This stage produces the plan + coverage table only; **no deletion is authorized.** The tasks below are the *design deliverables*, and (tracked-not-authored) the future removal-PR scope.

- [ ] **R3-T1** *(design)* Removal-preconditions checklist (R1 stable/A1 · R2 stable ≥2 gens, 假完成→0 · every §6.2 substitute green in dogfood · G4 live/A4) + the site→successor→tier→precondition coverage table. Keep the trust/update dialog (T3's one legitimate job). [Req: CP-R3.1] [Track B] [Dogfood: DF-4 (successor-covers-dispatch-readiness, run *before* any future deletion)]
- [ ] **R3-T2** *(design, tracked-not-authored)* Enumerate the eventual removal-PR scope: `collect_reply_for_dispatched_job_sync` + `"screen"`; the dispatch-readiness pane-diff gate (ah#17, second surviving pane-content-inference site the arbiter cross-references but doesn't own); `classify_terminality`'s `Terminal`-for-completion authority (→ turn-boundary + `DeferredBackgroundWork` hint only). [Req: CP-R3.1] [Track B] [Dogfood: DF-4]
- [ ] **R3-T3** *(open item, g1 §0 reinforcing find)* Add `classify_terminality`'s antigravity-only natural-language heuristic (`parser.rs:48-133`, English+Chinese phrase lists + regexes at `:69-129`) to the R3 removal-target list — it is itself reply-text inference driving completion, only obliquely covered in §6. [Req: CP-R3.1] [Track B] [Dogfood: DF-4]

---

## Group Provider-Matrix — Enforcement Tiering & Verification · [Track B]

- [ ] **PM-T1** One provider-agnostic declaration wire form + capability-tiered (not identity-tiered, not LCD) enforcement; `job_done`/`job_fail` as a **union** schema (never intersection); no fake synthesized block / PTY polling-block proxy for agy (convergence C12). [Req: CP-PM.1] [Track B] [Dogfood: DF-6]
- [ ] **PM-T2** *(highest-leverage open item, convergence §5.0 / g1 §7-f)* Verify whether agy/codex `hooks.json` `"Stop"` honors `{"decision":"block"}` — via **doc/source research** or an **isolated sacrificial e2e** (independent STATE_DIR + tmux socket + trap cleanup); **never** by injecting a block into a live working agent or DSR/PTY-probing the live stack. Fail-closed observe-only until answered; a positive verdict migrates agy/codex into the block tier. *(Executable by the e2e-gatekeeper lane on request.)* [Req: CP-PM.2] [Track B] [Dogfood: DF-6]
- [ ] **PM-T3** *(open item, Track B §7 / g1 §7-b)* codex `log_structured` bridge hardening — the one tolerated log-derived declaration path: (i) sentinel is a machine token the codex **adapter** recognizes (never a generic scraper); (ii) it carries the `attempt_cookie` and passes the MA-1 epoch check (not a cookie-bypass backdoor); (iii) **fail-closed** — sentinel absent ⇒ job does NOT complete, never falls back to scrape. [Req: CP-PM.1] [Track B] [Dogfood: DF-6]
- [ ] **PM-T4** *(open item, g1 §7-i)* Coalesced-turn attribution: a provider that coalesces two dispatches into one turn may emit one completion signal for two jobs — the per-attempt cookie is necessary but the coalescing behavior itself must be tested per provider. [Req: CP-PM.1] [Track B] [Dogfood: DF-6]

---

## Group Cross-Cutting — Seams, Metrics, Test-Pinning

- [ ] **XC-T1** *(§五 fifth clause / CP-X.1)* Ensure A1–A4 each have ≥1 automated regression test: A1↔R1-T2 (kill-9 replay), A2↔R2-T2+R2-T4, A3↔R2-T5+R3-T1, A4↔G4-T4. A passing `--lib` test never alone marks a requirement empirically closed — the paired DF node does. [Req: CP-X.1] [Both] [Dogfood: DF-A1/DF-1/DF-4/DF-A4]
- [ ] **XC-T2** *(open item, g1 §7-j / convergence E6)* Per-provider 假完成 fidelity metric so "did the protocol fix agy specifically" is falsifiable — tie DF-1/DF-2/DF-3 to a per-provider breakdown in `research/gen-efficacy-reports.md`. [Req: CP-X.2] [Both] [Dogfood: DF-1, DF-2, DF-3]
- [ ] **XC-T3** *(open item, g1 §7-e — non-blocking for merge, BLOCKING for implementation)* Arbiter-Q2 seam: the arbiter's narrow `Failed→Completed` late-evidence reopen and this module's evidence gate describe overlapping `Failed↔Completed` races from opposite sides. The merged `design.md` needs an explicit **edge-ownership table** (arbiter: perception-driven reopen for late evidence; this module: declaration-driven `DISPATCHED→COMPLETED/FAILED` + evidence-waived). Neither spec may silently own it. [Req: 证据闸门 ↔ ah-control-plane-refactor] [Joint — cross-module] [Dogfood: DF-5]
- [ ] **XC-T4** *(open item, g1 §7-g / convergence D5)* Semantic contract-drift self-check: G4 must assert the hook still *behaves* (blocks/fires as expected), not only that it is *wired* — an external harness silently changing its completion semantics would surface as a new 假完成 wave with no code change on our side (repo already has `.SUPERSEDED` churn). Extends JC-2. [Req: CP-G4.3] [Track A ← Track B surfaced] [Dogfood: DF-A4]
- [ ] **XC-T5** *(open item, Track B §7 / g1 §5(d))* Pin the exact `job_transitions.reason` value set (`explicit_done`/`explicit_fail`/`evidence_waived`) against `ah-job-events`'s existing reason convention — do **not** fork a new pattern. [Req: CP-R2.2] [Track B ↔ ah-job-events] [Dogfood: DF-1]
- [ ] **XC-T6** *(open item, Track B §5.7 / g1 E1 — flagged, not solved here)* Master-self-completion (裸等唤醒) is a fourth completion class — `handle_master_notify:978` flips BUSY/IDLE on `stop`, no block, and master's "done" is an orchestration predicate. This module covers **worker** completion only; the master seam is owned elsewhere, named here so neither spec silently owns it. [Req: Scope/out-of-scope] [Flag only] [Dogfood: n/a — out of this module]
