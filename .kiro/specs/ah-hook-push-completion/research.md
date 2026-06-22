# ah hook 推送式完成信号 research

## §A ah 现有 pull 完成检测架构

### 当前数据流

```text
orchestrator dispatch/resume
  -> collect initial provider log cursors
  -> register LogMonitorEntry(provider, log_root, LogReadState)
  -> spawn_log_monitor_task
  -> run_log_monitor_tick every 250ms
  -> read_provider_log_tail_with_state
  -> parse_provider_log_line
  -> mark_agent_idle_log_event
  -> CAS UPDATE agents ... state='IDLE', sub_state='LogEvent'
  -> job completion/cancel + state_change event + orchestrator wake
```

关键证据：

- `src/orchestrator/mod.rs:879-893` 收集初始 log cursor 并构造 `LogReadState::from_cursors`。
- `src/orchestrator/mod.rs:894-902` 注册 `LogMonitorEntry`。
- `src/orchestrator/mod.rs:903-910` 启动 `spawn_log_monitor_task`。
- `src/completion/registry.rs:9-13` registry entry 保存 `provider`、`log_root`、`LogReadState`、cancel tx。
- `src/completion/registry.rs:19-27` 新 monitor 注册会替换并取消旧 monitor。
- `src/completion/registry.rs:55-65` monitor tick 后写回持久化 read state。
- `src/completion/monitor.rs:9-10` pull tick 间隔是 250ms，最大等待使用 `DEFAULT_STUCK_THRESHOLD`。
- `src/completion/monitor.rs:20-28` `run_log_monitor_tick` 从 provider log tail 读取完成事件。
- `src/completion/monitor.rs:32-45` 对每个 `TurnComplete` 调 `db::state_machine::mark_agent_idle_log_event(...)`。
- `src/completion/monitor.rs:48-58` DB transition 成功后通知 job update、唤醒 orchestrator，并返回 completed。
- `src/completion/monitor.rs:75-129` `spawn_log_monitor_task` 只在 agent `WAITING_FOR_ACK` 或 `BUSY` 时循环拉取；完成、非 active、超时都会退出。

### log 主信号解析

- `src/completion/parser.rs:18` 是统一入口 `parse_provider_log_line`。
- `src/completion/parser.rs:23-27` provider 分发当前只识别 `"codex"` 和 `"claude"`；其他 provider 返回 `NotTerminal`。
- `src/completion/parser.rs:30-50` Codex 完成信号要求 top-level `"type":"event_msg"` 且 payload `"type":"task_complete"`，并提取 `turn_id`、`last_agent_message`。
- `src/completion/parser.rs:53-99` Claude 完成信号来自 assistant message 的 `stop_reason`，其中 `end_turn | stop_sequence | max_tokens` 被视为 `TurnComplete`；`tool_use` 被视为非终止；未知/缺失 `stop_reason` 进入 `UnknownDegrade`，见 `src/completion/parser.rs:85-98`。

### LogReadState 与 Claude armed guard

- `src/completion/reader.rs:10-14` `LogReadState` 保存 `cursors: LogCursorMap` 和 `claude_user_entry_seen_paths: BTreeSet<PathBuf>`。
- `src/completion/reader.rs:64-73` `read_provider_log_tail_with_state` clone 旧 state 并收集排序 provider log 文件。
- `src/completion/reader.rs:75-83` 每个文件按 cursor 增量读；Claude 若 cursor 超过文件长度会重置；跨 tick 使用 `claude_user_entry_seen_paths`。
- `src/completion/reader.rs:103-123` Claude 先看到 `UserMessage` 才 armed；只有 provider 非 Claude 或该路径 armed 时才接收 `TurnComplete`，接收后 disarm。
- `src/completion/reader.rs:132-139` scan 完后更新 cursor state。

### 状态机 transition 与 CAS 仲裁

