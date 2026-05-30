# Tasks: ah 全流程 E2E Grand Tour PR-2 (DRIFT + NEW)

## Scope Guard

PR-2 只落地 DRIFT + NEW 分支矩阵，新增单文件 `tests/ah_full_e2e_drift.rs` 承载 5 case。

- DRIFT 覆盖：ENV Drift、HOOKS Drift、PLUGINS Drift、NO_CHANGE。
- NEW 覆盖：运行中新增 agent block，触发 `session.realign` 的 `NEW` 分支。
- 不覆盖 ORPHAN / BUSY / ERROR；这三类留 PR-3。
- 复用 PR-1 Harness 形态：temp DB、temp state dir、temp project dir、隔离 tmux server、`dispatch` RPC helper、DB query helper。
- 新增 Harness helper：`assert_sandbox_file`、`assert_symlink_target`、`query_agent_events`；可选 `assert_json_contains` 用于 `.claude/settings.json` 和插件启用状态。
- 物理断言参考：`tests/pr4c_hooks_plugins.rs` 的 `std::fs::read_link` pattern；`src/provider/home_layout.rs` 中 `.claude/CLAUDE.md`、`.claude/hooks/*`、`.claude/plugins/cache/*` 物化逻辑。

## 主线矩阵

1. ENV Drift：修改 agent env -> `session.realign` -> `REALIGNED` -> 新 PID / 旧 PID 消失 -> `drift_realigned`。
2. HOOKS Drift：修改 hooks -> `session.realign` payload 含 hooks -> `REALIGNED` -> sandbox `.claude/hooks/<script>` symlink 指向新脚本。
3. PLUGINS Drift：修改 plugins -> `session.realign` payload 含 plugins -> `REALIGNED` -> sandbox `.claude/plugins/cache/<plugin>` 或 `.claude/plugins/<plugin>` symlink 更新。
4. NO_CHANGE：不改配置再 realign -> `NO_CHANGE` -> PID 不变 -> `drift_realigned` / `agent_spawned` 事件计数不增长。
5. NEW Agent：追加 `a2` block -> `session.realign` -> `NEW` -> `agent_a2` 到 `IDLE` -> `agent_spawned` payload reason=`NEW` -> `a1` 状态与 tmux 不受损。

## T1: Add ignored PR-2 drift matrix red-light skeleton

- 文件: Add `tests/ah_full_e2e_drift.rs`
- 依赖: PR-1 M2 Harness pattern
- 内容:
  - 新增 `mod common;`
  - 新增 `#[tokio::test(flavor = "multi_thread")] #[ignore] async fn grand_tour_drift_new_matrix()`
  - 先只 `panic!("red: PR-2 DRIFT + NEW matrix not implemented")`
  - 文件内预留 step 函数名：`case_01_env_drift`、`case_02_hooks_drift`、`case_03_plugins_drift`、`case_04_no_change`、`case_05_new_agent`
- 验收标准:
  - 命令: `CARGO_BUILD_JOBS=1 cargo test --test ah_full_e2e_drift -- --test-threads=1`
  - 绿灯信号: 默认 lane 显示 ignored，不执行主测试。
  - 命令: `CARGO_BUILD_JOBS=1 cargo test --test ah_full_e2e_drift -- --include-ignored --test-threads=1`
  - 红灯信号: test 名出现，失败信息包含 `PR-2 DRIFT + NEW matrix not implemented`。
- audit 视角:
  - a2 检查 `#[ignore]` 存在，PR-2 不污染默认 cargo test。
  - a3 检查文件只建红灯骨架，不提前塞半成品断言。

## T2: Port PR-1 Harness and add physical assertion helpers

