# Research: Corrected Master-Death Semantics

## PM-Corrected Semantics

When a master process exits unexpectedly, the intended invariant is:

1. First clean all workers/processes owned by that master/session, unconditionally.
2. If the master had in-flight work, relaunch the master and resume/continue the conversation.
3. If the master was idle, do not relaunch it.

This corrects the current PR #52 behavior, which revives the master while keeping existing workers alive. That behavior prevents data loss for the master but can accumulate zombie or orphan workers after master death.

## Current Code Path

`src/monitor/master_watch.rs:20` registers the master pidfd watcher with `expected_pid`, `expected_generation`, original `master_cmd`, `state_dir`, `EnvState`, and daemon unit context.

On pidfd readiness, `src/monitor/master_watch.rs:51` calls `classify_master_death`. If the decision is `Revive`, it calls `revive_master_after_exit`; `IntentionalExit` and `Stale` are ignored.

`src/monitor/master_watch.rs:86` implements the revive path. It:

- takes the per-session master spawn lock at `src/monitor/master_watch.rs:98`;
- respects retry backoff at `src/monitor/master_watch.rs:101`;
- claims a generation transition at `src/monitor/master_watch.rs:127`;
- records a revive attempt at `src/monitor/master_watch.rs:140`;
- reconstructs the master sandbox HOME/env at `src/monitor/master_watch.rs:173`;
- spawns a replacement master tmux window at `src/monitor/master_watch.rs:204`;
- writes the new pid/pane/generation and registers a fresh pidfd watcher at `src/monitor/master_watch.rs:221`.

There is no worker cleanup before or after the revive in this path.

`src/master_revival.rs:62` classifies master death using only `sessions.status`, `master_pid`, and `master_generation`: active + pid/generation match means `Revive`; non-active means `IntentionalExit`; mismatch/missing means `Stale`.

`src/master_revival.rs:96` claims a revive by incrementing `master_generation` under a CAS on active status, pid, and generation.

`src/master_revival.rs:236` records retry/backoff. On fuse, it marks the session failed and kills agents via lifecycle marking, but that is not the normal master-death cleanup path.

## PR #52 Change From Previous Behavior

`git diff main...feat/ahd-master-revive-oom --stat` shows 18 files changed, including `src/master_revival.rs` added, `src/monitor/master_watch.rs` heavily rewritten, new session revive columns in schema/migration, and `tests/r1_master_exit_shutdown.rs` rewritten for revive semantics.

The key semantic replacement is in `src/monitor/master_watch.rs`:

- old behavior logged "master process exited, cascading session kill";
- called `db::system::cascade_kill_session_agents_for_daemon(..., "MASTER_EXIT", ...)`;
- killed agent panes and tmux sessions;
- scheduled daemon shutdown if idle;
- removed the master monitor key.

The new behavior classifies and revives. It no longer calls the cascade cleanup path on normal unexpected master death and removed the daemon-wide active-agent shutdown check.

`src/db/schema.rs:15` adds `master_retry_count`, `master_next_retry_at`, `master_generation`, and `master_last_exit_reason` to sessions. `src/db/mod.rs` migrates the same columns.

`tests/r1_master_exit_shutdown.rs` now asserts the active raw master exit revives the master, keeps the worker active, and keeps the daemon alive. That test contract is directly opposite to the corrected "clean workers first" invariant.

## Existing Cleanup Primitives

`src/db/system.rs:144` has the closest reusable cleanup primitive: `cascade_kill_session_agents_with_runner_sync`.

Its behavior:

- marks the session `KILLED` if it was `ACTIVE` at `src/db/system.rs:151`;
- if already `KILLED` or `FAILED`, it still continues cleanup at `src/db/system.rs:159`;
- selects all agents in that session whose state is not `CRASHED` or `KILLED` at `src/db/system.rs:175`;
- stops agent systemd scopes and the session anchor when a daemon marker is available at `src/db/system.rs:189`;
- sends pidfd SIGKILL fallback at `src/db/system.rs:199`;
- marks each agent killed and cancels marker/parser registry state at `src/db/system.rs:206`.

`src/rpc/handlers/sessions.rs:74` uses this path for `session.kill`. It first calls `mark_session_intentional_killed` at `src/rpc/handlers/sessions.rs:85`, then calls `cascade_kill_session_agents_for_daemon` at `src/rpc/handlers/sessions.rs:91`, then kills panes, tmux sessions, sandboxes, master pane/session.

This primitive currently conflates "clean all workers" with "mark the whole session KILLED". Corrected master-death semantics need worker cleanup while keeping the session eligible for master relaunch in the in-flight case. That means either:

- split the worker cleanup operation from session terminal-state mutation; or
- add a new cleanup mode/reason that cleans agents/scopes without making the session terminal before revive classification completes.

Reusing `session.kill` as-is would block later revive because `classify_master_death` treats non-`ACTIVE` sessions as intentional exits.

## Busy vs Idle Signals

There is no single reliable master busy/idle field today.

Available data:

- `sessions.status` and master runtime columns in `src/db/schema.rs:8` track session liveness and master pid/generation, not master activity.
- agent states in `src/db/state_machine.rs:14` include `SPAWNING`, `WAITING_FOR_ACK`, `BUSY`, `PROMPT_PENDING`, `IDLE`, `CRASHED`, `KILLED`, and others.
- `is_active_state` in `src/db/state_machine.rs:59` treats `SPAWNING`, `WAITING_FOR_ACK`, and `BUSY` as active execution states.
- jobs in `src/db/schema.rs:82` have `QUEUED` and `DISPATCHED`; `src/db/jobs.rs` claims jobs as `DISPATCHED` and can query dispatched jobs per agent.

