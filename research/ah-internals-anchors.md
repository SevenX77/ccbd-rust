# ah Internals Anchors for `ah-config` / `ah-runtime-state`

Scope: grounded anchors for future built-in skill text. Every claim below is tied to current source file/line evidence. `Unreachable` means the brief named a mechanism that is not present in this checked working tree.

## 1. `.ah` layout and `ah.toml` declaration surface

### Project root discovery and `ah.toml`

- `ah.toml` is discovered upward from the start directory; `CCB_CONFIG_PATH` overrides discovery when set. Evidence: `src/cli/config.rs:144-145`, `src/cli/config.rs:257-268`.
- `ah.toml` deserializes into `ProjectConfig`, with top-level fields `version`, `master`, `completion`, `daemon`, `env`, `sandbox`, and `agents`. Evidence: `src/cli/config.rs:14-27`.
- `version` must be `"1"` and at least one `[agents.<id>]` must exist. Evidence: `src/cli/config.rs:148-155`.

### `ah.toml` structs and fields

- `[master]` is `MasterConfig`: `cmd: String`, `provider: Option<String>`, `readiness_timeout_s: u64`, `enabled: bool`, `window_size: TmuxWindowSize`, `hooks: HashMap<String, Vec<HookGroup>>`, `plugins: Vec<String>`, `skills: Vec<String>`, `bundle: Vec<String>`. Evidence: `src/cli/config.rs:44-67`.
- `[completion]` is `CompletionConfig`: `hook_push_enabled: bool`, default `false`. Evidence: `src/cli/config.rs:30-41`.
- `[daemon]` is currently empty: `DaemonConfig {}`. Evidence: `src/cli/config.rs:85-92`.
- `[agents.<id>]` is `AgentConfig`: `provider: String`, `env: HashMap<String, String>`, `hooks`, `plugins`, `skills`, `bundle`. Evidence: `src/cli/config.rs:94-107`.
- `[sandbox]` is `SandboxConfig`: `additional_ro_binds: Vec<String>`. Evidence: `src/cli/config.rs:109-112`.
- There is no direct `[master].mcp` or `[agents.<id>].mcp` field in `ProjectConfig` / `MasterConfig` / `AgentConfig`; MCP appears in `ExtensionConfig` and bundle contributions. Evidence: `src/cli/config.rs:44-67`, `src/cli/config.rs:94-107`, `src/provider/extensions.rs:17-19`.

### `.ah/` layout consumers

- `.ah/rules/<slot>.md`: `composed_rules_for_slot` reads `project_root/.ah/rules/{slot_id}.md`; missing file falls back to the role default. Evidence: `src/provider/home_layout.rs:535-549`.
- `.ah/skills/<name>/SKILL.md`: project skills are resolved under `project_root/.ah/skills`, must stay under that canonical root, and must contain `SKILL.md`. Evidence: `src/provider/skills.rs:30-40`, `src/provider/skills.rs:47-80`.
- `.ah/bundles/<name>/bundle.toml`: bundle resolution starts from `project_root/.ah/bundles`, checks the bundle directory and `bundle.toml`. Evidence: `src/provider/bundles.rs:77-95`, `src/provider/bundles.rs:185-210`.
- `.ah/bundles/` listing for CLI validation/listing comes from `list_bundle_names`. Evidence: `src/provider/bundles.rs:129-140`.

### Declaration flow into runtime

- `ExtensionConfig` carries `hooks`, `plugins`, `skills`, `bundle`, `rules`, `mcp`, `bundle_digest`, and `resolved_skills`. Evidence: `src/provider/extensions.rs:6-23`.
- `McpServerConfig` fields are `name`, `transport`, `command`, `args`, `env`, `url`, `headers`, `optional`; transport values are `stdio`, `http`, `sse`. Evidence: `src/provider/extensions.rs:26-42`, `src/provider/extensions.rs:44-50`.
- `ah start` sends master `hooks/plugins/skills/bundle/window_size` to `session.spawn_master_pane`. Evidence: `src/cli/start.rs:116-130`.
- `ah start` sends each worker `provider/env/hooks/plugins/skills/bundle`. Evidence: `src/cli/start.rs:140-150`, `src/cli/start.rs:260-266`.
- Worker spawn resolves bundle contributions before home materialization via `resolve_bundles_for_provider(..., BundleRole::Worker, &extensions)`. Evidence: `src/rpc/handlers/agent.rs:119-132`.
- Master spawn resolves bundle contributions before master home materialization via `resolve_bundles_for_provider(..., BundleRole::Master, &params.extensions)`. Evidence: `src/rpc/handlers/sessions.rs:289-305`.