- `src/db/state_machine.rs:31-32` 定义 `LOG_EVENT_TASK_COMPLETE` 和 `LogEvent`。
- `src/db/state_machine.rs:456-467` log-event transition 先读取当前 `(state,state_version)`。
- `src/db/state_machine.rs:475-480` 只接受当前状态 `WAITING_FOR_ACK` 或 `BUSY`。
- `src/db/state_machine.rs:516-520` CAS 更新 agent：`state='IDLE'`、`sub_state='LogEvent'`、`state_version=state_version+1`，并要求 `state_version=?`。
- `src/db/state_machine.rs:523-530` transition 成功后完成或取消当前 dispatched job。
- `src/db/state_machine.rs:532-549` 插入 `state_change` event，payload 带 `reason`、`provider`、`raw_path/raw_offset`、`provider_turn_id`、`schema_version`、`reply_source`。
- `src/db/state_machine.rs:551-554` CAS 竞争失败时吞掉结果，不重复 transition。
- `src/db/state_machine.rs:112-147` 通用状态 transition 同样按 `state_version` CAS 更新。

### 现有入站通道盘点

- `src/rpc/mod.rs:23-35` ahd 通过 Unix socket 监听 JSON-RPC，并对 socket chmod `0600`。
- `src/rpc/mod.rs:37-73` 每个连接按 JSON-lines 读取；`event.subscribe` 走特殊 streaming 路径，其余交给 router dispatch。
- `src/rpc/mod.rs:77-91` `event_subscribe_params` 只识别 `event.subscribe`。
- `src/rpc/mod.rs:93-108` socket bind 会清理 stale socket。
- `src/rpc/router.rs:15-42` 当前 method whitelist 包括 `agent.assert_state`、`evidence.insert`、`event.subscribe`、job/session 等，也包括现成 push-ingest 样板 `master.ack_ready`，见 `src/rpc/router.rs:20`。
- `src/rpc/router.rs:44-79` JSON-RPC parse 与 method 分发入口。
- `src/rpc/router.rs:80-105` dispatch 映射现有 handlers，其中 `src/rpc/router.rs:84` 把 `master.ack_ready` 分发到 `handle_master_ack_ready`；`event.subscribe` 是订阅事件，不是 ingest。
- `src/rpc/handlers/events.rs:8-30` 非 streaming `event.subscribe` 返回/backfill event frame 或 timeout。
- `src/rpc/handlers/events.rs:33-90` streaming subscription 读取现有 events 并订阅 pubsub，属于出站消费通道。
- `src/bin/ah.rs:145-158` 定义外部 CLI 子命令 `ah master ack-ready --cutover-id ...`；`src/bin/ah.rs:216-220` 把该子命令路由到 `cmd_master_ack_ready`；`src/bin/ah.rs:317-335` 组装 JSON-RPC `master.ack_ready`，payload 带 `cutover_id`、`pid`、`observed_socket`。
- `src/rpc/handlers/sessions.rs:720-732` `handle_master_ack_ready` 从 params 读取 `cutover_id`，调用 `mark_master_cutover_ack_ready` 写 DB 并返回 `ack_ready: true`。
- `src/db/master_cutovers.rs:190-212` `mark_master_cutover_ack_ready` 更新 `master_cutovers.ack_ready_at/readiness_mode/updated_at`，并限制 `state = 'VERIFYING'`。
- `src/rpc/handlers/sessions.rs:608-687` 正在等待 readiness 的 cutover loop 轮询 DB；`src/rpc/handlers/sessions.rs:625-628` 看到 `ack_ready_at` 后返回 readiness mode。
- `src/rpc/handlers/sessions.rs:1527-1550` 测试覆盖 `master.ack_ready` method 已注册；`src/rpc/handlers/sessions.rs:1552-1570` 测试覆盖 handoff/master rules 中包含 `ah master ack-ready --cutover-id "$AH_CUTOVER_ID"`。