Risk: these signals describe worker/job activity, not necessarily "master was running work". A conservative future design can infer in-flight work from any live non-terminal worker with `SPAWNING`, `WAITING_FOR_ACK`, `BUSY`, or any `DISPATCHED` job in the session. Whether `PROMPT_PENDING` or `QUEUED` should cause master relaunch is a product decision, not encoded today.

## Resume/Relaunch Behavior

The default master command is `claude --dangerously-skip-permissions --continue /remote-control` at `src/cli/config.rs:177`.

`src/rpc/handlers/sessions.rs:191` spawns the original master command and passes it to the master watcher at `src/rpc/handlers/sessions.rs:293`.

`src/monitor/master_watch.rs:193` relaunches using the same captured `master_cmd`. It does not synthesize extra resume flags. Resume behavior depends on the configured command, which by default includes `--continue`.

For corrected semantics, the relaunch path can reuse this captured command, but worker cleanup must happen before relaunch and the future tests should ensure the replacement master is a new generation while old workers are gone.

## Orphan/Reconcile Coverage

Startup reconcile is wired in `src/bin/ahd.rs:56` via `reconcile_startup_with_tmux_socket`.

`src/db/system.rs:939` runs startup reconcile. It crashes dead active agents, can re-register alive IO, sweeps stale tmux sockets, and runs orphan scope reconciliation.

`src/db/system.rs:363` implements orphan systemd scope reconciliation. It lists user scopes, recognizes own ccbd scopes via daemon marker/known refs, and stops scopes that have no live refs.

`src/sandbox/systemd.rs:17` and `src/sandbox/systemd.rs:52` wrap agent commands with descriptions like `ccbd-agent-{agent_id}@{daemon_marker}`. `src/sandbox/systemd.rs:122` adds `BindsTo` and `PartOf` when running under a daemon unit.

This helps daemon restart cleanup, but it is not a substitute for immediate master-death cleanup. The corrected path should still directly stop/kill workers on master exit; startup reconcile is only a recovery net for daemon crashes or missed cleanup.

## Test Impact

Tests that currently encode PR #52 revive-and-keep-worker behavior will need cutover:

- `tests/r1_master_exit_shutdown.rs` currently checks master revive while worker remains active.
- master watch unit tests in `src/monitor/master_watch.rs` likely need new assertions that worker cleanup happens before revive and idle sessions do not relaunch.
- `src/master_revival.rs` pure tests should separate classification from worker cleanup policy; current classification only handles active/stale/intentional status.

Recommended test axes for the future implementation:

1. Unexpected master death with in-flight worker: worker is killed/cleaned first, then master generation increments and replacement master is spawned.
2. Unexpected master death with no in-flight work: workers are still cleaned if any exist, master is not relaunched, and no zombie scopes remain.
3. Session kill/system shutdown: still classified as intentional and does not trigger revive.
4. Stale watcher generation: does not clean workers for a newer master generation.
5. Cleanup failure ordering: no relaunch before worker cleanup has at least been attempted and recorded.
6. Startup reconcile remains a fallback, not the only cleanup path.

## Main Risks

- The existing cascade cleanup mutates `sessions.status` to `KILLED`; using it directly before revive would make revive impossible.
- "Master was running work" is not explicitly represented; inferring it from worker/job state may be semantically lossy.
- Current tests assert worker preservation, so future cutover must deliberately update them to the corrected product contract.
- Systemd scope cleanup works best when `daemon_marker` is available. Unsafe/no-systemd mode still depends on pidfd/tmux fallback and registry cleanup.
- Default command resumes via `--continue`, but custom master commands may not. Corrected behavior should document whether resume is best-effort based on configured command or enforce a resume-capable command.

## Read / Grep Notes

Read or inspected:

- `/tmp/a1-research-master-death-corrected.md`
- `src/monitor/master_watch.rs`
- `src/master_revival.rs`
- `src/db/system.rs`
- `src/db/schema.rs`
- `src/db/mod.rs`
- `src/db/jobs.rs`
- `src/db/state_machine.rs`
- `src/rpc/handlers/sessions.rs`
- `src/rpc/handlers/system.rs`
- `src/sandbox/systemd.rs`
- `src/bin/ahd.rs`
- `src/cli/config.rs`
- `tests/r1_master_exit_shutdown.rs`
- `.kiro/specs/ah-oom-restart-resume/CONCLUSION.md`
- `.kiro/specs/ah-oom-restart-resume/design-master-revive.md`
- `/tmp/design-a2-revive-autoshutdown-summary.md`

Commands used:

- `rg -n "master.*death|master.*exit|spawn_master|spawn_master_pidfd_watch_task|classify_master_death|MasterDeathDecision|master_generation|master_pid|session.kill|system.shutdown|auto_shutdown|waitid|P_PIDFD|pidfd|master_command|respawn|revive|fuse|clean-exit|OOM" ...`
- `rg -n "STATE_|DISPATCHED|QUEUED|query_dispatched|claim_next_job|WAITING_FOR_ACK|BUSY|PROMPT_PENDING" ...`
- `rg -n "reconcile_startup|reconcile_orphan|orphan_scope|BindsTo|PartOf|ccbd-agent|daemon_marker|startup reconcile" ...`
- `rg -n "default_master_cmd|master_cmd|--continue|remote-control|codex|claude" ...`
- `git branch --show-current`
- `git diff main...feat/ahd-master-revive-oom --stat`
- `git diff main...feat/ahd-master-revive-oom -- src/monitor/master_watch.rs`
- `git diff main...feat/ahd-master-revive-oom -- src/master_revival.rs`
- `git diff main...feat/ahd-master-revive-oom -- src/db/schema.rs src/db/mod.rs src/db/system.rs src/rpc/handlers/sessions.rs src/rpc/handlers/system.rs tests/r1_master_exit_shutdown.rs`
