# ah master tell + observability design

## Scope

This round adds a PM-proxy -> master control path without moving master into the worker job model.

Target behavior:

- `ah tell master "..."` asynchronously injects text into the active master pane.
- PM-proxy treats master work start as confirmed only after master's `UserPromptSubmit` hook reaches `ahd`.
- PM-proxy treats master work completion as confirmed when master's `Stop` hook reaches `ahd`.
- The PM-proxy <-> user conversation never blocks on master execution.

Non-goals for this round:

- Do not put master into `agents`.
- Do not add worker `UserPromptSubmit` handling.
- Do not alter worker dispatch/STUCK state semantics except making the notify pipe event-generic enough for future worker UPS.

Evidence anchors already verified in this repo:

- Master runtime fields live on `sessions`, including `master_pane_id`, `master_pid`, `master_generation`, retry metadata, and exit reason (`src/db/schema.rs`, `src/db/mod.rs:198-253`).
- Workers use `agents.state`/`agents.sub_state` and `mark_agent_idle_hook_event` (`src/db/state_machine.rs:774`, `src/db/state_machine.rs:1865`).
- `sessions.master_pane_id` is persisted and updated through `set_session_master_pane_id_sync` (`src/db/sessions.rs:162`).
- Hook materialization currently installs only Stop in the Claude path (`src/provider/home_layout.rs:229`).
- `materialized_ah_hook(ctx, event)` ignores `event` and always passes `"stop"` to `build_ah_hook_command` (`src/provider/home_layout.rs:702-706`), while `build_ah_hook_command(ctx, event)` itself is event-generic (`src/provider/home_layout.rs:665-678`).
- `handle_agent_notify` rejects every event except `stop` before querying the agent (`src/rpc/handlers/agent.rs:608-617`), with a router test covering that first-release behavior (`src/rpc/router.rs:313`).
- Existing CLI master pane discovery pattern already queries active sessions and validates `master_pane_id` for attach (`src/bin/ah.rs:819-828`).

## 1. Master State Machine

### Storage

Add dedicated master observability columns to `sessions`:

```sql
ALTER TABLE sessions ADD COLUMN master_state TEXT NOT NULL DEFAULT 'IDLE'
ALTER TABLE sessions ADD COLUMN master_pending_tell_request TEXT
```

`master_state` allowed values:

- `IDLE`: master is alive or eligible for commands, and no confirmed provider turn is currently running.
- `BUSY`: master fired `UserPromptSubmit`; a provider turn is confirmed in progress.

`master_pending_tell_request` is a single correlation slot for the latest in-flight `ah tell master` request id for that session. It is observability metadata only: it must never gate or veto `master_state` transitions. Hooks without a request id still move master BUSY/IDLE.

Implementation should enforce `CHECK(master_state IN ('IDLE', 'BUSY'))` in `SCHEMA_DDL` for new databases. For existing SQLite migration, follow the current `migrate_sessions_master_*` style with `add_column_if_missing`, because SQLite cannot add a checked non-null column as freely once data exists.

Schema touch points:

- `src/db/schema.rs`: add `master_state TEXT NOT NULL DEFAULT 'IDLE'` and `master_pending_tell_request TEXT` to `sessions`.
- `src/db/mod.rs`: add `migrate_sessions_master_state(conn)` and `migrate_sessions_master_pending_tell_request(conn)` near existing master session migrations.
- Session query DTOs should expose `master_state` wherever `ah ps`, `ah tell`, or observability reads session summaries.

### Transitions

`UserPromptSubmit -> BUSY`

- Source: master provider hook sends `agent.notify` with master identity and `event=userpromptsubmit`.
- DB action: update the active session's `master_state` from any value to `BUSY`.
- Idempotency: repeated UPS while already `BUSY` is accepted and logged as `transitioned=false` or `changes=0`; it must not error.
- Meaning: this is the first reliable confirmation that the prompt reached provider input processing. A successful `ah tell master` paste is not enough.

`Stop -> IDLE`

- Source: master provider hook sends `agent.notify` with master identity and `event=stop`.
- DB action: update the active session's `master_state` from any value to `IDLE`.
- Idempotency: repeated Stop while already `IDLE` is accepted and logged as `transitioned=false`.
- Meaning: provider turn completed according to the provider Stop hook.

