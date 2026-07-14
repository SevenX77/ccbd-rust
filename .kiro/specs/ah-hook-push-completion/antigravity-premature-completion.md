# Antigravity Premature Completion: Root Cause + Fix Plan

Status: diagnosis/design only. No product code changes in this task.

Incident: a3 (`antigravity`) ran e2e work and ah marked the job `COMPLETED`, but `jobs.reply_text` only contained an interim sentence meaning "waiting for background cargo test". The real final conclusion, including `5 passed` and materialization-shape checks, remained only in the tmux pane and never updated the job record.

## Evidence Read

Read before diagnosis:

- `.kiro/specs/ah-hook-push-completion/design.md`
- `.kiro/specs/ah-hook-push-completion/step9-fix-research.md`
- `.kiro/specs/ah-hook-push-completion/SUPERVISOR-EVIDENCE-step9.md`
- `.kiro/specs/ah-hook-push-completion/a1-confirm-recapture-matcher.md`
- `.kiro/specs/ah-hook-push-completion/a1-2tick-recapture-test.md`
- `git log origin/main..origin/feat/ah-hook-push-completion --oneline`
- `src/provider/manifest.rs`
- `src/marker/matcher.rs`
- `src/agent_io/reader.rs`
- `src/pane_diff/mod.rs`
- `src/completion/log_layout.rs`
- `src/completion/monitor.rs`
- `src/completion/parser.rs`
- `src/db/state_machine.rs`
- `src/db/jobs.rs`
- `src/orchestrator/mod.rs`

Branch facts:

- `origin/feat/ah-hook-push-completion` has 13 commits over `origin/main`; the visible subjects include hook push, antigravity recapture backstop, codex hook fixes, and lifecycle reliability handoff.
- The old step-9 evidence said antigravity had no log signal and fell back to UI-only completion. Current code has moved forward: `src/completion/parser.rs` now has an antigravity parser, and `src/provider/manifest.rs` sets antigravity `completion_signal: LogAndUi`.
- Despite that, `src/pane_diff/mod.rs` still forces UI completion recapture for `matches!(provider, "antigravity" | "codex")`, independent of `CompletionSignalKind`.

## Current Completion Signals

Antigravity can currently be completed by three classes of signal:

1. **Log monitor path**
   - Dispatch calls `prepare_log_monitor_before_send` in `src/orchestrator/mod.rs`.
   - Antigravity log root is `<sandbox home>/.gemini/antigravity-cli` in `src/completion/log_layout.rs`.
   - `src/completion/parser.rs` treats an antigravity JSON line as terminal when:
     - `source == "MODEL"`
     - `type == "PLANNER_RESPONSE"`
     - `status == "DONE"`
     - `tool_calls` is empty
   - That terminal event marks the agent `IDLE/LogEvent` and completes the job using the `content` field as reply.

2. **Live FIFO marker path**
   - `src/agent_io/reader.rs` scans the live vt100 parser after output chunks.
   - `src/marker/matcher.rs` says antigravity is idle when the viewport bottom contains `? for shortcuts`.
   - Antigravity anti-pattern is only `(?m)^\s*esc to cancel\b`.
   - On match, `mark_agent_idle_matched` may complete the dispatched job from screen-derived reply text, unless a completion log monitor is active.

3. **Pane-diff UI recapture path**
   - `src/pane_diff/mod.rs` captures busy/stuck panes periodically.
   - It applies the same antigravity marker and requires stable matching content for the default 2 ticks.
   - If it fires, `mark_agent_idle_recaptured_with_pane` completes the job from pane-derived reply text, unless the state machine defers because a log monitor is still registered.

Codex/Claude differ because their healthy path is log authoritative:

- Codex parser waits for `task_complete` with `last_agent_message`.
- Claude parser waits for assistant message `stop_reason` such as `end_turn` and extracts text content.
- Marker completion is suppressed while the log monitor is active.
- Codex also recently gained anti-patterns for hook trust modal and working spinner: `esc to interrupt|Hooks need review|Trust all and continue|Continue without trusting`.

Antigravity is weaker in two ways:

- Its terminal log event is `PLANNER_RESPONSE DONE`, which is a provider turn boundary, not proof that shell/background work has ended.
- Its UI idle marker is just the input status line. A returned input prompt proves the model is ready for another prompt; it does not prove user-assigned work is semantically done.

## Root Cause

One-sentence root cause:

> ah currently equates antigravity provider-turn completion or input-prompt idleness with job completion, but antigravity can return to the prompt after launching or referring to background work; ah has no negative gate for "the agent says tests are still running / waiting for background cargo", so it freezes the interim reply into `jobs.reply_text`.

Detailed failure chain for this incident:

1. The agent started or observed a long `cargo test` run in the background.
2. Antigravity produced an interim answer like "waiting for background cargo test" and returned to an idle-looking UI state, or logged a `PLANNER_RESPONSE DONE` for that interim model turn.
3. ah accepted that as terminal through either `LogEvent` or UI `Matched`/recapture.
4. `mark_job_completed_conn_sync` wrote `jobs.status='COMPLETED'` and `reply_text=<interim sentence>`.
5. Later pane output contained the real final result, but completed jobs are not reopened or amended by later output chunks.

The exact signal in the observed job should be confirmed from the latest `state_change` payload:

- `sub_state=LogEvent` means the antigravity transcript parser accepted `PLANNER_RESPONSE DONE` too early.
- `sub_state=Matched` with reason `UI_COMPLETION_RECAPTURE_MATCHED` or `MARKER_MATCHED` means the prompt marker/UI recapture accepted an idle prompt too early.
- Either way, the product bug is the same: the completion criterion proves "turn idle", not "task done".

## Why The Existing Guards Did Not Save It

- `is_prompt_only_reply` only rejects empty/prompt-only replies. The interim sentence is real content, so it passes.
- `esc to cancel` only catches Antigravity's visible busy status. A background `cargo test` can continue without that status line being present in the provider TUI.
- 2-tick pane stability only proves the screen stayed unchanged. It does not prove the background command ended.
- The antigravity log parser ignores tool calls when present, but it does not track "open subprocess/background work" or deferred-work language.
- Hook push Stop, if active, would still fire on provider turn end. It would not solve background-work completion by itself.

## Fix Strategy

The fix should change the terminal condition, not only tune regexes.

### 1. Make antigravity log/hook terminality stricter

Add a terminality gate before `mark_agent_idle_log_event` / `mark_agent_idle_hook_event` / UI recapture can complete an antigravity job.

Initial gate:

- If provider is `antigravity` and candidate reply contains deferred-work language, do not complete.
- Match both English and Chinese forms:
  - `waiting for`, `still running`, `running in the background`, `background cargo`, `I'll wait`, `will report`, `I'll update`, `once it finishes`
  - `等待`, `等后台`, `后台.*跑`, `还在跑`, `仍在运行`, `跑完后`, `稍后回报`, `完成后.*报告`
  - Test-specific anchors: `cargo test`, `5 passed` absent while waiting phrase present.
- The gate should be provider-specific initially; do not globally block Claude/Codex without separate evidence.

Disposition when gated:

- Keep the job `DISPATCHED`.
- Keep or return agent to an active non-terminal state (`BUSY` is acceptable).
- Insert a `state_change` or `completion_deferred` event with reason `ANTIGRAVITY_DEFERRED_BACKGROUND_WORK`.
- Nudge the agent in the pane with a corrective prompt:
  - "The job is still open. Wait for the background command to finish, then report the final test result. Do not stop at 'waiting for cargo test'."
- Do not dispatch a new job to that agent until it produces a terminal reply.

Implementation note:

- Existing evidence-denial paths already demonstrate the pattern of refusing completion and nudging. Reuse that shape rather than inventing a parallel scheduler.

### 2. Make UI recapture a last-resort fallback for LogAndUi providers

Current `provider_uses_ui_completion_recapture` returns true for antigravity regardless of `completion_signal`.

Change policy:

- For `CompletionSignalKind::LogAndUi`, UI recapture must not complete while the log monitor is alive.
- After log monitor timeout/cancel, UI recapture may run as a fallback.
- For antigravity, add a longer grace before UI recapture completes after a fresh idle marker, because `? for shortcuts` is only an input-ready marker.

Acceptance expectation:

- If an antigravity log root and parser are available, a `PLANNER_RESPONSE DONE` with deferred-work text is deferred, and UI recapture cannot immediately override it to completed.
- If no log root exists, UI recapture still works for simple one-shot replies like the existing `charlie` / `delta` fixtures.

### 3. Strengthen antigravity busy anti-patterns

Extend the antigravity anti-pattern and pane-diff busy detection to cover visible deferred work:

- Status/UI anchors:
  - `esc to cancel`
  - `Thinking`
  - `Working`
  - spinner/braille glyphs if present
- Deferred-work text anchors:
  - waiting/background phrases above
  - `cargo test` combined with waiting/running language

This is not sufficient alone, but it prevents the obvious screen-marker path from completing while the pane visibly says work is still in progress.

### 4. Prefer provider transcript final answer when available

For antigravity logs, split parse results:

- `PlannerDoneInterim`: `PLANNER_RESPONSE DONE` with deferred-work text.
- `TurnComplete`: `PLANNER_RESPONSE DONE` that passes terminality gate.

Only `TurnComplete` should call `mark_agent_idle_log_event`. `PlannerDoneInterim` should update log monitor state but keep waiting.

If Antigravity transcript has a later stronger event for tool/process completion, parser should key on that instead of plain planner DONE. If that event shape is unknown, add a dogfood capture task to collect transcript lines around:

- user prompt
- command launch
- interim "waiting for cargo"
- final `5 passed`

### 5. Add observability for the exact winning signal

When a job completes, the terminal state event must make the winning signal visible:

