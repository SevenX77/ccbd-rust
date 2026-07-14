# ah Agent Lifecycle Control + Dynamic Topology Requirements

Status: requirement capture (design NOT started). Captured by operator 2026-07-12 from user verbatim directive. This is a **new control capability**, deliberately separated from `ah-orchestration-reliability` (which covers reliability *defects* in the reconciler/completion detector). Lifecycle control is a feature — the ability to mutate a running topology per-agent — not a bug fix. Keeping them in one spec would have conflated a defect class with a capability class.

## Source (user verbatim)

- 2026-07-12: "1个是重启单个agent，还有一个需求是随时改拓扑，热启动新的agent，或者随时停掉单独的agent"

## Background / current gap

Today the only lifecycle levers are whole-session or hard:

- `ah up` — whole-session reconcile against `ah.toml`; carries the ah#16 non-atomic realign hazard and can silently shrink/respawn siblings.
- `ah start` — cold start of a whole session.
- `ah kill` — hard kill of one agent/session.

There is **no** per-agent restart, no hot seat-add, and no graceful single-agent stop. So "restart just o1" today forces a whole-session `ah up` (dangerous) or a `kill`+manual dance.

## Requirement DT: Single-Agent Lifecycle Control and Dynamic Topology

The system MUST support per-agent lifecycle operations and runtime topology mutation on a live session, without a whole-session teardown or whole-session reconcile.

### DT.1 Single-Agent Restart

`ah agent restart <id>` MUST atomically respawn exactly one agent without touching sibling agents and without running a whole-session reconcile.

Acceptance criteria:

- Restart of agent X respawns X's pane/process and leaves every sibling's pid unchanged.
- No whole-session realign is triggered; the ah#16 destructive-realign path is never entered.
- Restart preserves X's assigned role/rules binding and pane identity (no respawn-pane-name-mismatch, cross-ref `ah-orchestration-reliability` respawn-pane-name-mismatch case).

### DT.2 Hot Add Seat

`ah agent add <id> --provider ...` MUST hot-insert a new agent seat into a live session without stopping the session.

Acceptance criteria:

- Add inserts a new pane+process; existing agents are untouched.
- The new agent reaches `IDLE` and is dispatchable.
- The added seat is persisted so a later reconcile does not treat it as an orphan to reap.

### DT.3 Graceful Single-Agent Stop

`ah agent stop <id>` MUST gracefully stop one seat, distinct from `ah kill` (hard kill) and from whole-session stop.

Acceptance criteria:

- Stop drains/ends one agent's process and removes its seat while siblings keep running.
- Exit semantics are graceful (agent allowed to finish/settle) and distinct from hard kill.
- A stopped seat is removed from desired-state so reconcile does not respawn it.

### DT.4 Runtime Topology Change

The operator MUST be able to add/remove/change agent seats on a live session ("随时改拓扑") composed from DT.1-DT.3 primitives, without a full `ah up` whole-session realign.

Acceptance criteria:

- A topology edit that adds one seat and removes another mutates only those two seats; untouched seats keep their pids.
- The edit is atomic per seat (no partial half-built seat left behind, cross-ref ah#16 realign-atomicity).

### DT.5 Authority Partition

master has no daemon-ops authority (`up`/`start`/`stop` are excluded from the master command set). If these per-agent commands are to be master-usable (e.g. master restarting a stuck worker), authority MUST be re-partitioned explicitly; otherwise they remain operator-only.

Acceptance criteria:

- The command-authority matrix explicitly states whether each of `agent restart/add/stop` is operator-only or master-usable.
- If master-usable, the grant is scoped (master may restart a worker it owns, but not add arbitrary provider seats) and documented.

## Relationship to adjacent specs

- **ah-orchestration-reliability** — owns the reconciler and the AGY completion-detector defect. DT commands MUST NOT re-introduce the reconcile hazards that spec fixes (ownership gates D1, atomicity). A `restart`/`stop` is a destructive action and MUST pass the same D1 ownership gates.
- **control-plane-refactor** — if the control-plane state machine is refactored, DT commands are new transitions it must model.

## Status

- DT.1-DT.5: NOT started (requirement capture only). Design owner: o1 diverge + d1 pen. Tracked in `research/REQUIREMENT-LEDGER.md` (R-DYN-1).
