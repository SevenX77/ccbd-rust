# Track B Design Draft — Cross-Lane Review by g1

- **Reviewer**: g1 (泳道1 闸门 / gatekeeper)
- **Target**: `design-draft-track-b-g2.md` (g2-authored) + `convergence-provider-matrix.md` (g2)
- **Type**: cross-lane adversarial review (not self-audit) — every load-bearing claim re-verified against `src/` by the reviewer, not trusted from the draft's citations.
- **Date**: 2026-07-10
- **Overall verdict**: **ACCEPT for merge into `design.md`.** No blocking issue found. The §5.0 premise correction — the one thing the operator flagged as decisive — is **code-verified TRUE**. Six non-blocking items must be resolved before *implementation* (marked NB-1..NB-6 below); none blocks the draft merging as design.

---

## 0. Headline: the §5.0 premise correction is TRUE (I re-verified line-by-line)

This was the operator's designated crux — that all three providers' Stop hooks are currently observe-only and *none* emits a block verdict, refuting the brief's "claude 有抓手、agy 没有". I re-ran the verification against `src/` myself. **Every claim holds; the reality is if anything stronger than the draft states.**

| Claim (draft §5.0 / convergence §1) | My independent check | Verdict |
|---|---|---|
| `CompletionSignalKind` has a single variant `LogOnly` | `manifest.rs:29-32` — enum body is literally `{ LogOnly, }` | **TRUE** |
| …assigned to every provider (bash/codex/claude/antigravity) | `manifest.rs:351,381,400,418` all `= CompletionSignalKind::LogOnly` | **TRUE** |
| claude wires `Stop` → `--event stop` | `home_layout.rs:232` `materialized_ah_hook(ctx,"Stop")` | **TRUE** |
| antigravity wires the same | `home_layout.rs:411` entry `"Stop"`, `:420` `build_ah_hook_command(ctx,"stop")` | **TRUE** |
| **codex wires the same too** (convergence ⚠-A) | `home_layout.rs:1172` entry `"Stop"`, `:1181` `build_ah_hook_command(ctx,"stop")` | **TRUE** |
| all route to the identical `ah agent notify … --event stop --hook-json` | `build_ah_hook_command:681-684` | **TRUE** |
| **no `{"decision":"block"}` anywhere in `src/`** | `grep -rn '"decision"' src/` → **zero hits**; `grep 'decision.*block'` → zero | **TRUE** |
| hook-json output is a hardcoded `"{}\n"` = "allow stop" | `ah.rs:562-564` `format_agent_notify_output`; **locked by test** `ah.rs:1863-1870` (`assert_eq!(…, "{}\n")`) | **TRUE — and test-pinned** |
| timeout 5000 (agy) vs 5 (claude), unit artifact not 1000× | `home_layout.rs:689-694`: `"antigravity" => 5000, _ => 5` — **codex also falls in `_ => 5`** | **TRUE** — confirms the convergence's honest "unit story not clean across harnesses" caveat (codex writes `hooks.json` yet gets `5`) |

**Conclusion on the crux: ACCEPT, high confidence.** The heterogeneity the matrix exists to model is absent from current code — all three are observe-only idle-markers flattened to `LogOnly`. §5's re-anchoring on this is correct, and the double-blind convergence discipline (verify g1's code-grounding independently rather than trust it) was actually executed, not just claimed. This alone justified the §5 rewrite.

One reinforcing find the draft under-uses: `classify_terminality` (`parser.rs:48-133`) hard-codes an **antigravity-only** natural-language heuristic (English + Chinese phrase lists + two regexes at `:69-129`) to decide `DeferredBackgroundWork` vs `Terminal`; for every non-agy provider it unconditionally returns `Terminal` (`:132`). That is itself reply-text inference driving completion — it strengthens the draft's thesis and should be named explicitly as one of the R3 removal targets (it is only obliquely covered in §6).

---

