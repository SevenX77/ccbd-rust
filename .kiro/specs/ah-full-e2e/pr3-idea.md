# Idea: ah 全流程 E2E Grand Tour PR-3 (ORPHAN + BUSY + ERROR)

## §1 第一性原理 (First Principles)

PR-3 的本质是“响应正确性”验证。与 PR-2 验证“功能对”不同，PR-3 侧重于验证系统对故障与资源占用的**处置边界**。
- **ORPHAN**: 验证拓扑收缩时进程与元数据的原子收割。
- **BUSY**: 验证活跃任务与配置调谐冲突时的“安全跳过”与“强制剥夺”策略。
- **ERROR**: 验证异常退出后的状态自愈隔离。

## §2 核心机制思路

- **串接策略**: 推荐采用**单长 Grand Tour 串联模式** (`grand_tour_realign_extra_matrix`)。
  - **理由**: 异常状态（如 BUSY）往往是长生命周期的中间态。串联测试能真实模拟“用户运行中修改配置 -> 发现 Agent 忙碌 -> 再次强制执行”的拓扑演进因果链。
- **Mock Provider 多模式设计**:
  - **机制**: 扩展 PR-2 的 `install_fake_claude` 脚本。
  - **触发**: 通过环境变量 `GRAND_TOUR_MOCK_BEHAVIOR` 控制。
    - `BUSY`: 脚本进入 `sleep infinity` 或无限循环，使 Agent 停留于 `BUSY` 状态。
    - `CRASH`: 脚本立即 `exit 1`，模拟 Provider 崩溃，触发 `reader` 逻辑更新 DB 为 `CRASHED`。
- **BUSY 状态构造原则**:
  - **推荐路径**: **真 Job 驱动**。通过 `job.submit` 派发一个触发 `BUSY` 行为的任务，在任务运行中途发起 `realign`。此路径验证了 `dispatch` 与 `realign` 的物理交汇。
  - **兜底场景**: 仅在纯逻辑验证且无需 PTY 参与时，使用裸 SQL `update_agent_state_direct` 注入 `BUSY` 状态。

## §3 关键决策

- **Decision 1: DRIFT 类型组合 (Mixed Drift)**
  - 采用 ENV + HOOKS 混合漂移。验证在 `SKIPPED_BUSY` 路径下，系统能否在 `drift_skipped` 事件 payload 中准确记录所有漂移维度，而不仅仅是单一类型。
- **Decision 2: Mock 行为切换粒度 (Per-Agent Control)**
  - 通过 `session.realign` payload 中的 `agents[i].env` 传入行为控制变量。支持在同一 Session 内让 a1 保持 `IDLE` (Echo 模式)，a2 进入 `BUSY` (Sleep 模式)，验证调谐的精确打击能力。
- **Decision 3: ERROR Documents-gap 测试断言**
  - **命名**: `case_10_error_recovery_known_gap`。
  - **形式**: 断言 `session.realign` 针对 `CRASHED` 节点的恢复请求返回特定错误（如 `AgentAlreadyExists` 或 DB 约束报错），并保持 `#[ignore]` 状态。目的是锁定该逻辑缺陷现状，作为 PR-4 修复的基准红灯。

## §4 Harness 扩展边界

- **新增 Helper**:
  - `h.wait_for_tmux_pane_gone(agent_id)`: 验证 ORPHAN force cleanup 物理副作用。
  - `h.update_agent_state_direct(agent_id, state)`: 设置 `CRASHED` 始态。
  - `h.query_agent_last_error(agent_id)`: 检索 `error_code` 与 `exit_code`。
- **复用**: 沿用 PR-1/2 的 `rpc` 调用、`query_agent_state`、`query_agent_events` 等基础组件。

## §5 矩阵案例命名与顺序

1. `case_06_orphan_audit_only`: 验证无 force 时仅报 ORPHAN 状态。
2. `case_07_orphan_force_cleanup`: 验证 force 模式下物理收割。
3. `case_08_busy_skip`: 验证 busy agent 漂移被跳过。
4. `case_09_busy_force_realign`: 验证 force 强制夺权并重生。
5. `case_10_error_recovery_known_gap`: 文档化 CRASHED 恢复缺陷。