### Session lifecycle rules

Creation/spawn:

- New session rows default `master_state='IDLE'`.
- New session rows default `master_pending_tell_request=NULL`.
- Master pane spawn and `set_session_master_pane_id_sync` must not imply `BUSY`.
- Master hooks must be materialized for the current `master_generation` at spawn time, so the notify identity contains the generation that produced the hook command.

Master death:

- Whenever monitor/revival marks a master dead, missing, invalid, killed, or no longer active, set `master_state='IDLE'` in the same logical lifecycle transition.
- Clear `master_pending_tell_request` in the same death transition; no Stop hook can be trusted to clear it after the master is gone.
- Rationale: a dead master cannot remain observably BUSY; otherwise PM-proxy waits forever for a Stop hook that cannot fire.
- `master_last_exit_reason` remains the failure evidence; `master_state` is only current activity state.

Revive:

- During revive/spawn of a replacement master, initialize or force `master_state='IDLE'` and `master_pending_tell_request=NULL` when the new `master_pid`, `master_pane_id`, and `master_generation` are committed.
- Do not inherit BUSY across generations. The old provider turn is gone.
- Re-materialize hooks for the new generation. A hook command from the old master generation is stale by definition.

Cutover:

- Before cutover activates a new master generation, the green/new master should be `IDLE`.
- On successful cutover, set the session row to the new runtime metadata, `master_state='IDLE'`, and `master_pending_tell_request=NULL`.
- On rollback, keep or restore the old active master's known runtime and set `master_state='IDLE'` with `master_pending_tell_request=NULL` unless a fresh UPS from that active master has already arrived after rollback.
- Hooks must be re-materialized for the generation that becomes active. Cutover/revive are active paths in this project; without generation isolation, a late Stop from an old master can flip the new master's BUSY to IDLE and create a false "done" signal.

Session close/delete:

- No special state is needed after delete.
- Session close should set or leave `master_state='IDLE'` and clear `master_pending_tell_request` before status leaves `ACTIVE`, so `ah ps` does not show a closed BUSY master.

## 2. `ah tell master` stuck-paste self-healing

`ah tell master` is an asynchronous delivery command. It returns after delivery verification or a bounded delivery failure. It never waits for master to finish the instructed work.

### Inputs and addressing

CLI shape mirrors `Cmd::Ask` enough to reuse parsing conventions:

```text
ah tell master "prompt text" [--session <session_id>] [--request-id <id>]
```

Differences from `ah ask`:

- No job id.
- No worker dispatch.
- No `--wait` for job completion in this round.
- Target is the active session's `master_pane_id`, resolved like `ah attach master` does today.

If no request id is supplied, generate a correlation id such as `tell_<timestamp>_<random>` and include it in tell delivery logs and the `master.tell_begin` registration.

Because provider hooks are installed ahead of time and do not naturally know which `ah tell` caused a turn, `ah tell` must register the correlation with `ahd` before pasting:

- `ah tell` sends a lightweight RPC such as `master.tell_begin(session_id, request_id, pane_id)` before `LOAD_BUFFER`.
- `ahd` writes that request id into `sessions.master_pending_tell_request` for the session. This is a single slot, not a queue or side table.
- On master `UserPromptSubmit`, notify handling reads the slot, attaches the request id to the BUSY log, and treats the slot as claimed by the current turn.
- On master `Stop`, notify handling logs the same slot value as the completed request id and clears `master_pending_tell_request`.
- If delivery fails before UPS, `ah tell` sends `master.tell_failed(request_id, stage, reason)` or records the failure through the same RPC surface, so PM-proxy can grep the failed request id and know no BUSY should be expected.

This request-id bookkeeping is observability metadata. It must not be required for the `master_state` transition itself; hooks without request ids still update BUSY/IDLE. Keeping the slot on `sessions` makes it survive `ahd` restart without adding a v1 table. A single slot is precise enough because master runs one provider turn at a time and `ah tell master` is naturally serialized per session: there is only one in-flight tell that can be claimed by the next master UPS.

### State machine

```text
START
  -> RESOLVE_TARGET
  -> LOAD_BUFFER
  -> PASTE
  -> DETECT_EXPAND_PROMPT
       -> SEND_ENTER_TO_EXPAND -> VERIFY_PANE_CLEARED
       -> VERIFY_PANE_CLEARED
  -> DELIVERED

Any bounded failure -> DELIVERY_FAILED
```