事实结论：当前完成信号入口形态是 monitor tick 拉 provider 原生日志；ahd 已有 UDS JSON-RPC 入站框架，且 `master.ack_ready` 已形成现成 push-ingest 样板：外部 CLI 子命令 -> JSON-RPC method -> handler 写 DB -> 正在跑的 saga/monitor loop 反应。这个模式与“hook 触发 ah 子命令并携带 agent 身份 -> RPC ingest -> handler 写状态”同构，可作为后续 hook completion ingest 的首选复用模式。

## §B 三 provider hook 能力对齐表

本节把一手证据和二手参考分开：cmux 文档是机制参考；本机二进制/config grep/strings 是当前环境事实。

| provider | 本机版本/路径证据 | 配置路径证据 | hook 事件/上下文证据 | 对齐事实 |
|---|---|---|---|---|
| Codex | `command -v codex` -> `/home/sevenx/.npm-global/bin/codex`；`codex --version` -> `codex-cli 0.135.0`。 | 当前隔离 HOME 缺少 `$HOME/.codex/hooks.json`；`$HOME/.codex/config.toml:1-6` 只有 trusted project 与 tui 配置；`docs/competitive/cmux-vibeyard-borrowable-for-ah.md:50` 说 cmux setup 写 `codex→~/.codex/hooks.json`。 | 对本机 `codex.js` 执行 `strings ... | rg -i 'hook|hooks|Stop|PreToolUse|...'` 没有返回 hook 事件字符串；ah 现有 Codex materialization 只覆盖 managed home/config/plugins，`src/provider/home_layout.rs:196-214`、`src/provider/home_layout.rs:669-705`。 | 本机一手证据不足以确认 Codex CLI 0.135.0 hook schema；cmux 文档给出 `~/.codex/hooks.json` 机制参考，但需后续用 Codex 官方/实际配置验证。 |
| Claude | `command -v claude` -> `/home/sevenx/.local/bin/claude`；`claude --version` -> `2.1.183 (Claude Code)`。 | 当前 `$HOME/.claude/settings.json:2-13` 有 `hooks.Stop` command；该命令是 `ccb-provider-finish-hook --provider claude --event finish --completion-dir ... --agent-name a3 --workspace ...`，属于现役同型 prior art：hook -> completion 推送，agent 身份由命令行参数注入；ah 可 materialize Claude hook 到 `.claude/hooks`，`src/provider/home_layout.rs:453-459`。 | `src/provider/home_layout.rs:555-580` 写 `.claude/settings.json` 并注入 hooks；`src/provider/home_layout.rs:583-612` Claude hook shape 是 `settings["hooks"][event]` array，含 `matcher` 与 `hooks[{type,command,timeout?}]`；`tests/pr4c_hooks_plugins.rs:17-48` 覆盖 `PreToolUse` command hook 与 symlink；`tests/pr1b_readfirst_hook.rs:367-395` 覆盖 Claude hook stdout protocol `hookSpecificOutput.permissionDecision`。 | 一手代码/配置证据确认 Claude 支持 command hook 配置；当前环境已有 `Stop` hook 配置。事件全集未从 Claude binary strings 得到可靠 schema，现有 repo 实证至少覆盖 `Stop`、`PreToolUse` 与 permission-decision 输出。 |
| Antigravity / agy | `command -v agy` -> `/home/sevenx/.local/bin/agy`；`agy --version` -> `1.0.7`。 | 当前 `$HOME/.gemini/antigravity-cli/settings.json` 不存在；`src/provider/home_layout.rs:217-239` materialize Antigravity home；`src/provider/home_layout.rs:242-270` 从 `.gemini/antigravity-cli/settings.json` copy settings 并写 trusted workspace。 | `strings /home/sevenx/.local/bin/agy | grep -aE ...` 返回 `HookArgs.GetPreToolHookArgs`、`GetPostToolHookArgs`、`GetPreInvocationHookArgs`、`GetPostInvocationHookArgs`、`GetStopHookArgs`、`HookSystemMessage`、`StopHookArgs.GetExecutionNum/GetTerminationReason/GetError/GetFullyIdle`、`CustomAgentConfig.GetPreInvocationHooks/GetPostInvocationHooks/GetStopHooks`、`DeclarativeMixinConfig.GetPreToolHooks/GetPostToolHooks/GetStopHooks`、`jsonhook.JSONHookSpec.IsEnabled`、`jsonhook.ParseHooksFile` 等。 | 一手 strings 证据确认 agy binary 内有 Pre/PostTool、Pre/PostInvocation、Stop 与 jsonhook 引擎符号；当前环境未见 antigravity settings 文件和 hooks 键。 |

