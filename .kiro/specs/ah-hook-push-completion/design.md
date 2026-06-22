# ah hook 推送式完成信号设计

GPL 红线：本设计只借鉴 cmux 的 hook push 机制范式；cmux 为 GPL-3.0，ah 必须原生重写，零源码拷贝。授权风险依据见 `docs/competitive/cmux-vibeyard-borrowable-for-ah.md:20-29`、`docs/competitive/cmux-vibeyard-borrowable-for-ah.md:120`。

## 1. 目标与原则

目标：为 ah 增加 provider lifecycle hook -> ahd push ingest -> agent/job state transition 的完成信号通道，消除 pull completion-lag，同时保留 completion v2 log monitor 作为兜底。

设计原则：

- Push-primary, pull-fallback。Push 提供更低延迟，但受 hook 进程启动和 UDS 往返约束，不能写成“近乎零”；pull monitor 继续覆盖 hook 配置失效、hook 进程崩溃、RPC 丢失、provider 不触发 hook 等失败面。
- 三厂商不降级。Codex、Claude、Antigravity 都必须有端到端 dogfood 证明，不能只做到 Claude ready。
- 身份由 ah 注入，不依赖 provider stdin/env。`agent_id`、`socket_path` 等 ah 关联字段在 materialize hook command 时焊进命令行或 env，类比 `master.ack_ready --cutover-id` 与现役 `ccb-provider-finish-hook --agent-name`。
- 状态仲裁复用 `state_version` CAS。Push 和 pull 谁先完成 transition，谁赢；重复信号被吞掉。

现状证据：

- 当前 pull 数据流从 monitor tick 读 provider log，再调用 `mark_agent_idle_log_event`，见 `.kiro/specs/ah-hook-push-completion/research.md:5-32`。
- 本 session 已复现 completion-lag：job `job_c3176678736d` 仍 running，但 pane 已 idle 且 research.md 已落盘，见 `.kiro/specs/ah-hook-push-completion/research.md:161`。
- `master.ack_ready` 已证明外部 CLI -> JSON-RPC -> handler -> DB -> saga loop 的 push-ingest 模式，见 `.kiro/specs/ah-hook-push-completion/research.md:60-77`。

## 2. 契约变更表

| 项 | 标记 | 变更 |
|---|---|---|
| CLI | [NEW] | 新增 `ah agent notify --agent-id <id> --event <stop|pre_tool|permission_request> [--provider <p>] [--event-id <id>] [--socket <path>]`。 |
| RPC | [NEW] | 新增 JSON-RPC method `agent.notify`，按 `master.ack_ready` 样板接入 router whitelist 和 dispatch。 |
| DB/state machine | [NEW] | 新增 push 专用 transition，例如 `mark_agent_idle_hook_event` 或参数化 `mark_agent_idle_signal_event`；不得伪装成 `LogEvent`。 |
| Provider materialization API | [BREAKING internal] | `prepare_home_layout_with_extensions` 当前无 `agent_id`，需扩内部签名或新增 context 参数，把 `agent_id`、`ahd_socket_path`、feature flag 传入 materialization。调用点同步更新。 |
| Provider home injection | [NEW] | Claude/Gemini 复用现有 hook 注入；Codex 扩 `prepare_managed_codex_home` 写 hooks/config；Antigravity 扩 settings hook 注入。 |
| Env/socket propagation | [NEW] | Worker spawn 必须确定性注入 `CCB_SOCKET=<ahd.sock>`，hook command 同时可携带 `--socket <path>`。 |
| Config flag | [NEW] | 增加灰度开关：默认可先关闭 push，开启后仍保留 pull fallback；一键禁用回纯 pull v2。 |

## 3. 核心机制：Hub-and-Spoke

Spoke：provider hook command。

每个 worker materialize provider home 时注入一个 ah-owned command hook：

```text
env CCB_SOCKET=<absolute-ahd-socket> ah agent notify \
  --agent-id <agent-id> \
  --event stop \
  --provider <provider> \
  --socket <absolute-ahd-socket>
```

`--agent-id` 必须在 ah materialize 阶段焊入，不依赖 provider 的 stdin/env。当前 `prepare_home_layout_with_extensions` 签名只有 `provider/sandbox_dir/workspace_path/role/extensions`，无 `agent_id`，见 `src/provider/home_layout.rs:107-113`；worker spawn 外层已有 `agent_id`，见 `src/rpc/handlers/agent.rs:96-117`。因此落地必须扩签名：