- 文件: Modify `tests/ah_full_e2e_drift.rs`
- 依赖: T1
- 内容:
  - 复用 PR-1 `Harness` 结构：`ctx`、`TmuxServerGuard`、temp DB、temp state dir、temp project dir。
  - 复用 `rpc(method, params)`：通过 `ccbd::rpc::router::dispatch` 发送 JSON-RPC，遇到 `error` 直接 panic。
  - 复用 DB helpers：`query_agent_state`、`query_agent_pid`、`query_agent_config_hash`、`query_session_config_hash`、`query_events_by_type`。
  - 新增 `query_agent_events(agent_id, event_type)`：按 `agent_id` + `events.event_type` 返回 payload JSON 数组。
  - 新增 `assert_sandbox_file(session_id, agent_id, sub_path)`：断 `state_dir/sandboxes/<session>/<agent>/<sub_path>` exists。
  - 新增 `assert_symlink_target(session_id, agent_id, sub_path, expected_target)`：用 `std::fs::read_link` 断 symlink 指向。
  - 可选新增 `assert_json_contains(path, pointer_or_key, expected)`：用于 `.claude/settings.json` 中 hooks/plugins 状态。
- 验收标准:
  - 命令: `CARGO_BUILD_JOBS=1 cargo test --test ah_full_e2e_drift -- --include-ignored --test-threads=1`
  - 红灯信号: Harness 编译通过，仍停在未实现 case。
  - 绿灯目标: 可以完成 start baseline，断 `agent_a1` session 存在、sandbox dir exists、`agents.state=IDLE`。
- audit 视角:
  - a2 检查 helper 查询字段使用真实 schema：`events.event_type` / `events.payload`，不是 `events.kind` / `payload_json`。
  - a3 检查 `assert_symlink_target` 用 `read_link` 物理断言，不只 assert path exists。

## T3: Add drift/new fixture builders and realign payload helper

- 文件: Modify `tests/ah_full_e2e_drift.rs`
- 依赖: T2
- 内容:
  - 新增 `build_drift_ah_toml(project_dir, DriftSpec)`：生成含 `a1` 的 `ah.toml`，可配置 env/hooks/plugins。
  - 新增 `build_new_agent_ah_toml(project_dir, DriftSpec, NewAgentSpec)`：在已有 `a1` 基础上追加 `a2` block。
  - 新增 host fixture helper：
    - `write_hook_script(name, body) -> PathBuf`
    - `write_claude_plugin_cache(name) -> PathBuf`
  - 新增 `realign_payload(session_id, master_spec, agents)`：显式构造 `session.realign` payload，确保 hooks/plugins/env 字段真实传给 RPC。
  - 新增 `run_realign(h, payload, force=false)`：调用 `session.realign` 并返回 `statuses`。
- 验收标准:
  - 命令: `CARGO_BUILD_JOBS=1 cargo test --test ah_full_e2e_drift -- --include-ignored --test-threads=1`
  - 红灯信号: fixture helper 编译通过，但 case 断言仍未全部实现。
  - 绿灯目标: baseline config 可以 spawn `a1`，realign payload 中可 grep 到 `env`、`hooks`、`plugins` 字段。
- audit 视角:
  - a2 检查 hooks/plugins 不只是写进 `ah.toml`，也进入 `session.realign` RPC payload。
  - a3 检查插件 cache fixture 路径匹配 provider home layout 的 `.claude/plugins/cache/<name>` 解析规则。

## T4: Case 1 ENV Drift

- 文件: Modify `tests/ah_full_e2e_drift.rs`
- 依赖: T3
- 内容:
  - 基线启动 `a1`，记录 `old_pid`、`old_agent_config_hash`、`drift_realigned` 事件计数。
  - 修改 `a1.env`，例如 `GRAND_TOUR_DRIFT_ENV = "v2"`。
  - 调 `session.realign`，payload 中 `agents[0].env` 必须包含新 key/value。
  - 断 RPC result 中 `a1.status=REALIGNED`，`event=drift_realigned`。
  - 断言取 `statuses[]` 数组中 `agent_id == "a1"` 那条记录的 `status` 字段 (per-agent 层级, handlers.rs:461 NEW / :470 NO_CHANGE / :516 REALIGNED), 不取 session 顶层聚合 `status` (handlers.rs:401)。
  - 等 `a1` 回到 `IDLE`，记录 `new_pid`；断 `new_pid != old_pid`，旧 pid 不再存活。
  - 断 `agents.config_hash` 变化，`query_agent_events("a1", "drift_realigned")` 计数增加。