共同子集事实：

- Claude 与 Antigravity 都有完成/停止类事实证据：Claude 当前配置有 `Stop` hook，Antigravity strings 有 `GetStopHookArgs`/`CallStopHook`/`runStopHooks`。
- Claude 与 Antigravity 都有 tool 前后事件事实证据：Claude repo test 覆盖 `PreToolUse`，Antigravity strings 有 `PreToolHookArgs`/`PostToolHookArgs`。
- Gemini/Antigravity 家族配置 shape 不完全同构：Gemini hook 注入代码 `src/provider/home_layout.rs:614-640` 写的是 event array 里的 `{type, command, matcher, timeout?}`；Claude `src/provider/home_layout.rs:583-612` 写的是 event array 里的 `{matcher, hooks:[...]}`。
- Codex 当前本机一手证据缺口最大：cmux 文档说有 `~/.codex/hooks.json`，但本机 Codex CLI strings 和当前 `.codex/config.toml` 没验证出 hook event schema。

## §C ah provider 配置注入点现状

配置 schema：

- `src/cli/config.rs:23-40` master config 支持 `hooks: HashMap<String, Vec<HookGroup>>` 和 `plugins`。
- `src/cli/config.rs:64-72` agent config 支持 `provider`、`env`、`hooks`、`plugins`。
- `src/provider/extensions.rs:4-10` `ExtensionConfig` 汇总 hooks/plugins。
- `src/provider/extensions.rs:12-16` `HookGroup` 包含 `matcher` 和 `hooks: Vec<HookItem>`。
- `src/provider/extensions.rs:18-46` hook group 可反序列化为短字符串 command 或结构化 group。
- `src/provider/extensions.rs:49-64` `HookItem` 包含 `type`、`command`、`timeout`，默认 type 是 `"command"`。

provider home materialization：

- `src/provider/home_layout.rs:14-22` auth whitelist 包括 Claude、Codex、Gemini、Antigravity 相关 credential 文件。
- `src/provider/home_layout.rs:107-139` `prepare_home_layout_with_extensions` 是 provider materialization 总入口，按 provider 分发到 `claude`、`gemini`、`codex`、`antigravity`。
- `src/provider/home_layout.rs:142-167` Claude home：创建 `.claude`、projects、session env，写 builtin rules/trust/plugins/hooks/settings，返回 `CLAUDE_CONFIG_DIR` 等 env。
- `src/provider/home_layout.rs:170-193` Gemini home：创建 `.gemini`、tmp，写 builtin rules/settings/trusted folders/hooks/state。
- `src/provider/home_layout.rs:196-214` Codex home：调用 `prepare_managed_codex_home` 并返回 `CODEX_HOME`。
- `src/provider/home_layout.rs:217-239` Antigravity home：创建 antigravity dir，写 settings/onboarding/builtin rules。
- `src/provider/home_layout.rs:301-335` builtin rules target：Claude `.claude/CLAUDE.md`、Gemini `.gemini/GEMINI.md`、Codex `.codex/AGENTS.md`、Antigravity `.gemini/AGENTS.md`；master rules 只对 Claude master。
- `src/provider/home_layout.rs:453-502` Claude/Gemini hooks 先 symlink 到 provider home 下的 hooks dir，再把 command rewrite 成 materialized path。
- `src/provider/home_layout.rs:505-530` Gemini settings 从 source `.gemini/settings.json` copy 后注入 hooks、写 auth selectedType。
- `src/provider/home_layout.rs:555-580` Claude settings 注入 bypass/settings/hooks/plugins。
- `src/provider/home_layout.rs:669-705` Codex managed home 创建 `.codex`、rules、sessions、`config.toml`、trust、plugins、version/migration。

