# State Contract Audit

Scope: static audit only. This grounds the current state surfaces in code so a follow-up state contract can separate discovery gaps from real schema gaps. Related phase-2 context: `.kiro/specs/ah-job-events/design.md` proposes RuntimeSnapshot schema v2 with additive `jobs[]`, `job_events[]`, and `job_event_cursor`; the current RuntimeSnapshot is schema version 1 and has no jobs surface.

## 1. Bare `ah start`

Claim: bare `ah start` starts or connects to the daemon before loading project config, then resolves config from `--config` or by searching for `ah.toml` from the current working directory. It does not silently synthesize a default project config when no config exists.

Evidence:

- CLI config input is only `--config`, a global optional path: `src/bin/ah.rs:45-49`.
- `ah start` dispatches to `cmd_start(&client, cli.config, wait)`: `src/bin/ah.rs:257-258`.
- `cmd_start` calls `ensure_daemon_running(client.socket())`, reads `std::env::current_dir()`, and passes that `cwd` to `start_from_options`: `src/bin/ah.rs:1176-1193`.
- With no explicit config path, `start_from_options` calls `find_config(&options.cwd)?`; only after that does it call `load_project_config`: `src/cli/start.rs:49-58`.
- `find_config` checks `CCB_CONFIG_PATH` first and errors if that env path is missing: `src/cli/config.rs:265-277`.
- If there is no env override, `find_config` walks upward from `start_dir` looking for `ah.toml`: `src/cli/config.rs:280-296`.
- If the walk finds no file, it returns `could not find ah.toml from ...; create one or set CCB_CONFIG_PATH`: `src/cli/config.rs:298-301`.
- Validation rejects an empty `agents` table: `src/cli/config.rs:153-160`.

Resolution order:

1. `--config` from the CLI is passed directly to `start_from_options`: `src/bin/ah.rs:1176-1190`, `src/cli/start.rs:53-56`.
2. If `--config` is absent, `CCB_CONFIG_PATH` is consulted inside `find_config`: `src/cli/config.rs:149-151`, `src/cli/config.rs:265-277`.
3. If that env var is absent, `ah.toml` is searched from CWD upward: `src/cli/config.rs:280-296`.
4. If no config is found, the command errors instead of using defaults: `src/cli/config.rs:298-301`.

Project identity and CWD behavior:

- When a config is found, `start_project` canonicalizes the passed `project_root` and derives `project_id` from that directory's final path component, falling back to `"project"` only if the name is absent: `src/cli/start.rs:61-76`.
- It creates the session with `absolute_path` equal to the canonicalized CWD/project root: `src/cli/start.rs:100-108`.

Verdict: a bare `ah start` with no discoverable `ah.toml` does not silently spin up a default project. It may start the daemon first, but project startup then fails at config discovery. A bare `ah start` with a discoverable config proceeds without an interactive guard or prompt.

## 2. `ah ps` Session Filter And `active_agents`

Claim: `ah ps` requests all session summaries from `session.list`; the DB query does not filter out terminal sessions. The human table then hides `status`, so terminal sessions can appear in `ah ps` without an obvious terminal marker.

Evidence:

- `cmd_ps` calls `session.list` with `{}` and renders every returned session row: `src/bin/ah.rs:1138-1146`.
- `handle_session_list` returns each session's `id`, `project_id`, `absolute_path`, `status`, `master_state`, `master_pane_id`, `active_agents`, and `created_at`: `src/rpc/handlers/sessions.rs:1161-1178`.
- `list_session_summaries_sync` selects from `sessions` with no `WHERE sessions.status ...` filter: `src/db/sessions.rs:363-377`.
- The CLI `SessionRow` fields are only `session_id`, `project_id`, `path`, `master_state`, and `active_agents`: `src/cli/output.rs:8-15`.
- `session_row` ignores the `status` value that RPC returned: `src/cli/output.rs:40-47`.

Claim: `active_agents` is a DB aggregate over agent state, not a live tmux/process truth.

Evidence:

- `SessionSummary.active_agents` is a stored response field: `src/db/sessions.rs:7-17`.
- The session summary query computes it as `SUM(CASE WHEN agents.state NOT IN ('CRASHED','KILLED') THEN 1 ELSE 0 END)`: `src/db/sessions.rs:368-376`.
- RuntimeSnapshot uses the same DB aggregate for session snapshots: `src/runtime_events.rs:316-340`.
- RuntimeSnapshot separately checks tmux liveness for agents and masters: master liveness is checked only for `status == "ACTIVE"` sessions at `src/runtime_events.rs:160-180`; agent `tmux_alive` is checked only for non-terminal agent states at `src/runtime_events.rs:184-213`.

Verdict: observed terminal rows in `ah ps` are expected from code. The authoritative JSON response from `session.list` includes `status`, but the text `ah ps` table does not show it. `active_agents` means "non-CRASHED/non-KILLED agents in DB", not "currently live tmux panes".

## 3. Master Normal `/exit` (`IDLE_MASTER_EXIT`)

Claim: when a watched active master exits, the pidfd watcher routes through master-death handling. If there is no active worker/job work, the session is marked `FAILED` with `master_last_exit_reason = 'IDLE_MASTER_EXIT'`.

Evidence:

- The master pidfd watcher waits for process exit and calls `handle_master_death_detected`: `src/monitor/master_watch.rs:417-490`.
- Death classification treats non-active sessions and in-flight cutovers as intentional/stale, and active matching pid/generation as `Revive`: `src/master_revival.rs:61-97`.
- `handle_master_death_detected` routes `MasterDeathDecision::Revive` into `revive_master_after_exit_locked`: `src/monitor/master_watch.rs:69-90`.
- The revive path snapshots session work; `IdleNoWork` is chosen when there is no active worker and no queued/dispatched job: `src/db/system.rs:220-227`.
- In the idle branch, master death handling calls `mark_session_failed_after_idle_master_death`, marks the recovery window `FAILED`, logs, and returns: `src/monitor/master_watch.rs:614-623`.
- `mark_session_failed_after_idle_master_death` writes `status = 'FAILED'`, `master_state = 'IDLE'`, clears `master_pending_tell_request`, and writes `master_last_exit_reason = 'IDLE_MASTER_EXIT'` for an active session: `src/monitor/master_watch.rs:1952-1973`.
- The sessions schema has `master_last_exit_reason TEXT`: `src/db/schema.rs:8-23`.

Claim: this path does not expose normal-close as a first-class RuntimeSnapshot lifecycle value today.

Evidence:

- RuntimeSessionSnapshot exposes `status`, `master_state`, `master_tmux_session`, `master_tmux_alive`, `master_pane_id`, `master_pid`, and `active_agents`; it does not expose `master_last_exit_reason`: `src/runtime_events.rs:71-83`.
- The runtime inventory query selects `sessions.status`, `sessions.master_state`, `sessions.master_pane_id`, `sessions.master_pid`, and `active_agents`, but not `master_last_exit_reason`: `src/runtime_events.rs:316-340`.
- `ah events` streams `runtime.subscribe` snapshots as JSON lines: `src/bin/ah.rs:107-110`, `src/bin/ah.rs:1310-1344`.
- `runtime.subscribe` builds an initial snapshot and then emits changed snapshots when runtime update broadcasts or interval tmux checks change the fingerprint: `src/rpc/handlers/runtime.rs:25-84`.

Tmux cleanup conclusion:

- The idle master-death branch shown above reaps worker runtime resources and writes DB state; in the cited branch there is no explicit master tmux session/pane teardown before returning: `src/monitor/master_watch.rs:584-623`.
- Worker cleanup can stop worker scopes and, depending on `preserve_session_anchor`, stop the session anchor: `src/db/system.rs:230-302`.

View comparison:

- `ah ps` and RuntimeSnapshot both read session state from DB-backed session inventory: `src/db/sessions.rs:363-395`, `src/runtime_events.rs:309-345`.
- They diverge in visible fields: `ah ps` hides `status` and all tmux-alive fields (`src/cli/output.rs:8-15`, `src/cli/output.rs:40-47`), while RuntimeSnapshot exposes `status`, `master_tmux_alive`, `master_pane_id`, and `master_pid` (`src/runtime_events.rs:71-83`, `src/runtime_events.rs:170-181`).
- RuntimeSnapshot still does not expose `master_last_exit_reason`, so a consumer cannot reliably distinguish `FAILED` because of normal idle master exit from other failed master paths. Other writers also store distinct reasons such as `OOM_OR_CRASH` during revive completion: `src/master_revival.rs:168-184`.

Verdict: the DB carries the reason (`IDLE_MASTER_EXIT`), but neither `ah ps` text nor RuntimeSnapshot v1 exposes it. RuntimeSnapshot is structurally closer to truth than `ah ps`, but the normal-close vs failed distinction is a genuine contract gap.

## 4. Dead-Pane-Only Tmux Session Reclamation

Claim: there are three relevant cleanup/reconcile mechanisms, but they cover different segments.

### Systemd scope and anchor dependencies

Evidence:

- Linux scope commands wrap processes in `systemd-run --user --scope --collect`: `src/platform/linux/scope.rs:125-138`, `src/platform/linux/scope.rs:199-212`, `src/platform/linux/scope.rs:290-305`.
- When a daemon unit is known, scope commands append `BindsTo=` and `PartOf=` dependencies: `src/platform/linux/scope.rs:308-314`.
- Agent scopes are described as `ccbd-agent-{agent_id}@{daemon_marker}`, which later cleanup code matches: `src/platform/linux/scope.rs:199-205`, `src/platform/linux/scope.rs:253-259`.
- If `ahd` is not under systemd and sandbox is not disabled, startup logs that cascade cleanup will rely on Startup Reconcile only: `src/bin/ahd.rs:70-73`.

Coverage: systemd can reap subprocess trees tied to the daemon/session anchor, and the DB cleanup paths can stop matching agent scopes and session anchors.

### Master-death reap

Evidence:

- Startup re-arm queries only active sessions with `master_pid > 0`: `src/monitor/master_watch.rs:321-345`.
- When re-arming, a missing or invalid stored master pane is treated as master death: `src/monitor/master_watch.rs:277-319`.
- A pidfd-open failure for an already-exited master also routes to `handle_master_death_detected`: `src/monitor/master_watch.rs:180-191`.
- Active matching master death is classified as revive-eligible and then goes through master-death cleanup: `src/master_revival.rs:61-97`, `src/monitor/master_watch.rs:584-623`.

Coverage: for active DB sessions, a dead/missing master pane or dead pid can cause DB recovery/failed-state handling. This is DB/state reconciliation; it is not, by itself, a tmux dead-pane removal API in the cited path.

### Session-watch anchor stop cascade

Evidence:

- Session watch polls `systemctl --user is-active` for `ahd-session-{session_id}.service`: `src/monitor/session_watch.rs:15-19`, `src/monitor/session_watch.rs:162-180`.
- After debounce, an inactive anchor calls `handle_confirmed_anchor_inactive`: `src/monitor/session_watch.rs:30-61`.
- That handler consults recovery-window cascade policy, then calls `cascade_kill_session_agents` on confirmed inactive anchors: `src/monitor/session_watch.rs:63-129`.
- Cascade marks active sessions as `KILLED`: `src/db/system.rs:368-387`.
- If the session is already `KILLED` or `FAILED`, cascade proceeds with cleanup; otherwise it returns without cleanup: `src/db/system.rs:388-401`.
- Cascade then marks non-terminal agents killed and clears marker/parser registries: `src/db/system.rs:404-441`.

Coverage: anchor-stop is an external lifecycle signal that can cascade agent cleanup and session DB state. It does not detect a dead tmux pane by querying tmux directly; it reacts to the systemd anchor becoming inactive.

### Startup reconcile and orphan-scope reconcile status

Evidence:

- `ahd` startup calls `db::system::reconcile_startup_with_tmux_socket(...)`: `src/bin/ahd.rs:84-101`.
- The async startup reconcile path calls `reconcile_active_agents_to_crashed_sync`, `reconcile_master_recovery_windows_with_runner_sync`, and `sweep_stale_tmux_sockets_sync`; it does not call orphan-scope reconciliation: `src/db/system.rs:1215-1235`.
- There is a sync helper `reconcile_startup_sync_with_state_dir` whose internal helper does call `reconcile_orphan_scopes_with_runner_sync`: `src/db/system.rs:530-563`.
- `reconcile_orphan_scopes_sync` is defined separately: `src/db/system.rs:565-571`.
- `reconcile_orphan_scopes_with_runner_sync` lists systemd scope units, filters to this daemon's known refs and orphan scopes, and stops them unless dry-run is enabled: `src/db/system.rs:577-615`.
- Grep found no production call to `reconcile_startup_sync_with_state_dir` or `reconcile_orphan_scopes_sync`; the visible production startup call is the async `reconcile_startup_with_tmux_socket` path above. The only direct hits for the sync startup/orphan functions are definitions and tests: `src/db/system.rs:530-563`, `src/db/system.rs:565-615`, `src/db/system.rs:1957`, `src/db/system.rs:2695-2823`.

Verdict: startup reconcile is wired, but the currently wired async startup path does not run orphan-scope reconciliation. Orphan-scope reconcile exists and is tested, but I did not find it wired into production startup.

## 5. RuntimeSnapshot Fields Vs Desired Shape

Current RuntimeSnapshot surface:

- RuntimeSnapshot schema v1 fields include top-level runtime state, tmux booleans, `sessions`, and `agents`: `src/runtime_events.rs:49-69`.
- RuntimeSessionSnapshot fields are `session_id`, `project_id`, `path`, `status`, `master_state`, `master_tmux_session`, `master_tmux_alive`, `master_pane_id`, `master_pid`, and `active_agents`: `src/runtime_events.rs:71-83`.
- RuntimeAgentSnapshot fields are `agent_id`, `session_id`, `provider`, `state`, `sub_state`, `pid`, `tmux_session`, and `tmux_alive`: `src/runtime_events.rs:85-95`.
- Active and inactive snapshots both set `schema_version: 1`: `src/runtime_events.rs:121-141`, `src/runtime_events.rs:239-258`.
- The fingerprint removes only `sequence` and `reason`; all other snapshot fields participate in suppression: `src/runtime_events.rs:261-270`.

Gap table:

| Desired field/shape | Exists, derivable, or gap | Current nearest | Evidence |
|---|---|---|---|
| Session terminal lifecycle subdivision: normal-close vs failed/crashed/stale | Genuine gap in RuntimeSnapshot. DB has `master_last_exit_reason`, but snapshot does not expose it. | `sessions[].status`, `sessions[].master_state`, `sessions[].master_tmux_alive` | DB column: `src/db/schema.rs:8-23`; idle normal close writer: `src/monitor/master_watch.rs:1952-1973`; snapshot fields omit reason: `src/runtime_events.rs:71-83`; query omits reason: `src/runtime_events.rs:316-340`. |
| Session phases like `closing`, `closed`, `stale` | Not present as explicit fields. Some parts are partially derivable from `status`, `master_tmux_alive`, and recovery-window behavior, but not reliably exposed as a lifecycle enum. | `RuntimeState::{Active,Inactive,Starting,Degraded}` and session `status` | RuntimeState values: `src/runtime_events.rs:21-28`; session fields: `src/runtime_events.rs:71-83`. |
| `cleanup.required` | Genuine gap. No cleanup object or cleanup-required boolean exists in session/agent snapshots. | `status`, `master_tmux_alive`, `active_agents`, `agents[].tmux_alive` | Snapshot structs: `src/runtime_events.rs:49-95`; live checks: `src/runtime_events.rs:160-213`. |
| `cleanup.safe_to_cleanup` | Genuine gap. Safety depends on policy/state not exposed in snapshot, such as recovery-window phase/defer decisions and active scope ownership. | Anchor cascade decision and recovery-window reconcile are internal only. | Session-watch consults recovery-window cascade policy before cleanup: `src/monitor/session_watch.rs:63-108`; startup recovery reconcile handles windows internally: `src/db/system.rs:636-707`; no such field exists in snapshot structs: `src/runtime_events.rs:49-95`. |
| Job inventory and job edge stream | Known v1 gap; phase-2 spec proposes `jobs[]`, `job_events[]`, and `job_event_cursor`. | `event.subscribe` has job terminal frames, but RuntimeSnapshot has no job arrays. | RuntimeSnapshot fields contain only sessions/agents: `src/runtime_events.rs:49-69`; `event_frame_for_job` synthesizes terminal `job_state_change` from `query_job`: `src/rpc/handlers/events.rs:289-310`. |