## 1. Review of the five operator-designated checkpoints

### (1) R2 protocol body — landable, internally coherent, interface-aligned with Track A? — **ACCEPT (2 seam notes)**

The mechanism is landable and the F3≠F2 severance is grounded exactly. I verified the coupling the refactor must sever exists in **three** places, and §MA-1 names all three correctly:
- matched path: `state_machine.rs:501` `mark_job_completed_conn_sync` inside `mark_agent_idle_matched_conn_inner`.
- hook-event path: `state_machine.rs:716` (§MA-1 cites `:716` ✓).
- log path: `state_machine.rs:885` (§MA-1 cites "and the log path" ✓).

So the "job completion leaves the idle transaction" refactor scope is **complete**, not partial. (Minor: the Design-Thesis intro cites the coupling as `state_machine.rs:711-718`; the function actually begins at `:545` and the completion call is `:716` — a cite imprecision, not a scope error, since §MA-1 enumerates all three.)

Interface alignment with Track A's outbox/ACK spine (my lane): consistent **at the described level** — `event_id` idempotency key, atomic `.tmp`→rename durability, inotify + cold-scan consume, cookie-attributed epoch check. Two seams to unify in the merged design (neither blocks):
- **NB-1 (seam): `reply_text` inlined into the outbox record.** The draft inlines the full declared reply into the `job_done` JSON. Track A's outbox is a control-event spine; if it carries any per-record size assumption, a multi-KB verdict (`--reply-file` can be large) is an impedance mismatch. Resolve: either confirm the outbox has no size bound, or carry the reply as a content-addressed file-ref rather than inline. Flag for the merged design.
- **NB-2 (mechanism, net-new): the bounded-release / block counters do not exist and are more than "tuning parameters."** §MA-2's "N=2 blocks then yield" and §4's "deny_count ≥ 2 then release" both assume a durable, dispatch-epoch-scoped, reset-on-redispatch integer counter. The only existing analog is `handle_completion_deferral_sync` (`state_machine.rs:1113`), which is **content-hash nudge de-dup** (`has_completion_deferred_event(agent,job,hash)` → nudge once per unique reply hash), **not** a monotonic counter and **not** a bounded-then-release. Also, Claude Code's `stop_hook_active` is a boolean re-entry flag, not a count of 2 — you cannot get N=2 from it alone; the hook subprocess must read/write a persisted counter. §7 files N under "tuning params seeded here"; that under-sells it — the *counting mechanism* (storage, keying, reset-on-epoch) is a new stateful component, not a knob. Accept the design direction; require the merged design to specify where the counter lives and its reset semantics.

### (2) reply-payload attribution — does it really kill #33 (screen-scrape path)? — **ACCEPT (scoped)**

Verified the disease: three `reply_source` values exist — `"hook"`/`"log"`/`"screen"` — and `"screen"` is produced by `collect_reply_for_dispatched_job_sync` whenever a structured reply is absent, at **three** call sites: `:398` (matched), `:650`/`:656` (hook), `:819`/`:825` (log), plus the two `unwrap_or_else(… "screen")` defaults at `:702`/`:871`. #33 is exactly this fallback capturing prompt-echo/fragment instead of the verdict.

MA-4 deletes `collect_reply_for_dispatched_job_sync` and the `"screen"` source and routes the reply inside the declaration (`reply_source="protocol"`). **This structurally closes #33's mechanism** — with the scrape path gone, no pane text can ever land in `reply_text`. ACCEPT.

Scoping caveat (not a reject — the draft is honest about it in B6/§5.3): #33's *class* was an **audit job** = read-only = `requires_physical_evidence=0` = **never gated by §4**. Killing the scrape guarantees `reply_text` is no longer pane-derived, but it does **not** guarantee fidelity if the worker runs `ah job done` with no/partial `--reply-file` (the flag is syntactically optional in the draft's CLI shape). Empty-honest beats wrong-fragment, so this is still a strict improvement, and the draft names the residual ("a trusted done-flag does not launder a garbage payload"). Recommend the merged design decide whether `ah job done` on an evidence-required-OR-audit job **requires** a non-empty reply (fail-closed) vs allows empty.

