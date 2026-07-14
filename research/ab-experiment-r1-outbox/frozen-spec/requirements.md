# ah Completion Protocol — Requirements

Status: **skeleton, g2-authored (泳道2 闸门执笔)**, produced alongside `tasks.md` during the 2026-07-11 design round. Both Track A (g1: R1 outbox/ACK/replay + G4 control-path self-check) and Track B (g2: R2 explicit protocol + physical-evidence gate + R3 teardown) have been cross-reviewed and bilaterally ACCEPTed (`review-track-a-by-g2.md` rev-2 all-clear; `review-track-b-by-g1.md` ACCEPT-for-merge). g1 is merging the two drafts into the formal `design.md`; **that file is the authority for mechanism** — this file states the *verifiable requirements* those mechanisms must satisfy, and `tasks.md` states the task shape. **Not yet cleared for implementation** — awaiting operator亲验 of the merged `design.md` per STAGE-PLAN §四.5 (g1-m1/g2-m1 stay IDLE until then); `tasks.md` is outline-only, no scheduling.

Source material (do not re-litigate; cite instead of re-deriving):
- `research/perception-layer-first-principles.md` — north star. §五 four acceptance definitions (A1–A4) + the fifth clause ("以上四条全部有自动化测试钉死") are the root each requirement below reverse-derives from. §三 gap table (G1/G2/G3/G4) and §四 R-ordering (R1→R5) are the grouping spine.
- `design-draft-track-a-g1.md` (rev-2) — R1 (R1-Q1..Q4) + G4 (G4-Q1..Q6) mechanism, code-grounded.
- `design-draft-track-b-g2.md` — R2 (MA-1..MA-4) + §4 physical-evidence gate + §5 provider matrix + §6 R3 teardown, code-grounded.
- `convergence-provider-matrix.md` — the double-blind reconciliation (o1 + g1 divergence drafts) whose §1 **overturned the brief's premise** (see Verified Baseline below). This correction is load-bearing and every provider-facing requirement is re-anchored on it.
- `review-track-a-by-g2.md` / `review-track-b-by-g1.md` — the two cross-lane reviews. Their non-blocking open items are carried forward as tasks (`tasks.md`), not dropped.

## Scope

In scope (five gap areas, mapped to north-star §三/§四):
- **R1** (G1) — hook delivery reliability: journal-first outbox, at-least-once + transport dedup, cold-scan replay. *(Track A)*
- **R2** (G2) — explicit completion protocol: dispatch job-identity + worker declaration, Claude Stop-hook enforcement, detection→watchdog demotion, reply-payload attribution. *(Track B)*
- **证据闸门** (convergence 2.4②) — job-level physical-evidence admission gate, deny-only. *(Track B)*
- **G4** — control-path self-check: three-tier startup check, ah-owned drift detection, multi-provider coverage, loud degradation + auto-repair. *(Track A)*
- **R3** (G3) — pane-lifecycle-inference removal: substitute-signal coverage table + preconditions. **Design only this stage** — no deletion PR is authorized until R1/R2 are stable (north-star §四 note). *(Track B)*