- 验收标准:
  - 命令: `CARGO_BUILD_JOBS=1 cargo test --test ah_full_e2e_drift -- --include-ignored --test-threads=1`
  - 红灯信号: 未实现前缺 `REALIGNED`、PID 未变、或无 `drift_realigned`。
  - 绿灯信号: ENV Drift case 独立通过，后续 case 可继续使用同一 session。
- audit 视角:
  - a2 检查 ENV drift 不是只改文件，而是真传 RPC env 并导致 hash 变化。
  - a3 检查 PID 断言有 wait/retry，避免旧进程退出 race。

## T5: Case 2 HOOKS Drift

- 文件: Modify `tests/ah_full_e2e_drift.rs`
- 依赖: T4
- 内容:
  - 写 host hook 脚本，例如 `hooks/pr2-audit-v2.sh`，内容可为 `#!/bin/sh\nexit 0\n`。
  - 修改 `a1.hooks`，例如 `PreToolUse = [{ matcher="*", hooks=[{ type="command", command="<script>" }] }]`。
  - 调 `session.realign`，payload 中 `agents[0].hooks` 必须非空。
  - 断 RPC result `REALIGNED` + reason 含 `hooks changed` 或 event 为 `drift_realigned`。
  - 断言取 `statuses[]` 数组中 `agent_id == "a1"` 那条记录的 `status` 字段 (per-agent 层级, handlers.rs:461 NEW / :470 NO_CHANGE / :516 REALIGNED), 不取 session 顶层聚合 `status` (handlers.rs:401)。
  - a1 落笔时 grep `fn drift_reason` 实证 reason 文本格式: 若不含 `hooks`/`plugins` 字样, 改为弱断言 `result["statuses"][a1]["message"] 含 "REALIGNED"` + 强断 sandbox symlink 指向; reason 文本断言降为 optional。
  - 用 `assert_sandbox_file(session_id, "a1", ".claude/settings.json")` 断 settings 存在。
  - 用 `assert_symlink_target(session_id, "a1", ".claude/hooks/pr2-audit-v2.sh", hook_path)` 断 sandbox hook symlink 指向新脚本。
  - 可选用 `assert_json_contains` 断 `.claude/settings.json` 的 `hooks.PreToolUse` command 指向 sandbox `.claude/hooks/pr2-audit-v2.sh`。
- 验收标准:
  - 命令: `CARGO_BUILD_JOBS=1 cargo test --test ah_full_e2e_drift -- --include-ignored --test-threads=1`
  - 红灯信号: RPC payload 无 hooks、sandbox `.claude/hooks/*` 不存在、或 read_link 目标不匹配。
  - 绿灯信号: HOOKS Drift 通过真实 symlink 物理断言。
- audit 视角:
  - a2 检查参考 `tests/pr4c_hooks_plugins.rs` 的 `std::fs::read_link` 风格。
  - a3 检查不是只断 `settings.json` 文本，必须断 symlink target。

## T6: Case 3 PLUGINS Drift