```rust
struct HookPushContext {
    agent_id: String,
    provider: String,
    ahd_socket_path: PathBuf,
    enabled: bool,
}
```

推荐新增 `HomeMaterializationContext` 或给 `prepare_home_layout_with_extensions` 增加 `Option<&HookPushContext>`，避免继续扩散裸参数。影响面是内部调用点；对外 RPC/API 不破坏。

Hub：ahd ingest。

- router：按 `master.ack_ready` 样板把 `agent.notify` 加入 method whitelist 和 dispatch。现有样板在 `src/rpc/router.rs:15-20` 和 `src/rpc/router.rs:79-84`。
- handler：新增 `handle_agent_notify(params, ctx)`。它校验 `agent_id`、`event`、可选 `provider`/`event_id`/`provider_payload_digest`，并调用 push 专用 state transition。
- DB：新增 push 专用 transition，不复用 log transition 的审计字段名。现有 log transition 写 `sub_state='LogEvent'`、`reason=LOG_EVENT_TASK_COMPLETE`、`raw_path/raw_offset/provider_turn_id`，见 `src/db/state_machine.rs:475-520` 和 `.kiro/specs/ah-hook-push-completion/research.md:51-58`。Push transition 应写 `sub_state='HookEvent'` 或 `sub_state='PushEvent'`，payload 包含 `source='hook'`、`hook_event`、`provider`、`event_id`、`received_at`、`schema_version`、`reply_source`。
- wakeup：transition 成功后通知 job update 并 `orchestrator::wake_up()`，沿用 pull monitor 行为，见 `src/completion/monitor.rs:48-58`。

## 4. 事件到状态映射

统一事件契约：

| ah event | 目标状态 | 首期处理 | 说明 |
|---|---|---|---|
| `stop` | `IDLE` | 必做 | 完成信号。只在 agent 当前 `WAITING_FOR_ACK` 或 `BUSY` 时生效。 |
| `pre_tool` | `BUSY`/可观测事件 | 可选后续 | 用于工具调用可观测性，不进入首期完成 gate。 |
| `permission_request` | `PROMPT_PENDING`/结构化 escalation | 后续 | 与 cmux needsInput 类似；避免首期扩大 scope。 |

Provider 映射：

| provider | provider event | ah event | 就绪度 |
|---|---|---|---|
| Claude | `Stop` | `stop` | Ready。现有 `.claude/settings.json` hook 注入 shape 已实现，见 `src/provider/home_layout.rs:583-612`；测试覆盖 `PreToolUse` materialization，见 `tests/pr4c_hooks_plugins.rs:17-48`。 |
| Gemini | `AfterAgent` | `stop` | Ready for existing Gemini path。Gemini settings hook shape 已实现，见 `src/provider/home_layout.rs:614-640`；测试覆盖 `BeforeAgent` materialization，见 `tests/pr4c_hooks_plugins.rs:176-208`。 |
| Codex | `Stop` | `stop` | Implementable with official mirror docs。Codex hook events/location/shape 在 `docs/agent-cli-knowledge-base/codex/hooks-official.md:30-50`、`:56-88`、`:120-145`、`:517-529`；当前 `prepare_managed_codex_home` 只写 config/trust/plugins，未写 hooks，见 `src/provider/home_layout.rs:669-705`。 |
| Antigravity | `Stop` (JSON named-hook) | `stop` | Pre-verify ✅ (`antigravity-hooks-preverify.md`)。官方 schema + `agy` binary strings 锁定: named-hook key `ah-completion-push`, 事件 key `Stop`, 路径 `<managed_home>/.gemini/config/hooks.json`。实现已落地 (`src/provider/home_layout.rs`)。**路径本体待 step-9 dogfood 实证** (真 agy session 确认加载) 才可标 ready。 |

首期验收门：三厂商 push 不降级。

每家 provider 必须有一条真 dogfood 或 integration proof：

