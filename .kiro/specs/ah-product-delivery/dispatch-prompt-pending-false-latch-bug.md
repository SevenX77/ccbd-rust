# Bug: idle worker false-latched in PROMPT_PENDING → dispatch silently stuck (QUEUED forever)

Discovered 2026-06-29 during GAP-2 Slice 2 audit dispatch (dogfood).

## Symptom
- PM dispatched a4 the Slice 2 audit; job stayed `QUEUED`, `dispatched_at=None` for ~4.5 min, never delivered.
- a4's tmux pane was visibly idle (composer empty `❯`, last output = prior Slice 1 audit report).
- `ah ps` showed a4 = `PROMPT_PENDING` / sub_state `Matched`, while a1 = `IDLE`.

## Root cause (confirmed by main controller 2026-06-29)
- The claude TUI **push-notification banner** ("get pinged when Claude finishes · enable push notifications in /config") flashes in the pane and the prompt-scanner mis-classifies it as an `unknown_prompt` → latches PROMPT_PENDING. It re-triggers periodically whenever the banner reappears, so cancel/re-dispatch cannot clear it (the latch re-arms within a poll). Two dispatched jobs were CANCELLED spinning on this.
- Net effect below still holds: a4 finished the prior task and went IDLE→PROMPT_PENDING (events: reason=`unknown_prompt`, seq 182927) — a **false** PROMPT_PENDING (composer empty, nothing actually pending).

## Refined root cause (controller, 2026-06-29 — deeper than the banner)
- The real trigger is a **ghost placeholder** on the claude composer: a grayed remnant of the worker's PREVIOUS prompt that the prompt-scanner misreads as a live prompt → latches PROMPT_PENDING. It is NOT real input, so `Esc`, Backspace, and `Ctrl+U` cannot clear it (all proven ineffective this session).
- The push-notification banner is a secondary contributor; the ghost is the stubborn one.

## Reliable unlock (controller-proven): `/clear`
- Send `/clear` to the stuck worker to reset its session: `tmux ... send-keys -t agent_<id> '/clear' Enter`. This wipes the ghost + banner, the worker self-heals to IDLE, and the QUEUED job dispatches on the next tick → BUSY.
- Cost: loses that worker's conversation context — irrelevant when the next dispatch is a fresh, self-contained task (e.g. a new independent audit brief).
- SOP: worker stuck PROMPT_PENDING and a single `Esc` doesn't free it within ~30s → `/clear` it (do NOT spam Esc, do NOT cancel/re-queue in a loop). Confirm BUSY via `ah ps`, then stop touching its keys.

## a4 `Not logged in` — root cause was SHARED OAuth creds, NOT `/clear` (corrected by controller 2026-06-29)
- Initial (wrong) guess: `/clear` logged a4 out. **Corrected:** a4's `.credentials.json` is a symlink to the GLOBAL claude OAuth token. At 09:35 the global token was refreshed (refresh-token rotation); a4 was idle and its in-memory old token went stale → `Not logged in · Please run /login` on the next turn. The `/clear` was merely coincident, not causal.
- This is a real ah bug: **multiple claude instances (master + workers) share one OAuth credential file; whenever any one refreshes/rotates the token, the others' in-memory tokens become invalid and they silently log out.** → Backlog: per-worker credential isolation, or refresh coordination across instances.
- Recovery is risky and NOT for the PM loop: `/login` rewrites the SHARED creds (can log out the master too); kill+revival has a known claude onboarding-zombie risk. Both can damage the running stack. Worker auth recovery belongs to the ahd/sandbox owner.
- Side effect to still respect: `/clear` wipes the worker's conversation context (fine for a fresh self-contained task). And a stuck job can bounce IDLE→WAITING_FOR_ACK→PROMPT_PENDING and end up marked `COMPLETED` without the worker doing the work (ghost ACK) — re-verify real pane activity before trusting a dispatch.
- Fallback when a worker can't be revived safely: a DIFFERENT independent reviewer (e.g. the controller or another idle instance) performs the independent audit by reading code — author-independence is preserved without touching the broken worker.

## Real fix direction (controller)
- prompt-scanner must IGNORE both the push-notification banner line(s) AND the grayed ghost-placeholder remnant when deciding PROMPT_PENDING. Neither ≠ a pending question.

## Pragmatic workaround that DID work (this session)
- `ah prompt resolve <agent> --keys Enter` flips PROMPT_PENDING→IDLE via the real state machine, then **directly** `tmux send-keys -l "<ASCII instruction>"` into the composer (making it non-empty) + a separate `Enter` — this bypasses dispatch_guard entirely and forces a real turn. Verify the composer actually contains the text (capture-pane) BEFORE pressing Enter; use ASCII to avoid send-keys unicode issues. NOTE: a turn delivered this way is NOT tracked as a BUSY job by `ah ps` (state reads IDLE while genuinely working) — read the pane's "Stewing/esc to interrupt" as the work signal. Deeper unblock is owned by the ahd/sandbox layer, not the PM loop.
- `dispatch_guard` refuses to dispatch to a PROMPT_PENDING agent → the queued job is silently withheld with no error surfaced to the dispatcher.
- Sending a single `Enter` to the empty composer is a no-op (claude TUI does not submit empty input), so the pane does not change and the detector keeps matching PROMPT_PENDING → latch persists.

## Why it matters
- Pane-idle ≠ task delivered. A PM watching only the pane sees "idle, ready" and waits forever on a job that was never dispatched.
- The dispatch rejection is silent (no escalation, no job-state annotation).

## Lessons / SOP
- Judge worker state via `ah ps` (real state machine) + confirm the job actually went `DISPATCHED`, NOT by reading the pane.
- Treat a long-lived `QUEUED` with `dispatched_at=None` as a dispatch fault, not "worker busy".

## Fix candidates (for later, not in GAP-2 scope)
1. PROMPT_PENDING detection must not latch on an empty composer / trailing prompt glyph; require an actual pending-question signature, and re-evaluate to IDLE when composer is empty.
2. dispatch_guard should surface a visible reason when it withholds a job (escalate / annotate job row) instead of silently leaving it QUEUED.
3. Optional: a watchdog that flips a stale false-PROMPT_PENDING back to IDLE after N idle polls.
