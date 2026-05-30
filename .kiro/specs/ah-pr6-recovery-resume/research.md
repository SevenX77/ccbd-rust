# Research: ah 全流程 Grand Tour PR-6 (ERROR Recovery + Provider Session Resume)

## §1 当前 CRASHED 行为图谱

当前 `ccbd` 对 `CRASHED` 状态的处理存在“观测盲区”，导致调谐失效。

- **观测盲区**: `running_agent_hashes` (`src/rpc/handlers.rs:637`) 显式排除了 `CRASHED` 和 `KILLED` 状态。
- **调谐偏移**: `handle_session_realign` (`src/rpc/handlers.rs:439`) 因看不见 `CRASHED` 节点，会将其判定为 `NEW` 节点并尝试走 `spawn` 路径。
- **报错根因**: `handle_agent_spawn` (`src/rpc/handlers.rs:684`) 检查 `agent_exists`。由于 `CRASHED` 节点的 DB 行依然存在，导致抛出 `AGENT_ALREADY_EXISTS` 错误。
- **状态转移**:
  - `CRASHED` 由 `reader` 捕获退出码并调用 `mark_agent_crashed_with_exit_sync` 触发。
  - 常量定义于 `src/db/state_machine.rs:24` (`STATE_CRASHED`)。

## §2 三家 Provider 的 Session Resume 机制

| Provider | Resume Flag | 说明 | 验证位 |
|---|---|---|---|
| **Claude** | `--continue` | 恢复最近一次 Session。若无历史 Session 则报错。 | `src/cli/config.rs:142` (Master 默认已带) |
| **Codex** | 尚未发现 | 暂无明确 CLI resume 标志，需进一步对齐。 | - |
| **Gemini** | 尚未发现 | 暂无明确 CLI resume 标志，需进一步对齐。 | - |

**抽象建议**: 在 `ProviderManifest` 中增加 `resume_command` 或 `resume_args` 字段，默认由 `ccbd` 根据 agent 历史记录决定是否追加。

## §3 ah Sandbox 跟 Provider Session 的交互

- **物理持久化**: `prepare_home_layout_with_extensions` 使用稳定的 `home_root` (基于 `sandbox_dir` 路径 hash)。这意味着跨 Agent 重启，`home_root` 保持不变。
- **目录保留**: `prepare_claude_overrides` 等函数调用 `fs::create_dir_all`，不会删除已存在的 `.claude/projects` 或 `.claude/session-env` 目录。
- **安全隔离**: Provider Session 文件存储在 Sandbox Home 内，受 XDG 规范隔离，不会随 `ccbd` 状态清除而丢失。

## §4 "首次启动 vs 恢复" 判断信号

- **信号源**: DB 中是否存在对应的 `agent_id`。
- **Realign 逻辑挑战**: `handle_session_realign` 在重生 Agent 前会调用 `delete_agent` 清除旧行。
- **方案选择**:
  - **Option A**: 修改 `spawn_realign_agent` 增加 `is_recovery` 参数。
  - **Option B**: `handle_agent_spawn` 内部在创建 `SandboxDirGuard` 后，检查 `home_root` 是否已初始化。
  - **推荐**: 显式传递信号。在 `realign` 流程中，只要该 Agent 曾在 `running_agents` (需包含 `CRASHED`) 列表中，即判定为 `is_recovery=true`。

## §5 SQL 影响面分析

- **修改点**: `running_agent_hashes` 包涵 `CRASHED` 状态。
- **副作用**: `handle_session_realign` 会开始比对 `CRASHED` 节点的 Hash。
- **预期行为**: 如果节点为 `CRASHED`，即便 Hash 一致，也必须触发重生，且此时应带上 `--continue`。

## §6 现有测试改造 (case_11)

- **目标**: `tests/ah_full_e2e_realign_extra.rs` 中的 `case_11` 应由“断言报错”转为“断言成功恢复”。
- **验证手段**:
  - 环境变量注入: 通过 `GRAND_TOUR_MOCK_BEHAVIOR` 模拟 Resume 成功标识。
  - 状态检查: 验证 `IDLE` 状态恢复且 PID 变更。
  - 事件流: 记录恢复产生的 `agent_spawned` 理由。

## §7 可能踩到的 PR-3 后向不兼容

- **Test Expectation**: PR-3 `case_11` 明确断言 `AGENT_ALREADY_EXISTS`。PR-6 必须修改此断言，并移除 `#[ignore]` 标记。
- **RPC Contract**: `session.realign` 对 `CRASHED` 节点的返回将从错误对象变为 `statuses[]` 中的 `REALIGNED` 成功项。

## §8 工程量估算 (LOC)

- SQL/DB 修正: ~30 LOC
- Provider Manifest 扩展: ~40 LOC
- RPC Handler 状态流转: ~60 LOC
- Test Case 11 升级: ~80 LOC
- 总计: **~210 LOC**。