- 文件: Modify `tests/ah_full_e2e_drift.rs`
- 依赖: T5
- 内容:
  - 在 temp HOME 下创建 Claude plugin cache，例如 `.claude/plugins/cache/pr2-claude-audit/plugin.json`。
  - 修改 `a1.plugins = ["pr2-claude-audit"]`，可同时保留 ENV 以覆盖混合漂移。
  - 调 `session.realign`，payload 中 `agents[0].plugins` 必须含 `pr2-claude-audit`。
  - 断 RPC result `REALIGNED` + reason 含 `plugins changed` 或 event 为 `drift_realigned`。
  - 断言取 `statuses[]` 数组中 `agent_id == "a1"` 那条记录的 `status` 字段 (per-agent 层级, handlers.rs:461 NEW / :470 NO_CHANGE / :516 REALIGNED), 不取 session 顶层聚合 `status` (handlers.rs:401)。
  - a1 落笔时 grep `fn drift_reason` 实证 reason 文本格式: 若不含 `hooks`/`plugins` 字样, 改为弱断言 `result["statuses"][a1]["message"] 含 "REALIGNED"` + 强断 sandbox symlink 指向; reason 文本断言降为 optional。
  - a1 落笔时 grep `fn materialize_claude_plugins` + `.claude/plugins` 全部出现位置 (`src/provider/home_layout.rs`), 确认 `.claude/plugins/<name>` (不带 cache) 是否真物化: 物化 → 保留断言; 未物化 → 删该断言, 仅保留 `.claude/plugins/cache/<name>`。
  - 用 `assert_symlink_target(session_id, "a1", ".claude/plugins/cache/pr2-claude-audit", host_plugin_cache)` 断 cache symlink。
  - 用 `assert_symlink_target(session_id, "a1", ".claude/plugins/pr2-claude-audit", host_plugin_cache)` 断 enabled plugin symlink。
  - 可选用 `assert_json_contains` 断 `.claude/settings.json` 中 `enabledPlugins.pr2-claude-audit=true`。
- 验收标准:
  - 命令: `CARGO_BUILD_JOBS=1 cargo test --test ah_full_e2e_drift -- --include-ignored --test-threads=1`
  - 红灯信号: RPC payload 无 plugins、plugin cache 未物化、或 symlink target 不匹配。
  - 绿灯信号: PLUGINS Drift 通过 sandbox 插件目录物理断言。
- audit 视角:
  - a2 检查插件 fixture 不访问网络、不拉真实 repo。
  - a3 检查 plugin path 使用 `.claude/plugins/cache/<name>` 与 `.claude/plugins/<name>` 两处断言。

## T7: Case 4 NO_CHANGE Idempotency

- 文件: Modify `tests/ah_full_e2e_drift.rs`
- 依赖: T6
- 内容:
  - 使用 T6 结束后的相同 master/agent specs 再调用一次 `session.realign`。
  - 记录调用前 `a1.pid`、`agents.config_hash`、`drift_realigned` 事件计数、`agent_spawned` 事件计数。
  - 断 RPC result 中 `a1.status=NO_CHANGE`，master 如参与则允许 `NO_CHANGE` 或 `SKIPPED`。
  - 断言取 `statuses[]` 数组中 `agent_id == "a1"` 那条记录的 `status` 字段 (per-agent 层级, handlers.rs:461 NEW / :470 NO_CHANGE / :516 REALIGNED), 不取 session 顶层聚合 `status` (handlers.rs:401)。
  - 断 `pid` 不变，`agents.config_hash` 不变。
  - 断 `drift_realigned` 与 `agent_spawned` 事件计数不增长。
- 验收标准:
  - 命令: `CARGO_BUILD_JOBS=1 cargo test --test ah_full_e2e_drift -- --include-ignored --test-threads=1`
  - 红灯信号: 相同配置仍触发 kill/spawn 或 `REALIGNED`。
  - 绿灯信号: NO_CHANGE case 证明 realign 幂等，无 PID/事件增长。
- audit 视角:
  - a2 检查幂等断言包含 RPC status + DB hash + event count 三层。
  - a3 检查不是仅靠 stdout 文本判断 no-op。

## T8: Case 5 NEW Agent

