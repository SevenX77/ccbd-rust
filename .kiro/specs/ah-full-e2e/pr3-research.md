# Research: ah 全流程 E2E Grand Tour PR-3 (ORPHAN + BUSY + ERROR)

## §1 PR-2 已覆盖 vs PR-3 待覆盖矩阵

PR-2 验证了 DRIFT (ENV/HOOKS/PLUGINS) 与 NEW 的 Happy Path。PR-3 聚焦于异常生命周期与调谐分支。

| 类别 | 场景 / 分支 | PR-2 (Baseline) | PR-3 (Target) | 验证重点 |
|---|---|---|---|---|
| **ORPHAN** | Audit-only (no force) | ❌ 未覆盖 | 覆盖 | 验证 RPC 返回 ORPHAN，DB 不删除，进程不杀 |
| **ORPHAN** | Force Cleanup | ❌ 未覆盖 | 覆盖 | 验证 agent 被 kill，DB 删除，tmux pane 销毁 |
| **BUSY** | SKIPPED_BUSY | ❌ 未覆盖 | 覆盖 | 验证忙碌 agent 漂移时被跳过，记录 drift_skipped |
| **BUSY** | FORCE_REALIGN | ❌ 未覆盖 | 覆盖 | 验证强行收割忙碌 agent 并重生，即便有任务运行 |
| **ERROR** | Provider Crash | ❌ 未覆盖 | 覆盖 | 验证退出码非 0 时进入 CRASHED，记录 error_code |
| **ERROR** | Recovery | ❌ 未覆盖 | 覆盖 | 验证通过 realign 或 spawn 使 CRASHED 节点回 IDLE |

## §2 ORPHAN 真实 RPC Chain 追踪 (Source Analysis)

当 `session.realign` 发现 DB 存在但 `ah.toml` 已移除的 agent 时：
1. **Handler 分支**: `src/rpc/handlers.rs:523` (loop over `running_agents`)。
2. **逻辑判断**: 如果 `requested_ids` (来自 ah.toml) 不包含当前 ID，则视为 ORPHAN。
3. **子路径**:
   - **Force=true**: 调用 `mark_agent_killed` (reason: `ORPHAN_FORCE_CLEANUP`) -> 触发 `cleanup_agent_runtime_resources` -> `insert_event` (`agent_killed`)。
   - **Force=false**: 仅向 `results` 数组 push `status: ORPHAN`。
4. **物理副作用**: Force 模式下应物理销毁 Tmux Pane 与清理 `agent_io` 注册表。

## §3 BUSY 子分支判别点

在 `handle_session_realign` 的主循环中：
- **判断位**: `src/rpc/handlers.rs:475`。
- **SKIPPED_BUSY**: 如果 `running.state == "BUSY" && !force`，插入 `drift_skipped` 事件并返回 `SKIPPED_BUSY` (:476-491)。
- **FORCE_REALIGN**: 如果 `running.state == "BUSY" && force`，destructive_reason 设为 `DRIFT_FORCE_REALIGN` (:493-497)，随后执行 delete & spawn。

## §4 ERROR (CRASHED) 触发与 Recovery

1. **触发机制**: Provider 进程退出时，由 `reader` 循环捕获 exit_code，通过 `mark_agent_crashed_with_exit_sync` (src/db/agents_lifecycle.rs:65) 更新状态。
2. **持久化**: 记录 `exit_code` 与 `error_code` (默认 `AGENT_UNEXPECTED_EXIT`)。
3. **Recovery 路径**:
   - **Current Gap**: `running_agent_hashes` 显式排除了 `CRASHED` 状态 (src/rpc/handlers.rs:637)，导致 realign 会尝试以 `NEW` 路径拉起，但 `handle_agent_spawn` 内部有 `agent_exists` 检查会报错。
   - **PR-3 验证**: 必须验证 realign 能否通过“先删后建”或 spawn 覆盖成功恢复 CRASHED 节点，或者揭示该逻辑缺陷。

## §5 Harness 复用度与扩展需求

PR-2 的 `Harness` 复用度极高 (~90%)。
**需扩展 Helper**:
- `h.update_agent_state_direct(agent_id, state)`: 直接操作 DB 模拟 BUSY/CRASHED 始态。
- `h.wait_for_tmux_pane_gone(agent_id)`: 验证 ORPHAN force cleanup。
- `h.query_agent_last_error(agent_id)`: 检查 `error_code` 字段。

## §6 现有相关测试 Gap

- `ack_fallback_lifecycle.rs`: 涉及部分异常退出，但侧重于 ACK 阶段。
- `ah_config_drift.rs`: 侧重配置解析，而非 realign 调谐流。
- **PR-3 价值**: 首次在 Grand Tour 视角下对 realign 的所有异常分支进行 4 维联合断言（OS/DB/FS/RPC）。

## §7 Mock Provider 扩展 (Fake Claude)

PR-2 的 `install_fake_claude` 仅支持 echo。PR-3 需要多模式支持：
- **BUSY Mode**: `while true; do sleep 1; done` (模拟长任务)。
- **ERROR Mode**: `exit 1` (模拟 Provider 崩溃)。
- **Trigger**: 可通过环境变量 (e.g. `MOCK_BEHAVIOR=CRASH`) 在 spawn 时控制。

## §8 工程量估算

预计新增 `tests/ah_full_e2e_realign_extra.rs`：
- Harness 扩展: ~60 LOC
- Fixture 增强: ~40 LOC
- Case 实施 (4-5 个): ~400 LOC
- 总计: **500 LOC** 左右。