## 2. Composition model: kernel + bundle + slot/default -> provider targets

### Rule composition order

- `compose_rules_with_layers` order is exactly: kernel, then each bundle layer, then `override_or_default`, joined by `\n\n---\n\n`. Evidence: `src/provider/home_layout.rs:523-532`.
- Slot override replaces the default role body: `composed_rules_for_slot` tries `.ah/rules/{slot_id}.md`; only when missing does it use `role_default_rules(role)`. Evidence: `src/provider/home_layout.rs:541-549`.
- Role kernel selection is `Master -> builtin::MASTER_KERNEL`, `Worker -> builtin::WORKER_KERNEL`. Evidence: `src/provider/home_layout.rs:552-556`.
- Role default selection is `Master -> builtin::DEFAULT_MASTER`, `Worker -> builtin::DEFAULT_WORKER`. Evidence: `src/provider/home_layout.rs:559-563`.

### Provider rule targets

- Claude rules target: `<home>/.claude/CLAUDE.md`. Evidence: `src/provider/home_layout.rs:502-505`.
- Codex rules target: `<home>/.codex/AGENTS.md`. Evidence: `src/provider/home_layout.rs:502-506`.
- Antigravity rules target: `<home>/.gemini/AGENTS.md`. Evidence: `src/provider/home_layout.rs:502-506`.
- Master role rules are only materialized for provider `claude`; non-Claude master rules return without writing. Evidence: `src/provider/home_layout.rs:492-499`.

### Built-in rule sources

- Built-in rule text is embedded into the binary via `include_str!`: `MASTER_KERNEL`, `WORKER_KERNEL`, `DEFAULT_MASTER`, `DEFAULT_WORKER`. Evidence: `src/provider/builtin.rs:1-6`.
- Since those are `include_str!` constants, changing their contents requires rebuilding the binary; they are not read dynamically from project files at runtime. Evidence: `src/provider/builtin.rs:3-6`.

### Project skills provider targets

- Claude project skills symlink from source dir to `<home>/.claude/skills/<name>`. Evidence: `src/provider/home_layout.rs:740-746`, `src/provider/skills.rs:86-90`, `src/provider/skills.rs:200-212`.
- Codex project skills symlink from source dir to `<home>/.codex/skills/<name>`. Evidence: `src/provider/home_layout.rs:750-754`, `src/provider/skills.rs:216-228`.
- Antigravity project skills symlink to `<home>/.gemini/config/skills/<name>`. Evidence: `src/provider/home_layout.rs:757-764`, `src/provider/home_layout.rs:1974-1982`.

### Built-in skills mechanism

- Unreachable in this checked tree: no `BUILTIN_SKILLS`, `BuiltinSkill`, `materialize_builtin_skills`, or `ah-commands` symbols are present under `src`, `assets`, or `tests` (`rg -n "BUILTIN_SKILLS|BuiltinSkill|materialize_builtin_skills|ah-commands" src assets tests -S` returned no matches). Evidence for existing `builtin.rs` only includes rule constants: `src/provider/builtin.rs:1-6`.
- Consequence for skill text: do not claim a built-in skill registry or built-in skill materialization exists unless a later commit adds it.

## 3. RuntimeSnapshot fields and state value domains

### Snapshot enums

- `RuntimeSnapshotReason` is serde `snake_case` with values `initial`, `inventory_changed`, `tmux_changed`, `agent_changed`, `shutdown`, `daemon_absent`, `daemon_lost`. Evidence: `src/runtime_events.rs:9-19`.
- `RuntimeState` is serde `snake_case` with values `active`, `inactive`, `starting`, `degraded`. Evidence: `src/runtime_events.rs:21-28`.

### `RuntimeSnapshot`