- 启动 ah-managed worker。
- 由 ah materialize hook，hook command 中包含正确 `--agent-id` 和 `CCB_SOCKET`/`--socket`。
- 触发 provider 完成事件。
- ahd 收到 `agent.notify`。
- agent 从 `WAITING_FOR_ACK` 或 `BUSY` 转为 `IDLE`，job completed/cancelled 语义与 pull v2 一致。
- `state_change` payload 标识 hook source。
- pull monitor 残留不会造成第二次 transition。

Codex、Claude、Antigravity 全部通过前，不能宣布“三厂商 push completion ready”。可分阶段 merge 到 feature flag 后面，但默认不替代 pull。

## 5. Provider 注入设计

### 5.1 通用 hook command 生成

新增内部 helper：

```text
build_ah_hook_command(ctx, event) -> HookItem
```

输出 command 必须包含：

- `CCB_SOCKET=<absolute>` env 前缀，确保 CLI 可达 ahd。
- `ah agent notify --socket <absolute>` 双保险。CLI 当前优先读 `CCB_SOCKET`，否则按 cwd 推导 state dir，见 `src/cli/rpc_client.rs:102-113`；hook 在 sandbox/provider cwd 内运行，不能依赖 cwd 推导。
- `--agent-id <id>`，身份由 ah 焊入。
- `--event stop`，provider event 到 ah event 的归一化结果。
- `--provider <provider>`，用于审计和防错。
- `--event-id` 可选。若 provider payload 没有稳定 id，可由 hook command wrapper 生成 `provider:event:timestamp:pid`，但幂等仍以 state_version CAS 为主。
- timeout：使用 `HookItem.timeout`，schema 已支持，见 `src/provider/extensions.rs:49-55`。默认建议 5s；Codex 官方默认 600s 太长，见 `docs/agent-cli-knowledge-base/codex/hooks-official.md:140-143`。

### 5.2 Claude/Gemini

Claude/Gemini 复用现有 `ExtensionConfig.hooks` -> `materialize_hooks` -> settings 注入管线：

- `ExtensionConfig.hooks` schema 见 `src/provider/extensions.rs:4-16`。
- `materialize_hooks` 目前要求 command 是 host script path，然后 symlink 到 provider home 并 rewrite command，见 `src/provider/home_layout.rs:469-502`。

设计选择：不要把 ah notify 伪装成用户配置的 external script。新增 internal hook path：

- 用户 hooks 继续走 `materialize_hooks`。
- ah push hook 由 provider-specific injector 直接追加 command string，不要求源文件存在。
- 或创建 ah-owned wrapper script 到 provider home，例如 `.claude/hooks/ah-agent-notify.sh` / `.gemini/hooks/ah-agent-notify.sh`，再按现有 `MaterializedHook` 注入。wrapper 内容由 ah 生成，包含固定 socket/agent_id/event。

推荐 wrapper script：可统一超时、日志、参数 quoting，并降低直接拼接 command 的风险。

### 5.3 Codex

Codex 当前 managed home 创建 `.codex/config.toml`、trust、plugins，但没有 hooks，见 `src/provider/home_layout.rs:669-705`。

设计：

- 扩 `prepare_managed_codex_home(source_home, codex_home, workspace_key, role, plugins, hook_push_ctx)`。
- 确保 `config.toml` 中 `[features] codex_hooks = true`，依据 `docs/agent-cli-knowledge-base/codex/hooks-official.md:20-23`。
- 写 user-layer `.codex/hooks.json` 或 inline `[hooks]`。优先 `.codex/hooks.json`，避免改写现有 `config.toml` 结构过多；Codex 支持 `hooks.json` 和 inline `[hooks]`，见 `docs/agent-cli-knowledge-base/codex/hooks-official.md:33-50`。
- `Stop` hook shape 采用官方 JSON 三层结构，见 `docs/agent-cli-knowledge-base/codex/hooks-official.md:56-88` 和 `:120-135`。`Stop` matcher ignored，见 `docs/agent-cli-knowledge-base/codex/hooks-official.md:517-529`。
- 如果已有 user `.codex/hooks.json`，不得覆盖；需要 merge ah-owned matcher group 或写 inline config with clear ah-owned section。Codex 会加载多个 hook source，见 `docs/agent-cli-knowledge-base/codex/hooks-official.md:47-50`。

Codex 不再视为阻塞设计的“未知 provider”，但仍需要 implementation test 验证当前本机 Codex 版本实际读取该 config。