- 文件: Modify `tests/ah_full_e2e_drift.rs`
- 依赖: T7
- 内容:
  - 在已有 `a1` session 上追加 `a2` agent block，provider 使用同一 mock provider。
  - 调 `session.realign`，payload agents 必须同时包含 `a1` 与 `a2`，避免把 `a1` 误判 ORPHAN。
  - 断 RPC result 中 `a2.status=NEW`、`action=spawned`。
  - 等 `a2` 到 `IDLE`，断 `agent_a2` tmux session 存在，且与 `agent_a1` 不同 session/pane。
  - 断 `query_agent_events("a2", "agent_spawned")` 至少一条 payload reason=`NEW`。
  - 断 `a1` 状态仍为 `IDLE` 或测试前状态，`a1.pid` 未因新增 `a2` 改变。
  - 断 `a2` sandbox dir exists，基础 `.claude/CLAUDE.md` exists。
- 验收标准:
  - 命令: `CARGO_BUILD_JOBS=1 cargo test --test ah_full_e2e_drift -- --include-ignored --test-threads=1`
  - 红灯信号: `a2` 未 spawn、卡在 `SPAWNING`、无 `agent_spawned reason=NEW`、或 `a1` 被误杀/重启。
  - 绿灯信号: NEW Agent case 通过，并且无 ORPHAN/BUSY/ERROR 分支行为。
- audit 视角:
  - a2 检查 NEW payload 包含完整 agents list，避免遗漏 `a1` 触发 ORPHAN。
  - a3 检查不依赖瞬时 `SPAWNING`，只等待最终 `IDLE` + event reason。

## T9: Local run notes and final grep guard

- 文件: Modify `tests/ah_full_e2e_drift.rs` comments only if needed; no docs outside this task file unless PR review later要求
- 依赖: T8
- 内容:
  - 在测试文件顶部或 task follow-up notes 中保留本地运行命令。
  - 明确默认 lane 不运行 ignored Grand Tour。
  - 明确 PR-2 scope 不含 ORPHAN / BUSY / ERROR。
- 验收标准:
  - Rust PR-2 ignored lane: `CARGO_BUILD_JOBS=1 cargo test --test ah_full_e2e_drift -- --include-ignored --test-threads=1`
  - Rust PR-2 default lane: `CARGO_BUILD_JOBS=1 cargo test --test ah_full_e2e_drift -- --test-threads=1`
  - Grep guard:
    - `rg -n "ENV Drift|HOOKS Drift|PLUGINS Drift|NO_CHANGE|NEW Agent" tests/ah_full_e2e_drift.rs`
    - `rg -n "assert_sandbox_file|assert_symlink_target|query_agent_events" tests/ah_full_e2e_drift.rs`
    - `rg -n "ORPHAN|SKIPPED_BUSY|FORCE_REALIGN|ERROR" tests/ah_full_e2e_drift.rs` 应只出现在 scope 注释或不出现，不应有实现路径。
  - 绿灯信号: 5 case 全部通过；默认 lane skipped；无新 failed。
- audit 视角:
  - a2 检查 final grep 覆盖 5 case + helper + scope guard。
  - a3 检查 PR-2 没有偷偷实现 BUSY/ORPHAN/ERROR，CI lane 仍保持 ignored。

## PR-2 Final Verification

- `CARGO_BUILD_JOBS=1 cargo test --test ah_full_e2e_drift -- --include-ignored --test-threads=1`
- `CARGO_BUILD_JOBS=1 cargo test --test ah_full_e2e_drift -- --test-threads=1`
- `rg -n "ENV Drift|HOOKS Drift|PLUGINS Drift|NO_CHANGE|NEW Agent" tests/ah_full_e2e_drift.rs`
- `rg -n "assert_sandbox_file|assert_symlink_target|query_agent_events|assert_json_contains" tests/ah_full_e2e_drift.rs`
- `rg -n "ORPHAN|BUSY|ERROR" tests/ah_full_e2e_drift.rs`