- `schema_version: u16`: snapshot schema version; inactive snapshots set it to `1`. Evidence: `src/runtime_events.rs:49-52`, `src/runtime_events.rs:121-128`.
- `event: String`: event label; inactive snapshots set `"snapshot"`. Evidence: `src/runtime_events.rs:51-52`, `src/runtime_events.rs:121-125`.
- `sequence: u64`: monotonically increasing per subscription/local stream sequence. Evidence: `src/runtime_events.rs:53`, `src/rpc/handlers/runtime.rs:40-48`, `src/rpc/handlers/runtime.rs:67-83`.
- `reason: RuntimeSnapshotReason`: why this snapshot was emitted. Evidence: `src/runtime_events.rs:54`, `src/rpc/handlers/runtime.rs:41-48`, `src/rpc/handlers/runtime.rs:57-70`.
- `runtime_state: RuntimeState`: derived aggregate state. Evidence: `src/runtime_events.rs:55`, `src/runtime_events.rs:216-230`.
- `config_path: Option<String>` and `workspace_path: Option<String>`: request/local context propagated into snapshots. Evidence: `src/runtime_events.rs:56-57`, `src/rpc/handlers/runtime.rs:13-17`.
- `state_dir: Option<String>` and `tmux_socket: Option<String>`: state/socket context. Evidence: `src/runtime_events.rs:58-59`.
- `ahd_alive: bool`: true for daemon-built snapshots, false for local inactive snapshots. Evidence: `src/runtime_events.rs:60`, `src/runtime_events.rs:121-130`.
- `active: bool`: aggregate all-needed-runtime-is-alive boolean. Evidence: `src/runtime_events.rs:61`, `src/runtime_events.rs:216-218`.
- `ahd_has_inventory: bool`: true when any session has `sessions.status == "ACTIVE"`. Evidence: `src/runtime_events.rs:62`, `src/runtime_events.rs:151`.
- `tmux_server_alive`, `master_tmux_alive`, `worker_tmux_alive`: tmux liveness booleans. Evidence: `src/runtime_events.rs:63-65`, `src/runtime_events.rs:151-168`, `src/runtime_events.rs:184-218`.
- `worker_tmux_expected_count: usize`: count of non-terminal agents expected to have tmux sessions. Evidence: `src/runtime_events.rs:66`, `src/runtime_events.rs:184-188`.
- `sessions: Vec<RuntimeSessionSnapshot>` and `agents: Vec<RuntimeAgentSnapshot>`: structured inventories. Evidence: `src/runtime_events.rs:67-68`, `src/runtime_events.rs:170-214`.

### `RuntimeSessionSnapshot`

- Fields: `session_id`, `project_id`, `path`, `status`, `master_state`, `master_tmux_session`, `master_tmux_alive`, `master_pane_id`, `master_pid`, `active_agents`. Evidence: `src/runtime_events.rs:71-83`.
- Source query maps `sessions.id`, `sessions.project_id`, `projects.absolute_path`, `sessions.status`, `sessions.master_state`, `sessions.master_pane_id`, `sessions.master_pid`, active agent count, and `sessions.created_at`. Evidence: `src/runtime_events.rs:313-341`.

### `RuntimeAgentSnapshot`

- Fields: `agent_id`, `session_id`, `provider`, `state`, `sub_state`, `pid`, `tmux_session`, `tmux_alive`. Evidence: `src/runtime_events.rs:85-95`.
- Source query maps `agents.id`, `session_id`, `provider`, `state`, `sub_state`, `pid`, and `created_at`. Evidence: `src/runtime_events.rs:347-373`.

### State value domain disambiguation

