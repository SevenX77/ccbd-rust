# Spike#3 Report: Existing Claude Credential-Failure Observability

Date: 2026-07-12
Scope: read-only static audit. No build/test, no live stack, no credential/network commands.

## Inputs Checked

- Design context: `.kiro/specs/ah-per-worker-credentials/design-rev.md` §3 "P2 如何被满足" and §6 "Spike#3".
- Code paths inspected:
  - `src/db/state_machine.rs`
  - `src/db/events.rs`
  - `src/db/events_progress.rs`
  - `src/db/jobs.rs`
  - `src/db/agents_lifecycle.rs`
  - `src/monitor/agent_watch.rs`
  - `src/agent_io/reader.rs`
  - `src/provider/init_probe.rs`
  - `src/provider/init_probe_task.rs`
  - `src/prompt_handler/*`
  - `src/rpc/handlers/agent.rs`
  - recovery adjuncts in `src/db/recovery.rs` and `src/orchestrator/mod.rs`

## Design Anchor Verification

- `STATE_IDLE` and `STATE_BUSY` are still at `src/db/state_machine.rs:16` and `:18`.
- `STATE_CRASHED` is at `src/db/state_machine.rs:26`. The design note saying `src/db/events_progress.rs:103` is not a definition; that line is a test assertion that states remain `["IDLE", "IDLE", "CRASHED"]`.
- Generic `state_change` events are inserted by state-machine/lifecycle mutations, e.g. `transit_agent_state_conn_inner` at `src/db/state_machine.rs:169-179`, marker-match idle at `src/db/state_machine.rs:505-516`, crashed at `src/db/agents_lifecycle.rs:201-212`.
- `events::insert_event` does call `mark_agent_idle_matched` on ah-idle marker output at `src/db/events.rs:237-265`.
- `mark_dispatched_job_cancelled_if_agent_idle_sync` is at `src/db/jobs.rs:618`, and its gate is exactly `st == "IDLE" || st == "UNKNOWN"` at `src/db/jobs.rs:643`.
- `agents.sub_state` already exists in schema at `src/db/schema.rs:82`, and runtime snapshots expose it at `src/runtime_events.rs:95-101`.

## Current Failure Paths

### 1. If Claude exits after credential failure

The spawned worker runs inside a tmux pane (`src/rpc/handlers/agent.rs:274-276`). ahd opens a pidfd and starts `spawn_agent_pidfd_watch_task` (`src/rpc/handlers/agent.rs:305-438`).

When the process dies, `src/monitor/agent_watch.rs:68-93` reads an optional exit code and calls `mark_agent_crashed_with_exit`. That writes:

- `agents.state = 'CRASHED'`
- `agents.exit_code = <optional code>`
- `agents.error_code = 'AGENT_UNEXPECTED_EXIT'`, or event reason `EXIT_CODE_UNAVAILABLE_NON_CHILD` when no exit code is available
- a `state_change` event with `{from,to:"CRASHED",reason,exit_code}` (`src/db/agents_lifecycle.rs:177-212`)

So a clean process exit is observable, including an exit code if pidfd/waitid can retrieve it. It is not credential-specific. There is no mapping from a Claude auth stderr string or exit code to `CRED_INVALID`.

### 2. If Claude prints an auth/login message but stays alive

Pane output is piped to a FIFO and read by `spawn_agent_io_reader_task_with_config`. Every chunk is inserted as an `output_chunk` event with raw text at `src/agent_io/reader.rs:140-158`.

This preserves the text if Claude prints something like "not logged in" or "run /login", but current code does not parse output chunks for credential/auth phrases. The only live semantic scan in the reader is the idle marker scan (`src/agent_io/reader.rs:166-207`).

Result: raw output is available in events, but credential failure remains opaque to the agent state machine unless another generic detector fires.

### 3. During startup

Claude startup readiness is only the TUI-ready predicate in `src/provider/init_probe.rs:27-32`: banner gone, prompt present, and a model marker (`Sonnet`/`Haiku`/`Opus`) present.

`init_probe_task` may:

- mark IDLE after seed readiness (`src/provider/init_probe_task.rs:540-560`);
- scan generic prompts via prompt-handler (`src/provider/init_probe_task.rs:143-228`, `:470-504`);
- mark `SPAWNING_INTERVENTION` with `UNKNOWN_PATTERN_STABLE` for stable unknown startup screens (`src/provider/init_probe_task.rs:236-265`);
- mark `UNKNOWN` on startup marker timeout via `mark_agent_unknown(..., "STARTUP_MARKER_TIMEOUT", ...)` (`src/marker/timer.rs:126-155`, `src/db/state_machine.rs:1297-1367`).

None of these classify auth failure. A stable credential error screen can become an unknown startup/prompt condition, but it is not distinguishable as credentials.

### 4. During a dispatched job

`agent.send` CASes to `WAITING_FOR_ACK`, writes text to the pane, then starts a BUSY marker timer (`src/rpc/handlers/agent.rs:1098-1122`, `:1190-1222`). If no completion/idle signal arrives:

- prompt scan can defer during active dispatch instead of parking (`src/prompt_handler/integration.rs:127-143`);
- the BUSY timeout eventually marks `STUCK` (`src/marker/timer.rs:91-124`, `src/db/state_machine.rs:1185-1267`);
- pane diff / health paths also feed generic stuck behavior.

Again, this is observable but not credential-specific.

## Answer 1: Is Credential Failure Observable Today?

Partially, only through generic signals:

- observable as `CRASHED` plus optional `exit_code` if Claude exits;
- observable as raw `output_chunk` text if Claude prints an auth/login error;
- observable as `UNKNOWN`, `SPAWNING_INTERVENTION`, `UNKNOWN_PROMPT_DETECTED`, or `STUCK` if generic startup/prompt/hang logic catches the screen;
- observable in job failure if generic crash/stuck/unknown paths fail the dispatched job.

There is no existing `cred_failure`, `CRED_INVALID`, auth-specific error code, stderr parser, or 401/invalid_grant signal in the ah state machine. If Claude hangs or sits on a login/auth screen, current Layer 3 would see only an opaque generic state, not a credential-invalid state.

## Answer 2: Best Insert Points for `CRED_INVALID` / `cred_failure`

Recommended minimal insertion is an explicit DB/state helper plus one or more detectors:

1. Add a state mutation/event helper in `src/db/state_machine.rs`.
   - Pattern after `mark_agent_unknown_sync` (`src/db/state_machine.rs:1297-1367`) or prompt-pending transaction (`src/prompt_handler/integration.rs:670-743`).
   - Suggested behavior: CAS current state in a conservative allow-list, set `sub_state = 'CRED_INVALID'`, optionally set `error_code = 'CRED_INVALID'`, and insert `events(event_type='cred_failure', payload={...})`.
   - If the desired UI contract is a visible state change, insert a companion `state_change` event with same state and new `sub_state`, because event backfill already maps arbitrary event types but `state_change` is the established status timeline.

2. If using raw pane-output detection, hook `src/agent_io/reader.rs` after chunk creation and before/after `insert_event` (`src/agent_io/reader.rs:140-163`).
   - This sees both stdout/stderr as rendered in the tmux pane.
   - It can match explicit Claude auth/login strings once Spike#2 provides real strings.
   - It is the lowest-level passive detector for a live process that does not exit.

3. If using startup-screen classification, hook `src/provider/init_probe_task.rs`.
   - Before generic prompt handling around `src/provider/init_probe_task.rs:143-228`, inspect the captured pane for confirmed credential-failure patterns and call the DB helper.
   - This avoids waiting for generic UNKNOWN timeout when the failure is already visible at spawn.

4. If using process-exit classification, hook `src/db/agents_lifecycle.rs` only after there is reliable evidence.
   - `mark_agent_crashed_sync` currently receives only `exit_code` and a generic reason (`src/db/agents_lifecycle.rs:151-212`), not stderr text.
   - To classify here, the caller would need to pass a prior detector result or recent pane/output evidence. Exit code alone is probably insufficient unless Spike#2 proves a stable unique code.

If choosing only one insertion point before Spike#2 real-output evidence exists, add the DB helper first and wire detection later. If choosing one detector after Spike#2, prefer `agent_io/reader.rs` for live visible auth strings, plus `init_probe_task.rs` for faster startup classification.

## Answer 3: Can Layer 3 Reuse the `mark_dispatched_job_cancelled_if_agent_idle_sync` Gate?

It can reuse the transaction/CAS pattern, but not the function directly.

What is reusable:

- short immediate transaction;
- query the current agent/job row under the transaction;
- act only if the observed state is still idle-ish;
- return `0` on mismatch instead of forcing state;
- notify/wake only when changes occur.

Hidden incompatibilities:

- The existing function is job-specific. It requires a dispatched job with `cancel_requested`, then transitions that job to `CANCELLED` (`src/db/jobs.rs:633-651`). Layer 3 restart is agent-scoped and may have no dispatched job.
- Its idle definition includes `UNKNOWN` (`src/db/jobs.rs:643`). That is safe for settling a requested cancel, but risky for credential restart. `UNKNOWN` can mean startup timeout or opaque evidence state, not necessarily no in-flight work. For Layer 3 "IDLE 时重启", use `state = 'IDLE'` as the default gate unless design explicitly authorizes UNKNOWN.
- Existing worker recovery is CRASHED-oriented. `try_claim_agent_recovery_sync` only claims `state = 'CRASHED'` (`src/db/recovery.rs:604-623`), and atomic replacement requires the row to be `KILLED` (`src/db/recovery.rs:451-481`). There is no current primitive for "idle live agent restart from IDLE".
- `agent.kill` marks `KILLED`, sends SIGKILL, and removes the sandbox dir (`src/rpc/handlers/agent.rs:796-835`). Directly using it for credential restart would likely discard the home that Layer 1 just refreshed unless a restart-specific cleanup policy is added.
- `ReplaceKilledAndRequeue` can respawn from a stored `AgentSpawnSpec`, but it assumes a KILLED row and is tied to recovery/realign flows (`src/rpc/handlers/agent.rs:378-399`, `src/db/recovery.rs:434-499`).

Conclusion: implement a new `restart_agent_if_idle` / `claim_cred_restart_if_idle` helper that copies the gate style, probably `WHERE id = ? AND state = 'IDLE' AND state_version = ?`, records `cred_failure`, then performs a controlled kill/re-spawn path using the stored `AgentSpawnSpec`. Do not call `mark_dispatched_job_cancelled_if_agent_idle_sync` directly, and do not inherit its `UNKNOWN` allowance without an explicit additional no-in-flight proof.

## Final Conclusion

P2 is not satisfied as a credential-specific observable contract today. The system has enough generic observability to avoid total black holes in many cases, but credential invalidation is currently opaque unless a human or future detector interprets raw output or generic `CRASHED`/`UNKNOWN`/`STUCK` states.

For Layer 3, add a dedicated `cred_failure` event and/or `sub_state = CRED_INVALID` at the state-machine boundary, then feed it from raw pane/startup detectors once Spike#2 provides real Claude failure strings/exit behavior. Reuse the idle-gated transaction pattern, not the existing job-cancel function.
