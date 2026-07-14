# Per-Provider Completion-Gap Matrix — Double-Blind Convergence

- **Date**: 2026-07-10
- **Stage**: Track C step 3 · overlap analysis of two independent blind divergence drafts + code verification
- **Author**: g2 (泳道2 闸门)
- **Inputs (both blind, mutually unseen at authoring time)**:
  - `divergence-provider-matrix-o1.md` (o1/antigravity) — the first blind draft; my original §5 was drafted off this one alone, which the operator correctly flagged as a double-blind violation.
  - `divergence-provider-matrix-g1.md` (g1/claude) — the *second*, independent blind draft; written without reading o1's draft.
- **Third independent check**: I re-verified g1's code-grounding claims against `src/` myself (not trusting the draft's citations) — §1 below. Where my read of the code *corrects* a blind draft (o1 or g1), I say so.

**Discipline.** Two independent drafts converging on the same gap = high-confidence structural gap (§2). A gap only one draft raised = recorded as **单方发散**; I decide adopt-vs-defer *with a stated reason*, not auto-exclude (§3). The whole point of the second blind draft is to catch what a single-source §5 would have missed — so §3's single-sided items get real scrutiny, not a rubber stamp.

---

## §0 — Method

For each candidate gap I asked: (a) did **both** drafts raise it independently? (b) does the **code** confirm or refute the premise it rests on? (c) does it change the §5 立论基础 (argument foundation), or just add a residual to log? The output is a merged problem list (§5) that the Track B §5 rewrite consumes.

---

## §1 — Code verification of g1's key independent finding (the crux)

