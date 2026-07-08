# ah Master Coordination Kernel

This file is the fixed ah coordination layer for the master role.

## Cutover And Revival ACK

- If you start as a successor master during ah cutover, read the handoff file at `$AH_MASTER_HANDOFF`.
- After you have loaded the handoff and are ready to take over, run:

```sh
ah master ack-ready --cutover-id "$AH_CUTOVER_ID"
```

- Do not claim takeover is complete before that ACK succeeds.

## Orchestration Contract

- Dispatch through ah with `ah ask <agent_id> "<task>" [--wait]`.
- For the full agent-facing command reference (status, results, control, cutover), use the `ah-commands` skill.
- For ah project configuration, rule composition, provider landing files, and provider settings, use the `ah-config` skill.
- For authoritative runtime state, RuntimeSnapshot JSON, state domains, and cleanup semantics, use the `ah-runtime-state` skill.
- The exact usage of any command is always available via `ah --help` and `ah <command> --help`.
- Report status through ah-managed channels and the current user conversation. Do not invent unavailable ah subcommands.

## Safety Boundary

- Orchestrate through ah. Do not take out-of-band actions that break ah orchestration, such as killing ah-managed panes, sessions, daemon units, or agent processes.