## Real Gaps Vs Discovery Gaps

| Reported pain / question | Classification | Grounded conclusion |
|---|---:|---|
| "Bare `ah start` silently creates a default daemon/project." | A: discovery gap / incorrect assumption | It starts the daemon first, but config discovery then requires `--config`, `CCB_CONFIG_PATH`, or an `ah.toml` found upward from CWD; missing config errors: `src/bin/ah.rs:1176-1193`, `src/cli/start.rs:49-58`, `src/cli/config.rs:265-301`. |
| "`ah ps` terminal sessions appear, so is it filtering wrong?" | A/B mixed | RPC intentionally returns all sessions; text `ah ps` hides the returned `status`. The all-session behavior is discoverable in `session.list`; the text view is lossy: `src/db/sessions.rs:363-395`, `src/rpc/handlers/sessions.rs:1161-1178`, `src/cli/output.rs:8-15`. |
| "`active_agents` does not match live tmux reality." | B: genuine contract/label gap | It is a DB count of agents not in `CRASHED`/`KILLED`, not a liveness count: `src/db/sessions.rs:368-376`. RuntimeSnapshot has separate tmux booleans but no explicit "live active agent count": `src/runtime_events.rs:184-213`. |
| "Studio is scraping `ah ps` + tmux instead of using events." | A: already served by RuntimeSnapshot discovery for sessions/agents | `ah events` streams RuntimeSnapshot JSON lines from `runtime.subscribe`: `src/bin/ah.rs:107-110`, `src/bin/ah.rs:1310-1344`, `src/rpc/handlers/runtime.rs:25-84`. That surface has structured session/agent state and tmux booleans today. |
| "Need normal-close vs failure in session lifecycle." | B: genuine schema gap | DB stores `master_last_exit_reason`, but RuntimeSnapshot does not expose it: `src/db/schema.rs:8-23`, `src/monitor/master_watch.rs:1952-1973`, `src/runtime_events.rs:71-83`, `src/runtime_events.rs:316-340`. |
| "Need cleanup.required / safe_to_cleanup." | B: genuine schema gap | No cleanup fields exist. Safety logic is internal to recovery/session-watch/orphan-scope paths: `src/runtime_events.rs:49-95`, `src/monitor/session_watch.rs:63-108`, `src/db/system.rs:577-615`, `src/db/system.rs:636-707`. |
| "Dead-pane-only tmux sessions need a reclamation contract." | B: genuine contract gap | Active-session dead master detection exists, and anchor/systemd cleanup exists, but there is no exposed cleanup signal and the wired startup path omits orphan-scope reconcile: `src/monitor/master_watch.rs:277-319`, `src/monitor/session_watch.rs:63-129`, `src/db/system.rs:1215-1235`, `src/db/system.rs:530-615`. |
| "Jobs are missing from RuntimeSnapshot." | B: known phase-2 gap | RuntimeSnapshot v1 has no `jobs[]`/`job_events[]`; current job terminal frames are separate and synthesized from job state: `src/runtime_events.rs:49-69`, `src/rpc/handlers/events.rs:289-310`. |
