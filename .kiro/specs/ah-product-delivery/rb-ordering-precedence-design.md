# R-B/ordering precedence design — seed-matcher must precede IdleMarker

Status: 1d audit CONVERGED (a1 engineering + a3 PM-proxy, round 1, 0 blocking must-fix). Decision below is the implementation spec.

## CONVERGED DECISION (1d synth, Master PM)

**This PR = reorder ONLY. Keep codex_update_01 action = `Key("2")+Key("Enter")` unchanged.**

New `classify_capture` order (a1's safe refinement of a2's idea — learned stays AFTER IdleMarker
to avoid turning clean idle ticks into a DB failure surface, gating.rs:128-166 / runner.rs:403-417):

```
1. empty-spawning skip      (gating.rs:64-75  — UNCHANGED, safety invariant, test :460 green)
2. same-hash skip           (gating.rs:78-87  — UNCHANGED, cost invariant, runner sets prev_hash only post-action :322-324)
3. seed matcher             (match_prompt_for_scan; if Matched → KnownAction)   ← MOVED ABOVE IdleMarker
4. IdleMarker               (if marker_matches → Skip{IdleMarker})              ← now only on seed NoMatch
5. learned-lookup / Unknown (gating.rs:128-166 — stays AFTER IdleMarker)
```

Invariants verified to stay green by both auditors: `marker_like_text_does_not_become_unknown_when_marker_matches`
(:436 — plain `> ` → seed NoMatch → IdleMarker Skip, unchanged), ACK startup (:355-374 —
matcher.rs:71-78 already skips codex_update/trust_path in AckVisualDiff), empty-spawning (:460).

Per-tick cost accepted (a1): matcher scans active-region only (matcher.rs:50-51), 2 default seeds,
same-hash already blocks post-action rescans. No new gate; regex-cache deferred until KB grows.

**Why reorder-only closes the goal:** at spawn the menu dismisses via "2"+Enter and the "2"
leaks (`↳ 2`) but spawn STILL reaches idle + takes dispatch — i.e. the leak is cosmetic, not
blocking. Reorder makes the seed reachable on resume too → same cosmetic-leak dismiss → idle →
dispatch. Dogfood is the judge.

## SPLIT OUT → follow-up R-B2 (NOT this PR)

Down+Enter keystroke change is deferred to a separate change because both auditors flagged:
- **Unproven + catastrophic failure mode** (a3 Q4): if codex's default cursor / item order differs,
  Down+Enter could land on "1. Update now" → npm install → crash → the exact disaster R-B prevents.
  → dogfood-prove BEFORE merge is a HARD gate, never merge-then-prove.
- **Undeclared cutover** (a3 M1): changing the action breaks ~6 `["key:2","key:Enter"]` assertions
  in src/prompt_handler/runner.rs (multiple dispatch-guard tests), tests/pr4a_lifecycle_contract.rs,
  src/prompt_handler/resolve.rs — must sync all per SOP-05.
- **Bigger blast radius**: affects every codex_update_01 dismiss site (spawn/resume/dispatch-guard).

R-B2 is only pulled forward into THIS PR if dogfood proves the "2" leak actually blocks 续派单
(contradicting spawn evidence). M3 watch: after seed-first, a real idle frame with residual
"npm install -g @openai/codex" scrollback could mis-fire the seed — dogfood must watch for false-fire
(active_prompt_region bottom-region extraction should mitigate; verify).

## Goal (the wedge to close)

codex is SIGKILLed (OOM sim) → ahd marks CRASHED → `ah up` → `codex resume <uuid>`.
codex resume paints an "Update available" menu. ahd MUST auto-dismiss it → codex reaches
real idle → resumed session keeps taking dispatch. This is the last gap in "OOM 后 resume 续断点".

## Root cause (instrumented dogfood, live ah not ccb — CONFIRMED)

`›` is **semantically overloaded** in codex: it is BOTH the idle input prompt AND the menu
selection cursor (`› 1. Update now`). `classify_capture` (src/prompt_handler/gating.rs:50)
checks the IdleMarker (`marker_matches`, gating.rs:90) BEFORE `match_prompt_for_scan`
(gating.rs:106). Order in source today:

1. empty-spawning skip (gating.rs:64)
2. same-hash skip (gating.rs:78)
3. **IdleMarker → Skip{IdleMarker}** (gating.rs:90-104)  ← short-circuits
4. match_prompt_for_scan → KnownAction / learned / Unknown (gating.rs:106-178)

Once the resume update-menu paints, IdleMarker hits the `›` cursor on `› 1. Update now`,
returns `Skip{IdleMarker}`, and the seed matcher (which CAN match this menu — verified) never
runs. The agent then goes through the input-probe (R-C), which correctly finds `›` is not a
real input box (NotCandidate), and the agent wedges at `PROMPT_PENDING (unknown_prompt)` →
dispatch refused → resume self-heal fails.

spawn works only because of a timing accident: spawn has no "Running scope as unit" line, so
an early seed scan hits the menu before IdleMarker takes over. resume adds that line, delaying
the paint past the early scans; by the time the menu is fully drawn, IdleMarker wins every tick.

## a2's precedence design (1c idea)

**Q1 — precedence is fundamentally wrong.** Correct priority: **Seed-Matcher (specific, known
scenarios) → IdleMarker (generic heuristic) only on NoMatch.** In gating.rs: run
`match_prompt_for_scan(kb)` first; if a known popup hits → return KnownAction to dismiss; only
on `NoMatch` let IdleMarker judge idle. Per-tick cost is low (regex + active-region extraction).

**Q2 — probe NotCandidate routing.** Should lead to PROMPT_PENDING (unknown_prompt), but ONLY
after the seed-matcher filter. With corrected ordering, by the time the probe runs, the seed
matcher has already rejected the text, so no "go back to seed" loop is needed.

**Q3 — timing race is a logic-order problem, not a time-domain problem.** Fix with correct
decision ORDER (logic domain). "Wait for pane to stabilize" (time domain) is a fragile patch.
Frame-by-frame: with seed-first, every frame is safe (Frame1/2 no-match→wait; Frame3
menu-complete→Seed hits→dismiss).

**Q4 — dismiss keystroke.** Abandon digit selection; use **arrow-key navigation + Enter
(Down + Enter)**. The digit "2" leaks (inquirer.js-style select closes on digit, or "2" backs
up in the tty buffer and spills into chat input — observed as `↳ 2 ↳ 2`). Arrow keys are
leak-safe even if they spill. (NOTE: Down+Enter selecting "Skip" is a codex-TUI behavior to be
PROVEN via dogfood, not assumed — the audit should flag this as an empirical claim.)

## Master PM fact-check (SOP-07 supervision)

- gating.rs:90-104 IdleMarker-before-`match_prompt_for_scan`(:106): CONFIRMED.
- seeds.rs:17-21 codex_update_01 action = `Key("2")` + `Key("Enter")`: CONFIRMED (matches the leak).
- `match_prompt_for_scan` exists at gating.rs:106: CONFIRMED.
- Reordering interacts with: same-hash skip (gating.rs:78), the learned-prompt lookup currently
  nested inside the NoMatch branch (gating.rs:129), and the existing test
  `marker_like_text_does_not_become_unknown_when_marker_matches` (gating.rs:436) which encodes
  the present IdleMarker-first behavior — these are the audit's load-bearing concerns.

## Open audit questions (1d)

1. Does seed-before-idle break any existing invariant? Specifically the test at gating.rs:436,
   the SPAWNING empty-capture defer (gating.rs:64,460), and `ack_scan_ignores_startup_notification`
   (gating.rs:355) — do any of these depend on IdleMarker running first?
2. Where does the learned-prompt layer (gating.rs:129) sit in the new order — still inside
   match-NoMatch, before or after IdleMarker?
3. Per-tick cost: today IdleMarker short-circuits to avoid running the full seed regex set on
   every idle tick. Is running `match_prompt_for_scan` every tick acceptable (a2 says yes)?
   Any cheap guard (e.g. only on hash change — already covered by same-hash skip at :78)?