### Step details

`RESOLVE_TARGET`

- Query active sessions.
- If `--session` is provided, require exactly that active session.
- Without `--session`, require exactly one active session with a non-empty `master_pane_id`.
- Parse/validate the pane id before sending.
- Failure return examples:
  - `no active session with a master pane`
  - `multiple active sessions; pass --session <session_id>`
  - `stored master_pane_id is invalid`

`LOAD_BUFFER`

- Put prompt text into a tmux buffer instead of typing through `send-keys`.
- Include a trailing newline only if the existing `ah ask` path requires it for provider submission. The preferred flow is paste text, then send Enter explicitly, because it gives tell a separate expansion recovery step.
- Detection: tmux command success and buffer exists for the duration of paste.
- Failure: return `DELIVERY_FAILED` with tmux stderr and request id.

`PASTE`

- Paste buffer into `master_pane_id`.
- Send Enter once to submit.
- Detection: tmux command success.
- Failure: return `DELIVERY_FAILED`; do not mark master BUSY.

`DETECT_EXPAND_PROMPT`

- Capture the target pane after paste/Enter.
- Detection predicate: captured visible text contains the provider/tmux paste guard message, normalized case-insensitively. Primary known phrase:
  - `paste again to expand`
- Use a short bounded polling window, for example 100-250 ms between captures up to 1-2 seconds. This is delivery verification only, not work wait.
- If no expansion prompt is present, move to `VERIFY_PANE_CLEARED`.

`SEND_ENTER_TO_EXPAND`

- Send one additional Enter to the same pane.
- Rationale: the first Enter can stop at a large-paste expansion prompt instead of submitting to provider. The second Enter expands/submits.
- Detection: tmux command success.
- Failure: return `DELIVERY_FAILED` with request id.

`VERIFY_PANE_CLEARED`

- Capture the pane after the final Enter.
- Main stuck predicate remains the paste guard phrase, not a full-screen body search. Keep a small configurable phrase table, initially including:
  - `paste again to expand`
- Confirmation must restrict inspection to the bottom composer/input area of the captured pane and confirm the tell body is not still sitting in that input area.
- Do not scan the full capture for the body substring. The transcript can legitimately contain the submitted prompt after delivery; treating that as stuck would create false delivery failures.
- The pane does not need to be visually empty in full; provider status banners, previous transcript, and submitted prompt history can remain.
- Use bounded polling, for example up to 2 seconds total.
- Ambiguous evidence returns `DELIVERY_FAILED_UNCONFIRMED` with request id, pane id, and the last detection reason.

Pane verification is deliberately light. The real start-of-work authority is the `UserPromptSubmit` hook, not pane text parsing. The pane check is only a cheap belt-and-suspenders delivery guard: look for known paste guard phrases, inspect only the bottom input/composer area, and when unsure fail unconfirmed rather than reporting false success or marking BUSY.

`DELIVERED`

- Return immediately after delivery verification.
- Output should be explicit that execution is not complete:
  - `delivered request_id=<id>; waiting for master UserPromptSubmit/Stop hooks is observable via ah ps/logs`

`DELIVERY_FAILED`

- Return non-zero.
- Never set `master_state=BUSY`.
- This is the desired "stuck paste is visible" behavior: if paste did not reach provider submission, no `UserPromptSubmit` hook fires, so PM-proxy sees no false BUSY.

## 3. Event-generic notify pipe and master/worker split

### Required first fix: event must be real

Before adding a second event, fix hook materialization so `event` flows through:

- `materialized_ah_hook(ctx, event)` must call `build_ah_hook_command(ctx, event)`.
- The hook key/event name must remain provider-correct (`UserPromptSubmit`, `Stop` as provider hook keys where required).
- The CLI notify payload must use normalized lowercase event values:
  - `userpromptsubmit`
  - `stop`

The existing hard-coded `"stop"` path would silently convert `UserPromptSubmit` hooks into Stop notifications and break the state machine.

### Handler contract

Keep the existing RPC method name `agent.notify` for compatibility, but make its semantics role-aware:

```text
agent.notify {
  agent_id: string,
  event: "userpromptsubmit" | "stop",
  provider: string,
  socket: string,
  hook_json: ...,
  event_id?: string
}
```

