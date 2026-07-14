# Kill Path Touchpoint Survey

Branch: `feat/kill-path-ownership-guard`

Authoritative incident: `research/incident-stale-session-kill-cascade-2026-07-08.md`

Precedent: `38bae3a` added `stored_master_pane_still_matches` in `src/monitor/master_watch.rs`, validating stored `master_pane_id` by reading current `#{pane_pid}` through `TmuxServer::get_pane_pid` before arming master watches.

## Inventory

| file:line | what it kills | locator used | current ownership validation | fix |
|---|---|---|---|---|
| `src/bin/ah.rs:1240` | CLI entry for `ah kill`; dispatches either `session.kill` or `agent.kill` RPC | operator-provided `target_id`, plus `--session` boolean | N/A in CLI; no runtime ownership here | Defer to RPC rows below |
| `src/rpc/handlers/sessions.rs:96` | `ah kill --session` handler; marks session intentional killed and starts teardown | DB session row from `query_session_by_id`, including `status`, `project_id`, `master_pane_id`; agent ids from DB | N. It queries the row, but does not branch terminal statuses into DB-only cleanup before tmux teardown | FIX-2 |
| `src/rpc/handlers/sessions.rs:113` | cascades all session agents during `session.kill` | `session_id` plus current daemon tmux socket marker | Partial, non-pane. DB cascade only selects agents for that session; systemd scope matching includes daemon marker; pidfd kill uses monitor key by agent id. It does not validate tmux pane ownership because tmux is not touched here directly | No pane fix inside DB cascade; call-site still needs FIX-2 |
| `src/rpc/handlers/sessions.rs:121` | per-agent pane during `session.kill` | in-memory `agent_io::pane_id(agent_id)` | N. Pane id is trusted if present; no current tmux pid/session check before `kill_pane` | FIX-1 |
| `src/rpc/handlers/sessions.rs:124` | per-agent tmux session during `session.kill` | derived tmux session name `agent_session_name(agent_id)` | Weak. Session name is deterministic per agent id, but no current pane/session ownership check and stale/reused tmux session names are not fenced | FIX-1, or rely on FIX-2 for terminal sessions plus name-scoped validation for active sessions |
| `src/rpc/handlers/sessions.rs:133` | master tmux session during `session.kill` | derived `master_session_name(session.project_id)` | N. This kills by project id, not session id or generation. Multiple DB sessions can share project id, so this is a broad kill target | FIX-1 for active sessions; FIX-2 for terminal sessions |
| `src/rpc/handlers/sessions.rs:143` | stored master pane during `session.kill` | DB `sessions.master_pane_id` parsed as `TmuxPaneId` | N. This is the incident-class bug: stale `%0` is trusted and killed without checking current owner/pid/generation | FIX-1 and FIX-2 |
| `src/db/system.rs:144` | synchronous session-agent cascade wrapper | `session_id`, reason | DB-only wrapper. No direct tmux kill | No direct FIX-1; audit callers |
| `src/db/system.rs:368` | session-agent cascade core | `session_id`, optional daemon socket marker, DB agent ids, pidfd monitor keys, systemd scope descriptions | Partial. DB restricts agents by `session_id`; systemd scope descriptions include `ccbd-agent-{agent_id}@{daemon_marker}`; pidfd monitor key is agent id. No tmux pane kill here | No direct FIX-1; keep as cascade primitive |
| `src/db/system.rs:418` | systemd agent scopes and session anchor during cascade | daemon marker and `unit_name_for_session(session_id)` | Y for scopes by description marker; anchor by session-id unit name. Not tmux | Not one of FIX-1/2/3 |
| `src/db/system.rs:428` | worker process fallback during cascade | pidfd monitor registered under agent id | Y-ish for process identity by pidfd handle, not pane ownership | Not FIX-1 |
| `src/db/system.rs:697` | startup reconcile cascades expired master recovery windows | recovery window `session_id`; daemon marker | Uses DB recovery-window target; no tmux pane kill directly | No direct FIX-1; downstream cascade only |
| `src/db/system.rs:1072` | startup reconcile cleanup for dead agent pids | agent id into registry cleanup | Indirect. Registry entry includes pane/session/socket captured at registration but cleanup kills by agent session name, without current tmux ownership validation | FIX-1 in registry cleanup helper |
| `src/db/system.rs:1161` | async session-agent cascade wrapper | `session_id`, reason | DB-only wrapper. No direct tmux kill | No direct FIX-1; audit callers |
| `src/db/system.rs:1172` | async daemon cascade wrapper | `session_id`, reason, daemon marker | DB/systemd/pidfd only; no direct tmux kill | No direct FIX-1; audit callers |
| `src/monitor/session_watch.rs:117` | anchor unit stopped -> cascade session agents | `session_id` from watch task | DB/systemd/pidfd cascade only; no tmux pane kill directly | No direct FIX-1; downstream cascade only |
| `src/monitor/master_watch.rs:596` | master death cleanup reaps workers | DB session snapshot worker ids; optional daemon marker | DB/systemd/pidfd/registry cleanup. The registry cleanup path kills tmux agent sessions without validating current ownership | FIX-1 in registry cleanup helper |
| `src/monitor/master_watch.rs:840` | newly spawned revived master pane when finalize CAS goes stale | fresh `pane` returned by same spawn call | Mostly Y by construction: pane was just created in this function and stale DB finalize was detected. No current pid/session check before kill | Optional FIX-1 belt-and-suspenders |
| `src/monitor/master_watch.rs:1032` | master revive readiness budget missing -> cascades agents | `session_id` | DB/systemd/pidfd cascade only | No direct FIX-1; downstream cascade only |
| `src/monitor/master_watch.rs:1038` | failed revived master reap after readiness budget missing | `expected_pid`, `runtime_expected_generation`, `pane` from current revive attempt | Partial. `reap_failed_revive_master_best_effort` first checks DB `master_pid` and `master_generation`, then SIGKILLs by pid/pidfd and kills pane. It does not check tmux `#{pane_pid}` immediately before pane kill | FIX-1 |
| `src/monitor/master_watch.rs:1075` | master revive readiness timeout -> cascades agents | `session_id` | DB/systemd/pidfd cascade only | No direct FIX-1; downstream cascade only |
| `src/monitor/master_watch.rs:1081` | failed revived master reap after readiness timeout | `expected_pid`, `runtime_expected_generation`, `pane` | Partial DB runtime generation fence, no tmux pane ownership check | FIX-1 |
| `src/monitor/master_watch.rs:1157` | recovered worker readiness timeout -> cascades agents | `session_id` | DB/systemd/pidfd cascade only | No direct FIX-1; downstream cascade only |
| `src/monitor/master_watch.rs:1163` | failed revived master reap after worker readiness timeout | `expected_pid`, `runtime_expected_generation`, `pane` | Partial DB runtime generation fence, no tmux pane ownership check | FIX-1 |
| `src/monitor/master_watch.rs:1192` | post-error failed revive runtime lookup | DB `session_id`, `master_generation` -> `master_pid`, `master_pane_id` | DB generation lookup only; actual pane kill happens later | Feed into FIX-1 at pane kill |
| `src/monitor/master_watch.rs:1251` | failed revived master process and pane | DB-fenced `session_id`, `master_pid`, `generation`, `pane` | Partial. `master_runtime_generation_matches` checks DB row still matches pid/generation before process/pane reap; no current tmux pane pid/session check | FIX-1 |
| `src/monitor/master_watch.rs:1362` | failed revived master process | pidfd monitor key `master:{session_id}:{generation}` or raw `master_pid` | Partial. pidfd path is strong; raw pid fallback can be stale if pid reused. This is process, not pane ownership | Consider pid liveness hardening, outside requested pane FIX-1 |
| `src/monitor/master_watch.rs:1410` | failed revived master pane | `TmuxPaneId` plus generation for duplicate-event suppression | N. Duplicate suppression is not ownership validation; it kills pane id directly | FIX-1 |
| `src/rpc/handlers/sessions.rs:435` | newly spawned master pane when claimed generation CAS is stale | fresh `pane` returned by same spawn call | Mostly Y by construction, but no current tmux pid/session check before kill | Optional FIX-1 belt-and-suspenders |
| `src/rpc/handlers/sessions.rs:522` | master cutover rollback scope | cutover row by `cutover_id`, `new_master_pane_id`, `session_id` | Partial. Cutover row ties pane to cutover/session in DB, but no current tmux pane pid/session check before kill | FIX-1 |
| `src/rpc/handlers/sessions.rs:540` | new master pane during cutover rollback | DB `master_cutovers.new_master_pane_id` | N for current tmux ownership; DB cutover state only | FIX-1 |
| `src/rpc/handlers/sessions.rs:585` | cutover rollback worker cleanup | agent id into registry cleanup | Indirect registry kill by agent session name, no current tmux ownership validation | FIX-1 in registry cleanup helper |
| `src/rpc/handlers/agent.rs:701` | `ah kill <agent>` process kill | DB `agents.pid` | N for process reuse; no tmux pane kill here and no registry cleanup in this handler | Not one of FIX-1/2/3, but raw pid kill is a separate reuse risk |
| `src/rpc/handlers/agent.rs:1235` | cleanup after failed agent spawn | freshly spawned `pane_id` in same function | Mostly Y by construction; no current tmux ownership check before pane kill | Optional FIX-1 belt-and-suspenders |
| `src/agent_io/registry.rs:101` | shared agent runtime cleanup | in-memory registry entry: `agent_id`, `session_id`, `pane_id`, `socket_name`, fifo path | Partial. Entry is removed atomically from registry and captures pane first, then kills `agent_session_name(agent_id)`. It does not verify that the current tmux session/pane still belongs to that agent/session | FIX-1 |
| `src/agent_io/registry.rs:114` | agent tmux session during registry cleanup | derived `agent_session_name(agent_id)` on recorded socket | Weak. Name-scoped but no current pane/session validation | FIX-1 |
| `src/db/agents_lifecycle.rs:113` | mark-agent-killed cleanup path | agent id into registry cleanup | Indirect; depends on registry cleanup validation | FIX-1 in registry cleanup helper |
| `src/db/agents_lifecycle.rs:219` | mark-agent-crashed cleanup path | agent id into registry cleanup | Indirect; depends on registry cleanup validation | FIX-1 in registry cleanup helper |
| `src/orchestrator/mod.rs:562` | reap-only crashed worker cleanup | agent id into registry cleanup | Indirect; depends on registry cleanup validation | FIX-1 in registry cleanup helper |
| `src/orchestrator/mod.rs:577` | inactive-session crashed idle worker cleanup | agent id into registry cleanup | Indirect; depends on registry cleanup validation | FIX-1 in registry cleanup helper |
| `src/cli/start.rs:187` | start rollback calls `session.kill` after agent spawn failure | freshly created `session_id` | No local validation; delegates to `session.kill`. Since this should target a new active session, FIX-2 should not suppress it, but FIX-1 still protects tmux operations | FIX-1 in RPC path |
| `src/cli/up.rs:63` | `ah up` chooses session for realign from `session.list` | `session.list` array, `absolute_path`, `id` | N. It uses all rows; unlike `src/cli/start.rs:230`, it does not filter `status == ACTIVE`, so terminal corpses can produce "multiple running sessions match" | FIX-3 |
| `src/db/sessions.rs:363` | `session.list` summaries source for `ah up` | all sessions; includes `status`, `active_agents`, `master_pane_id` | N/A; intentionally lists all sessions. Caller must filter | FIX-3 at `src/cli/up.rs:63` |
| `src/bin/ahd.rs:207` | daemon shutdown tmux cleanup | session names from daemon shutdown inventory; socket name | Broad intentional shutdown cleanup. Not an `ah kill --session` path; kills all owned tmux resources for this daemon socket, then `kill-server` | Not FIX-1/2/3 for incident, but keep socket scoping |
| `src/bin/ahd.rs:219` | daemon shutdown `tmux kill-server` | current daemon tmux socket name | Broad intentional daemon shutdown; socket-scoped | Not FIX-1/2/3 for incident |
| `src/tmux/session.rs:397` | low-level `kill-pane` wrapper | raw `TmuxPaneId` on this `TmuxServer` socket | N. It is intentionally low-level and performs no ownership validation | Add guarded higher-level helper or require callers to guard |
| `src/tmux/session.rs:406` | low-level `kill-session` wrapper | raw tmux session name on this `TmuxServer` socket | N. Low-level wrapper | Add guarded higher-level helper or require callers to guard |
| `src/tmux/session.rs:436` | low-level `kill-window` wrapper | raw `session:window` target | N. No production call found in kill/cascade paths, but same class if used | Guard before future use |