spawn 注入路径：

- `src/rpc/handlers/agent.rs:96-123` worker agent spawn 时，如果 provider `requires_home_materialization`，调用 `prepare_home_layout_with_extensions(..., HomeLayoutRole::Worker, extensions)`，然后把返回 env 加入 spawn env。
- `src/rpc/handlers/sessions.rs:291-300` master spawn 只对 Claude master materialize `HomeLayoutRole::Master` 并扩展 env。
- `src/sandbox/systemd.rs:13-24` 和 `src/sandbox/systemd.rs:29-45` systemd wrapper 把 env 前缀注入 provider command。
- `src/provider/manifest.rs:8-24` provider manifest 定义 command、resume_args、env passthrough、injected env vars、`requires_home_materialization`。
- `src/provider/manifest.rs:27-36` 当前 recovery eligible providers 是 `codex`、`claude`、`antigravity`。

事实结论：ah 已有 per-provider home/config materialization 入口；Claude/Gemini 已有 hook 注入实现；Codex 当前 materialization 偏 config/plugins，没有 hook 注入证据；Antigravity 当前只 copy settings/onboarding/trust/rules，没有注入 hooks 的现有实现证据。

## §D cmux push 机制参考要点

授权红线：

- `docs/competitive/cmux-vibeyard-borrowable-for-ah.md:20-29` 明确 cmux 是 GPL-3.0，允许借机制/范式，严禁拷源码；落地方式必须是理解机制后自行实现。
- `docs/competitive/cmux-vibeyard-borrowable-for-ah.md:120` 再次列出“不适用”：直接拷 cmux 源码会有 GPL-3.0 传染风险。

机制级事实：

- `docs/competitive/cmux-vibeyard-borrowable-for-ah.md:47-58` 借鉴点 1 是 hook 推送式“完成/求关注”信号，来源是 cmux `docs/agent-hooks.md`、`docs/notifications.md`、CLI `cmux notify`。
- `docs/competitive/cmux-vibeyard-borrowable-for-ah.md:50` cmux 用 `cmux hooks setup` 写入各 provider 配置：Codex `~/.codex/hooks.json`、Gemini `~/.gemini/settings.json`、Claude 包装注入。
- `docs/competitive/cmux-vibeyard-borrowable-for-ah.md:51-53` lifecycle 映射：`Stop` -> idle，`PreToolUse` -> running，`PermissionRequest`/`Notification` -> needsInput。
- `docs/competitive/cmux-vibeyard-borrowable-for-ah.md:54` 确定性来自程序 hook 事件处理器，不靠模型主动判断。
- `docs/competitive/cmux-vibeyard-borrowable-for-ah.md:55-56` 文档对比 ah 当前 pull log completion 与 cmux push hook completion。
- `docs/competitive/cmux-vibeyard-borrowable-for-ah.md:59-62` 文档称三个 provider 覆盖，其中 Antigravity 通过 `agy` binary hook engine 符号和 `.gemini/antigravity-cli/settings.json` opt-in。
- `docs/competitive/cmux-vibeyard-borrowable-for-ah.md:91-98` 借鉴点 4 是结构化“求拍板”信号：hook 把审批请求推成带 workspace/surface/title/body 的结构化信号。
- `docs/competitive/cmux-vibeyard-borrowable-for-ah.md:133-136` 附录给出 cmux 复查入口：`docs/agent-hooks.md`、`docs/notifications.md`、`docs/feed.md`、LICENSE。

