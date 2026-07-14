# ah Completion Protocol — Track B Design Draft (R2 + Physical-Evidence Gate + R3 Teardown)

Status: draft, **g2-authored (泳道2 闸门执笔)**, pending g1 cross-lane review of the whole document (incl. §5 provider matrix). This is Track B of the STAGE-PLAN 2026-07-11 split; Track A (g1) owns R1 (outbox/ACK/replay) + G4 (control-path self-check). This draft does **not** re-design those — it consumes them as dependencies and names the exact seams.

Inputs pinned (not reopened):
- `research/perception-layer-first-principles.md` — north star. F1–F5, T0–T3 grading, R1–R5 dependency order, §五 four acceptance definitions.
- `research/perception-final-convergence-2026-07-09.md` — §二 (task-true-completion verdict): 2.3 explicit-protocol-as-primary + sd_notify attribution race, 2.4② physical-evidence gate. **Two refuted red lines are NOT cited here** (K8s asymmetric absence defaults; K8s API-enforced /status single-write).
- `.kiro/specs/ah-job-events/design.md` — `job_transitions` is the pinned carrier. The completion declaration hangs on this spine; no second event channel is minted.
- `.kiro/specs/ah-perception-arbiter/design.md` — sibling module owning `agents.state` single-write. Q4 outbox + per-dispatch-attempt cookie is reused verbatim here for delivery/attribution.
- `research/OPERATOR-HANDOFF-2026-07-11.md` §域1 — live diseases this draft must kill.
- `.kiro/specs/ah-completion-protocol/divergence-provider-matrix-o1.md` + `…-g1.md` — the **two** independent double-blind divergence drafts (problems only). §5 answers both, reconciled via `convergence-provider-matrix.md` (which also re-verified the code-grounding that corrected §5's original premise — see §5.0).

Boundary declaration (three modules, three write authorities — kept decoupled on purpose):

```text
  [Perception Arbiter]   -> writes agents.state   (F1/F2/F4: alive/turn-boundary/input-wait)
  [THIS: Completion Protocol] -> writes jobs.status COMPLETED/FAILED  (F3: task result)
  [ah-job-events]        -> job_transitions carrier (the durable spine both ride)
```

The core first-principle this draft enforces: **F3 ≠ F2.** Today they are conflated — an agent going IDLE (a turn boundary, F2) triggers job-completion *inference* (F3) in the same DB transaction (`src/db/state_machine.rs:711`–`718`: `mark_agent_idle_hook_event_sync` flips the agent to IDLE **and** calls `mark_job_completed_conn_sync` in one tx). Track B's whole job is to sever that: agent-idle is the arbiter's business; **job completion is driven only by an explicit worker declaration.**

---

## Design Thesis — 停下≠完成, made structural

The disease, grounded in code and incidents:

1. **end_turn/task_complete inferred as completion.** `src/completion/parser.rs:48` `classify_terminality` returns `Terminal` from `end_turn`/`stop_sequence`/`max_tokens`/`task_complete` (`parser.rs:230`, `parser.rs:182`). north star G2: "从 end_turn/task_complete 回合信号**推断**任务完成". This is F2 masquerading as F3. Live cost: Gen-0 假 COMPLETED 11 例/日 (handoff §域6); agy 假完成占道 Gen-2 3/3, Gen-3 2 例 incl. g2-m1 卡 12h (handoff §域1).
2. **reply payload scraped off the screen.** When no structured reply is present, `mark_agent_idle_hook_event_sync` falls back to `collect_reply_for_dispatched_job_sync(...)` with `reply_source = "screen"` (`src/db/state_machine.rs:646`–`658`). Observation log #33: a COMPLETED audit job whose `reply_text` was a **brief fragment** ("(self-contained, post-restart)"), the real ACCEPT verdict living only in the pane. "不止状态撒谎,reply 载荷也会错."
3. **pane content participates in lifecycle.** Ghost/banner text read by the dispatch-readiness recheck → 恒拒发 (obs #24/#36 = public **ah#17**), the third of a three-incident same-family structural bug (banner→ghost→dispatch-recheck).

The fix is one protocol with four load-bearing decisions (MA-1..MA-4), one job-level admission gate (§4), a provider gap matrix (§5), and a teardown plan (§6). Each decision below is annotated with **[Accept:Ax]** = which north-star §五 acceptance definition it satisfies, and **[Dogfood:DF-x]** = which live-stack observation closes it empirically (verification-debt discipline: code-closed ≠ empirically-closed).

North-star §五 acceptance definitions (the bar, verbatim intent):
- **A1** kill -9 ahd then relaunch → event stream has no hole (outbox replay proves it).
- **A2** agent with a live background task hits end_turn → job is **not** judged complete; watchdog alerts "停了未声明".
- **A3** ghost text / banner appears arbitrarily → **zero** lifecycle impact.
- **A4** hook config manually deleted → next-startup self-check alarms and auto-repairs, loudly, in the interim.

Dogfood anchor nodes (all under the Gen-4 open window, `research/dogfood-ledger-2026-07-10.md`; efficacy verdicts pushed via `research/gen-efficacy-reports.md`):
- **DF-1** live agent parks a background task and ends its turn → assert job stays DISPATCHED, no COMPLETED. (A2)
- **DF-2** STOPPED_UNDECLARED_ALERT rate on the live stack: exactly-once per genuine stop-without-declare; no 1887-in-48h floods (obs vs Gen-3 baseline).
- **DF-3** sampling audit: every COMPLETED job's `reply_text` is the declared result, never a brief fragment (kills #33). (A2)
- **DF-4** inject banner/ghost text into a live pane → assert job lifecycle unchanged, dispatch still fires (kills ah#17). (A3)
- **DF-5** mutating job declares done with empty diff → assert denied + nudged; 3rd try released + escalated; read-only job never gated.
- **DF-6** agy假BUSY占道时长 (Gen-4 vs Gen-2/3): watchdog budget catches 哑火 without killing long legitimate reasoning turns.

---

## Must-Answer 1: Dispatch Carries a Job ID; Completion Is an Explicit Declaration

**Decision: dispatch injects a per-attempt job identity; completion is a worker-initiated declaration, never an inference.** The interface has two shapes — a CLI command (the concrete artifact) and its outbox record (the wire form).

**Dispatch side (what the worker receives).** Every dispatch injects, into the agent's environment/sandbox, the identity it must quote back:
- `AH_JOB_ID` — the `jobs.id` of the dispatched job.
- `AH_JOB_ATTEMPT_COOKIE` — the per-dispatch-attempt cookie from arbiter Q4 (format `{job_id}:{dispatch_seq}`, arbiter design MA-4). This is **not** optional and **not** just `job_id`: a fast redispatch's stale, late-arriving declaration must not misattribute to the new attempt (o1 §五.2 epoch drift; arbiter Q4 replay hazard). Reuse the arbiter's cookie; do not mint a second one.

**Completion side (what the worker runs).**

```
ah job done <job_id> [--reply-file <path> | --reply-stdin]     # declare COMPLETED, attach the actual result
ah job fail <job_id> --reason <text>                           # declare FAILED, honest exit (no queue pollution)
```

Both resolve `<job_id>` against `AH_JOB_ID`/`AH_JOB_ATTEMPT_COOKIE` from the environment; a mismatch (wrong id, stale cookie) is refused loudly at the CLI before anything is written (defends o1 §三.2 ID-tampering / cross-job "隔山打牛"). Providing **both** `done` and `fail` is deliberate: o1 §三.1's "调了 ah job done 但 reply 里诚实说我搞不定" ambiguity is resolved by giving the model an honest FAILED exit, so "I give up" never has to be smuggled through the COMPLETED path.

**Wire form (rides the R1 outbox, not a new channel).** `ah job done` does **not** hit the RPC socket directly. It writes a durable outbox record — the same outbox Track A/arbiter Q4 defines — then returns:

```text
{agent_home}/outbox/{event_id}.tmp  --rename(atomic)-->  {event_id}.json
  { "kind": "job_done",             # or "job_fail"
    "event_id": "<uuid>",           # idempotency key, dedup at consume
    "job_id": "job_...",
    "attempt_cookie": "job_...:<seq>",
    "reply_text": "<the actual declared result>",   # T1 structured, NOT scraped
    "declared_at": <monotonic> }
```

ahd consumes (inotify + cold-scan-on-restart, arbiter Q4), validates the attempt cookie against the current dispatch epoch, and applies the transition. A declaration for a stale epoch (arrived after a realign) is **rejected and logged, not applied** (o1 §五.2). The write is durable the instant `rename()` returns; the worker process may die immediately after (fire-and-forget on the sender is safe *because* the outbox already persisted — this is exactly the sd_notify barrier property, convergence 2.3).

**[Track-A/Track-B contract — confirmed against Track A rev-2 R1-Q2, F-2.]** That consume step first passes through R1-Q2's single **transport-level dedup boundary** — `INSERT INTO outbox_consumed(event_id) … ON CONFLICT DO NOTHING`, applied to *every* outbox record **before** the `kind` fork — so a replayed `job_done`/`job_fail` (an outbox file re-read by ahd's cold-scan after a restart) is recognized as already-consumed and **cannot double-apply** to `job_transitions`. This draft explicitly does **not** rely on the `events`-table `event_id` for dedup: that column does not exist (the id lives inside the JSON `payload`, `state_machine.rs:727`) and would not cover this draft's `job_transitions` consume path (`apply_job_done_declaration_sync`) at all. The idempotency home for *both* the F2 `events` path and this draft's F3 `job_transitions` path is that one shared transport ledger — this draft **adopts** it (dedup-then-route) rather than minting its own key. The dedup insert and this draft's `apply_job_done_declaration_sync` effect commit in one transaction (per R1-Q2), so a crash between apply and reap cannot double-apply.

**Structural refactor this forces (the load-bearing change).** Job completion moves out of the agent-idle transaction. Concretely:
- `mark_agent_idle_hook_event_sync` / idle-marker / log paths keep updating `agents.state` (F2 — legitimate, arbiter's carrier) but **must no longer call `mark_job_completed_conn_sync`** (`src/db/state_machine.rs:716`, `:501`, and the log path). They stop being completion authorities.
- A new `apply_job_done_declaration_sync(conn, job_id, attempt_cookie, reply_text)` is the **sole** writer of `DISPATCHED -> COMPLETED`, triggered only by a consumed `job_done` outbox record. It records the transition on `job_transitions` with `reason = "explicit_done"` and `reply_source = "protocol"` (retiring `"screen"`; see MA-4).
- Symmetrically `job_fail` → `DISPATCHED -> FAILED`, `reason = "explicit_fail"`.

This is the direct structural analog of convergence 2.3's sd_notify precedent: "管理者只认显式报告、不认输出启发式."

**[Accept:A2]** (agent end_turn no longer completes a job — completion now requires an explicit act). **[Dogfood:DF-1, DF-3]** — a live agent that backgrounds work and ends its turn leaves the job DISPATCHED; every COMPLETED job carries a declared, non-scraped reply.

*Verification-debt note:* unit tests can prove the code path (declaration → COMPLETED, idle → no completion). Empirical closure is DF-1/DF-3 on the Gen-4 live stack — code-closed ≠ empirically-closed.

---

## Must-Answer 2: Claude Stop-Hook Enforcement Layer (block + reason)

**Decision: the Claude Stop hook is a *soft, bounded* enforcement — it blocks a stop that has no declaration, feeds a reason, and after a bounded number of blocks yields to the watchdog.** It is not an infinite hard block (that would deadlock on a genuinely stuck agent), and it is not a completion signal (the hook does not complete anything — it only refuses to let the turn end silently).

*Grounding (this is net-new, not a tweak — convergence §5.0).* Today claude's `Stop` hook is **observe-only**: it routes to `ah agent notify --event stop`, marks the agent idle, and the CLI emits a hardcoded `"{}"` = "allow stop" (`src/bin/ah.rs:562-564`). **No `{"decision":"block"}` is emitted anywhere in `src/` for any provider.** So the claude "block 带病下班" capability the matrix leans on does **not exist yet** — MA-2 *builds* it (Claude-Code's Stop contract honors `decision:block`; the 5s hook timeout is ample headroom). This is why §5.0/§5.1 mark claude's block as *buildable*, not *current*, and why the config self-check below matters: enforcement being absent-by-default is the pre-existing state, not a regression.

Mechanism, using Claude Code's native `Stop` hook contract:

```text
Claude worker reaches turn end
        |
   [Stop hook fires]  reads AH_JOB_ID + reads the agent's own outbox
        |
   Has a job_done|job_fail record for AH_JOB_ID been written this dispatch?
     |-- yes --> allow stop (turn ends cleanly; declaration already durable)
     |-- no, and block-count < N --> return {"decision":"block",
     |         "reason":"You have not declared completion for <AH_JOB_ID>.
     |                   Run `ah job done <id>` with your result, or
     |                   `ah job fail <id> --reason ...`. Do not end the turn undeclared."}
     |-- no, and block-count >= N --> allow stop, BUT emit STOPPED_UNDECLARED
               (hand off to the MA-3 watchdog; do not loop forever)
```

Key properties:
- **Bounded, not infinite.** Claude Code passes `stop_hook_active` on re-entry precisely to prevent infinite block loops. The hook tracks a per-dispatch block counter; after `N` (proposed **N=2**, tunable — same spirit as the §4 nudge cap) it stops blocking and lets the turn end *undeclared*, escalating to the watchdog. Rationale: a hard-infinite block on an agent that is genuinely wedged (context-corrupted, tool-call broken) would reproduce the 12h-park disease from the other side. The enforcement's job is to *prompt* the declaration, not to imprison the process.
- **The hook never writes state.** It reads the outbox to decide block-vs-allow; it never sets `agents.state` or `jobs.status`. This keeps the single-writer discipline intact (arbiter owns state; §MA-1 declaration owns completion).
- **Config self-check dependency (Track A / G4).** The Stop hook is exactly the "零件好的忘了装" risk class: if the hook config is deleted or drifts, enforcement silently vanishes and we regress to inference. Its presence/wiring **must** be an assertion in Track A's G4 startup self-check (hook config diff + synthetic trigger). This draft does not design that check; it *registers the requirement* against it. This is the one place Track B touches A4.

**[Accept:A2, A4]** (A2: undeclared stop is caught, not inferred-complete; A4: the enforcement hook itself is under the config self-check umbrella). **[Dogfood:DF-1, DF-2]** — live: an agent that tries to end without declaring is blocked once/twice and prompted; if it still won't, exactly one STOPPED_UNDECLARED alert fires (not a flood).

*Open item for g1:* whether N=2 blocks is right, or whether the first block should be softer (a reminder) and the second harder. Deferred to review — the count is a tuning parameter, not a structural choice.

---

## Must-Answer 3: Detection Demoted to a Watchdog ("停了却没声明" = alert, never inference)

**Decision: the completion *detectors* (log/idle-marker/pane inference) stop being completion authorities and become a single reconcile watchdog whose only output is an alert.** Nothing about "stopped" ever *infers* "done" again.

The watchdog is a periodic reconcile (rides the existing tick, not a new thread). Its predicate:

```text
For each agent A:
  if arbiter says A.state == IDLE   (F2: A has stopped, awaiting input)
  and A has a DISPATCHED job J with no explicit job_done|job_fail declaration
  and (now - A.last_activity) > watchdog_budget(provider)      # per-provider, see §5
  then emit STOPPED_UNDECLARED_ALERT(A, J)      # alert only
       optionally: one nudge ("declare completion for J")
       never: mark J COMPLETED or FAILED
```

- **Alert, not a verdict.** This is the systemd-watchdog semantic from convergence 1.4 / 2.3: keep-alive absence is a *loud* condition, surfaced to master/operator, not a silent fallback to a lower-tier signal. The job stays DISPATCHED (or is explicitly cancelled by master), it is never auto-completed off a timeout.
- **`classify_terminality`'s completion role is retired.** `end_turn`/`task_complete` stop being `Terminal` verdicts. They may survive only as *turn-boundary* signals feeding the arbiter's F2 (agent went idle) and as watchdog corroboration/telemetry — never as job-completion authority. `DeferredBackgroundWork` (the current band-aid at `parser.rs`) becomes unnecessary for gating completion, because completion no longer fires on turn-end at all; it is only relevant as a hint for the nudge text.
- **Anti-flood.** DF-2's live target directly addresses the Gen-3 pathology (STOPPED_UNDECLARED_ALERT 刷 1887 条/48h): the alert must be edge-triggered (one per stop-without-declare episode), de-duplicated by `(agent, job, dispatch_epoch)`, not re-emitted every tick.

**[Accept:A2]** — the exact A2 sentence ("job 不判完,看门狗告警") is this section. **[Dogfood:DF-2, DF-6]** — live alert cadence sane; watchdog budget catches genuine 哑火 (agy 假BUSY) without guillotining long legitimate reasoning turns.

---

## Must-Answer 4: Reply Payload Attribution Made Explicit (kill the "screen" source)

**Decision: the reply travels *inside* the completion declaration; the pane-scraping reply path is deleted, not merely deprioritized.**

Grounding: today `reply_source` has three values — `"hook"` (structured, from a hook payload), `"log"` (transcript), and `"screen"` (`collect_reply_for_dispatched_job_sync`, pane scrape). The `"screen"` fallback fires whenever a structured reply is absent (`src/db/state_machine.rs:646`–`658`, and again at `:699`–`702`). Obs #33 is precisely this fallback capturing prompt echo instead of the verdict.

The protocol closes it:
- `ah job done <id> --reply-file <path>` carries the worker's *own stated result* in the outbox record's `reply_text`. That value is what lands in `jobs.reply_text` when `apply_job_done_declaration_sync` runs. `reply_source = "protocol"`.
- **`"screen"` is removed.** `collect_reply_for_dispatched_job_sync` and its call sites are deleted as part of the MA-1 refactor (it only existed to feed the scraped fallback). There is no code path left that reads pane text into `reply_text`.
- **`"log"` is demoted to fallback-with-caveat, not authority.** For providers whose harness genuinely cannot run `ah job done` inline (see §5 codex/agy), a *structured* log token (not free-text scrape) may carry the reply as a transitional bridge — but that path is `reply_source = "log_structured"` and is explicitly a §5 gap, reviewed by g1, not a blessed default.

Delivery/attribution: identical to MA-1 — the reply rides the outbox record, attributed by `attempt_cookie`, durable before sender exit. A late reply from a stale attempt is dropped with the whole stale declaration (MA-1 epoch check), so a redispatch cannot inherit the previous attempt's reply text.

**[Accept:A2, A3]** (A2: reply is the declared result; A3: no pane text ever becomes payload → ghost/banner text cannot corrupt `reply_text`). **[Dogfood:DF-3, DF-4]** — sampling shows declared replies only; injected ghost text never appears in any `reply_text`.

---

## 4. Physical-Evidence Gate (job-level admission gate, convergence 2.4②)

**Decision: physical evidence is a job-level *admission gate* on the completion declaration — it can only DENY (bounce a declaration back with a nudge). It is never a T0–T3 lifecycle signal, never a completion trigger, and it is not part of the arbiter state machine.** This is the single most attacked area in o1's divergence (§四); the answers are structural.

Placement in the pipeline:

```text
job_done declaration consumed
        |
   is J.requires_physical_evidence (static, set at dispatch)?
     |-- no  --> apply COMPLETED (read-only jobs are never gated)
     |-- yes --> compare git diff (sandbox worktree) vs dispatch-time baseline
                 |-- diff present (tracked-file change) --> apply COMPLETED
                 |-- empty diff, deny_count < 2 --> DENY: bounce nudge
                 |         "SYSTEM DENY: mutating job, no git diff. Produce your change or `ah job fail`."
                 |         (reuses EVIDENCE_DENY_MESSAGE machinery, src/db/state_machine.rs:30)
                 |-- empty diff, deny_count >= 2 (3rd declaration) -->
                           apply COMPLETED_EVIDENCE_WAIVED + emit EVIDENCE_GATE_ESCALATION (human/master)
```

The two guardrails convergence 2.4② mandates, both present:
1. **Static `is_mutating` tag at dispatch.** Reuse the existing `jobs.requires_physical_evidence` column (`src/db/schema.rs:168`), set by the dispatcher/master at submit time. Read-only jobs (research, audit, log-grep) carry `requires_physical_evidence = 0` and are never gated → directly forecloses o1 §四.1's read-only livelock.
2. **Bounded interception → release + escalate.** After 2 denials, the 3rd declaration is *released* so a mis-tagged or permission-blocked job can never wedge forever (Gen-3's 12h park is the anti-pattern). But release is **not** a clean pass: it lands as `COMPLETED_EVIDENCE_WAIVED` and fires a mandatory `EVIDENCE_GATE_ESCALATION` alert a human sees. This answers o1 §四.1's "拦截上限成安全漏洞 / 模型磨过两次限制" — gaming the cap doesn't buy a silent green tick, it buys a flagged completion under human review. A flagged pass beats an infinite deny.

o1 §四's contamination attacks, answered:
- **§四.1 signal-level ownership / conflict deadlock.** Resolved by refusing to give the evidence check a T-level at all. It is orthogonal to T0–T3: it never sets state, never completes, only refuses to apply a completion. So "级间冲突 = 上抛人工" doesn't apply — there is no level to conflict with; the deny simply bounces the declaration.
- **§四.2 non-causal / non-replayable mutation.** The gate is **edge-triggered at the instant of an explicit declaration**, not a continuous git poll. So the "ahd crashes mid-write, rereads git dirty, who wins" ambiguity does not arise — there is no background git reader. Baseline is snapshotted at dispatch; comparison is `git diff` of **tracked files** in the **agent's own sandbox worktree**. Environmental noise (`target/`, test logs) is excluded by tracked-files-only + `.gitignore`. Concurrent human edits on another worktree cannot leak in because the check is scoped to the agent's own worktree, not the shared repo (answers §四.2 并发人类修改).
- **Why not promote 产物轨 into the FSM (o1 §四, whole section).** Because promoting it would make it a control signal with the very contamination modes o1 lists. Keeping it a *deny-only admission gate on an explicit declaration* keeps all of git's noise on the DENY side (worst case: a false deny → nudge → 3rd-try release), never on the COMPLETE side. Git never *causes* a completion; it can only *delay* one.

**[Accept:A2]** (a mutating agent cannot silently "带病下班" — no diff, no clean completion). **[Dogfood:DF-5]** — live: mutating job with empty diff is denied+nudged, 3rd try released+escalated; a read-only job declares done and is never gated.

*Verification-debt note:* the git-diff comparison must be exercised on a real sandbox worktree on the live stack (DF-5), not only a unit fixture — the tracked-files-only scoping and worktree isolation are the parts most likely to behave differently in vivo.

---

## 5. Per-Provider Completion-Gap Matrix (Track C conclusions, answering BOTH double-blind drafts)

This section answers the **two independent double-blind divergence drafts** — `divergence-provider-matrix-o1.md` (o1) and `divergence-provider-matrix-g1.md` (g1) — reconciled in `convergence-provider-matrix.md`. **An earlier version of this section was drafted off o1's draft alone; the operator flagged that as a double-blind violation.** The convergence pass (a) confirmed the gaps both drafts raised independently (high confidence), (b) judged the single-sided ones on merit, and (c) — decisively — **re-verified g1's code-grounding against `src/` and found it corrects the premise this section originally rested on.** The correction is recorded honestly in §5.0; the rest of §5 is re-anchored on it. g1 still cross-reviews the whole draft.

### 5.0 Verified baseline — the premise correction (convergence §1)

The brief's framing "claude 有 Stop-hook 抓手、agy 没有" is **not true in the current code.** Verified line-by-line (`convergence-provider-matrix.md` §1):

- `CompletionSignalKind` has a **single** variant `LogOnly` (`src/provider/manifest.rs:29-32`), assigned to **every** provider — bash/codex/claude/antigravity (`:351/:381/:400/:418`). The manifest flattens all heterogeneity to one kind.
- **All three real providers wire the *identical* Stop hook today.** claude (`home_layout.rs:232`), antigravity (`inject_antigravity_hook_push:408-428`, cmd `:420`), **and codex** (`merge_codex_hook_push:1167-1191`, cmd `:1181`) each inject a `"Stop"` hook routing to the **same** `ah agent notify --event stop` (`build_ah_hook_command:674-687`).
- That path is **observe-only**: `handle_agent_notify:838-950` → `mark_agent_idle_hook_event` marks the agent idle; the CLI's `--hook-json` output is a hardcoded `"{}\n"` (`src/bin/ah.rs:562-564`), which in the hook contract means **"allow stop."** A grep of all `src/` finds **no** code path anywhere that emits `{"decision":"block"}` — not for agy, and **not for claude either.**

**Consequence for this section's 立论基础.** The real axis is **not "has a Stop *event*" (all three have one) but "does the Stop event carry an enforceable *block* verdict" — and currently NONE of them does.** The claude-vs-agy asymmetry is therefore **future/aspirational, not current**:
- **claude** block is **buildable-by-contract** (Claude-Code's Stop hook *does* honor `{"decision":"block","reason":…}`; the 5s timeout is ample headroom). MA-2 is the section that *builds* it. It is not a capability we already have.
- **agy** block is **UNVERIFIED** — a Stop hook fires with a generous timeout, but whether gemini's harness *honors* a returned block verdict is undocumented. **This is a first-class open item.** If it honors → the asymmetry largely evaporates and agy can share claude's enforcement tier. If not → agy is observe-only, and the gap is a *harness-capability* property, not a "no Stop event" one.
- **codex** block is **UNVERIFIED**, same as agy.

(Aside: the `5000` vs `5` hook-timeout numbers are most likely a **units artifact** between two harness config schemas both meaning ≈5s — not a real 1000× gap and not a barrier to enforcement. Convergence §1.3.)

### 5.1 Per-provider capability grid (replaces the old "physical block" matrix)

Following g1's D6, the honest grid — **"current" = wired-and-behaving today; "buildable/unverified" = not a present capability.** Cells are deliberately marked unverified rather than asserted.

| Capability | Claude | Antigravity/agy | Codex |
|---|---|---|---|
| Fires a completion-relevant signal today | ✅ `Stop`→`--event stop` (`:232`) | ✅ `Stop`→`--event stop` (`:420`) | ✅ `Stop`→`--event stop` (`:1181`) |
| That signal is treated as completion today | ❌ idle-marker only (`LogOnly`) | ❌ idle-marker only (`LogOnly`) | ❌ idle-marker only (`LogOnly`) |
| **Can *block* until declaration** | **buildable** (Claude-Code honors `decision:block`) → MA-2 | **UNVERIFIED** (does gemini honor block verdict?) | **UNVERIFIED** |
| Carries a structured payload | via `ah job done --reply-file` (MA-1) — provider-agnostic wire form | same wire form | same wire form; `task_complete` field TBD |
| Distinguishes success/failure | via `ah job fail` (MA-1) | via `ah job fail` | via `ah job fail`; native discriminant TBD (D4) |
| Re-enterable after a nudge | in-turn re-prompt (once block is built, MA-2) | **only as a fresh dispatch** (no block → nudge = new turn, A6) | as fresh dispatch |
| Silence profile | active stream, rare dead-silence | reasoning models → long PTY silence (backgrounding is *normal*) | batch → total silence bursts |
| Primary backstop under this protocol | Stop-hook block (once built) + watchdog | **watchdog + evidence gate** | **watchdog + evidence gate** |

The shared abstraction is the **declaration wire form** (outbox `job_done` + cookie + reply, MA-1); the grid's holes are in *enforcement*, which is tiered (§5.5), not in the wire form.

### 5.2 agy — the gap is "no enforceable block verdict," not "no Stop event" (convergence C1–C4, C12; A5/A6/E2)

**Conclusion: agy's completion is *currently* unenforceable at the source (its Stop event has no verified block verdict), so it leans on *three* backstops in priority order — watchdog, evidence gate, explicit `fail` — and never on inference. Whether agy can be lifted to a claude-like block tier is an open verification item (§5.0), not assumed either way.**

- **Open item first (the honest reframe).** Before any of the below, someone must verify whether agy's `hooks.json` `"Stop"` hook honors a `{"decision":"block"}` verdict. Until that is answered, this section designs agy as **observe-only**, which is the fail-closed assumption. If verification comes back positive, agy migrates into MA-2's block tier and this subsection's watchdog-first posture relaxes.
- **The "耍赖不报" gap (o1 §二.1 / g1 A2).** With pane inference removed, an agy that finishes but never calls `ah job done` sits DISPATCHED until the watchdog budget expires → STOPPED_UNDECLARED_ALERT (§MA-3). There is no scrape to "rescue" it, and that is correct: a loud unresolved alert (master cancels/redispatches) is strictly better than a wrong silent COMPLETED. The true progress anchor for agy remains the **产物轨 (fix worktree git HEAD)** — but per §4 that is a *deny gate*, not a completer; master reads it to decide the redispatch, ahd never auto-completes off it.
- **The "suspended-pending-background" third state (g1 E2).** agy's *normal* pattern — end_turn, background task runs, self-wake later — is neither done nor not-done. The watchdog budget *is* the mechanism that tolerates this third state without mis-slotting it as done or failed: "alive + no declaration + long silence" is an *alert threshold*, not a verdict, precisely because for agy it can mean legitimate backgrounding. This is why the same observable means opposite things for claude (anomaly) and agy (maybe-normal).
- **Watchdog budget is per-provider, not global (o1 §二.1 悖论/§六.1, g1 A5/C2).** A single hardcoded budget (e.g. 300s) is rejected: too short guillotines legitimate long agy/reasoning turns mid-storm; too long lets 假BUSY squat 12h. Proposal: budget keyed on `(provider, job.requires_physical_evidence)`, seeded from the existing `MAX_LOG_MONITOR_WAIT = 900s` (`src/completion/monitor.rs:10`, reused not restated, matching arbiter Q2) as the agy floor, with the arbiter's per-signal-class Unknown budgets as the ceiling reference. Exact numbers are a tuning parameter → DF-6 supplies the live data; **not hardcoded blind in this draft.**
- **A nudge for agy is a *fresh dispatch*, not an in-turn re-prompt (g1 A6).** With no block, the §MA-3 nudge cannot re-open the same turn — it lands as a new turn with fresh context and can be misread as new work. So the nudge text must be self-identifying against the same `AH_JOB_ID` ("re-declare completion for *this* job, do not restart it"). This is a concrete divergence from claude's in-turn re-prompt (MA-2) and is called out so the two nudge paths are not conflated.
- **turn-end must NOT be reused as a completion probe (o1 §二.2, g1 A5/C3).** Reusing agy turn-end to "probe completion" would re-introduce G2 (multi-turn 修改→编译→报错→再修改 each yields control → each turn-end would false-complete). Under this protocol turn-end feeds only F2 (arbiter idle) + watchdog; it never gates F3. PTY injection / DSR probing to force a turn-end read stays **rejected** (convergence 1.4/2.4③: fail-dangerous, breaks running external long-connection processes = env pollution / security-boundary击穿 per o1 §二.2, no precedent, violates the投键铁律).
- **Do NOT synthesize a fake block for agy (o1 §五.1, g1 E7 — consensus C12).** Both drafts independently flag the temptation to give agy a claude-like "阻断感" via a PTY polling-and-block proxy or a synthesized signal. Rejected: a self-made sync barrier is a fresh deadlock/race surface and re-introduces exactly the inference pane-text was rejected for. If agy cannot block natively (pending §5.0 verification), it stays in the watchdog tier — we do not fake it.
- **Sudden-death escape (o1 §二.3, g1 E3).** If the agy sandbox is OOM-killed / torn down before it can declare, there is no `ah job done`. Verdict: the job is **not** auto-FAILED off the missing declaration alone. T0 (arbiter's OS-layer: process/scope death) drives the agent to a crash/UNKNOWN verdict; the *job* becomes recovery-eligible (requeue) per the control-plane recovery path — **not** a terminal FAILED that later can't accept a late-but-real result. This mirrors the arbiter Q2 correction (a perception verdict feeds an evidence-check consumer; it is not a one-way guillotine into terminal Failed). Dirty partial writes in the dead sandbox are exactly what §4's evidence gate + master's 产物轨 read reconcile on redispatch.

### 5.3 Done-tool "high-confidence lies" — control-flow trust ≠ correctness trust (o1 §三; g1 B-axis)

**Conclusion: the protocol trusts the *declaration* for control flow but never for correctness — the evidence gate + human escalation are the correctness backstop. Two things it does NOT claim to solve are stated as accepted residuals, not hidden.**

- **Two distinct trust boundaries (g1 B2).** "The signal was *delivered* reliably" (transport — R1's outbox solves this exactly-once) and "the signal is *true*" (semantic — unsolved) are different layers. R1 guaranteeing a `job_done` arrives once does **not** make the task done. This draft owns the *transport* attribution + the *deny gate*; it explicitly does **not** own a semantic-truth oracle. Naming this boundary prevents R1's reliability from making the semantic gap *look* solved.
- **讨好式/幻觉 done (o1 §三.1, g1 B1/B3/B4).** A mutating agy that declares done on a broken first pass is caught by §4: no diff → deny → nudge; or diff-present-but-wrong → COMPLETED, a *known, accepted* residual — the protocol's job is to stop F2-inference and silent scraping, not to judge code correctness (that is the gatekeeper's審計 job, out of this module's authority). **Anti-sycophancy residual (g1 B4):** *mandatory* declaration could make 假完成 **worse** for a reflexive model that learns "I must emit done to end my turn." We do not claim it won't; DF-1/DF-3 must *measure* whether forced declaration raises confident-but-false declarations, and the metric (E6) must be able to falsify "the protocol fixed agy."
- **The non-mutating class is the evidence gate's permanent blind spot — and it correlates with the highest 假完成 risk (o1 §四.1 + g1 B5/C2/C7 — consensus C8).** Read-only jobs (审计, 设计发散, e2e) legitimately produce zero git diff, so §4 never gates them — but that is exactly the class where the worst live 假完成 occurred (job_e817301f: COMPLETED while monitors ran). This is an **accepted structural residual**, stated plainly: the evidence gate cannot corroborate the highest-risk class, and no git-derived signal will. Its only backstops there are the explicit declaration + human审计.
- **Is "explicit protocol + corroboration" quietly circular? (g1 B7).** Every candidate corroborator is entangled with the model: OS-liveness (T0) answers "alive," not "done"; log-tail (T2) derives from the model's own output; git HEAD is independent of the model's *claim* but not its *actions*. There is **no** completion-corroborating signal genuinely orthogonal to the model. We accept this as an epistemic limit rather than pretend a truly independent oracle exists.
- **Honest "I can't" (o1 §三.1 third bullet, g1 E8).** Resolved by MA-1's `ah job fail --reason`: the model has a first-class FAILED exit, so "I give up" never rides the COMPLETED path and never pollutes downstream job dependencies. This also gives a *negative* completion channel (g1 E8: "I finished and I failed" distinct from "I didn't finish").
- **reply payload错位 is a *separate* trust surface (g1 B6, obs #33).** "task is done" and "here is the result" both ride self-report but can diverge (done=true, payload=garbage). MA-4 treats reply-fidelity as its own concern (reply rides *inside* the declaration, `reply_source="protocol"`), so a trusted done-flag does not launder a garbage payload.
- **Replay / stale-history done (o1 §三.2).** The `AH_JOB_ATTEMPT_COOKIE` epoch check (MA-1) rejects a `done` whose cookie doesn't match the current dispatch epoch — a context-replayed historical `ah job done` for an old id/attempt is dropped, logged, not applied.
- **Wrong-id "隔山打牛" (o1 §三.2).** The CLI refuses a `<job_id>` that doesn't match the injected `AH_JOB_ID`/cookie before writing anything (MA-1).
- **Parser ghost-call (o1 §三.2).** Because the declaration is a real subprocess exec of `ah job done` writing a real outbox file (not a markdown/JSON block scraped from the transcript), "the model *mentioned* `ah job done` in prose" cannot trigger a declaration. There is no transcript-scraping tool-call parser in this path to fool.
- **Intra-provider signal conflict (g1 D7).** agy now demonstrably emits *both* a bare `Stop` event (F2 idle-marker) *and*, under this protocol, an explicit `ah job done` (F3). R1's event_id dedups *transport*; **semantically the explicit declaration wins and the bare Stop is F2-only** — the Stop event never competes with the declaration for the completion verdict.

### 5.4 codex `task_complete` semantic boundary (o1 §一/§五.2, g1 D2/D4)

**Conclusion: `task_complete` is a *turn-boundary* signal (F2), demoted from its current completion role; codex must still emit an explicit done, via a structured bridge if it cannot exec a CLI inline. codex already wires the same Stop hook as the others (§5.0) — so the "future provider" framing understates how much is already in-tree.**

- Today `classify_terminality` treats codex `task_complete` as `Terminal` (`parser.rs:182`, test `parser.rs:349`), *and* codex injects the same `Stop`→`--event stop` idle-marker as claude/agy (`merge_codex_hook_push:1181`). Under R2 both are demoted: `task_complete` = "the turn's `last_agent_message` is final for this turn" (F2), and the Stop event = idle-marker (F2) — neither gates F3.
- **Does codex `task_complete` mean "loop done" or "task done"? (g1 D2).** If it fires at "the agentic loop terminated" rather than a turn boundary, codex imports the same F3≠F2 confusion as agy via a different primitive — so it is demoted for the same reason, not trusted as a native completion.
- **Union, not intersection, wire form (g1 D4).** Normalizing three primitives to a lowest-common "done" must **not** discard codex's success/failure discriminant if it has one. The outbox `job_done`/`job_fail` record is a *union* schema (carries per-provider fields, mostly-null for providers that lack them), never an *intersection* that strips the richest provider's information down to a bare "done."
- If a codex harness genuinely cannot run `ah job done` as an inline subprocess (batch/API execution, o1 §一), the transitional bridge is a **structured done token** the provider adapter emits into `last_agent_message` (a machine sentinel the codex adapter — not a generic scraper — recognizes and converts into a real outbox `job_done` record). This is `reply_source = "log_structured"`, explicitly a bridge, explicitly flagged for g1: the one place the protocol tolerates a log-derived declaration, and it must be a structured contract (adapter-owned), never free-text inference.
- **Provider-switch epoch (o1 §五.2, consensus C13).** A late `task_complete`/done from a previous provider (g1 failed → master switched to g2) must not punch through the new provider's epoch. Same cookie/epoch mechanism (MA-1) — the stale signal is scoped to the dead attempt.

### 5.5 Least-common-denominator degradation — refused, but the tiering is *capability-conditional* (o1 §五.1, g1 D6 — consensus C11)

Both drafts' sharpest structural worry: does "one protocol for all providers" force us to *downgrade Claude's block* to agy's weak async notify, throwing away the ability to block 带病下班?

**Conclusion: no. The protocol is one *declaration contract*, but enforcement is provider-*tiered*, not lowest-common-denominator — and (correcting the original draft) the tier is assigned by *verified capability*, not by provider *identity*.** Claude, once MA-2 builds its block, gets the Stop-hook enforcement tier. agy/codex get the watchdog + evidence-gate tier — **unless §5.0's verification shows their harness honors a block verdict**, in which case they move up. We do **not** cripple Claude to match agy, and we do **not** build a PTY polling-and-block proxy to fake sync-block (o1 §五.1 / g1 E7 — rejected on the same grounds as DSR probing). The wire form (outbox `job_done` + cookie + reply) is identical across all tiers; only *enforcement strength* varies, and it varies by what each provider *provably* supports — not by a fixed "claude strong / agy weak" table, which §5.0 showed does not describe current reality.

### 5.6 Attribution race across teardown (o1 §六.2, g1 E9 — consensus C14)

Fully inherited from arbiter Q4 (not re-designed): outbox `rename()` makes the declaration durable *before* the sender/sandbox can be reaped, so a `done` written just before host-level teardown survives (there is no "ACK barrier before exit" requirement because durability is achieved at `rename()`, not at ACK). This is the same sd_notify barrier property (convergence 2.3); Track A owns the outbox implementation, this draft owns only the `job_done` record shape riding it.

### 5.7 Cross-cutting scope & new requirements surfaced by the second draft (g1 E1/E4/D5)

- **Master is a fourth completion class, flagged not solved here (g1 E1).** Verified: `handle_master_notify:978` also just flips BUSY/IDLE on `stop`, no block — master's stop is an idle-marker too, and master's "done" is a different predicate (it orchestrates). This matrix covers **worker** completion; master-self-completion (裸等唤醒) is a sibling concern owned elsewhere, named here so neither spec silently owns the seam.
- **G4's self-check must validate *every* provider's hook path, not only claude's (g1 E4).** A hook that can't reach ahd's socket from inside a sandbox is a per-provider *silent* completion loss that looks identical to 假完成. The MA-2 dependency on Track A/G4 therefore requires the self-check to assert agy's `hooks.json` and codex's `hooks.json` wiring (§5.0 paths), not just claude's `settings.json`. **[Discharged in Track A rev-2 — confirmed against `src/`.]** Track A's G4-Q1/Q3/Q4 now enumerate all three provider config shapes as first-class check targets — claude `settings.json` (`hooks.Stop[]`), agy `hooks.json` (wrapper key `ah-completion-push`, matcher `""`, `home_layout.rs:408-428`), codex `hooks.json` (wrapper key `hooks`, matcher `"*"`, `:1167-1190`) — located via the shared `is_ah_owned_hook_item` predicate (`:1202-1207`), and the G4-Q4 synthetic round-trip exercises *each* provider's own installed hook, not the claude shape as a stand-in. **One gate Track A additionally closed that this draft had not surfaced:** codex's `hooks.json` Stop hook is inert unless `features.hooks = true` is also set in codex's config (`home_layout.rs:1163`), so G4's codex check must validate that feature switch too — otherwise a shape-correct codex config reads as "wired" while the hook never fires (a false-positive silent completion loss, exactly the failure class this bullet names). This requirement is considered **discharged by Track A rev-2**; both drafts now reference the same G4 contract.
- **Silent contract drift (g1 D5).** claude's Stop-hook contract, agy's `hooks.json` shape, codex's `task_complete` are all *external* contracts that evolve under us (the repo already has `.SUPERSEDED` rule churn). A silent semantic change would manifest as a new 假完成 wave with no code change on our side. There must be a G4-style self-check for the *semantic* contract (does the hook still block/fire as expected?), not just the wiring — deferred to §7.

**[Accept:A2, A3]** across §5. **[Dogfood:DF-6]** (agy budget) + **DF-1/DF-3/DF-4** (declaration/reply/ghost) as applicable per provider; **DF-2/E6** must make per-provider 假完成 rate *measurable* so "did the protocol fix agy" is falsifiable.

---

## 6. R3 Teardown Plan — Pane Lifecycle Inference Removal (design only, not executed this round)

north star R3 is explicit: pane lifecycle inference is removed **only after R1/R2 are stable** — "拆早了没有替代信号." This round designs the removal + the substitute-signal coverage table; it does not delete anything.

### 6.1 Removal preconditions (all must hold before any deletion PR)

1. **R1 stable** — outbox/ACK/replay live, declarations durable across ahd restart (A1 proven).
2. **R2 stable** — explicit protocol + Stop-hook enforcement live for **≥2 gens** with the 假完成 rate trending to zero (DF-1/DF-3 green, efficacy verdict 治愈-实证 not 未观测).
3. **Every substitute signal in §6.2 has a green dogfood observation** — no row deleted whose replacement is unproven in vivo.
4. **G4 self-check (Track A) live** — so a silently-missing hook loudly degrades (A4) instead of falling back to the pane we are about to delete.

### 6.2 Substitute-signal coverage table (each pane-inference site → its T0–T2 successor)

| Current pane-inference site (code) | Post-teardown successor signal | Tier | Precondition |
|---|---|---|---|
| Completion via scraped reply (`collect_reply_for_dispatched_job_sync`, `"screen"` source, `state_machine.rs:646`) | Explicit `ah job done` declaration; reply rides protocol (MA-1/MA-4) | **T1** | R2 stable, DF-3 green |
| Dispatch-readiness recheck reads ghost/banner pane text → 恒拒发 (ah#17, obs #24/#36) | Arbiter `agents.state == IDLE` from hook/OS; dispatch gates on *state*, never pane content | **T1/T0** | Arbiter Phase 2 live, DF-4 green |
| STUCK inferred from timeout + pane text | STOPPED_UNDECLARED watchdog alert (MA-3) + arbiter `Stalled` explicit-true signal (convergence 1.3) | **T1 + watchdog** | DF-2 green (no alert flood) |
| "Background task vs 哑火" distinction from pane output | Declaration present/absent (MA-1) + `workload.scope` cgroup `populated` (arbiter Q3 PoC) | **T1/T0** | Arbiter Q3 PoC passes (C8 gate) |
| `classify_terminality` end_turn/task_complete → completion | Demoted to F2 turn-boundary + watchdog corroboration only (MA-3) | **T1/T2** | R2 stable |
| trust/update interaction dialog driving | **KEPT** — T3's one legitimate job (north star T3 line) | **T3** | — (not removed) |

### 6.3 What actually gets deleted (the removal PR's scope, next round)

- `collect_reply_for_dispatched_job_sync` + the `"screen"` `reply_source` branch (`state_machine.rs:646`–`658`, `:699`–`702`).
- The dispatch-readiness pane-diff gate (the second surviving pane-content-inference site the arbiter design cross-references but does not own; ah#17).
- `classify_terminality`'s authority to return `Terminal` for completion — reduced to turn-boundary emission + `DeferredBackgroundWork` hint text.

Everything else pane-related (trust/update dialog) stays. The deletion is **one focused PR gated on §6.1**, tracked but not authored this round.

**[Accept:A3]** — once R3 lands, "ghost text/banner appears → zero lifecycle impact" is structurally true, not defended by patch. **[Dogfood:DF-4]** — the ah#17 injection test is the acceptance gate for the removal PR, run *before* deleting the dispatch-readiness gate to confirm the successor (arbiter state) already covers it.

---

## 7. Open Items Deferred to g1 Review / Implementation (do not block draft merge)

- **N (Stop-hook block count) and per-provider watchdog budgets** are tuning parameters seeded here (N=2; agy floor = `MAX_LOG_MONITOR_WAIT`), finalized against DF-2/DF-6 live data. Not hardcoded blind.
- **codex `log_structured` bridge** (§5.4) is the one tolerated log-derived declaration path — g1 should stress whether the structured-token contract is tight enough to never degrade into scraping.
- **`COMPLETED_EVIDENCE_WAIVED` as a distinct terminal status vs a flag on COMPLETED** (§4) — schema choice deferred; either way the escalation alert is mandatory.
- **Exact `job_transitions.reason` value set** (`explicit_done`/`explicit_fail`/`evidence_waived`) — align with ah-job-events design's existing reason strings; do not fork a new pattern (mirrors arbiter's "reuse existing convention" open item).
- **Seam with the arbiter's Q2 late-evidence reconciliation** — the arbiter's `Failed -> Completed` narrow reopen (arbiter Q2 correction) and this draft's evidence gate describe overlapping races from two sides; kickoff consistency check belongs in the merged design.md, flagged here so neither spec silently owns the seam.
- **[NEW, convergence §5.0] agy/codex block-verdict verification** — does gemini's / codex's `hooks.json` `"Stop"` hook honor a `{"decision":"block"}` return? The claude-vs-agy asymmetry the whole matrix is premised on is *unverified* until this is answered (currently NO provider blocks — all three are observe-only idle-markers). This is the single highest-leverage open item: a positive answer collapses much of §5.2's watchdog-first posture into a shared block tier.
- **[NEW, convergence D5] semantic contract-drift self-check** — G4 must assert not just that the hook is *wired* but that it still *behaves* (blocks/fires as expected); an external harness silently changing its completion semantics would surface as a new 假完成 wave with no code change on our side.
- **[NEW, convergence C5/git] 产物轨 git-plumbing failure modes + Windows** — index-lock contention, detached HEAD, amended/squashed commits, submodules, git-not-on-PATH; on the Windows-native target the evidence path may degrade to platform-conditional. §4's gate must be hardened against these before it is trusted on non-Linux.
- **[NEW, convergence E9] coalesced-turn attribution** — a provider that coalesces two dispatches into one turn may emit one completion signal for two jobs; the per-attempt cookie is necessary but the coalescing behavior itself must be tested per provider.
- **[NEW, convergence E6] per-provider 假完成 fidelity metric** — without a per-provider completion-signal-fidelity measurement, "did the protocol fix agy specifically" stays unfalsifiable from the data we collect (ties DF-1/DF-2/DF-3 to a per-provider breakdown).

---

## Cross-references (so no seam is silently owned)

- **Delivery/attribution (outbox, cookie, epoch, dedup):** owned by Track A (R1) + arbiter Q4. This draft consumes them; the `job_done`/`job_fail` record shape is Track B's only contribution to that channel. **Idempotency is the shared transport ledger** (`outbox_consumed(event_id)`, Track A R1-Q2, dedup-before-`kind`-routing) — this draft routes its declarations through it (see MA-1) and does **not** assume the `events`-table `event_id` covers the `job_transitions` path (Track A F-2, pinned rev-2).
- **Agent-state single-write:** owned by arbiter. This draft never writes `agents.state`; it removes completion from the agent-idle transaction (MA-1) so the two authorities stop overlapping.
- **Carrier:** `job_transitions` (ah-job-events). `explicit_done`/`explicit_fail` are new `reason` values on the existing spine, `reply_source = "protocol"` retiring `"screen"`.
- **G4 startup self-check:** Track A. Stop-hook presence assertion (MA-2) registers a requirement against it — and per §5.7 the assertion must cover **every** provider's hook path (claude `settings.json`, agy + codex `hooks.json`, plus codex's `features.hooks = true` gate), plus a semantic-behavior check (§7), not only claude's wiring. **This is discharged in Track A rev-2** (G4-Q1/Q3/Q4, three-shape enumeration via `is_ah_owned_hook_item`); the multi-provider coverage requirement is now landed on both sides.
- **Control-plane recovery / late-evidence reconcile:** ah-control-plane-refactor. agy sudden-death → recovery-eligible requeue (§5.2) and the `Failed -> Completed` seam (§7) live there.