## Current tmux ownership introspection

Existing wrapped API:

- `src/tmux/session.rs:299` reads `#{pane_pid}` with `tmux -L <socket> display-message -p -t <pane> "#{pane_pid}"`.
- `src/tmux/session.rs:464` lists pane ids for a window target with `list-panes -F "#{pane_id}"`.
- `src/tmux/session.rs:415` sets pane title, but current master titles are only `master (<cmd>)` at `src/rpc/handlers/sessions.rs:397` and `src/monitor/master_watch.rs:825`; agent titles are only `{agent_id} ({provider})` at `src/rpc/handlers/agent.rs:266`.

Available but not currently wrapped:

- tmux can report current pane owner fields with `display-message -p -t <pane>`, for example `#{session_name}`, `#{window_name}`, `#{pane_id}`, `#{pane_pid}`, and `#{pane_title}`.
- tmux user options could store explicit metadata with `set-option`/`show-options`, but no current code writes `session_id` or `master_generation` into pane/window/session metadata.

Conclusion: there is currently no reliable way to read `session_id + master_generation` back from tmux. The real mechanism available today is PID validation: compare the current `#{pane_pid}` for the pane against the DB row's expected `master_pid` and `master_generation`, matching the `38bae3a` precedent. For exact session/generation ownership, implementation would need to start writing explicit tmux metadata, such as pane title or user options, at spawn time.

## Coverage completeness

I swept production and test code with these patterns: `kill-session`, `kill-pane`, `kill-server`, `kill_pane`, `kill_session`, `kill_server`, `kill_`, `tmux`, `cascade`, `cleanup`, `reap`, `ah kill`, `session.kill`, and `agent.kill`.

Directories covered included `src/monitor`, `src/db`, `src/rpc`, `src/cli`, `src/orchestrator`, `src/tmux`, `src/agent_io`, `src/bin`, and broader `src`. I also checked `rg --files src/cli` to confirm there is no separate `src/cli/kill.rs`; CLI kill is in `src/bin/ah.rs`.

Test-only tmux `kill-server` and `kill-session` cleanup sites exist in modules such as `src/rpc/handlers.rs`, `src/tmux/mod.rs`, `src/tmux/session.rs`, `src/monitor/master_watch.rs`, and integration tests. They are not operational kill/cascade paths, so they are not assigned fixes above except where the same helper is a production wrapper.