本 research 未 clone 或读取 cmux 源码；以上只来自 repo 内已有调研文档。

## §E 关键事实、约束、开放问题

关键事实：

- ah 当前完成检测是 pull：monitor tick tail provider session log，再走 `mark_agent_idle_log_event` 的 CAS state transition。
- 当前 pull parser 只对 Codex/Claude 有完成信号解析；Antigravity 在 `parse_provider_log_line` 中没有 terminal parser 分支。
- state machine 已有多信号竞争仲裁事实：`state_version` CAS 只允许一个 transition 赢。
- ahd 已有 UDS JSON-RPC 入站框架，且 `master.ack_ready` 是已验证的 push-ingest 样板；当前缺的是 hook completion 专用 ingest contract，不是缺少“进程外 CLI -> RPC -> handler -> DB -> saga 反应”的通道范式。
- ah 已有 provider home materialization 体系；Claude/Gemini hooks 已可通过现有 config schema 注入；Codex/Antigravity hook 注入没有现有实现证据。
- 本机 Antigravity binary 有完整 hook 相关 strings；本机 Codex CLI 0.135.0 未通过 strings/config 验证出 hook schema。
- 动机现场：本 session 的 `ccb ask --wait` job `job_c3176678736d` 长时间保持 `status=running`，但 a1 pane 已显示 `Worked for 7m01s` 并回到 idle 占位符，且 `research.md` 已落盘；这是 pull completion-lag 的活体复现，正是 push hook completion 要消除的痛点。

约束：

- cmux 只能作为机制参考，不能复制源码。
- 后续跨 provider 抽象不能只依赖 Claude shape；Claude/Gemini/Antigravity hook 配置 shape 至少已知不完全同构。
- Codex hook 能力需要额外一手验证；仅凭 cmux 文档不足以锁定 ah 侧合同。
- Hook push 与现有 pull 若并存，必须继续尊重 state machine 的 CAS 单赢家事实。
- Hook payload 到 agent/job/session 的关联字段仍需实证确定；但关联不必全靠 provider stdin/env：`master.ack_ready` 已用 `--cutover-id` 参数把 cutover 身份注入 CLI，当前 ccb/codex-dual Claude `Stop` hook 也用 `--agent-name a3` 注入 agent 身份。真正待补的 ah 工程事实是：`rg -n -e 'agent.?id|--agent' src/provider/home_layout.rs` 当前无输出，说明 ah 的 provider hook materialization 还没有把 agent 身份自动焊进 hook 命令行。

开放问题：

- Codex CLI 0.135.0 的 `~/.codex/hooks.json` schema、事件名、payload 字段是什么，当前本机 strings/config 未证实。
- Claude Code hook stdin/env 中可稳定取得哪些字段：session id、cwd、transcript path、event name 是否可作为 ah job 关联依据，当前 repo 内没有一手行号证据；已验证可行的出路是 ah materialize 时通过 hook command 参数注入 agent/job/session 标识，类比 `master.ack_ready --cutover-id` 与 ccb finish-hook `--agent-name`。
- Antigravity `.gemini/antigravity-cli/settings.json` 的 hooks JSON shape、事件命名、command payload 需用官方或实际配置样本验证；当前只确认 binary 内有 hook engine 符号。
- ahd 现有 JSON-RPC 已有 `master.ack_ready` 这种 push-ingest 样板；hook completion 是扩展新 ingest method 还是复用/泛化既有 method，属于后续设计问题，本 research 不作方案判断。
- Push 完成信号与 pull monitor 的竞态、重复、幂等边界需要后续设计基于现有 CAS 事实展开。

## 本次读取文件