g1's draft opens with a set of code-grounding facts that o1's draft does **not** have (o1 took the brief's "claude 有抓手、agy 没有" at face value). The operator singled this out and told me to verify it in the source myself. I did. **Every g1 grounding claim is TRUE, and the reality is even stronger than g1 stated.** Two places my own read *corrects* a blind draft are marked ⚠.

### 1.1 What the code actually shows (verified line-by-line)

| Claim | Source | Verdict |
|---|---|---|
| `CompletionSignalKind` has a **single** variant `LogOnly` | `manifest.rs:29-32` | **TRUE** |
| …assigned to **every** provider — bash / codex / claude / antigravity | `manifest.rs:351 / 381 / 400 / 418` | **TRUE** — the manifest flattens all heterogeneity to one kind |
| claude injects a `"Stop"` hook | `home_layout.rs:232` (`materialized_ah_hook(ctx,"Stop")`) | **TRUE** |
| antigravity injects a `"Stop"` hook (`inject_antigravity_hook_push`) | `home_layout.rs:408-428`, cmd built `:420` | **TRUE** |
| both route to the identical `ah agent notify … --event stop` | `build_ah_hook_command:674-687` | **TRUE** |
| ah-side handler marks agent idle, **no** `{"decision":"block"}` returned | `handle_agent_notify:838-950` → `mark_agent_idle_hook_event:916`; CLI `--hook-json` output is hardcoded `"{}\n"` (`ah.rs:562-564`) | **TRUE** — grepped all of `src/`: there is **no** code path anywhere that emits a hook block verdict |
| antigravity hook timeout 5000 vs claude 5 | `hook_timeout_for_provider:689-694` | **TRUE numerically** (see ⚠-B on the unit) |

### 1.2 ⚠-A — the reality is *stronger* than g1 stated: **codex wires the same Stop hook too**

g1 cited claude + antigravity both wiring `"Stop"`. In fact **codex does as well**: `merge_codex_hook_push` (`home_layout.rs:1167-1191`) writes a `"Stop"` hook into codex's `hooks.json` routing to the **same** `build_ah_hook_command(ctx,"stop")` → `ah agent notify --event stop`. So **all three heterogeneous providers currently converge on the identical wiring**:

```
claude   (:232)   ┐
antigravity(:420) ├─→ "Stop" hook → `ah agent notify --event stop`
codex    (:1181)  ┘        → handle_agent_notify → mark_agent_idle_hook_event (idle-marker)
                           → CLI emits "{}"  (= "allow stop" in the hook contract; never blocks)
```

The heterogeneity the matrix exists to model is **entirely absent from current code**. Today all three are wired identically as **observe-only idle-markers**, all flattened to `LogOnly`. This is the single most important convergence output: the claude-vs-agy asymmetry the brief is premised on **does not exist in the code yet** — it is a *future* asymmetry, not a current one.

### 1.3 ⚠-B — correcting g1's "1000× timeout gap" inference

g1 (A4) read `5000` vs `5` as "5000ms vs 5ms, a 1000× gap ⇒ agy has heavier hook startup." I do **not** think the code supports that inference. The value is written verbatim into each *external harness's* config, and the timeout **unit is defined by that harness, not by our code** — which I cannot fully pin from this repo alone. The far more likely reading:

- claude's `5` lands in Claude-Code `settings.json`, whose hook `timeout` is documented **in seconds** → ~5s. (A literal 5ms would be absurd — no unix-socket round-trip to ahd completes in 5ms; the engineers would not set a timeout that guarantees the hook always times out. So it is seconds.)
- antigravity's `5000` lands in gemini's `hooks.json`, plausibly **in milliseconds** → also ~5s.

So the 5000-vs-5 numeric gap is most likely a **units artifact between two harness schemas, both meaning ≈5s**, **not** a real 1000× timing difference. (Caveat: codex *also* writes a `hooks.json` yet gets `5`, so the unit story is not perfectly clean across harnesses — which is exactly why the honest statement is "timeout unit is a per-harness external contract, unverified here," not g1's confident 1000×.) **Consequence:** g1's derived worry — "a 5s block would stall every turn-end" / "5ms is too short to return a verdict" — is not code-supported. The real reason enforcement is absent is **not** the timeout (5s is ample headroom to build a block verdict); it is purely that the handler emits `{}` and there is no block-decision code path. My earlier private reasoning that "claude's short timeout defeats a block" was wrong and is not carried into the rewrite.

### 1.4 The corrected truth this forces onto §5

The premise "claude has a natural completion 抓手, agy doesn't" is **not currently true**. Restated precisely:

- **The real axis is not "has a Stop *event*" (all three have one) but "does the Stop event carry an enforceable *block* verdict" — and currently NONE of them does.**
- **claude**: block is **buildable** — Claude-Code's Stop-hook contract *does* honor `{"decision":"block","reason":…}`, and the 5s timeout is ample. So MA-2's enforcement layer is a *real, latent* capability we can turn on. **But it is aspirational, not current.**
- **agy**: block is **unverified** — a Stop hook fires with a generous timeout, but whether gemini/antigravity's harness *honors* a returned block verdict is undocumented. If it does → the claude/agy asymmetry largely evaporates and agy could get a claude-like enforcement tier. If it does not → agy is observe-only and the asymmetry is real but is a *harness-capability* property, not a "no Stop event" property.
- **codex**: block is **unverified**, same as agy; a Stop hook is wired, honor-semantics unknown.

This changes the §5 立论基础: my original §5.1 matrix presented "Claude = Strong sync block" as a **current physical characteristic**. That is wrong. It is a *to-be-built* capability for claude and an *open verification question* for agy/codex. The rewrite (§5 of the Track B draft) corrects this without abandoning the tiered-enforcement conclusion (§5.5) — which survives, but is now explicitly **capability-conditional**, not fixed by provider identity.

---

## §2 — Consensus gaps (raised independently by BOTH drafts = high confidence)

These are the load-bearing gaps. Both authors, blind to each other, hit them — they are real.

| # | Gap | o1 | g1 | Note |
|---|---|---|---|---|
| C1 | **agy 耍赖不报 / no enforcement primitive at the source** — finishes but never declares → what state holds the turn? | §二.1 | A2, A5 | Core. Answered by §MA-3 watchdog (not a scrape rescue). |
| C2 | **Watchdog budget must be per-provider, not global** — too short guillotines legit long reasoning turns; too long lets 假BUSY squat (12h). Same observable ("alive, silent, N s") means *opposite* things for claude (anomaly) vs agy (normal backgrounding). | §二.1 悖论, §六.1 (300s例) | A5, E3 | Strongest convergence — both reach the identical "same observable, opposite meaning" insight independently. Answered §5.2 budget keyed on `(provider, requires_physical_evidence)`. |
| C3 | **turn-end must NOT be reused as a completion probe** — multi-turn 修改→编译→报错→再修改 each yields control; probing each turn-end re-introduces G2 假完成. | §二.2 | A5, E2 | Answered §5.2/§MA-3: turn-end feeds F2 + watchdog only, never F3. |
| C4 | **sudden-death / OOM before declare** — no `ah job done` fires; FAILED vs Unknown; dirty partial writes. | §二.3 | E3 | Answered §5.2: not auto-FAILED; T0 crash verdict → recovery-eligible requeue. |
| C5 | **讨好式/幻觉 done — "high-confidence lie"** — trust the declaration for control flow but never for correctness; self-report from a known-unreliable narrator. | §三.1 | B1, B3, B4 | Answered §5.3. g1 B4 adds the sharp anti-sycophancy twist (mandatory declaration may make 假完成 *worse*) — adopted as a residual, §5.3. |
| C6 | **honest "I can't" needs a first-class FAILED exit** — "I give up" must not ride the COMPLETED path. | §三.1 (3rd bullet) | E8 | Answered by MA-1 `ah job fail --reason`. |
| C7 | **产物轨 into the FSM — which T-level? signal-ownership conflict/deadlock** | §四.1 | C1 | Answered §4: refuse to give it a T-level at all; deny-only admission gate. |
| C8 | **read-only / non-mutating task is the evidence gate's permanent blind spot** — and (g1's sharpening) it *correlates with the highest 假完成 risk* (job_e817301f 审计单假完成). | §四.1 (read-only livelock) | B5, C2(FN), C7 | High-value. Answered §4 guardrail 1 (`requires_physical_evidence=0` never gated) — but g1's "highest-risk-class = the blind spot" is a **stated accepted residual**, not something the gate fixes. Adopted into §5.3. |
| C9 | **evidence-gate guardrails are themselves attack surface** — 3rd-attempt release = a cap the model can game; `is_mutating` misannotation shifts trust to whoever annotates. | §四.1 (拦截上限, 误标) | C6 | Answered §4: gaming the cap buys a *flagged* `COMPLETED_EVIDENCE_WAIVED` + mandatory human escalation, not a silent green tick. |
| C10 | **non-causal / environmental mutation noise** — `target/`, test logs, concurrent human edits on another worktree. | §四.2 | C3, C5(partial) | Answered §4: edge-triggered at declaration, tracked-files-only, scoped to the agent's own worktree. |
| C11 | **LCD degradation** — does "one protocol for all" force downgrading claude's block to agy's weak notify? | §五.1 | D6 (weakest/strongest/per-tier trilemma) | Both frame the exact same trilemma. Answered §5.5: **no** — one declaration *contract*, provider-*tiered* enforcement. |
| C12 | **do NOT synthesize a fake block/signal for agy** — PTY polling-block proxy (o1) / wrap-agy-to-synthesize (g1) = re-introduces the rejected inference, fresh deadlock surface. | §五.1 (2nd bullet) | E7 | Both independently flag the "fake a block for agy" temptation as dangerous. Answered §5.2/§5.5: rejected on the same grounds as DSR probing. |
| C13 | **epoch / provider-switch late signal** — previous provider's delayed completion must not punch through the new epoch. | §五.2 | D7 (partial) | Answered §5.4 via `AH_JOB_ATTEMPT_COOKIE`. |
| C14 | **teardown attribution race / sd_notify ACK-barrier missing** — fire-and-forget loses the signal if the sandbox is reaped before ahd consumes. | §六.2 | E9 (adjacent) | Answered §5.6: `rename()` durability *is* the barrier; no ACK-before-exit needed. o1 is the stronger source here. |
| C15 | **codex `task_complete` semantic point** — is it "turn boundary" or "loop terminated"? does it import F3≠F2 like agy? | §一 (table) | D2 | Answered §5.4: demoted to F2 turn-boundary. |
| C16 | **three primitives, three semantic points → unify without `match provider`** | §五 (whole) | D3 | Answered §5.5 + §5.4: shared *wire form*, tiered *enforcement*, adapter seam bounded. |

---

## §3 — Single-发散 gaps (only one draft raised) — adopt/defer with reason

Not auto-excluded. Each judged on merit.

### 3.1 g1-only (the code-grounded lens o1 structurally lacked)

| Gap | Decision | Reason |
|---|---|---|
| **[KEY] Code reality: all-`LogOnly`; agy (and codex) DO wire a Stop hook; block enforcement unbuilt even for claude; timeout asymmetry** (A1/A3/A4/D1) | **ADOPT — decisive** | Verified §1. This is the finding that corrects the §5 立论基础. o1 could not have it (blind to code). Highest-value single-发散 item in the whole exercise. |
| **B2 — two distinct trust boundaries**: "signal *delivered* reliably" (R1 solves) vs "signal is *true*" (unsolved). | **ADOPT** | Clean layering insight; sharpens why R1's reliability makes the semantic gap *look* solved when it isn't. Folded into §5.3. |
| **B6 — reply payload错位 (#33) as a *separate* trust surface** — done-truth vs payload-fidelity can diverge. | **ADOPT** | Directly matches my MA-4. o1 never touched reply payload. Strengthens §5 cross-ref to MA-4. |
| **B7 — is any completion-corroborating signal genuinely orthogonal to the model? is "protocol + corroboration" quietly circular?** | **ADOPT as stated residual** | Sharp and honest: OS-liveness answers "alive" not "done"; log/HEAD derive from the model's own output/actions. §5.3 states this as an accepted epistemic limit, not a solved problem. |
| **D4 — normalization union vs intersection** — flattening three primitives to a lowest-common "done" may *lose* codex's success/failure discriminant. | **ADOPT** | Concrete schema risk. §5.4/§5.5: the wire form is a *union* (carries per-provider fields, mostly-null), not an intersection — stated explicitly. |
| **D5 — silent contract drift** — external harness completion semantics evolve under us; a silent change manifests as a new 假完成 wave with no code change on our side. | **ADOPT as open item** | Real (repo already has `.SUPERSEDED` rule churn). Added to §7 deferred / DF observability. |
| **D6 — explicit per-provider capability grid** (fires / can-block / structured-payload / success-fail / re-enterable). | **ADOPT — becomes the matrix backbone** | This is the right skeleton for §5.1. The rewrite replaces the old "physical block: strong/none/none" row with this grid, with honest "unverified" cells. |
| **D7 — idempotency *within* one provider** — if agy emits Stop-event *and* an explicit `ah job done`, R1 dedups transport but which wins *semantically*? | **ADOPT** | Now doubly relevant given §1 (agy's Stop event AND a future explicit done both exist). §5.4: explicit declaration wins; bare Stop is F2 only. |
| **E1 — master as a 4th completion class** (`handle_master_notify`, master's "done" ≠ a worker's). | **ADOPT as scope note** | Verified: `handle_master_notify:978` flips BUSY/IDLE, also no block. Master's stop is idle-marker too. §5 adds an explicit scope line: this matrix covers *worker* completion; master-self-completion is a sibling concern flagged, not solved here. |
| **E2 — the "suspended-pending-background" third state** — agy's normal end_turn+background+self-wake is neither done nor not-done. | **ADOPT** | Directly shapes §MA-3/§5.2: the watchdog budget *is* the mechanism that tolerates this third state without mis-slotting it as done or failed. |
| **E4 — does the hook even *fire* under the sandbox/credential model? is G4's self-check claude-shaped?** | **ADOPT** | A per-provider *silent* completion loss that looks identical to 假完成. Ties to my MA-2 G4 dependency; §5 adds: the G4 self-check must validate agy's `hooks.json` + codex's `hooks.json` paths, not only claude's `settings.json`. |
| **E6 — observability: how do we *measure* per-provider 假完成 rate?** unfalsifiable from current manual attribution. | **ADOPT** | Ties directly to my DF anchors. §5/DF: per-provider completion-signal-fidelity must be measurable or "did the protocol fix agy" stays unfalsifiable. |
| **A6 — is a "nudge to re-declare" even coherent for agy?** (no block → nudge lands as a *new* turn with fresh context, may be read as new work). | **ADOPT** | Refines §MA-3: for agy the nudge is a fresh dispatch, not an in-turn re-prompt; the nudge text must be self-identifying against the same `AH_JOB_ID`. |
| **E9 — coalesced turns** — a provider that coalesces two dispatches into one turn may emit one completion signal for two jobs. | **ADOPT as open item** | Attribution edge case; the cookie is per-attempt, but coalescing is a provider behavior to test. §7. |
| **E5 — onboarding/first-turn window** — hook not wired yet but agent already accepts a task. | **DEFER (log only)** | Real but narrow; owned more by the sandbox/onboarding path than the completion protocol. Noted, not folded into §5 body. |
| **A7 — non-Stop hook points** (tool-call / pre-exit / heartbeat) as alternate completion carriers. | **DEFER (log only)** | Speculative until we know each harness's hook surface; adopting now would invent per-provider special-cases we can't yet verify. Parked. |
| **C4(g1) — baseline captured atomically with dispatch?** | **ADOPT (already implied)** | Folded into §4's "baseline snapshotted at dispatch"; g1 sharpens that the capture must be *atomic* with dispatch or a just-before-dispatch commit miscounts. Noted in §4 verification-debt. |
| **C5(g1) — git plumbing failure modes + Windows platform-conditional** (index-lock, detached HEAD, submodules, git-not-on-PATH). | **ADOPT as open item** | Real operational surface for §4; on the Windows-native target 产物轨 may degrade to platform-conditional. §7. |

### 3.2 o1-only (concrete attack scenarios g1 abstracted or skipped)

| Gap | Decision | Reason |
|---|---|---|
| **§三.2 parser ghost-call** — "I should *not* run `ah job done`" in prose mis-parsed as a real tool trigger. | **ADOPT (already answered)** | §5.3 keeps the answer: the declaration is a real subprocess exec writing a real outbox file, not a transcript scrape — there is no parser to fool. |
| **§三.2 ID-tampering / 隔山打牛** — wrong `job_id` passed, cross-job mis-completion. | **ADOPT (already answered)** | §5.3: CLI refuses a `<job_id>` not matching injected `AH_JOB_ID`/cookie before writing. |
| **§三.2 replay / stale-history done** — context残留 old `done` call replayed. | **ADOPT (already answered)** | §5.3: cookie epoch check drops it. |
| **§二.2 PTY/DSR injection breaks a running external long-connection process** (env pollution, security boundary). | **ADOPT (reinforces rejection)** | The concrete env-pollution angle strengthens §5.2's rejection of PTY probing beyond "no precedent." |
| **§四.2 environmental byproduct concreteness** (`target/`, local DB writes, git-hook auto-commit). | **ADOPT** | Folded into §4/C10's tracked-files-only + own-worktree scoping. |
| **§六.2 sd_notify ACK-barrier / high-latency physical loss** concreteness. | **ADOPT (already answered)** | §5.6: durability at `rename()`, not at ACK. |

**No single-发散 item was rejected outright.** Two (E5, A7) are parked as log-only because adopting them now would either duplicate ownership (E5 → onboarding path) or invent unverifiable per-provider special-cases (A7). Everything else is folded into the §5 rewrite or the §7 deferred list.

---

## §4 — Net impact on the Track B §5 rewrite

1. **§5.1 matrix — the big correction.** Replace the "Physical block: strong / none / none" row (which stated an asymmetry as *current fact*) with g1's **capability grid** (D6), with honest **"unverified"** cells and a clear **"current code: all three identical, observe-only"** baseline row (§1). The asymmetry becomes *future/aspirational + verify-first*, not current.
2. **§5.2 (agy)** — reframe around the verified axis: **the gap is not "no Stop event" but "no enforceable block verdict on the Stop event," and currently NONE of the three has one.** Add the agy-harness-block-verification as a first-class open item (does gemini honor a block verdict?). Keep the watchdog/evidence/fail three-backstop conclusion (survives).
3. **§5.5 (LCD refusal)** — survives, but recast as **capability-conditional**: enforcement tiering is by *verified capability*, not by provider *identity*; if agy turns out to honor block verdicts it moves up a tier. Add D4's union-not-intersection wire-form statement.
4. **§5.3/§5.4** — fold in B2 (two trust boundaries), B7 (corroboration-circularity residual), B6 (payload as separate surface), D7 (intra-provider signal conflict), D5 (silent contract drift).
5. **New scope line** — master (E1) is a sibling completion class, flagged not solved here.
6. **New requirement against G4 (E4)** — the self-check must validate agy/codex `hooks.json` paths, not only claude `settings.json`.
7. **§7 deferred additions** — D5 (contract drift), C5/git-plumbing+Windows, E9 (coalesced turns), E6 (per-provider fidelity metric), A6 (agy nudge = fresh dispatch).

---

## §5 — Merged problem list (the checklist the §5 rewrite must answer)

**Verified-truth corrections (must land):**
- V1. All three providers currently wire an identical observe-only Stop→idle path; `CompletionSignalKind` = single `LogOnly`. No block verdict exists anywhere.
- V2. "claude blocks, agy doesn't" is a *future* asymmetry; claude's block is buildable-by-contract, agy's/codex's is verify-first.
- V3. The 5000-vs-5 timeout gap is most likely a units artifact, not a 1000× timing gap; it does not block building enforcement.

**Consensus gaps (§2, both drafts): C1–C16** — all already answered by the Track B draft; the rewrite just re-anchors §5.1/§5.2/§5.5 on the corrected premise.

**Adopted single-发散 (§3): B2, B6, B7, D4, D5, D6, D7, E1, E2, E4, E6, A6, E9, C4(g1), C5(g1)** + o1's concrete attacks (parser ghost-call, ID-tampering, replay, PTY env-pollution, env byproducts, ACK-barrier).

**Parked (log only): E5 (onboarding window), A7 (non-Stop hook points).**
