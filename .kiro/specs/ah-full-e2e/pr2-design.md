# Design: ah 全流程 E2E Grand Tour PR-2 (DRIFT + NEW)

## §1 第一性原理 + 目标

- **Grand Tour 的纵向深度**: PR-1 验证了“全生命周期”的横向连通性（从 Start 到 Stop）。PR-2 的目标是验证该生命周期在“配置变更”与“动态拓扑演进”时的纵向稳定性。
- **配置一致性保障**: 在 Grand Tour 维度，DRIFT 矩阵不仅验证 RPC 返回 REALIGNED，更重要的是验证 L2 调度层能否在不丢失 Session 上下文的前提下，通过“原位收割并重生物化”使物理环境（Sandbox/Hooks/Plugins）与期望状态（ah.toml）达到最终一致。
- **拓扑动态性**: NEW 分支验证系统在 Session 运行中途动态追加 Agent 的能力，确保新增节点不会对现有活跃节点的 PTY/Tmux 环境产生副作用。

## §2 核心机制思路

- **长生命周期长串联**: 复用 PR-1 的 `Harness` 与同一持久化 DB，不重置环境。在完成 PR-1 的 Mainline 后，或在一个独立的长测试方法中，连续触发 5-6 次 `session.realign`。
- **扩展 Harness Helper**:
  - **物理副作用探针**: 增加 `assert_sandbox_file`；新增 `assert_symlink_target`（验证软链接真实指向）与 `assert_json_contains`（验证 JSON 关键键值对）。
  - **事件序列追踪**: 增加 `query_agent_events`，精确断言 `drift_realigned` 与 `agent_spawned` 的因果链，而非仅看最终 State。
- **多次调谐幂等性**: 连续执行两次相同的 `ah up` 应触发 `NO_CHANGE` 路径，验证系统不会因指纹计算的微小偏差而陷入“死循环重启”。

## §3 关键决策

- **DRIFT 类型组合 vs 单一**: 采用“渐进叠加”策略。
  - Step 1: 仅 ENV 漂移（验证 Hash 对比逻辑）。
  - Step 2: 仅 Hooks 漂移（验证 FS 物理物化逻辑）。
  - Step 3: Plugins + ENV 混合漂移（验证复杂配置下的原子性）。
- **FS 物理断言粒度**: 不仅检查目录存在，必须检查 Sandbox 内 Rules 文件内容与 Hook Symlink 指向，确保 `prepare_home_layout` 的 Rust 重写版本在真实调谐流中无漏项。
- **NEW 分支隔离**: NEW Agent 的创建必须在 `session_id` 已存在且有活跃 Agent (a1) 的背景下进行。验证 a1 的 PTY 流量与 Tmux Pane 不受 a2 启动过程的干扰（无闪烁、无串号）。
- **验证终态判定**: NEW 分支不再轮询瞬时的 `SPAWNING` 状态，改为“阻塞等待 IDLE + DB 记录 agent_spawned 事件”的离散验证模式，提高测试稳定性。

## §4 测试矩阵 (5 Cases + 4 维联合断言)

每个 Case 必须执行：[OS Tmux 检查] + [SQLite 状态位/事件检查] + [FS Sandbox 物理副作用检查] + [RPC Result 结构断言]。

1. **ENV Drift**: 修改环境变量 key/value -> 触发 `REALIGNED` -> 验证新 PID 产生且旧 PID 消失。
2. **HOOKS Drift**: 修改 `ah.toml` 指向新脚本，且 **`session.realign` RPC payload 显式包含新 hooks 字段** -> 触发 `REALIGNED` -> 验证 Sandbox 内 `.claude/hooks/` 出现新软链接。
3. **PLUGINS Drift**: 修改插件列表，且 **`session.realign` RPC payload 显式包含新 plugins 字段** -> 触发 `REALIGNED` -> 验证 Sandbox 插件物化目录更新。
4. **NO_CHANGE (Idempotency)**: 不改配置再次 `ah up` -> 验证 RPC 返回 `NO_CHANGE` -> 验证 PID 与事件计数器无增长。
5. **NEW Agent**: `ah.toml` 新增 a2 块 -> 触发 `NEW` 分支 -> 验证 a2 状态到达 `IDLE` 且 `agent_spawned` 理由为 `NEW`，同时断言 a1 维持 `IDLE` 或 `BUSY` 不受损。

## §5 物理断言风格细化

- **Sandbox FS**: 必须覆盖 `state_dir/sandboxes/<session>/<agent>` 下的 `.claude/CLAUDE.md` (Rules) 与 `.claude/hooks/` (Extensions) 的物理校验。
- **Tmux 侧**: 验证 `agent_a2` (NEW) 的 Pane ID 与 Session 名符合 L2 命名规范（按 `agent_session_name`），且与 a1 处于不同的 Tmux Pane。
- **SQLite 侧**: 关键在于 `agents.config_hash` 的新旧对账，以及 `events.event_type` 必须精确匹配 `drift_realigned` (针对 1-3) 或 `agent_spawned` (针对 5)。

## §6 实施切片与工程平衡

- **文件命名**: 新建 `tests/ah_full_e2e_drift.rs` 承载整个矩阵。不建议拆分 drift 和 new，因为它们共享高度一致的 realign 调谐环境，合并可减少 Harness 初始化开销。
- **工程量**: 预计 **500-700 LOC**。其中 Harness Helper (T2 扩展) 预计占 **80-150 LOC**，重点在于构造多种漂移形态的 `ah.toml` Fixture 辅助函数。
- **不重置策略**: 5 个 Case 顺序执行，前序 Case 的终态是后序 Case 的始态，最大化模拟真实用户长期运行下的配置演进。

## §7 决议汇总 (Finalized)

- **CI Lane**: 归入 `#[ignore]` Grand Tour 专项，与 PR-1 一同在 Nightly 执行，不阻塞 Main 流程。
- **工作量**: 500-700 LOC，优先级 P0 (PR-2)。
- **决策点**: 显式拒绝在 PR-2 中处理 **ORPHAN / BUSY / ERROR** 分支，留待 PR-3 处理以保持测试逻辑专注。
- **Helper 要求**: 必须在 Harness 中实现 `assert_sandbox_file` 等物理断言 Helper。