- `session.status`: default `ACTIVE`; cascade can set `KILLED`; rollback/cutover paths can set `FAILED`. Evidence: `src/db/schema.rs:8-23`, `src/db/system.rs:368-381`, `src/rpc/handlers/sessions.rs:601-612`.
- `sessions.master_state`: only `IDLE` or `BUSY` by schema check; it is separate from agent state. Evidence: `src/db/schema.rs:20-21`, `src/db/sessions.rs:191-210`.
- `agent.state`: constants are `SPAWNING`, `SPAWNING_INTERVENTION`, `IDLE`, `WAITING_FOR_ACK`, `BUSY`, `PROMPT_PENDING`, `STUCK`, `FAILED`, `CRASHED`, `KILLED`, `UNKNOWN`. Evidence: `src/db/state_machine.rs:13-28`, `src/db/state_machine.rs:3024-3037`.
- Active agent-state classification for state-machine tests: `SPAWNING`, `WAITING_FOR_ACK`, `BUSY` are active; `IDLE`, `SPAWNING_INTERVENTION`, `PROMPT_PENDING`, `STUCK`, `FAILED`, `CRASHED`, `KILLED` are not active. Evidence: `src/db/state_machine.rs:3039-3050`.
- `agent.sub_state`: optional free string stored in `agents.sub_state`; examples include `Asserted`, `LogEvent`, `HookEvent`, but no schema enum constrains it. Evidence: `src/db/schema.rs:72-88`, `src/db/state_machine.rs:31-33`, `src/db/state_machine_assert.rs:57-81`.
- `jobs.status`: default `QUEUED`; claim changes `QUEUED -> DISPATCHED`; terminal statuses include `COMPLETED`, `FAILED`, `CANCELLED`; `cancel_requested` is separate from status. Evidence: `src/db/schema.rs:155-173`, `src/db/jobs.rs:56-58`, `src/db/jobs.rs:209-212`, `src/db/jobs.rs:380-382`, `src/db/jobs.rs:395-408`, `src/db/jobs.rs:455-482`.
- CLI terminal wait accepts `COMPLETED`, `FAILED`, `CANCELLED`, `KILLED` event frame states, but job table writes shown above use `COMPLETED`, `FAILED`, `CANCELLED`. Evidence: `src/bin/ah.rs:1452-1460`, `src/db/jobs.rs:380-482`.
- `evidence.status`: default `PENDING`; assertion path updates to `REVIEWED`. Evidence: `src/db/schema.rs:137-153`, `src/db/state_machine_assert.rs:75-81`.
- `master_cutovers.state`: `PREPARING`, `SPAWNING`, `VERIFYING`, `ACTIVE`, `ROLLED_BACK`, `FAILED`, `RELEASED`. This is not `session.status`, `agent.state`, or `job.status`. Evidence: `src/db/schema.rs:25-53`.
- `master_recovery_windows.phase`: `DETECTED`, `WORKERS_REAPED`, `MASTER_SPAWNING`, `MASTER_RUNNING`, `MASTER_VERIFYING`, `WORKERS_REPROVISIONING`, `COMPLETED`, `FAILED`, `FUSED`. This is recovery-window phase, not job status. Evidence: `src/db/schema.rs:55-70`.
- `agent_recovery_intents.action`: `REVIVE`, `REVIVE_IDLE`, `REAP_ONLY`. This is recovery intent action, not state/status. Evidence: `src/db/schema.rs:101-119`.
- `RUNNING` was not found as a DB state/status enum in the inspected schema/state constants; it appears in prose/log contexts only, not as one of the authoritative status domains above. Evidence: `src/db/schema.rs:8-23`, `src/db/schema.rs:72-88`, `src/db/schema.rs:155-173`, `src/db/state_machine.rs:13-28`.

## 4. Reading authoritative state and CLI output shapes

### `ah events`

- `ah events` supports only `--format json`; any other format errors. Evidence: `src/bin/ah.rs:1310-1315`.
- It resolves config path, derives socket/state dir, then subscribes to RPC method `runtime.subscribe` and prints each streamed line directly. Evidence: `src/bin/ah.rs:1317-1343`.
- On daemon close, absent daemon, or I/O loss, it emits a local inactive `RuntimeSnapshot` with reason `daemon_lost` or `daemon_absent`, then reconnects. Evidence: `src/bin/ah.rs:1345-1399`.
- The runtime subscription writes each `RuntimeSnapshot` as one JSON line with a trailing newline. Evidence: `src/rpc/handlers/runtime.rs:25-52`, `src/rpc/handlers/runtime.rs:87-105`.
- The subscription starts with an `initial` snapshot, then emits changed snapshots on runtime update broadcasts or timer-driven `tmux_changed` checks; unchanged fingerprints are suppressed. Evidence: `src/rpc/handlers/runtime.rs:41-83`.
- Runtime JSON includes `schema_version` because it is a serialized field of `RuntimeSnapshot`. Evidence: `src/runtime_events.rs:49-68`.

### `ah ps`

- `ah ps` calls `session.list` and prints a `tabled` table under `sessions`, then calls `system.dump` and prints a `tabled` table under `agents`, then prints a tmux hint. Evidence: `src/bin/ah.rs:1138-1158`.
- Session table columns are `session_id`, `project_id`, `path`, `master_state`, `active_agents`. Evidence: `src/cli/output.rs:8-15`, `src/cli/output.rs:40-47`.
- Agent table columns are `agent_id`, `provider`, `state`, `sub_state`, `pid`. Evidence: `src/cli/output.rs:17-24`, `src/cli/output.rs:30-37`.
- Therefore `ah ps` is human-facing text/table output, not the full structured runtime schema.

### Recommended authoritative read path