Routing:

- If `agent_id` is a master sentinel, route to master notify handling.
- Otherwise, route to existing worker notify handling.

Do not query `agents` before deciding master vs worker. Master is not in `agents`, so a master event would otherwise fail as `AgentNotFound`.

Supported event matrix this round:

| Role | userpromptsubmit | stop |
| --- | --- | --- |
| master | update `sessions.master_state=BUSY` | update `sessions.master_state=IDLE` |
| worker | reject or ignore with explicit unsupported-worker-event error | existing `mark_agent_idle_hook_event` |

Worker `stop` behavior should stay as close as possible to today's flow:

- Validate provider against the `agents` row.
- Call `mark_agent_idle_hook_event`.
- Cancel completion registry and wake orchestrator when changes > 0.

Worker `userpromptsubmit` must not mutate worker state in this round. Prefer returning an explicit unsupported role/event error if a worker hook is accidentally installed. This prevents accidental STUCK/dispatch behavior changes.

### Event ordering and idempotency

Master events are advisory state edges, not a strictly ordered distributed log. Handling must be safe for duplicates and mild reordering.

Master v1 notify identity must include `master_generation`. This is not optional for normal spawn/revive/cutover paths: each master hook command must be materialized with the generation that was active when that master was spawned.

Rules:

- Repeated `userpromptsubmit`: `BUSY -> BUSY`, no error.
- Repeated `stop`: `IDLE -> IDLE`, no error.
- `stop` before `userpromptsubmit`: set `IDLE` only if the event generation matches `sessions.master_generation`; if a later UPS arrives for the same active generation, set `BUSY`. This can happen if hooks are delivered out of order; logs must expose both events.
- `userpromptsubmit` after already completed Stop: set `BUSY` only if it belongs to the current active master generation. Stale generation events must be logged and ignored.

Mandatory stale-event guard:

- Parse `session_id` and `master_generation` from the master sentinel.
- On notify, require `session_id` to match an active session and require parsed generation to equal `sessions.master_generation`.
- If generation differs, classify the event as stale, log `ignored_stale=true`, and do not mutate `master_state` or `master_pending_tell_request`.
- Only if a specific spawn path truly cannot access generation during hook materialization may it temporarily degrade to session-active validation. That degradation must log `generation_absent=true` on every event. It is not the normal v1 path.

### Provider validation

Worker provider validation remains against `agents.provider`.

Master provider validation should use the provider/config that spawned the master if the session stores it today; if not, this round should accept the canonicalized provider but log it. Do not invent an `agents` row just to validate master provider.

## 4. Master notify identity

Use a reserved, non-agent sentinel:

```text
--agent-id master:<session_id>:<generation>
```

Why this shape:

- Existing `agent_id` transport can be reused without adding a new RPC method immediately.
- It cannot collide with normal agent ids if worker ids are validated to exclude `master:` prefix.
- It carries the session id, allowing the handler to update exactly one `sessions` row without guessing from global active sessions.
- It carries the master generation, allowing the handler to reject late hooks from old revive/cutover generations before they can corrupt the new master's BUSY/IDLE state.

Required contracts:

- Worker creation must reject ids with reserved prefix `master:` if not already impossible by convention.
- Master hook push context must support a master mode where `ctx.agent_id` becomes `master:<session_id>:<generation>` even though no agent row exists.
- Master hook materialization must happen per generation on spawn, revive, and cutover activation. The generation is baked into the hook command.
- `hook_debug_log_path` can safely produce `hooks-debug/master:<session_id>:<generation>.log` on Unix, but a sanitized filename such as `master_<session_id>_<generation>.log` is cleaner.

Alternative considered and rejected:

- `--agent-id master`: too ambiguous when multiple sessions exist and risks updating the wrong master.
- `--agent-id master:<session_id>`: session-scoped but not generation-safe; a late Stop from an old master could flip a new BUSY master to IDLE and destroy the feature's reliability.
- Adding `--role master` while leaving `--agent-id` empty: cleaner semantically but larger CLI/RPC surface change. It can be added later; the sentinel is enough for this round.

## 5. `ahd` dual-event log format

Every notify receipt and processing result should be grep-friendly. Use structured tracing fields, not only prose.

