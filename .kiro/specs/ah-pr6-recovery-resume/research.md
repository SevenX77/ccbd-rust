# Research: ah 全流程 Grand Tour PR-6 (ERROR Recovery + Provider Session Resume)

## §1 当前 CRASHED 行为图谱

当前 `ccbd` 对 `CRASHED` 状态的处理存在“观测盲区”，导致调谐失效。

- **观测盲区**: `running_agent_hashes` (`src/rpc/handlers.rs:629`) 显式排除了 `CRASHED` 和 `KILLED` 状态（`:637`）。
- **调谐偏移**: `handle_session_realign` (`src/rpc/handlers.rs:360`) 因看不见 `CRASHED` 节点（调用 `:439`），会将其判定为 `NEW` 节点并尝试走 `spawn` 路径。
- **报错根因**: `handle_agent_spawn` (`src/rpc/handlers.rs:670`) 检查 `agent_exists`（`:684`）。由于 `CRASHED` 节点的 DB 行依然存在，导致抛出 `AGENT_ALREADY_EXISTS` 错误。
- **状态转移**:
  - `CRASHED` 由 `reader` 捕获退出码并调用 `mark_agent_crashed_with_exit_sync` 触发。
  - 常量定义于 `src/db/state_machine.rs:24` (`STATE_CRASHED`)。

## §2 三家 Provider 的 Session Resume 机制对比 (Agent Worker)

经核实，当前 Agent Worker 的 Manifest 配置均不含 Resume 标志。

| Provider | Current Command (Manifest) | Resume Flag | 验证位 (Manifest) |
|---|---|---|---|
| **Claude** | `claude --dangerously-skip-permissions` | `--continue` | `src/provider/manifest.rs:202` |
| **Codex** | `codex --dangerously-bypass-approvals...` | 尚未发现 | `src/provider/manifest.rs:157` |
| **Gemini** | `gemini --yolo` | 尚未发现 | `src/provider/manifest.rs:185` |

**重要校正**: 此前 research 误引用的 `--continue` 证据位 `src/cli/config.rs:175` 属于 **Master CLI** 默认配置，并非 Agent Worker。PR-6 的核心任务是在 `wrap_command` (`src/sandbox/systemd.rs:8`) 及其调用链中，根据恢复信号动态注入该标志。

## §3 ah Sandbox 跟 Provider Session 的交互

- **物理持久化**: `prepare_home_layout_with_extensions` 使用稳定的 `home_root` (基于 `sandbox_dir` 路径 hash)。这意味着跨 Agent 重启，`home_root` 保持不变。
- **目录保留**: Provider 物化逻辑（如 `prepare_claude_overrides`）采用 `fs::create_dir_all`，**不会清理**已存在的 Provider Home 内容。
- **安全隔离**: Provider Session 文件存储在 Sandbox Home 内，受 XDG 规范隔离，不会随 `ccbd` 状态清除而丢失。

## §4 "首次启动 vs 恢复" 判断信号

- **显式信号**: `is_recovery` = DB 中此前已存在该 `agent_id` 的记录（即该 ID 曾在 `running_agents` 列表中，包含 `CRASHED` 状态）。
- **逻辑分支**:
  - **First Spawn**: DB 无 row -> 正常启动，不带 resume 标志。
  - **Recovery Spawn**: DB 有 row -> 追加 `--continue` 等 resume 标志。
- **Realign 兼容**: `handle_session_realign` 在识别出 Agent 曾存在后，即使随后执行了 `delete_agent` 以准备重生，也必须向 `spawn` 接口透传 `is_recovery=true` 信号。

## §5 SQL 影响面分析

- **修改点**: `running_agent_hashes` 包含 `CRASHED` 状态。
- **预期行为**: `handle_session_realign` 将开始比对 `CRASHED` 节点的 Hash。即便 Hash 一致，调谐器也应识别出其物理进程已失，触发 REALIGNED 路径执行恢复重启。

## §6 现有测试改造 (case_11)

- **目标**: `tests/ah_full_e2e_realign_extra.rs` 中的 `case_11` 应由“断言报错”转为“断言成功恢复”。
- **验证手段**:
  - **Flag 注入验证**: 扩展 `fake claude` 脚本，启动时将 `"$@"` 写入 `GRAND_TOUR_RESUME_ARG_MARKER` 环境变量指向的文件。
  - **物理对账**: 测试端读取 marker 文件，断言包含 `--continue`。
  - **状态断言**: 验证恢复后状态回到 `IDLE`，PID 发生变更，且产生的 `agent_spawned` 事件理由为 `DRIFT_REALIGN` 或 `RECOVERY` 约定值。

## §7 Scope 约束与后向兼容

- **PR-6 Scope**: **Claude Agent Worker Resume Only**。Codex/Gemini 的 Resume 机制调研与实现延后至 PR-7+。
- **Test Compatibility**: PR-3 `case_11` 的 `AGENT_ALREADY_EXISTS` 断言将被废弃，移除 `#[ignore]` 标记。
- **RPC Contract**: `session.realign` 对 `CRASHED` 节点的返回将从 RPC Error 变为 `statuses[]` 数组中的成功项。

## §8 工程量估算 (LOC)

- SQL/DB 观测点修正: ~30 LOC
- Provider Manifest & `wrap_command` 扩展: ~100 LOC
- RPC Handler 信号透传: ~80 LOC
- Test Case 11 升级与 Marker 验证: ~120 LOC
- 总计: **250-350 LOC**。