- **NB-3 (precision): §6.3 deletion anchor list is under-enumerated.** §4 prose correctly says "delete `collect_reply_for_dispatched_job_sync` and its call sites" (all of them). But §6.3's anchor table lists only `state_machine.rs:646-658, :699-702` (the hook path). It **omits** the log-path scrape (`:815-827`, `:868-871`) and the matched-path call (`:398`). If the removal PR follows the anchor table literally, a live `"screen"` scrape survives in the log path. Fix the table to enumerate all three `collect_reply` sites + both `"screen"` defaults. Also note `:398`'s `reply_text` additionally feeds `is_prompt_only_reply` (`:404`) and `classify_terminality` (`:422`), so its deletion is entangled — those consumers fall away with the declaration-driven model, which is fine, but the design should say so rather than leave it implicit.

### (3) physical-evidence gate — do the two guardrails have holes? — **ACCEPT with 3 concrete holes (all non-blocking)**

Guardrail 1 (static `requires_physical_evidence` tag → read-only never gated) and guardrail 2 (2 denies → 3rd releases as `COMPLETED_EVIDENCE_WAIVED` + mandatory escalation) are the right shape and answer o1 §四 structurally (deny-only, no T-level, edge-triggered). But I found three concrete holes:

- **NB-4a (mechanism divergence — the important one): §4's "compare git diff vs dispatch-time baseline" is NOT what the existing machinery does.** The existing gate (`evidence_denial_for_job`, `state_machine.rs:1004-1027`) checks for **recorded evidence *events*** via `has_job_evidence_sync(…, &["mtime_changed","diff_generated"])` and `&["test_passed"]` — i.e. "did some subsystem record a `diff_generated` event for this job?" — it is **not** a live `git diff` of the worktree. The draft claims to "reuse EVIDENCE_DENY_MESSAGE machinery (`:30`)", but `:30` is only the deny *string*; the live-git-diff comparison is **net-new AND divergent in kind** from the event-presence model already in tree. The merged design must decide: does git-diff become the *producer* of `diff_generated` evidence events (fits existing `has_job_evidence_sync`), or does it *replace* that model? As written, two evidence mechanisms would coexist unreconciled. Also: the existing gate already handles a **second dimension, `requires_test_evidence`** (`:1019-1025`, `test_passed`) that §4 never mentions — the design silently drops it. This is the single item most likely to cause an implementation to build the wrong thing; resolve before coding.
- **NB-4b (hole in guardrail 1): mis-tag as read-only is a silent, un-flagged bypass.** The draft answers *cap-gaming* (3rd-try release → flagged WAIVED + escalation, C9). But a mutating job **mis-tagged `requires_physical_evidence=0`** (dispatcher/master error, o1 §四.1 误标) skips the gate **entirely with no flag** — a strictly stronger bypass than cap-gaming, and unescalated. There is no post-hoc "this 'read-only' job produced a diff → suspicious" check. Name this as an accepted residual or add a cheap mis-tag detector (a completed read-only job whose worktree is dirty = telemetry, not a gate).
- **NB-4c (hole in guardrail 2): "tracked-files-only `git diff`" false-denies new-file creation.** §4.2 scopes the check to `git diff` of **tracked** files (to exclude `target/`/logs). But brand-new files are **untracked**, so a job whose legitimate deliverable is a *new* module/test/scaffold shows an **empty tracked-diff** → DENY → 2 nudges → 3rd release as WAIVED+escalation. That is a whole common class of real work spuriously routed through the waiver path. Fix: count untracked-but-non-`.gitignore`d files (e.g. `git status --porcelain` minus ignored, or `git add -A && git diff --cached`) as evidence, not just tracked `git diff`.

