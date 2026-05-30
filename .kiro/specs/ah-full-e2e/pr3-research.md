# Research: ah 全流程 E2E Grand Tour PR-3 (ORPHAN + BUSY + ERROR)

## §1 PR-2 已覆盖 vs PR-3 待覆盖矩阵

PR-2 验证了 DRIFT (ENV/HOOKS/PLUGINS) 与 NEW 的 Happy Path。PR-3 聚焦于异常生命周期与调谐分支。

| 类别 | 场景 / 分支 | PR-2 (Baseline) | PR-3 (Target) | 验证重点 |
|---|---|---|---|---|
| **ORPHAN** | Audit-only (no force) | ❌ 未覆盖 | 覆盖 | 验证 RPC 返回 ORPHAN，DB 不删除，进程不杀 |
| **ORPHAN** | Force Cleanup | ❌ 未覆盖 | 覆盖 | 验证进程+tmux销毁，DB行保留为 KILLED state |
| **BUSY** | SKIPPED_BUSY | ❌ 未覆盖 | 覆盖 | 验证忙碌 agent 漂移时被跳过，记录 drift_skipped |
| **BUSY** | FORCE_REALIGN | ❌ 未覆盖 | 覆盖 | 验证强行收割忙碌 agent 并重生，即便有任务运行 |
| **ERROR** | Provider Crash | ❌ 未覆盖 | 覆盖 | 验证退出码非 0 时进入 CRASHED，记录 error_code |
| **ERROR** | Recovery (Gap) | ❌ 未覆盖 | 覆盖 (a) | 锁定 recovery 失败现状，断言报错信息 (Documents Gap) |

## §2 ORPHAN 真实 RPC Chain 追踪 (Source Analysis)

当 `session.realign` 发现 DB 存在但 `ah.toml` 已移除的 agent 时：
1. **Handler 分支**: `src/rpc/handlers.rs:523` (loop over `running_agents`)。
2. **逻辑判断**: 如果 `requested_ids` (来自 ah.toml) 不包含当前 ID，则视为 ORPHAN。
3. **子路径**:
   - **Force=true**: 调用 `mark_agent_killed` (reason: `ORPHAN_FORCE_CLEANUP`) -> 触发 `cleanup_agent_runtime_resources` -> `insert_event` (`agent_killed`)。注意：`mark_agent_killed` 仅将 state 更新为 `KILLED`，不会从 `agents` 表物理删除行。
   - **Force=false**: 仅向 `results` 数组 push `status: ORPHAN`。
4. **物理副作用**: Force 模式下应物理销毁 Tmux Pane 与清理 `agent_io` 注册表。

## §3 BUSY 子分支判别点

在 `handle_session_realign` 的主循环中，漂移判断逻辑如下：
- **判断位**: `src/rpc/handlers.rs:475-476`。
- **SKIPPED_BUSY**: `!force` 路径优先执行 skip (:476-491)。如果 `running.state == "BUSY"`，插入 `drift_skipped` 事件并返回 `SKIPPED_BUSY`。
- **FORCE_REALIGN**: 剩余 BUSY 情况进入 force 处理 (:493-497)。destructive_reason 设为 `DRIFT_FORCE_REALIGN`，随后执行 kill & respawn。

## §4 ERROR (CRASHED) 触发与 Recovery

1. **触发机制**: Provider 进程退出时，由 `reader` 循环捕获 exit_code，通过 `mark_agent_crashed_with_exit_sync` 更新状态为 `CRASHED` (src/db/state_machine.rs:24 定义常量)。
2. **持久化**: 记录 `exit_code` 与 `error_code` (默认 `AGENT_UNEXPECTED_EXIT`)。
3. **Recovery 处置策略 (选 a)**:
   - **决策**: PR-3 采用方案 (a)，即写一个 `#[ignore]` case 记录已知缺陷。
   - **理由**: PR-3 定位为测试 PR，应遵循设计保守原则，通过测试锁定现状（即当前 recovery 会因 `agent_exists` 报错而失败），而非在测试过程中顺带修复 src。真正的修复应在后续 PR-4 中进行。
   - **验证重点**: 断言报错信息，确保逻辑缺陷被文档化且可复现。

## §5 Harness 复用度与扩展需求

PR-2 的 `Harness` 复用度约为 **75-80%**。
**Caveat & 扩展**:
- **BUSY 状态构造**: 优先使用“真 long-sleep mock”自然产生。通过 dispatch 一个长耗时任务使 DB state 变为 `BUSY`后再触发 realign，避免裸 SQL 注入导致的 stub bypass。
- **CRASHED 状态构造**: 考虑到模拟真实进程崩溃复杂度较高，允许使用裸 SQL helper (`h.update_agent_state_direct`) 作为兜底手段来设置 `CRASHED` 始态。
- **物理断言**: 需新增 `h.wait_for_tmux_pane_gone(agent_id)`。

## §6 现有相关测试 Gap

- **Out-of-Scope (留 PR-5+ future)**:
  - **Master Pane Lifecycle Drift**: master cmd 变更触发的 realign 逻辑。
  - **Session-level 聚合 Status**: `handlers.rs:386/401` 处的 session 级聚合返回，目前仅验证了 per-agent 数组。
- **PR-3 价值**: 完善 Realignment 调谐矩阵的异常分支覆盖。

## §7 Mock Provider 扩展 (Fake Claude)

PR-3 需要多模式支持：
- **BUSY Mode**: 通过环境变量触发 `while true; do sleep 1; done`。
- **ERROR Mode**: 模拟退出码非 0 情况。

## §8 工程量估算

预计新增 `tests/ah_full_e2e_realign_extra.rs`：
- Harness 扩展: ~80 LOC
- Case 实施 (ORPHAN x2, BUSY x2, ERROR x1): ~450 LOC
- 总计: **550 LOC** 左右。