- `/tmp/ah-hook-push-research-brief.md`
- `/tmp/ah-hook-push-research-fix.md`
- `docs/competitive/cmux-vibeyard-borrowable-for-ah.md`
- `src/completion/parser.rs`
- `src/completion/reader.rs`
- `src/completion/monitor.rs`
- `src/completion/registry.rs`
- `src/db/state_machine.rs`
- `src/db/master_cutovers.rs`
- `src/orchestrator/mod.rs`
- `src/bin/ah.rs`
- `src/rpc/mod.rs`
- `src/rpc/router.rs`
- `src/rpc/handlers/events.rs`
- `src/provider/home_layout.rs`
- `src/provider/extensions.rs`
- `src/provider/manifest.rs`
- `src/rpc/handlers/agent.rs`
- `src/rpc/handlers/sessions.rs`
- `src/sandbox/systemd.rs`
- `src/cli/config.rs`
- `tests/pr4c_hooks_plugins.rs`
- `tests/pr1b_readfirst_hook.rs`
- `$HOME/.gemini/settings.json`
- `$HOME/.claude/settings.json`
- `$HOME/.codex/config.toml`

## 本次 grep/strings 命令

- `rg -n "task_complete|stop_reason|end_turn|LogReadState|run_log_monitor_tick|spawn_log_monitor_task|LOG_EVENT_TASK_COMPLETE|LogEvent|state_version|compare|UPDATE agents|event_subscribe|UDS RPC|ahd.sock|UnixListener|handle_client|dispatch\\(" src/completion src/db src/orchestrator src/rpc src/bin/ahd.rs`
- `rg -n "CODEX_HOME|CLAUDE_CONFIG_DIR|GEMINI_CLI_HOME|ANTIGRAVITY|antigravity|codex|claude|prepare_home_layout|materialize|onboarding|settings.json|config.toml|AGENTS.md|builtin rules|HomeLayoutRole|Provider" src/provider src/sandbox src/rpc src/cli assets tests`
- `rg -n "struct Hook|HookConfig|hooks|HookGroup|matcher|timeout|source|command" src/provider src/config src/cli tests`
- `rg -n "master\\.ack_ready|ack_ready|ack-ready|ack ready|AckReady" src/rpc/router.rs src/rpc/handlers/sessions.rs src/cli src`
- `rg -n "ack-ready|master ack|master\\.ack_ready|cutover-id|cutover_id" src/cli src/rpc tests`
- `rg -n -e 'agent.?id|--agent' src/provider/home_layout.rs`
- `command -v codex || true; command -v claude || true; command -v agy || true; command -v antigravity || true; codex --version 2>/dev/null || true; claude --version 2>/dev/null || true; agy --version 2>/dev/null || true`
- `for f in "$HOME/.gemini/antigravity-cli/settings.json" "$HOME/.gemini/settings.json" "$HOME/.claude/settings.json" "$HOME/.codex/hooks.json" "$HOME/.codex/config.toml"; do ... nl -ba "$f" ...; done`
- `strings "$(readlink -f "$(command -v codex)")" 2>/dev/null | rg -n -i 'hook|hooks|Stop|PreToolUse|PostToolUse|Notification|PermissionRequest|SessionStart|PreCompact'`
- `strings "$(readlink -f "$(command -v claude)")" 2>/dev/null | grep -aE 'hook_event_name|PreToolUse|PostToolUse|PermissionRequest|Notification|SessionStart|PreCompact|Stop|transcript_path|session_id|cwd|hookSpecificOutput|permissionDecision'`
- `strings "$(command -v agy)" 2>/dev/null | grep -aE 'jsonhook\\.JSONHookSpec|PreInvocationHook|PostInvocationHook|StopHook|PreToolHookArgs|PostToolHookArgs|HookSystemMessage|EXECUTOR_TERMINATION_REASON_TERMINAL_CUSTOM_HOOK|PreTool|PostTool|InvocationHook|CustomHook|jsonhook'`