### 5.4 Antigravity

Antigravity 是目标 provider，不能降级或跳过。

现状：

- `prepare_antigravity_overrides` 当前只创建 settings/onboarding/rules，不接 `extensions`，见 `src/provider/home_layout.rs:217-239`。
- `materialize_antigravity_settings` 只 copy `.gemini/antigravity-cli/settings.json` 并维护 `trustedWorkspaces`，见 `src/provider/home_layout.rs:242-270`。
- Hook engine 证据来自 binary strings，不等于配置 schema 已验证；research 见 `.kiro/specs/ah-hook-push-completion/research.md:87`。

设计：

- 扩 `prepare_antigravity_overrides(..., extensions, hook_push_ctx)`。
- 新增 `materialize_antigravity_hooks` / `inject_antigravity_hooks`，写 `<managed_home>/.gemini/config/hooks.json` 的 named-hook 配置。**[CORRECTED 2026-06-22]** 早前本行写 `.gemini/antigravity-cli/settings.json` 是 pre-verify 前的猜测；pre-verify (`antigravity-hooks-preverify.md`, 2026-06-21) 已用官方 schema + `agy` binary strings (`DefaultHooksPath`/`auto-loaded/hooks.json`) 确认正确位置是 global `~/.gemini/config/hooks.json` → managed home `<managed_home>/.gemini/config/hooks.json`，named-hook 外层 key `ah-completion-push`，事件 key `Stop`。实现 (`src/provider/home_layout.rs`) 已按此落地，design 此行同步修正以消除双真相 (a4 audit F2)。
- 实施前置 verify：✅ 已完成，见 `antigravity-hooks-preverify.md`。schema/事件名/command/timeout 已按官方文档锁定。**路径本体仍标记 dogfood 阶段实证** (step-9 用真 agy session 确认 `<managed_home>/.gemini/config/hooks.json` 被加载)；未完成 step-9 dogfood 前，feature flag 不得让 Antigravity push 标记 ready。
- 如果 verify 发现 Antigravity JSON hook schema 与 Gemini/Claude 都不同，HookProvider 抽象必须容纳 provider-specific renderer，不得把 Gemini shape 强套到 Antigravity。

## 6. Socket 与环境可达性

风险：hook 进程运行在 provider 进程环境中，调用 `ah agent notify` 必须找到正确 ahd socket。

现状：

- RPC socket chmod 0600，同用户可连，见 `src/rpc/mod.rs:28-35`。
- CLI 解析 socket 时优先 `CCB_SOCKET`，否则按 cwd/config 推导，见 `src/cli/rpc_client.rs:102-113`。
- `CCB_SOCKET` 在 provider env passthrough 白名单，见 `src/provider/manifest.rs:235-253`；但 `collect_spawn_env` 只透传宿主进程已有环境变量，见 `src/provider/manifest.rs:450-468`。
- Worker spawn 当前从 `extra_env_vars` 和 home overrides 构造 env，见 `src/rpc/handlers/agent.rs:96-123`；systemd command 最终把 env 前缀到 provider command，见 `src/sandbox/systemd.rs:190-214`。

设计：

- ahd 在 `agent.spawn` 构造 `spawn_env_vars` 时，强制插入 `CCB_SOCKET=ctx.state_dir.join("ahd.sock")` 或 ctx 持有的 daemon socket path；不能依赖启动 ahd 时已有 `CCB_SOCKET`。
- hook command 仍显式传 `--socket <same path>`，避免 `CCB_SOCKET` 被 provider 清理或 hook shell 丢 env。
- 对 systemd/sandbox：如 ahd socket 位于 state_dir 中，确保 provider sandbox/namespace 能访问该 path。若 `systemd-run` scope 没有文件系统隔离，目前路径可见；若未来启用更强 sandbox，需把 socket parent dir bind in。`append_read_only_bind_overrides` 仅处理额外 ro bind，见 `src/sandbox/systemd.rs:150-156`，socket 需要 rw/connect path，不应误用 ro bind。
- `agent.notify` handler 应校验 `agent_id` 存在且 provider 可选匹配 DB 中 provider，防止误发。

## 7. P3F 并存与竞态

Push-primary:

- Hook 到达后直接尝试 push transition。
- 成功后 notify job update + `orchestrator::wake_up()`。
- 主动 `completion::registry::cancel(agent_id)`，减少 pull monitor 最多 250ms 残留。pull monitor 当前非 active 会 cancel，见 `src/completion/monitor.rs:98-104`；主动 cancel 是优化，不是正确性依赖。

Pull-fallback:

- 现有 `spawn_log_monitor_task` 保留；hook 没到、hook timeout、provider 崩溃、RPC 失败时仍靠 log monitor / UI stuck fallback。
- `LOG_MONITOR_POLL_INTERVAL` 为 250ms，见 `src/completion/monitor.rs:9-10`。

CAS 仲裁：

- Push transition 只能接受 `WAITING_FOR_ACK` 或 `BUSY`，与 log transition 一致，见 `src/db/state_machine.rs:475-520`。
- transition 读取当前 `state_version`，`UPDATE ... WHERE state_version=?`，第一个成功者完成 job 和 state_change，后续重复信号 changes=0。
- `event_id`/timestamp 只用于审计/去重日志；正确性以 CAS 为准。

Reply 策略：

- Stop hook payload 可能不包含完整 assistant reply；push transition 应复用 log transition 中 “reply absent -> collect screen reply” 的语义，现有 log transition在 `reply` absent 时走 screen collect，见 `src/db/state_machine.rs:494-505`。
- 若 provider hook payload 有 reply 字段，作为 `reply_source='hook'`；否则 `reply_source='screen'`。

## 8. HookProvider 抽象

这是组织层 nit，但建议在设计中吸收，避免 `home_layout.rs` 继续堆 provider-specific JSON。

可新增：

```rust
trait HookProvider {
    fn provider(&self) -> &'static str;
    fn render_stop_hook(&self, command: &str, timeout_s: u64) -> ProviderHookPatch;
    fn inject(&self, settings_or_home: &mut ProviderHome, patch: ProviderHookPatch) -> Result<()>;
}
```

放置建议：

- `src/provider/extensions.rs` 当前只有数据 schema，见 `src/provider/extensions.rs:4-64`；如果 trait 也放这里，需要避免把 JSON IO 逻辑混进去。
- 更清晰的落点是新增 `src/provider/hooks.rs`，`extensions.rs` 继续保留 user-facing config types。
- `home_layout.rs` 调用 trait renderer，保留文件 IO 和 provider home path 处理。

Provider JSON 差异：

- Claude shape：`hooks[event] = [{ matcher, hooks: [{ type, command, timeout? }] }]`，见 `src/provider/home_layout.rs:583-612`。
- Gemini shape：`hooks[event] = [{ type, command, matcher, timeout? }]`，见 `src/provider/home_layout.rs:614-640`。
- Codex shape接近 Claude 三层结构，但需要 `[features] codex_hooks=true` 或 hooks source，见 `docs/agent-cli-knowledge-base/codex/hooks-official.md:20-23`、`:56-88`。
- Antigravity shape待 verify，不可假定同 Gemini。

## 9. 失败、降级与可观测性

Failure modes:

- Hook command timeout：command 必须自带 timeout；provider config timeout 也要设置。`HookItem.timeout` 已存在，见 `src/provider/extensions.rs:49-55`。
- Hook 进程僵尸：hook wrapper 应使用短 timeout；pull fallback 不能被禁用，除非显式 debug 模式。
- RPC 失败：`ah agent notify` 非 0 不应阻断 provider 正常完成；失败只记录 hook stderr/provider hook log。状态由 pull fallback 补。
- RPC 高并发：ahd 当前每连接 `tokio::spawn`，无显式并发限制，见 `src/rpc/mod.rs:37-73`。首期只把 `stop` 走 push；`pre_tool`/`permission_request` 延后或加 per-agent/event coalescing。若后续启用工具级事件，需要限流、短连接预算和重复事件去重。
- Duplicate hooks：Codex 多 hook source 会全部运行，见 `docs/agent-cli-knowledge-base/codex/hooks-official.md:47-50`。ah-owned hook 需要可识别、可 merge、可移除，避免重复注入。

Observability:

- `agent.notify` response 返回 `{agent_id,event,accepted,transitioned,affected_job_id?,state_version?}`。
- state_change payload 增加 `source='hook'`、`hook_event`、`provider`、`event_id`、`socket_path_kind`、`schema_version`。
- 记录 hook push success/failure counters，至少以 structured tracing 开始。

