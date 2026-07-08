---
name: ah-commands
description: Use when you need to inspect agent or job status, dispatch a task to a worker agent, wait for a dispatched job to finish, follow or retrieve a running agent's output, cancel or kill tasks, attach to a tmux session, stream lifecycle events, resolve a blocked PROMPT_PENDING agent, or report master cutover readiness. Authoritative CLI reference for 'ah' agent-facing orchestration commands (ah ps, ask, tell, pend, watch, logs, events, cancel, kill, attach, master ack-ready, prompt resolve). Not for operational commands like start, stop, up, doctor, setup, config, or bundle.
---

# ah agent-facing commands

Authoritative reference for orchestrating through `ah`. The exact, current usage for any command is always available via `ah --help` and `ah <command> --help` — use that as the ground truth if anything here looks out of date. This skill covers only the agent-facing orchestration subset; operational commands (start / stop / up / doctor / setup / config / bundle) are intentionally excluded — the master orchestrates, it does not operate the daemon.

## Status inspection & monitoring
- `ah ps` — List sessions, agents, and pending evidence. See the running topology and spot a stuck or backed-up agent.
- `ah events [--format json]` — Stream runtime lifecycle snapshots as JSON lines. Watch state-machine transitions across the system.

## Dispatch & async communication
- `ah ask <agent_id> <text> [--wait] [--request-id <id>]` — Submit a task to a worker; returns a job id. Delegate a unit of work; add `--wait` to block until it finishes.
- `ah tell <target> <text> [--session <s>] [--request-id <id>]` — Deliver text to the master pane or an agent without blocking. Async notices/status where no reply is awaited.

## Result tracking & log retrieval
- `ah pend <job_id>` — Block until a submitted job finishes. Await an async `ah ask` before the next decision.
- `ah watch <agent_id> [--since-event-id <n>]` — Stream an agent's output events live. Follow a running agent.
- `ah logs <agent_id> [--since <n>]` — Print an agent's stored output. Read a finished or errored agent's full output at once.

## Runtime intervention & debugging
- `ah cancel <job_id>` — Cancel a queued or running job. When a dispatched task is stale, misparametrized, or no longer needed.
- `ah kill <target_id> [--session] [--force]` — Kill an agent, or a whole session with `--session`. Terminate an unresponsive agent or tear down a session.
- `ah attach <target> [subject] [--session <s>]` — Attach to an agent or master tmux session (`target` = master / agent / legacy id). A manual escape hatch for direct tmux inspection.

## Role handover & interactive resolution
- `ah master ack-ready [--cutover-id <id>]` — Report successor-master readiness to ahd. Run after loading the handoff during cutover, before claiming takeover.
- `ah prompt resolve <agent_id> [--action <a>] [--keys <k>] [--save-to-kb]` — Answer a worker blocked at an interactive prompt (PROMPT_PENDING). Unblock a hung worker by submitting its choice or input.