4. Down+Enter dismiss for codex's "1.Update now / 2.Skip / 3.Skip until next" select — is this
   the right keystroke, and does it select Skip (not trigger Update now's npm install)? Empirical;
   prove via dogfood.

---

# R-B3: banner-vs-menu cutover (dogfood-2 root cause, 2026-06-12)

## Triple-confirmed evidence

Dogfood-2 (`bsirz4pm5`, state `/tmp/ah-dfRBC`): R-B2 Down+Enter killed the "2" keystroke leak
(step 7 clean, step 8 IDLE), but step [9] resume-dispatch STILL wedged: DB
`PROMPT_PENDING reason=unknown_prompt` (seq=118, depth=1).

Three independent sources agree on the cause:

1. **DB event seq=119 `UNKNOWN_PROMPT_DETECTED.pane_screenshot`** (the exact snapshot classify
   decided on — events.rs:33+36 reuse the same `snapshot`):
   ```
   ╭─ ✨ Update available! 0.135.0 -> 0.139.0 ─╮
   │ Run npm install -g @openai/codex to update. │
   │ See full release notes: .../releases/latest │
   ╰──────────────────────────────────────────────╯
   ╭─ >_ OpenAI Codex (v0.135.0) … YOLO mode ─╮
     Tip: [tui.keymap] …
   › Improve documentation in @filename          ← IDLE composer (placeholder hint)
     gpt-5.5 default · /home/sevenx/coding/ccbd-rust
   ```
   This is the **non-interactive banner** variant: codex is ALREADY at the idle composer (`›`),
   the update notice is just scrollback. NOT a blocking menu.

2. **`journalctl --user -u ahd.service`** gate trace at the wedge:
   ```
   matcher matched case codex_update_01 → runner executing depth=0 (Down+Enter)
   matcher matched case codex_update_01 → runner executing depth=1
   matcher matched case codex_update_01 → runner executing depth=2
   …  runner same-hash stuck (same_hash_skips=4 > max_depth=3)
   orchestrator dispatch guard refused: unknown_prompt
   ```

3. **Code path** (runner.rs:240-247 same-hash-stuck → `Pending{block_reason:"unknown_prompt"}`):
   the seed matched the persistent banner every depth, Down+Enter is a no-op on an idle composer,
   the screen never changed → same-hash stuck → wedge.

## Root cause

The seed `codex_update_01` regex (seeds.rs:14) has a second alternation branch
`|npm\s+install\s+-g\s+@openai/codex` that matches the **informational banner** even when codex
is idle. The handler keeps "dismissing" a banner that is not a blocking prompt → no-op loop →
same-hash wedge → dispatch refused. The `›` ambiguity (R-B1/R-B2) was real but secondary; the
over-broad `npm install` seed branch is the dogfood-2 blocker.

## Decision (cutover, A-class — in service of locked resume续断点 goal)

Narrow `codex_update_01` to match ONLY the **interactive numbered menu** (the first alternation
branch: `1. update now … 2. skip … 3. skip until next version … press enter to continue`).
**Drop the standalone `npm install` branch.**

Resulting behavior (all evidence-backed):
- **Interactive menu** (codex blocked, cursor on `› 1. Update now`): seed first-branch matches →
  Down+Enter selects Skip → dismissed. (dogfood-1 variant)
- **Idle + banner** (composer `›` present): seed NoMatch → IdleMarker matches → confirm_can_input
  confirms (R-C bounded-retry, composer not destabilized) → NoActionNeeded → dispatch proceeds.
  (dogfood-2 variant)
- **Bare banner, composer not yet rendered** (startup transient): seed NoMatch → marker NoMatch →
  Unknown → `transient_unknown_prompt` stabilization defers until composer appears, then idle.

## Test cutover (flips the wrong contract)

These existing tests encode the now-disproven "bare banner → dismiss" assumption and MUST flip:
- matcher.rs:298 `codex_update_matches_builtin_skip_action` (banner-only → was Matched)
- matcher.rs:388 (banner + `ready\n  ›` → was Matched; now idle, NOT dismiss)
- matcher.rs:434 (startup-prefix + banner-only → was Matched)
- gating.rs:331 (banner-only → was KnownAction)
- gating.rs:389 (banner + `ready\n  ›` → was KnownAction; now IdleMarker/idle)
- seeds.rs:112 (regex.is_match banner-only → was true)

KEEP green (interactive menu still dismissed):
- matcher.rs:324 `codex_update_matches_live_skip_menu_action`
- matcher.rs:348 `..._when_install_command_wraps`
- gating.rs:354 `codex_resume_update_menu_seed_beats_idle_marker`

ADD: red→green test feeding the EXACT dogfood-2 banner screenshot → seed NoMatch → IdleMarker
idle path (NOT KnownAction, NOT unknown_prompt).

Verdict gate (goal closure): re-dogfood `/tmp/ah-dogfood-rbc-resume.sh` → step [9] ask2 rc=0
returns the recall token + step [10] DB has NO `unknown_prompt`/`depth_exceeded` PROMPT_PENDING.

## VERDICT — PROVEN via dogfood v2 (gated, real ah) 2026-06-12

`/tmp/ah-dogfood-rbc-v2.sh` (gates on true IDLE + 4s settle before dispatch, drives real
crash→CRASHED→resume→recall). State dir `/tmp/ah-dfRBCv2`, binary built 04:35 with R-B3.

Evidence (`/tmp/ah-dfRBCv2.out`):
- **step[3] dispatch after settle: `ask rc=0`, token `DOGFOOD-RBC2` echoed** — no `missing_api_key`,
  no banner over-match wedge. Confirms R-B3 narrowing works in the live daemon for the settled-idle
  composer + lingering banner case.
- **step[5] SIGKILL → `CRASHED`** detected by health worker (`ah ps` t+2s) — the real crash path.
- **step[6] `ah up` → recovery respawn `is_recovery=True`**; **step[8] recall `ask2 rc=0` returned
  the token** — 续断点 confirmed (resumed codex retained prior context).
- **step[9] DB lifecycle clean**: `IDLE→WAITING_FOR_ACK→IDLE (LOG_EVENT_TASK_COMPLETE)`,
  NO `unknown_prompt` / `depth_exceeded` / `PROMPT_PENDING`.

Unit gate (current worktree): `cargo test --lib prompt_handler::` = **91 passed / 0 failed**;
`cargo test --test pr4a_lifecycle_contract` = **9 passed / 0 failed**.

R-B (banner over-match) is CLOSED. The interactive-menu variant remains non-deterministic vs the
informational-banner variant (codex version-check timing); both are now handled (menu → Down+Enter
Skip; banner+idle → IdleMarker dispatch). Follow-up vigilance: if a future run hits the interactive
menu at resume, confirm Down selects Skip (not Update-now).

## Follow-ups surfaced by v2 (NOT R-B scope)

1. **Dispatch-before-settle `missing_api_key` race** (dispatch-guard robustness): when a dispatch
   guard scan hits a cold-spawn/recovery mid-render window (welcome boxes drawn, composer `›` not
   yet rendered), classification falls through seed→IdleMarker→learned→Unknown→LLM-fallback; with
   no LLM key (OAuth-only env) it wedges to `PROMPT_PENDING` instead of deferring. In production ah
   dispatches to settled IDLE agents, but the resumed/cold-spawn render window can still race. Candidate
   hardening: treat LLM-fallback `missing_api_key` during SPAWNING/recovery as a transient defer
   (re-scan) rather than a terminal PROMPT_PENDING. → fold into R-A recovery hardening.
2. **Recovery categorized as `DRIFT_REALIGN "config changed"`** rather than a first-class
   crash-recovery transition (no `CRASHED` `state_change` event recorded even though `ah ps` showed
   CRASHED live). Recovery works (is_recovery=True, context restored), but the eventing/categorization
   is the explicit subject of R-A (task #13: OOM auto-recovery trigger as a conscious path).
