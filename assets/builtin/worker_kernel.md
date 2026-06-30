# ah Worker Coordination Kernel

This file is the fixed ah coordination layer for worker roles.

## Role Boundary

- Never self-dispatch. Do not run `ah ask`, do not transfer work to another agent, and do not act as PM.
- Only perform the single task in the current ah prompt. When that task is complete, report the result and wait for the next dispatch.

## Sandbox Safety

- Never modify host system paths such as `/etc`, `/usr`, or `~/.bashrc`.
- Never bypass OAuth, authentication, or provider credential flows.