- For structured current state, prefer `ah events --format json`: it exposes `RuntimeSnapshot` fields including `ahd_alive`, `tmux_server_alive`, `master_tmux_alive`, `worker_tmux_alive`, `sessions[]`, and `agents[]`. Evidence: `src/runtime_events.rs:49-95`, `src/bin/ah.rs:1310-1343`.
- Do not reconstruct authority by scraping `ah ps` plus ad hoc tmux commands: `ah ps` omits many RuntimeSnapshot fields and is formatted via table rendering. Evidence: `src/bin/ah.rs:1138-1158`, `src/cli/output.rs:8-24`.

## 5. Cleanup semantics and external delivery feasibility

### Cleanup / reap semantics

- Per-agent runtime cleanup removes the registered agent I/O entry, aborts reader, removes FIFO, captures pane at death, kills the agent tmux session, and removes the agent sandbox home under default policy. Evidence: `src/agent_io/registry.rs:97-128`.
- Cleanup also cancels marker, completion, parser, and monitor registries. Evidence: `src/agent_io/registry.rs:148-153`.
- Session cascade marks `sessions.status = 'KILLED'` when active, notifies runtime inventory changed, then selects non-terminal agents for cleanup. Evidence: `src/db/system.rs:368-409`.
- `clean_worker_runtime_resources_with_runner_sync` clears in-memory registries, stops matching systemd scopes/session anchor when available, sends pidfd SIGKILL where possible, and marks workers killed for master-death cleanup. Evidence: `src/db/system.rs:230-347`.
- `remove_agent_sandbox_dir_sync` maps `state_dir/sandboxes/<session>/<agent>` to a materialized home and removes that home. Evidence: `src/db/system.rs:887-895`.
- `SandboxDirGuard` also removes the derived sandbox home and sandbox dir on drop unless released. Evidence: `src/sandbox/path.rs:23-71`.

### How ah locates an agent/master home

- `resolve_sandbox_dir(state_dir, session_id, agent_id)` creates `<state_dir>/sandboxes/<session_id>/<agent_id>`. Evidence: `src/sandbox/path.rs:6-20`.
- Home materialization then derives `home_root` from that sandbox dir and creates it. Evidence: `src/provider/home_layout.rs:139-152`.
- Provider home env always sets `HOME=<home_root>` and may add provider-specific env entries such as `CLAUDE_CONFIG_DIR` or `CODEX_HOME`. Evidence: `src/provider/home_layout.rs:1615-1624`, `src/provider/home_layout.rs:239-267`.
- Worker spawn uses `resolve_sandbox_dir` and, when the provider requires home materialization, calls `prepare_home_layout_with_extensions_for_slot`; this is the path that produces the managed sandbox home. Evidence: `src/rpc/handlers/agent.rs:133-172`.
- Master spawn uses the same pattern for `agent_id = "master"` when sandboxing is enabled. Evidence: `src/rpc/handlers/sessions.rs:320-346`.

### External delivery feasibility

- No CLI subcommand named `skills`, `skill`, `export`, or `install` exists in `Cmd`; available top-level verbs are ping/version/ps/start/up/ask/tell/pend/cancel/kill/watch/events/logs/attach/stop/master/agent/doctor/setup/config/bundle/prompt. Evidence: `src/bin/ah.rs:53-169`.
- `rg -n "Skills|Skill|Export|Install|cmd_skills|skills" src/bin/ah.rs src/cli -S` found config/start/up/master-cutover skill fields and bundle/setup mentions, but no skills export/install verb. Evidence: `src/bin/ah.rs:53-169`, `src/cli/start.rs:116-130`, `src/cli/start.rs:260-266`.
- Current project skills are materialized into ah-managed provider homes under the sandbox home, by symlink, not into a user's real `~/.claude/skills`, `~/.codex/skills`, or `~/.gemini/config/skills`. Evidence: `src/provider/home_layout.rs:139-152`, `src/provider/home_layout.rs:740-764`.
- No current mechanism was found that writes rules or skills to an ah-managed sandbox之外 external agent home. The only provider rule/skill materialization targets are under `home_root`, and `home_root` is derived from the ah sandbox dir. Evidence: `src/provider/home_layout.rs:139-152`, `src/provider/home_layout.rs:502-516`, `src/provider/home_layout.rs:740-764`.
- Conclusion: external delivery of skills/rules to a non-ah-managed agent home would be a new mechanism or subcommand in this tree.
