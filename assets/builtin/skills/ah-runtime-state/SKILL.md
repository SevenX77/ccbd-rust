---
name: ah-runtime-state
description: Use when you need to inspect authoritative ah runtime state, consume RuntimeSnapshot JSON, separate session, master, agent, and job status domains, or reason about cleanup and reap behavior.
---

# ah runtime state

Use `ah events --format json` as the authoritative structured read path. It streams one `RuntimeSnapshot` JSON object per line from `runtime.subscribe`, plus local inactive snapshots when the daemon is absent or lost. Do not scrape `ah ps` text and combine it with ad hoc tmux commands as your state authority; `ah ps` is a human-facing table and omits most RuntimeSnapshot fields.

## RuntimeSnapshot

`RuntimeSnapshot` fields:

- `schema_version`
- `event`
- `sequence`
- `reason`
- `runtime_state`
- `config_path`
- `workspace_path`
- `state_dir`
- `tmux_socket`
- `ahd_alive`
- `active`
- `ahd_has_inventory`
- `tmux_server_alive`
- `master_tmux_alive`
- `worker_tmux_alive`
- `worker_tmux_expected_count`
- `sessions`
- `agents`

`reason` is one of `initial`, `inventory_changed`, `tmux_changed`, `agent_changed`, `shutdown`, `daemon_absent`, or `daemon_lost`. `runtime_state` is one of `active`, `inactive`, `starting`, or `degraded`.

## RuntimeSessionSnapshot

`RuntimeSessionSnapshot` fields:

- `session_id`
- `project_id`
- `path`
- `status`
- `master_state`
- `master_tmux_session`
- `master_tmux_alive`
- `master_pane_id`
- `master_pid`
- `active_agents`

## RuntimeAgentSnapshot

`RuntimeAgentSnapshot` fields:

- `agent_id`
- `session_id`
- `provider`
- `state`
- `sub_state`
- `pid`
- `tmux_session`
- `tmux_alive`

## State domains

Keep these domains separate:

- `session.status`: `ACTIVE`, `KILLED`, or `FAILED`.
- `sessions.master_state`: `IDLE` or `BUSY` only.
- `agent.state`: `SPAWNING`, `SPAWNING_INTERVENTION`, `IDLE`, `WAITING_FOR_ACK`, `BUSY`, `PROMPT_PENDING`, `STUCK`, `FAILED`, `CRASHED`, `KILLED`, or `UNKNOWN`.
- `jobs.status`: `QUEUED`, `DISPATCHED`, `COMPLETED`, `FAILED`, or `CANCELLED`.
- `evidence.status`: `PENDING` or `REVIEWED` in the current evidence paths.
- `master_cutovers.state`: `PREPARING`, `SPAWNING`, `VERIFYING`, `ACTIVE`, `ROLLED_BACK`, `FAILED`, or `RELEASED`.
- `master_recovery_windows.phase`: `DETECTED`, `WORKERS_REAPED`, `MASTER_SPAWNING`, `MASTER_RUNNING`, `MASTER_VERIFYING`, `WORKERS_REPROVISIONING`, `COMPLETED`, `FAILED`, or `FUSED`.

RUNNING is not a DB enum in the authoritative schema or state constants. Treat it as prose or UI wording unless live source adds it as a real state.

## Cleanup and reap

Per-agent cleanup removes the registered runtime entry, aborts the reader, removes the FIFO, captures pane-at-death evidence, kills the agent tmux session, and removes the ah-managed sandbox home under the default cleanup policy. Cleanup also cancels marker, completion, parser, and monitor registries.

Session cascade marks an active session as `KILLED`, notifies runtime inventory watchers, then selects non-terminal agents for cleanup. Master-death cleanup additionally clears registries, stops matching systemd scopes and session anchors when available, sends pidfd SIGKILL where possible, and marks workers killed for that cleanup reason.