Out of scope (explicitly deferred, do not fold in):
- `agents.state` single-write arbitration — owned by `ah-perception-arbiter` (this module writes `jobs.status`, never `agents.state`).
- The `job_transitions` carrier schema — owned by `ah-job-events`; this module adds `reason` values on the existing spine, mints no second channel.
- Master-self-completion (裸等唤醒) — master is a fourth completion class (Track B §5.7 / g1 E1); flagged here so neither spec silently owns the seam, not designed here.
- Control-plane recovery / late-evidence `Failed→Completed` reopen — owned by `ah-control-plane-refactor` (the seam with this module's evidence gate is a named joint contract, not resolved here — see the arbiter-Q2 seam in `tasks.md`).
- Telemetry (R5 / G5) — separate spec, rides the same spine later.

## Verified Baseline — the premise correction (MUST read before any provider-facing requirement)

**The brief's framing "claude 有 Stop-hook 抓手、agy 没有" is NOT true in the current code.** The double-blind convergence pass re-verified this line-by-line against `src/` (`convergence-provider-matrix.md` §1; independently re-confirmed by g1 in `review-track-b-by-g1.md` §0):

- `CompletionSignalKind` has a **single** variant `LogOnly` (`src/provider/manifest.rs:29-32`), assigned to **every** provider — bash/codex/claude/antigravity (`:351/:381/:400/:418`).
- **All three real providers wire the *identical* `Stop`→`ah agent notify --event stop` hook today** — claude (`home_layout.rs:232`), antigravity (`:420`), codex (`merge_codex_hook_push:1181`).
- That path is **observe-only**: it marks the agent idle and the CLI emits a hardcoded `"{}\n"` = "allow stop" (`src/bin/ah.rs:562-564`). A grep of all `src/` finds **no** `{"decision":"block"}` for any provider — **not for claude either** (test-pinned at `ah.rs:1863-1870`).

**Consequence for downstream implementers (do not miss this):** the real axis is not "has a Stop *event*" (all three do) but "does the Stop event carry an enforceable *block* verdict" — and **currently NONE of them does.** Therefore:
- **claude's block layer is BUILDABLE, not existing.** Requirement CP-R2.3 *builds* it; do not implement against an assumption that claude already enforces. Treating the block layer as pre-existing is the single most likely way to build the wrong thing.
- **agy's and codex's block-honor is UNVERIFIED** — a first-class open item (CP-PM.2), fail-closed assumption = observe-only until proven.
- Enforcement is therefore **capability-tiered, not identity-tiered** (CP-PM.1): the declaration *wire form* is provider-agnostic; only *enforcement strength* varies, by what each provider *provably* supports.

---

## Group R1 — Hook Delivery Reliability *(Track A; serves A1)*

### CP-R1.1: Durability Begins Before Any Socket Attempt (journal-first)
`ah agent notify` MUST make the report durable — atomic write to a per-agent host-visible outbox (`.tmp` + `fsync` + `rename()` + directory `fsync`) — **before** any RPC/socket call. The RPC is a demoted fast-path optimization, not the durability point.

Acceptance criteria:
- The invariant **"exit 0 ⇔ a durable outbox record exists"** holds. RPC failure ⇒ hook still exits 0 (durability already achieved). Journal/fsync/rename failure (`ENOSPC`/`EIO`/`EROFS`/read-only outbox) ⇒ **loud `tracing::error!` + non-zero exit**, never a silent exit-0. *(Track A rev-2 F-1 — resolved; this criterion pins it.)*
- Baseline contrast: today the hook exits non-zero on RPC failure (`ah.rs:547-557`); this requirement flips *RPC* failure to exit-0-safe **without** letting a *journal* failure exit 0.

TDD RED→GREEN: RED = a fault-injected journal write (simulated `rename` failure) that today would exit 0; GREEN = it exits non-zero and logs loudly.
Testability: `--lib` for the exit-code/branch logic; real-FS-full is CI-integration.

### CP-R1.2: At-Least-Once + Single Transport-Level Dedup Ledger
Redelivery MUST be safe via a dedicated transport-level dedup ledger checked at the outbox-consume boundary **before** routing by `kind` — `INSERT INTO outbox_consumed(event_id) … ON CONFLICT DO NOTHING`, dedup + handler-effect in **one transaction**.

Acceptance criteria:
- One ledger covers **both** consume paths — the F2 `events` path (hook idle-marker) *and* Track B's F3 `job_transitions` path (`apply_job_done_declaration_sync`). A replayed `job_done`/`job_fail` cannot double-apply. *(Track A rev-2 F-2 — the **cross-track contract**; Track B adopts this ledger and does not assume the `events`-table `event_id` covers its path.)*
- The design must **not** claim "reuse the `events` table's `event_id`": `events` has no `event_id` column (id lives inside JSON `payload`, `state_machine.rs:727`) and its existing UNIQUE is `(agent_id, request_id)` (`schema.rs:135`) with hook-path `request_id = NULL`. State the actual schema delta (new `outbox_consumed(event_id TEXT PRIMARY KEY)` table). *(Track A review F-5 — folds in here.)*

TDD RED→GREEN: RED = feed the same `event_id` outbox record twice → asserts two `job_transitions` rows (double-apply); GREEN = second is dropped at the ledger, one row.
Testability: `--lib`.

### CP-R1.3: Cold-Scan Replay on Restart + Error-Book Quarantine
On startup, **before serving RPC**, ahd MUST enumerate every agent's outbox and replay each record through the same idempotent consume path (CP-R1.2); records that can never apply are quarantined to a dead-letter book, never dropped and never hot-looped.

Acceptance criteria:
- A `job_done` written before a `kill -9 ahd` and not yet consumed is picked up on restart and applied exactly once (not lost, not doubled). This is the direct proof of A1.
- Replay order is `event_id`-sorted (ULID/UUIDv7 time-prefix); reserved-prefix ids (e.g. G4 `selfcheck:…`) are documented as exempt from the ordering assumption. *(Track A review F-6.)*
- A periodic sweep backstops any inotify drop (level-triggered durable file is re-readable).

Testability: `--lib` for the replay/quarantine logic; the kill-9 timing race is CI-integration.

### CP-R1.4: ACK = Durable DB Commit; Reap-After-Commit; Sender Never Blocks
The ACK is ahd's durable DB commit of the event's effect + ledger row; the outbox file is reaped (deleted) strictly **after** that commit; the hook process never blocks on ahd.

Acceptance criteria:
- A crash between "apply" and "reap" cannot double-apply (guarded by CP-R1.2's single-tx) and cannot lose (the file is still present for cold-scan).
- `.tmp`→`.json` naming is pinned to **one** convention across all three drafts (sandbox writer glob == host scanner glob). *(Track A review F-7.)*

Testability: `--lib`.

---

## Group R2 — Explicit Completion Protocol *(Track B; serves A2, and A3 via CP-R2.5)*

### CP-R2.1: Dispatch Carries Job Identity; Completion Is an Explicit Declaration
Every dispatch MUST inject `AH_JOB_ID` (the `jobs.id`) and `AH_JOB_ATTEMPT_COOKIE` (the arbiter-Q4 per-dispatch-attempt cookie, format `{job_id}:{dispatch_seq}` — reused, not re-minted). Completion is a worker-initiated declaration — `ah job done <id> [--reply-file|--reply-stdin]` / `ah job fail <id> --reason <text>` — writing an outbox record (CP-R1.1 wire form), **never** an inference. *(MA-1.)*

Acceptance criteria:
- A `<job_id>` mismatch against injected `AH_JOB_ID`/cookie is refused loudly at the CLI before anything is written (defends "隔山打牛" wrong-id / ID-tampering).
- A declaration whose `attempt_cookie` is stale against the current dispatch epoch is **rejected and logged, not applied** (defends fast-redispatch misattribution; context-replayed historical `ah job done` dropped).
- Both `done` and `fail` exist so an honest "I can't" (`fail --reason`) never rides the COMPLETED path (convergence C6).

TDD RED→GREEN: RED = a stale-cookie declaration applied to the new attempt; GREEN = rejected+logged, new attempt unaffected.
Testability: `--lib`.

### CP-R2.2: F3 Severed From F2 — Completion Leaves the Agent-Idle Transaction
Job completion MUST move out of the agent-idle transaction. The idle paths (`mark_agent_idle_hook_event_sync` `:716`, matched `:501`, log `:885`) keep writing `agents.state` (F2) but **must no longer call `mark_job_completed_conn_sync`**. A new `apply_job_done_declaration_sync` is the **sole** `DISPATCHED→COMPLETED` writer, triggered only by a consumed `job_done` record; `job_fail`→`FAILED`. *(MA-1; this is the load-bearing refactor.)*

Acceptance criteria:
- After the change, an agent going IDLE with a live background task produces **no** `jobs.status` change (the exact A2 scenario).
- All three coupling sites (`:501`/`:716`/`:885`) are severed — verified by test, not grep alone.
- `reason = "explicit_done"|"explicit_fail"` on `job_transitions`; `reply_source = "protocol"`.

TDD RED→GREEN: RED = idle transition still flips a DISPATCHED job to COMPLETED; GREEN = it stays DISPATCHED; a consumed declaration is the only thing that completes it.
Testability: `--lib`.

### CP-R2.3: Claude Stop-Hook Enforcement Layer — BUILDABLE, soft-bounded (block + reason)
A **net-new** Claude Stop-hook enforcement layer MUST block a stop that has no declaration, feed a corrective `reason`, and after a bounded count yield to the watchdog. *(MA-2.)*

Acceptance criteria:
- **This capability does not exist today and must be built** (see Verified Baseline). The hook returns `{"decision":"block","reason":…}` only when no `job_done`/`job_fail` for `AH_JOB_ID` exists this dispatch **and** block-count < N (proposed N=2). At block-count ≥ N it allows the stop and emits `STOPPED_UNDECLARED` to the watchdog — **never an infinite block** (a hard-infinite block re-creates the 12h-park disease from the other side).
- The hook **never** writes `agents.state` or `jobs.status` — it reads the outbox to decide block-vs-allow only (single-writer discipline intact).
- Its presence/wiring is a hard dependency on G4's self-check (CP-G4.3): if the hook config drifts/deletes, enforcement silently vanishes and the system regresses to inference — G4 must catch this loudly.

TDD RED→GREEN: RED = stop with no declaration ends the turn silently; GREEN = first N stops are blocked with a reason; the (N+1)th allows + emits `STOPPED_UNDECLARED`.
Testability: `--lib` for the block-count/branch logic; whether Claude Code honors the block verdict end-to-end is CI/e2e (native contract, ample under the 5s timeout).

### CP-R2.4: Detection Demoted to a Watchdog — "停了却没声明" = alert, never inference
The completion *detectors* (log/idle-marker/pane inference) MUST stop being completion authorities and become a single reconcile watchdog whose only output is an alert. Nothing about "stopped" ever infers "done" again. *(MA-3 — this is the literal A2 sentence "job 不判完,看门狗告警".)*

Acceptance criteria:
- Predicate: `arbiter says IDLE` + `DISPATCHED job with no declaration` + `silence > watchdog_budget(provider)` ⇒ `STOPPED_UNDECLARED_ALERT` (alert only; job stays DISPATCHED, never auto-COMPLETED/FAILED).
- `classify_terminality`'s completion role is **retired**: `end_turn`/`task_complete` stop being `Terminal` verdicts; they survive only as F2 turn-boundary + watchdog corroboration/nudge-hint.
- **Anti-flood**: the alert is edge-triggered, de-duplicated by `(agent, job, dispatch_epoch)` — one per stop-without-declare episode, **not** re-emitted every tick (directly targets the Gen-3 1887-alert/48h pathology).

TDD RED→GREEN: RED = a timeout marks a job COMPLETED/STUCK by inference; GREEN = it emits exactly one alert and leaves the job DISPATCHED.
Testability: `--lib` for predicate + dedup; live cadence is DF-2.

### CP-R2.5: Reply Payload Rides Inside the Declaration — the "screen" scrape path is deleted
The reply MUST travel *inside* the completion declaration; the pane-scraping reply path is **deleted**, not deprioritized. *(MA-4; serves A2 + A3.)*

Acceptance criteria:
- `ah job done --reply-file` carries the worker's own stated result into `jobs.reply_text` (`reply_source = "protocol"`).
- `collect_reply_for_dispatched_job_sync` and the `"screen"` `reply_source` are removed — **no** code path reads pane text into `reply_text` (kills obs #33 structurally; ghost/banner text cannot corrupt payload → A3).
- `"log"` demoted: for a harness that genuinely cannot exec `ah job done` inline, a **structured** log token (`reply_source = "log_structured"`, adapter-owned, never free-text scrape) is a flagged §5 bridge, not a blessed default.

TDD RED→GREEN: RED = absent structured reply falls back to a scraped pane fragment; GREEN = no scrape path exists; reply is the declared value or explicitly empty-honest.
Testability: `--lib`; fidelity is DF-3.

---

## Group 证据闸门 — Physical-Evidence Gate *(Track B; serves A2)*

### CP-EG.1: Deny-Only Job-Level Admission Gate — no T-level, read-only never gated
Physical evidence MUST be a job-level admission gate on the completion declaration that can only **DENY** (bounce a declaration with a nudge). It is never a T0–T3 lifecycle signal, never a completion trigger, not part of the arbiter state machine. *(§4; convergence C7/C8/C10.)*

Acceptance criteria:
- Static `jobs.requires_physical_evidence` (`schema.rs:168`), set at dispatch, gates only mutating jobs; read-only jobs (research/audit/e2e) carry `=0` and are **never** gated (forecloses read-only livelock).
- The gate is **edge-triggered at the declaration instant**, not a continuous git poll; comparison is scoped to the agent's **own sandbox worktree** (concurrent human edits elsewhere cannot leak in).
- Git never *causes* a completion; it can only *delay* one (all git noise stays on the DENY side).

Testability: `--lib` for the gate branch; real-worktree diff is DF-5 / CI-integration.

### CP-EG.2: Bounded Interception → Release-and-Escalate (no infinite deny)
After 2 denials, the 3rd declaration MUST be released as `COMPLETED_EVIDENCE_WAIVED` **with a mandatory `EVIDENCE_GATE_ESCALATION`** a human sees — a flagged pass, never a silent green tick, never an infinite deny. *(§4 guardrail 2; convergence C9.)*

Acceptance criteria:
- Gaming the cap (magic-3rd-try) buys a *flagged* completion under human review, not a clean pass.
- A mis-tagged / permission-blocked mutating job can never wedge forever (Gen-3's 12h park is the anti-pattern being killed).
- The deny-count is a durable, dispatch-epoch-scoped, **reset-on-redispatch** counter — a net-new stateful component, **not** a tuning knob and **not** the existing `has_completion_deferred_event` hash-dedup. *(g1 NB-2 — pinned as a requirement, carried to tasks.)*

Testability: `--lib`.

### CP-EG.3: Evidence-Signal Definition Reconciled With the Existing Event Model
The "what counts as a diff / as evidence" definition MUST be reconciled with the machinery already in tree, and must not spuriously deny legitimate work. *(g1 NB-4a/b/c — pinned; each is a task.)*

Acceptance criteria:
- Reconcile §4's live-`git diff` with the existing event-presence model (`has_job_evidence_sync` over `mtime_changed`/`diff_generated`/`test_passed`, `state_machine.rs:1004-1027`): decide whether git-diff *produces* `diff_generated` events or *replaces* the model — do not leave two evidence mechanisms coexisting unreconciled. The untouched `requires_test_evidence` dimension (`:1019-1025`) must be addressed, not silently dropped. **(NB-4a — highest build-the-wrong-thing risk.)**
- **Untracked-but-non-ignored** new files count as evidence (`git status --porcelain` minus ignored, or `git add -A && git diff --cached`) — a new-module/new-test job must not false-deny on an empty tracked-diff. **(NB-4c.)**
- Mis-tag-as-read-only is named as an accepted residual **or** a cheap detector added (a completed read-only job whose worktree is dirty = telemetry). **(NB-4b.)**

Testability: `--lib` for the diff-classification; new-file / dirty-worktree scenarios are CI-integration; empirical is DF-5.

---

## Group G4 — Control-Path Self-Check *(Track A; serves A4)*

### CP-G4.1: Three-Tier Startup Self-Check (light blocking-repair / medium non-blocking / deep `ah doctor`)
Startup MUST run a three-tier control-path check: **light** (always-on, blocking-with-repair, per-agent dispatch-readiness — does **not** block ahd's own boot), **medium** (always-on, non-blocking synthetic round-trip through the live daemon), **deep** (opt-in via `ah doctor`, with the daemon unambiguously up). *(G4-Q1/Q2/Q5.)*

Acceptance criteria:
- The light tier blocks *per-agent dispatch*, never ahd boot (avoids the boot-self-probe deadlock — verified sound in Track A review §B). Medium is non-blocking for exactly that deadlock reason.
- Deep adds confirmed-ahd-consume round-trip + a `outbox/dead/` dead-letter census + the cross-agent wiring matrix, mapping to `DoctorStatus::{Pass,Warn,Fail}`.

Testability: `--lib` for tier gating; live-daemon round-trip is CI-integration; deep is `ah doctor` operator-facing.

### CP-G4.2: Drift Detection Scoped to ah-Owned Entries Only (not whole-config fingerprint)
Drift/delete detection MUST compare **only ah's own materialized hook entries** (`is_ah_owned_hook_item`, `home_layout.rs:1202-1207` = `command.contains("ah agent notify")`) against what ahd would render — **NOT** the whole-config `compute_config_hash`. A delete is the degenerate drift case (ah-owned entry absent). *(G4-Q3; Track A rev-2 F-3 — resolved.)*

Acceptance criteria:
- A benign out-of-band operator edit (own hook/skill/setting) does **not** false-fire drift or permanently block dispatch (the F-3 contradiction — whole-config fingerprint + merge-don't-clobber + block-until-match cannot all three hold; scoping to ah-owned entries dissolves it).
- Operator hooks/skills/settings/plugins are out of the comparison entirely.

TDD RED→GREEN: RED = an operator adds a benign hook → drift fires → dispatch blocked forever; GREEN = only a deleted/mutated *ah-owned* Stop entry fires drift.
Testability: `--lib`.

### CP-G4.3: Multi-Provider Coverage — all three config shapes are first-class check targets
The self-check MUST enumerate **every** provider's hook config shape, not only claude's — claude `settings.json` (`hooks.Stop[]`), agy `hooks.json` (wrapper `ah-completion-push`, matcher `""`, `:408-428`), codex `hooks.json` (wrapper `hooks`, matcher `"*"`, `:1167-1190`), **plus codex's `features.hooks = true` gate** (`:1163` — a shape-correct codex config is inert without it). The synthetic round-trip (CP-G4.1 medium) MUST exercise **each** provider's own installed hook, not the claude shape as a stand-in. *(G4-Q1/Q3/Q4; Track A rev-2 F-4 — resolved. This is a **cross-track contract** Track B §5.7 counts on.)*

Acceptance criteria:
- A deleted/drifted **codex** or **agy** Stop hook is detected (today's claude-only grounding would miss it for 2 of 3 providers — a per-provider silent completion loss indistinguishable from 假完成).
- The codex `features.hooks` false-positive vector (shape-correct but hook never fires) is a check target.

Testability: `--lib` for per-shape locators; per-provider synthetic round-trip is CI-integration; empirical is DF-A4.

### CP-G4.4: Loud Degradation + Idempotent Auto-Repair + Mandatory Re-Check
On light-tier detection ahd MUST (1) emit a loud structured log, (2) emit a self-check event on the same spine, (3) attempt an **idempotent, merge-preserving** re-materialize (must not clobber operator/provider config it didn't author), (4) require a re-check pass before the agent is dispatch-eligible — repair is *attempted*, never *assumed*. *(G4-Q6; this is the literal A4 scenario.)*

Acceptance criteria:
- Manually deleting an agent's Stop hook config → next startup alarms loudly, auto-repairs, and the re-check passes before dispatch resumes; during the interim the degradation is loud, never silent.
- Repair merges rather than overwrites (consistent with CP-G4.2's ah-owned scoping — the re-check converges under benign edits).

Testability: `--lib` for the repair/re-check sequence; the delete→startup→repair loop is CI-integration; empirical is DF-A4.

---

## Group R3 — Pane-Lifecycle-Inference Removal *(Track B; serves A3 — DESIGN ONLY this stage)*

### CP-R3.1: Substitute-Signal Coverage Table + Removal Preconditions (no deletion authorized yet)
This stage MUST produce (design only) the removal preconditions and a table mapping **every** pane-inference site to its T0–T2 successor signal. **No deletion PR is authorized until R1/R2 are stable** — north-star §四: "拆早了没有替代信号"; STAGE-PLAN R3-after-R1/R2 iron rule. *(§6.)*

Acceptance criteria:
- Removal preconditions all pinned: R1 stable (A1 proven) · R2 stable ≥2 gens with 假完成 rate → 0 (DF-1/DF-3 green, efficacy 治愈-实证) · every §6.2 substitute has a green dogfood observation · G4 self-check live (A4) so a missing hook loudly degrades instead of falling back to the pane about to be deleted.
- The coverage table names each site → successor → tier → precondition, including: scraped-reply → explicit declaration (T1); dispatch-readiness ghost-text recheck (ah#17) → arbiter `agents.state` (T1/T0); timeout+pane STUCK → watchdog + arbiter `Stalled` (T1); background-vs-哑火 → declaration + `workload.scope` cgroup `populated` (T1/T0); `classify_terminality` completion → F2 demotion (T1/T2); trust/update dialog → **KEPT** (T3's one legitimate job).
- The deletion PR's scope is enumerated (`collect_reply_for_dispatched_job_sync` + `"screen"`; dispatch-readiness pane-diff gate; `classify_terminality`'s `Terminal`-for-completion authority) but **tracked, not authored** this round.

Testability: N/A this stage (design only); the eventual deletion PR's acceptance gate is DF-4 run *before* deleting the dispatch-readiness gate.

---

## Group Provider-Matrix — Enforcement Tiering & Verification *(Track B; serves A2/A3)*

### CP-PM.1: One Declaration Contract, Capability-Tiered Enforcement (not identity-tiered, not LCD)
The wire form (outbox `job_done` + cookie + reply, MA-1) MUST be provider-agnostic and identical across all tiers; **enforcement strength** varies by *verified capability*, never by provider *identity*, and never by downgrading claude to a lowest-common-denominator. *(§5.1/§5.5; convergence C11.)*

Acceptance criteria:
- claude does **not** get crippled to match agy; agy/codex are **not** given a fake synthesized block / PTY polling-block proxy (convergence C12 — rejected on the same grounds as DSR probing; violates the 投键铁律).
- The `job_done`/`job_fail` record is a **union** schema (carries per-provider fields, mostly-null where absent), never an intersection that strips the richest provider's success/failure discriminant (g1 D4).
- A tier assignment is justified by what the provider *provably* supports (a positive CP-PM.2 verdict moves agy/codex up into the block tier; a negative one keeps them watchdog+evidence-gate).

Testability: `--lib` for the wire-form union + tier dispatch.

### CP-PM.2: agy/codex Block-Verdict Verification (highest-leverage open item, method-scoped)
Whether gemini's / codex's `hooks.json` `"Stop"` hook honors a `{"decision":"block"}` return MUST be verified — it is a property of the **external harness**, unknowable from `src/`, and the whole matrix's asymmetry is *unverified* until answered. *(§5.0/§7; convergence §5.0; g1 checkpoint 4.)*

Acceptance criteria:
- Verification uses **only** legitimate methods: (1) external-harness doc/source research, or (2) an **isolated sacrificial e2e** — throwaway agent in an independent `STATE_DIR` + independent tmux socket + trap cleanup, injecting a synthetic block return. **Forbidden** (must never be the path): injecting a block into a *live working* agent's turn, or DSR/PTY probing the live stack.
- Until answered, agy/codex are designed **observe-only** (fail-closed). A positive result migrates them into CP-R2.3's block tier and relaxes §5.2's watchdog-first posture.

Testability: doc-research (no code) OR isolated-sacrificial-e2e (CI/e2e, never live-stack); empirical per-provider is DF-6 + the E6 fidelity metric.

---

## Cross-Cutting — Automated-Test Pinning *(serves the §五 fifth clause)*

### CP-X.1: Each A1–A4 Acceptance Definition Is Pinned by an Automated Test
North-star §五's fifth clause — "以上四条全部有自动化测试钉死" — is itself a requirement: **A1, A2, A3, A4 each MUST have at least one automated regression test** so a future regression fails CI, not only a dogfood observation.

Acceptance criteria:
- A1 ↔ CP-R1.3 kill-9-replay test. A2 ↔ CP-R2.2/CP-R2.4 (idle-no-complete + watchdog-alert-not-inference). A3 ↔ CP-R2.5/CP-R3.1 (ghost-text-cannot-corrupt-payload + successor-covers-dispatch-readiness). A4 ↔ CP-G4.4 (delete→alarm→repair→re-check).
- **Verification-debt discipline (code-closed ≠ empirically-closed):** every requirement above additionally carries a dogfood node (DF-A1/DF-1..DF-6/DF-A4) in `tasks.md`; a passing `--lib` test never by itself marks a requirement empirically closed.

### CP-X.2: Per-Provider 假完成 Fidelity Metric (E6) — makes "did the protocol fix agy" falsifiable
There MUST be a per-provider completion-signal-fidelity metric tying DF-1/DF-2/DF-3 to a per-provider breakdown; without it, "did the protocol fix agy specifically" stays unfalsifiable from collected data. *(g1 §7-j / convergence E6.)*

Acceptance criteria: the dogfood efficacy report (`research/gen-efficacy-reports.md`) can render 假完成 rate per provider per gen, not only aggregate.
Testability: telemetry/reporting; empirical, ties to DF-1/2/3.