## 10. 灰度开关与迁移

Config:

```toml
[completion]
hook_push_enabled = false
hook_push_events = ["stop"]
hook_push_providers = ["claude", "codex", "antigravity"]
```

迁移阶段：

1. Default off：写入代码但不注入 hook。
2. Provider opt-in：单 provider dogfood，pull monitor仍运行。
3. All-provider dogfood：Codex、Claude、Antigravity 都有真端到端证明后，才允许项目级默认开启。
4. Stable：仍保留 `hook_push_enabled=false` 一键回纯 pull v2。

回滚：

- 关闭 config 后，materialization 不再注入 ah-owned hook。
- 已存在 provider home 中的 ah-owned hook 需要被移除或标记 disabled；设计实现时必须给 ah-owned hook 固定 id/comment/key，支持清理。
- Pull v2 不移除，因此功能回滚只影响 push 优化。

## 11. Implementation Plan

1. Add `agent notify` CLI and RPC skeleton.
   - Add CLI command under `Cmd::Agent` or equivalent user-facing namespace.
   - Add `agent.notify` to router whitelist/dispatch using `master.ack_ready` pattern.
   - Add params validation and no-op dry test.

2. Add push state transition.
   - Extract shared logic from `mark_agent_idle_log_event` or add `mark_agent_idle_hook_event`.
   - Preserve CAS allowed states `WAITING_FOR_ACK|BUSY`.
   - Use `sub_state='HookEvent'` and hook-specific payload.
   - Notify job update + wake orchestrator on changes > 0.

3. Add materialization context.
   - Extend worker spawn to build `HookPushContext { agent_id, provider, ahd_socket_path, enabled }`.
   - Ensure `CCB_SOCKET` deterministic injection into `spawn_env_vars`.
   - Pass context into provider home materialization.

4. Implement Claude/Gemini hook push injection.
   - Add ah-owned wrapper script or direct command injection.
   - Keep user hooks intact.
   - Add tests for settings JSON and `--agent-id`/socket inclusion.

5. Implement Codex hook push injection.
   - Enable `codex_hooks`.
   - Write or merge `.codex/hooks.json`.
   - Validate Stop hook event locally.

6. Antigravity pre-verify and implementation.
   - Verify settings hooks schema with isolated sample.
   - Only after schema proof, implement `inject_antigravity_hooks`.
   - Add dogfood test.

7. P3F integration.
   - Keep log monitor registered.
   - Push success cancels monitor registry for that agent.
   - Verify duplicate push/pull arrival is idempotent.

## 12. Acceptance Criteria

Must pass before feature can be considered ready:

- `agent.notify` is covered by router/handler tests and rejects missing/unknown agent.
- Push transition test proves:
  - `WAITING_FOR_ACK` -> `IDLE`
  - `BUSY` -> `IDLE`
  - `IDLE` duplicate changes=0
  - stale `state_version` loses
  - job is completed/cancelled consistently with pull v2
  - state_change payload uses hook source, not log raw_path/raw_offset
- Worker spawn injects deterministic `CCB_SOCKET`.
- Materialized hook command contains `--agent-id <id>` and the correct socket.
- Claude dogfood: real Stop hook completes an ah job through push.
- Codex dogfood: real Stop hook completes an ah job through push.
- Antigravity dogfood: real Stop hook completes an ah job through push.
- Pull fallback still completes if hook command exits non-zero or times out.
- Feature flag off produces pure pull v2 behavior and no ah-owned hook injection.

## 13. Open Questions

- Antigravity settings hook schema and Stop event name remain implementation pre-verify blockers.
- Should `agent.notify` accept only `stop` in first release, or parse but ignore `pre_tool`/`permission_request` behind separate flags?
- Should ah-owned hook wrapper live as generated script in provider home or as `ah agent notify` direct command? Wrapper is safer for quoting/timeouts; direct command is simpler.
- Where should `HookProvider` trait live: `extensions.rs` for proximity to hook schema, or a new `provider/hooks.rs` to keep config types separate?
- Should `agent.notify` require a per-agent nonce/token in addition to same-user UDS permissions? Socket chmod 0600 is current security boundary, but a nonce could prevent accidental local misuse.