- provider
- source: `log` / `hook` / `marker` / `ui_recapture`
- reply_source
- raw path and offset for log events
- pane content hash for UI recapture
- terminality gate disposition: `accepted` or `deferred`

This lets the next incident answer "which signal completed the job?" without reading tmux manually.

## Implementation Tasks

### T1: Regression fixture for this incident

Add a test fixture that simulates antigravity returning:

```text
等后台 cargo test 跑完，我再看最终结果。
? for shortcuts ... Gemini ...
```

Then later output contains:

```text
test result: ok. 5 passed
物化形状核对全绿
```

Acceptance:

- The first pane/log candidate must not complete the job.
- The job stays `DISPATCHED`.
- A deferral event is recorded.
- The later final candidate completes the job with a reply containing `5 passed`.

Suggested tests:

- `antigravity_deferred_background_cargo_reply_does_not_complete_from_log`
- `antigravity_deferred_background_cargo_reply_does_not_complete_from_ui_recapture`
- `antigravity_final_test_result_completes_after_deferred_candidate`

### T2: Terminality classifier

Add a small pure classifier, ideally near completion parsing/state-machine code:

```text
CompletionTerminality::Terminal
CompletionTerminality::DeferredBackgroundWork { reason }
```

Inputs:

- provider
- candidate reply
- optional pane snapshot
- optional prompt text

Acceptance:

- Antigravity deferred cargo/waiting phrases classify as deferred.
- Simple antigravity answers from existing real fixtures classify as terminal.
- Codex/Claude behavior is unchanged unless explicitly opted in.

### T3: Gate log/hook/UI completion

Apply the classifier before terminal state transition:

- `mark_agent_idle_log_event_outcome_sync`
- `mark_agent_idle_hook_event_outcome_sync`
- `mark_agent_idle_recaptured_outcome_sync_with_pane_inner`
- marker path if it can complete antigravity jobs without log monitor

Acceptance:

- A gated candidate cannot call `mark_job_completed_conn_sync`.
- The state machine returns enough information for caller to nudge the agent or at least log an actionable event.

### T4: Nudge on deferred background work

Add caller behavior to send a corrective prompt after deferral.

Acceptance:

- A fake writer test or state-machine wrapper test proves the nudge text is emitted once per deferred job/candidate hash.
- No tight loop: repeated identical interim panes do not spam the agent every watcher tick.

### T5: UI recapture policy cleanup

Replace the current hard-coded `matches!(provider, "antigravity" | "codex")` with a policy that distinguishes:

- `UiOnly`: UI recapture can be primary fallback.
- `LogAndUi` with active log monitor: UI recapture observes but does not complete.
- `LogAndUi` after log timeout/cancel: UI recapture can complete if terminality gate passes.

Acceptance:

- Existing `REAL-a3-*` simple reply tests continue to pass in no-log scenarios.
- New deferred-background tests fail before the fix and pass after it.
- Codex trust-modal and working-spinner anti-pattern tests stay green.

## Validation Commands For Implementation

```text
CARGO_BUILD_JOBS=1 cargo test --lib antigravity_deferred_background -- --test-threads=1
CARGO_BUILD_JOBS=1 cargo test --lib completion::parser -- --test-threads=1
CARGO_BUILD_JOBS=1 cargo test --lib pane_diff -- --test-threads=1
CARGO_BUILD_JOBS=1 cargo test --test ah_full_e2e_drift -- --test-threads=1
```

If implementation adds a focused integration test file, prefer:

```text
CARGO_BUILD_JOBS=1 cargo test --test antigravity_completion_terminality -- --test-threads=1
```

## Non-Goals

- Do not remove antigravity UI recapture; it is still valuable for burst output and missing output chunks.
- Do not rely on hook push alone; Stop hooks signal provider turn end, not necessarily task completion.
- Do not globally block every reply containing "cargo test"; only defer when the reply says the test is still running or will be reported later.
- Do not reopen already completed jobs as the primary fix. The safer invariant is "do not complete until terminality passes".

## Commands Run For This Diagnosis

```text
git log origin/main..origin/feat/ah-hook-push-completion --oneline
find .kiro/specs/ah-hook-push-completion -maxdepth 1 -type f | sort
rg -n "completion|idle|busy|anti-pattern|antipattern|anti_pattern|hook_push|Stop|PROMPT|prompt|AGENT|antigravity|gemini|cargo|background|pane|reply_text" src tests .kiro/specs/ah-hook-push-completion -S
git branch -r | rg "ah-94|idle|completion|codex|hook"
rg -n "collect_reply_for_dispatched_job_sync|distill_reply|is_prompt_only_reply|contains_prompt_text|reply_text|mark_job_completed" src/db src -S
rg -n "PLANNER_RESPONSE|USER_EXPLICIT|tool_calls|terminal|antigravity|background|cargo test|status.*DONE|TOOL|COMMAND|EXEC" .kiro src tests research -S
```
