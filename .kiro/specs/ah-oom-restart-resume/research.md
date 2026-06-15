# ahd restart / kill-state / resume research

## 1. ahd startup and systemd restart model

Current ahd is bootstrapped by the `ah` CLI, not by a checked-in static unit file. `ah` first tries the socket and removes a stale socket if connect fails (`src/bin/ah.rs:241-248`), locates the sibling `ahd` binary (`src/bin/ah.rs:250-254`), then prefers `systemd-run --user --unit=ahd.service` (`src/bin/ah.rs:276-290`). If that bootstrap fails, it falls back to direct `Command::spawn` (`src/bin/ah.rs:292-310`, `src/bin/ah.rs:326-339`).

Important current-state correction: the transient systemd unit already has restart policy. `build_ahd_systemd_run_command_with_env` sets `Restart=on-failure`, `RestartSec=1s`, `StartLimitIntervalSec=60`, and `StartLimitBurst=5` (`src/cli/start.rs:47-65`). The fallback direct spawn path has no restart supervision.

On ahd startup, it initializes `ahd.sqlite`, creates a tmux server, then calls `reconcile_startup_with_tmux_socket` before spawning the orchestrator (`src/bin/ahd.rs:48-75`). Normal SIGTERM/SIGINT goes through `cleanup_tmux_resources`, removing tmux sessions, master sandboxes, session anchors, tmux server, and socket (`src/bin/ahd.rs:106-175`). OOM/SIGKILL bypasses that cleanup; recovery then depends on systemd scope dependencies, startup reconcile, and the orchestrator recovery loop.

Agent/master systemd scopes are tied to the daemon only when ahd is itself under systemd: agent scopes add `BindsTo=<daemon unit>` and `PartOf=<daemon unit>` through `append_daemon_unit_dependencies` (`src/sandbox/systemd.rs:40-45`, `src/sandbox/systemd.rs:113-119`).

## 2. KILLED, CRASHED, and DISPATCHED semantics

Agent states are string constants in `src/db/state_machine.rs:13-28`. `KILLED` is the intentional terminal path. `mark_agent_killed_sync` updates active non-terminal agents to `KILLED`, increments `state_version`, fails any dispatched jobs for that agent, emits a `state_change` event, and cleans runtime resources (`src/db/agents_lifecycle.rs:8-54`).

`CRASHED` is the unexpected-exit/recoverable failure path. `mark_agent_crashed_with_exit_sync` delegates to `mark_agent_crashed_sync`, which updates agents to `CRASHED` only from states not already `CRASHED`, `KILLED`, or `PROMPT_PENDING`, records exit/error metadata, fails dispatched jobs, emits `state_change`, and preserves recoverable provider homes for eligible providers (`src/db/agents_lifecycle.rs:56-155`). Runtime pidfd death enters this path from `agent_watch` after liveness confirmation (`src/monitor/agent_watch.rs:47-76`).

`DISPATCHED` is a job status, not an agent state. Jobs are persisted in `jobs.status` with dispatch timestamps and evidence flags (`src/db/schema.rs:78-96`, `src/db/schema.rs:197-212`). Dispatch changes a queued job to `DISPATCHED` and moves the agent to busy/ack state in one flow (`src/db/jobs.rs:136-141`, `src/db/jobs.rs:210-229`). Both `KILLED` and `CRASHED` paths currently fail dispatched jobs.

Startup reconcile also marks dead active agents as `CRASHED`, but only for `SPAWNING`, `WAITING_FOR_ACK`, `BUSY`, and `IDLE`; it then fails dispatched jobs with the startup reconcile reason (`src/db/system.rs:751-788`).

## 3. Cascade-kill anti-orphan model

The central cascade path is `cascade_kill_session_agents_with_runner_sync` (`src/db/system.rs:144-201`). It first marks the session `KILLED`, selects agents in the session whose state is not `CRASHED` or `KILLED`, optionally stops matching systemd agent scopes and the session anchor, sends pidfd SIGKILL as fallback, marks each agent `KILLED`, cancels marker/parser state, and returns the count.

Known call sites:

- Explicit `session.kill` calls `cascade_kill_session_agents_for_daemon`, stops the session anchor, kills tmux panes/sessions, and removes agent/master sandboxes (`src/rpc/handlers/sessions.rs:70-135`).
- Master process exit triggers `cascade_kill_session_agents_for_daemon(..., "MASTER_EXIT", ...)`, kills agent tmux sessions, then may schedule daemon shutdown if idle (`src/monitor/master_watch.rs:36-67`).
- Session anchor disappearance triggers `cascade_kill_session_agents(..., "ANCHOR_UNIT_STOPPED")` after debounce (`src/monitor/session_watch.rs:28-75`).
- Session create anchor failure rolls back through `cascade_kill_session_agents(..., "ANCHOR_CREATE_FAILED")` (`src/rpc/handlers/sessions.rs:53-63`).
- Explicit `agent.kill` is per-agent, not session cascade: it calls `mark_agent_killed`, then SIGKILLs the stored pid and removes the agent sandbox (`src/rpc/handlers/agent.rs:407-435`).

Design implication: a deliberate master/session/anchor stop is encoded as session `KILLED` plus agent `KILLED`. A daemon OOM/restart should avoid reusing cascade-kill semantics unless it is intentionally abandoning the session.

## 4. Startup reconcile and orphan scope wiring

There are two similar-looking reconcile entry points with different behavior:

- `reconcile_startup_sync_with_state_dir` calls both `reconcile_active_agents_to_crashed_sync` and `reconcile_orphan_scopes_sync` (`src/db/system.rs:295-306`).
- The ahd runtime path calls async `reconcile_startup_with_tmux_socket`, which only calls `reconcile_active_agents_to_crashed_sync` and sweeps stale tmux sockets (`src/bin/ahd.rs:56-61`, `src/db/system.rs:944-958`).

So orphan-scope reconciliation exists in the codebase, but the ahd startup path currently does not invoke the scope-cleaning variant. This is the main wiring gap for restart-time orphan cleanup.

For living agents after ahd restart, startup reconcile can re-register pidfd watches, parser registry, IO reader, and marker timers when it can reattach FIFO/tmux state (`src/db/system.rs:805-879`). Dead active agents become `CRASHED`; recoverable providers preserve their home materialization for later recovery (`src/db/system.rs:751-802`).

## 5. Persisted cross-restart state

Persisted state is enough to reconstruct many facts but does not encode a single explicit "resume this killed-inflight session" decision:

- `sessions`: `id`, `project_id`, `master_pid`, `master_pane_id`, `status`, `config_hash`, `created_at` (`src/db/schema.rs:8-16`).
- `agents`: `provider`, `state`, `state_version`, `pid`, `exit_code`, `error_code`, `sub_state`, `config_hash`, recovery retry/backoff fields, timestamps (`src/db/schema.rs:18-36`).
- `agent_spawn_specs`: persisted respawn snapshot with provider/config/spec JSON and `ON DELETE CASCADE` (`src/db/schema.rs:38-45`, `src/db/recovery.rs:7-16`, `src/db/recovery.rs:51-71`).
- `jobs`: queued/dispatched/completed/failed/cancel state, prompt/reply, dispatch sequence, and evidence requirements (`src/db/schema.rs:78-96`).
- `events`: state changes and self-recovery attempts are durable audit signals (`src/db/schema.rs:47-58`).

Recovery candidate selection is currently agent-centered: `run_recovery_once_with_respawn` scans `CRASHED` agents, filters provider eligibility, backoff, and snapshot presence, then CAS-claims via `try_claim_agent_recovery_sync` (`src/orchestrator/mod.rs:219-290`, `src/db/recovery.rs:107-126`). `KILLED` agents are intentionally excluded.

Gap: because crash/killed lifecycle paths fail dispatched jobs (`src/db/agents_lifecycle.rs:33-35`, `src/db/agents_lifecycle.rs:99-105`, `src/db/system.rs:779-785`), cross-restart "continue the exact in-flight job" is not represented as a pending durable intent after startup reconcile. A restarted CRASHED agent may be physically resumed, but the previously DISPATCHED job can already be marked failed.

## 6. resume_args and cold restart behavior

Provider resume has both static and dynamic layers. `ProviderManifest` has static `resume_args` (`src/provider/manifest.rs:7-15`), while `compute_recovery_args` returns dynamic recovery args for `claude`, `antigravity`, and `codex` (`src/provider/manifest.rs:27-38`). Codex scans latest rollout metadata and falls back to `resume --last`; antigravity scans conversation files and falls back to `--continue` (`src/provider/manifest.rs:40-203`).

The spawn layer carries these through `RecoverySpawn { is_recovery, args }` (`src/sandbox/systemd.rs:7-11`). On recovery, command construction appends dynamic args first; if dynamic args are empty it falls back to `manifest.resume_args`; non-recovery spawns do not append resume args (`src/sandbox/systemd.rs:49-76`, `src/sandbox/systemd.rs:152-175`).

The physical recovery call path is:

1. `run_once` delegates to `run_once_with_recovery_respawn` (`src/orchestrator/mod.rs:62-69`).
2. Each tick dispatches queued work, then calls `run_recovery_once_with_respawn` (`src/orchestrator/mod.rs:70-196`).
3. Recovery scans `CRASHED` agents with snapshots and backoff eligibility, CAS-claims, deletes the old row, and respawns via `spawn_realign_agent(..., is_recovery=true, ...)` (`src/orchestrator/mod.rs:219-340`).
4. Recovery spawn enters `handle_agent_spawn_with_recovery`, computes provider recovery args from the materialized home, and calls `wrap_command_with_recovery` (`src/rpc/handlers/agent.rs:60-130`).

Cold ahd restart does trigger this machinery indirectly: ahd runs startup reconcile, then starts the orchestrator task (`src/bin/ahd.rs:56-75`), and the orchestrator loop immediately runs `run_once` before waiting on `WAKER` (`src/orchestrator/mod.rs:52-59`). The semantic caveat is that startup reconcile may already have failed dispatched jobs before the recovery respawn occurs.

## Design open questions

1. Should ahd startup call `reconcile_startup_sync_with_state_dir` or otherwise include `reconcile_orphan_scopes_sync` in the async startup path, so restart-time orphan scope cleanup is actually wired?
2. Should daemon OOM/restart preserve DISPATCHED jobs for recovery-eligible providers instead of failing them during startup reconcile, or should resumed provider processes be treated as best-effort physical continuity with the old job marked failed?
3. Do we need a durable "daemon epoch / shutdown reason / session recovery intent" marker to distinguish deliberate `session.kill` / master exit from ahd crash/OOM, instead of inferring from `sessions.status`, agent state, and events?
4. Should direct-spawn ahd fallback be considered unsupported for OOM auto-restart, or should it get an alternate supervisor/keeper story?
5. Should `KILLED` remain strictly never-resume, including cases where KILLED was produced by anchor/master cascade after daemon restart races?
6. If resuming in-flight jobs becomes a goal, which table owns the durable continuation contract: `jobs` status, `agent_spawn_specs`, a new recovery-intent table, or an event-sourced marker?