Guardrail 2's release-then-escalate philosophy (a flagged pass beats an infinite deny) is sound and I ACCEPT it. The holes above are about *what counts as a diff* and *what bypasses the gate*, not about the release logic.

### (4) §5.0 highest-leverage open item — can the agy/codex block-honor question be verified without violating the 运行铁律? — **the draft's "leave as TODO" is correct; I can tighten it to a *scoped method*, but cannot close it in this review**

Grounded answer: whether gemini's/codex's `hooks.json` `"Stop"` hook honors a returned `{"decision":"block"}` is a property of the **external harness**, not our repo — our code only *writes* the hook (`merge_codex_hook_push`, `inject_antigravity_hook_push`) and *emits* `{}`; nothing in `src/` can tell us how the harness reacts to a block verdict. So it genuinely cannot be closed by reading `src/`.

Can it be verified without breaking the 铁律 (no 投键 to working agents, no active probing of the live stack)? **Yes — via two legitimate paths, neither of which touches a live working agent:**
1. **External-harness doc/source research** — read gemini-cli / codex hook-contract docs or source for block-verdict handling. This is research, not probing; permitted.
2. **Isolated sacrificial e2e** — a throwaway agent in an independent `STATE_DIR` + independent tmux socket + trap cleanup, injecting a synthetic `{"decision":"block"}` hook return and observing whether that *sacrificial* turn is actually blocked. This is within e2e-gatekeeper authority and does **not** 投键 a *working* agent or touch the live stack.

What is **forbidden** and must never be the verification path: injecting a block return into a live working agent's turn, or DSR/PTY probing the live stack (o1 §二.2 / convergence 1.4 — env-pollution / security-boundary). So: **keep it a spec TODO** (I cannot and must not close it inside a doc review), but upgrade §7's open-ended "must be verified" to a **scoped TODO with method = doc-research OR isolated-sacrificial-e2e, explicitly NOT live-stack probing.** I flag that the isolated-e2e path is executable by my lane on request. ACCEPT the draft's handling with that tightening.

### (5) §7 open items — per-item ruling

| # | §7 item | Ruling | Note |
|---|---|---|---|
| a | N (block count) + per-provider watchdog budgets as tuning params | **ACCEPT values, CHALLENGE framing** | Numbers can be tuned against DF-2/DF-6; but the *counter mechanism* is net-new (NB-2), not a knob. Budget seed = `MAX_LOG_MONITOR_WAIT` verified = `Duration::from_secs(15*60)` = 900s (`monitor.rs:10`) ✓. |
| b | codex `log_structured` bridge — is the structured-token contract tight enough? | **ACCEPT as bounded exception, +hardening** | Require: (i) the sentinel is a machine token the **codex adapter** recognizes, never a generic scraper; (ii) it **carries the `attempt_cookie`** and passes the same MA-1 epoch check, so `log_structured` isn't a cookie-bypass backdoor for a replayed token; (iii) **fail-closed** — sentinel absent ⇒ job does NOT complete, never falls back to scrape. With (i)-(iii): non-blocking. |
| c | `COMPLETED_EVIDENCE_WAIVED`: distinct status vs flag on COMPLETED | **ACCEPT deferral, LEAN to flag** | Recommend a **flag/`reason`**, not a new terminal status. A new status ripples into every `status == 'COMPLETED'` consumer (dispatch-readiness, recovery-eligibility, master queries, `runtime_events`) — high blast radius. A flag preserves COMPLETED semantics and matches the existing "evidence via events, not new statuses" pattern. Either way escalation stays mandatory. |
| d | exact `job_transitions.reason` value set | **ACCEPT** | Correct to defer to ah-job-events' existing reason convention; do not fork a pattern. Non-blocking. |
| e | seam with arbiter Q2 late-evidence reconcile (`Failed→Completed` reopen vs this gate) | **ACCEPT as flagged seam, ELEVATE priority** | This is the one seam with genuine *correctness* risk — two modules describing overlapping `Failed↔Completed` races from opposite sides. Non-blocking for draft-merge (it is named), but **blocking for implementation**: the merged `design.md` needs an explicit edge-ownership table (arbiter: perception-driven reopen for late evidence; this draft: declaration-driven `DISPATCHED→COMPLETED/FAILED` + evidence-waived). |
| f (NEW) | agy/codex block-verdict verification | **ACCEPT — highest-leverage TODO** | Scope the method per checkpoint (4) above. |
| g (NEW) | semantic contract-drift self-check (G4 behaves, not just wired) | **ACCEPT** | Real (repo already has `.SUPERSEDED` churn). Belongs in G4/Track A. |
| h (NEW) | 产物轨 git-plumbing failure modes + Windows | **ACCEPT, +connect to NB-4c** | The list (index-lock/detached-HEAD/submodules/git-not-on-PATH) is incomplete — **add untracked-new-files** (NB-4c) as a first-class failure mode, not a Windows-only concern. |
| i (NEW) | coalesced-turn attribution | **ACCEPT** | Cookie is per-attempt; coalescing is a per-provider behavior to test. Non-blocking open item. |
| j (NEW) | per-provider 假完成 fidelity metric (E6) | **ACCEPT** | Without it "did the protocol fix agy" stays unfalsifiable; ties DF-1/2/3 to a per-provider breakdown. |