Receipt fields:

- `event`: normalized event name (`userpromptsubmit` or `stop`)
- `role`: `master` or `worker`
- `agent_id`: raw notify identity
- `session_id`: for master, parsed from sentinel; for worker, from agent row if available
- `provider`
- `request_id`: for master, read from `sessions.master_pending_tell_request` when present; it is not expected to come from the hook notify payload
- `event_id`: provider hook event id if present
- `master_generation`: for master, parsed from sentinel and compared with `sessions.master_generation`
- `hook_source`: `agent.notify`

Processing fields:

- `previous_state`
- `new_state`
- `transitioned`
- `ignored_stale`
- `reason`

Sample receipt:

```text
INFO ahd hook_source=agent.notify role=master event=userpromptsubmit agent_id=master:s_123:7 session_id=s_123 provider=claude request_id=tell_20260704T120001Z_ab12 event_id=ups-456 master_generation=7 received hook
```

Sample processing:

```text
INFO ahd hook_source=agent.notify role=master event=stop agent_id=master:s_123:7 session_id=s_123 request_id=tell_20260704T120001Z_ab12 master_generation=7 previous_state=BUSY new_state=IDLE transitioned=true ignored_stale=false processed hook
```

Sample delivery failure from `ah tell`:

```text
WARN ahd command=ah.tell target=master session_id=s_123 pane_id=%42 request_id=tell_20260704T120001Z_ab12 stage=VERIFY_PANE_CLEARED result=delivery_failed reason=paste_expand_prompt_still_visible
```

These fields let PM-proxy distinguish:

- command delivery failed: no UPS should be expected;
- delivery succeeded but provider did not start: no UPS seen for request id within PM's chosen observation window;
- provider started and is still running: UPS seen, no Stop yet;
- provider finished: Stop seen after UPS.

## 6. Worker `UserPromptSubmit` fast-follow assessment

This round intentionally does not implement worker UPS. The event-generic pipe should leave a clean future slot for it.

### State transitions it would touch

Current worker path has `ah ask`/dispatch semantics where ah itself marks the worker as having assigned work, and Stop marks it idle through `mark_agent_idle_hook_event`.

Adding worker UPS would raise these questions:

- If dispatch marks an agent BUSY before the provider actually accepts input, UPS would become a stronger "actually started" signal.
- If dispatch currently starts STUCK timers immediately, UPS could reset or arm the STUCK timer only after provider acceptance.
- If paste stalls before UPS, the worker may be in a "dispatched but not provider-started" limbo; that likely needs a distinct sub-state rather than forcing BUSY.
- Stop without UPS can happen for old hooks, provider quirks, or reordered delivery; worker logic must not complete an unstarted job incorrectly.

### Code areas likely affected

- CLI/dispatch send path for worker ask (`src/bin/ah.rs`, agent send/ask RPC handlers).
- Worker state transitions in `src/db/state_machine.rs`, especially BUSY, IDLE, ACK/waiting, and STUCK-related sub-state handling.
- Orchestrator wake and STUCK detection logic.
- Hook installation in `src/provider/home_layout.rs` for worker `UserPromptSubmit`.
- `handle_agent_notify` worker event matrix.
- Router tests that currently assert non-stop rejection.
- Completion/log monitor interactions that may currently assume Stop is the only hook event.

### Risk surface

- False BUSY: marking BUSY on UPS may be correct, but if dispatch already marked BUSY, changing timing can break job visibility.
- False STUCK: if STUCK timers are tied to dispatch rather than provider start, delayed paste or provider prompt guards can still look like agent failure.
- Lost completion: if Stop arrives before UPS and the future state machine rejects it, jobs can remain stuck.
- Duplicate events: providers may retry hooks; transitions need event-id or state-version tolerance.
- Backward compatibility: existing provider homes may only have Stop hooks until regenerated.

### Recommended fast-follow direction

Introduce an explicit worker sub-state boundary instead of directly reusing master semantics:

- dispatch accepted by ah: `BUSY/sub_state=DELIVERING` or existing equivalent;
- worker UPS: `BUSY/sub_state=RUNNING`;
- Stop: existing completion transition to IDLE.

This keeps dispatch reliability, provider-start observability, and STUCK detection separate. It also avoids retrofitting master `master_state` semantics into worker jobs.
