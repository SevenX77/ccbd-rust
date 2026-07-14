# Incident: claude-provider worker marked COMPLETED while background work still running (2026-07-08)

## Phenomenon
Worker a4 (claude) was dispatched the PR1 e2e job (`job_fc6ed4bd`). At **653s the job was marked COMPLETED**, but a4's reply tail literally said *"cargo test still running; I'll wait for the background waiter's completion notification."* At that moment a4's pane still had **2 live shells** (the serial `cargo test` full-regression run had NOT finished).

## Mechanism (high confidence, operator-diagnosed)
- claude issued a genuine `end_turn` — ending its turn to await a background-task completion notification (its normal pattern for long background work).
- ah's **completion detection interpreted `end_turn` (with background work still in flight) as task completion**, closing the job.
- Once the job is closed, a4's subsequent output (when its harness wakes it on cargo exit) does **not** flow into the original job's reply.

This is a **claude-provider premature-completion** case. Distinct source from the earlier codex U+2022 premature-completion (different root cause; same class of "sibling job judged done too early").

## Impact this round
- a4's e2e TESTS did pass (7/7: 4 e2e + 3 harness), committed as `1b949f5` on `feat/state-contract-pr1-schema-v2`.
- But a4's **full-serial regression confirmation** and its **final e2e verdict** were never captured — the reply is not the final output.
- Master must NOT treat that reply as the e2e conclusion of record.

## Handling (this round)
- PR1's e2e conclusion is taken from a4's **final** output (collected via a fresh job after a4's pane genuinely finishes), NOT the premature reply. PR1 does not enter the merge report until then.
- Master independently ran the full serial suite to confirm no regression (supplementary, not a substitute for a4's verdict).

## Fix direction (backlog — orchestration-reliability; NOT drive-fixed in state-contract PR1-4)
Completion detection treats "end_turn + background task still running" as done. Design options:
- Worker rule: forbid `end_turn` while a background shell/job is still live (worker must foreground-wait or poll to true completion before ending the turn); OR
- Completion detection becomes shell-aware: a job is not COMPLETE while the worker's pane has live child shells / running processes it spawned.
Decide in the orchestration-reliability track.