---

## 2. Blocking vs non-blocking

**Blocking merge into `design.md`: NONE.** The draft is structurally sound, the crux premise is code-verified, and every gap below has a correct *direction* — the issues are completeness/precision, not wrong design.

**Non-blocking, must resolve before implementation (carry into merged `design.md`):**
- **NB-1** confirm outbox has no size bound for inlined `reply_text`, else file-ref it.
- **NB-2** specify the durable, epoch-scoped, reset-on-redispatch **counter mechanism** for both the Stop-hook block count and the evidence deny_count — it is net-new, not a tuning value; `stop_hook_active` (bool) and `has_completion_deferred_event` (hash de-dup) do not provide it.
- **NB-3** fix §6.3's `"screen"`-deletion anchor list to cover all three `collect_reply` sites (`:398` matched, `:650` hook, `:819` log) + both `"screen"` defaults (`:702`, `:871`); note the `:398` entanglement with `is_prompt_only_reply`/`classify_terminality`.
- **NB-4a** reconcile §4's live-git-diff with the existing evidence-**event** model (`has_job_evidence_sync` over `mtime_changed`/`diff_generated`/`test_passed`); address the untouched `requires_test_evidence` dimension.
- **NB-4b** name mis-tag-as-read-only as an accepted residual or add a cheap dirty-worktree-on-read-only-completion detector.
- **NB-4c** count untracked-non-ignored files as evidence so new-file-creating jobs aren't spuriously denied.
- **§7-b/c/e/f** hardening/lean/seam rulings above.

**Verification-debt honesty:** this review closes the *code-grounding* debt (the §5.0 premise) empirically against `src/`. It does **not** close the two things that are genuinely external: (i) the agy/codex block-honor question (checkpoint 4 — needs doc-research or isolated e2e), and (ii) the live-stack efficacy of the whole protocol (DF-1..DF-6 on Gen-4). Both correctly remain open in the draft.

---

## 3. What I did NOT re-derive / trust from the draft
- I did not trust the draft's line citations — every `manifest.rs`/`home_layout.rs`/`state_machine.rs`/`parser.rs`/`ah.rs`/`monitor.rs`/`schema.rs` anchor above was opened and read.
- I did not probe any live agent or the live stack; the block-honor question is left explicitly open rather than closed by a forbidden probe.
- I did not re-review my own Track A draft's internals here (out of scope for this cross-review); NB-1 flags the one Track-A/Track-B seam that needs joint sign-off in the merged design.
