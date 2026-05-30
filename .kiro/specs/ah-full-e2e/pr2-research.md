# Research: ah 全流程 E2E Grand Tour PR-2 (DRIFT + NEW)

## §1 PR-1 覆盖现状 vs PR-2 目标矩阵

PR-1 在 `tests/ah_full_e2e_main.rs` 的 Step 7-8 中通过手动构造 `session.realign` 参数实现了初步的指纹漂移测试。按 design.md §7.2 规划，PR-2 聚焦于 DRIFT 与 NEW 分支的扩展。

| 特性 | PR-1 (Mainline) | PR-2 (Branch Matrix) | 状态 |
|---|---|---|---|
| **ENV Drift** | 已覆盖 (GRAND_TOUR_DRIFT) | 混合漂移验证 | 增强 |
| **HOOKS Drift** | ❌ 仅写了 ah.toml, 未传 RPC | 覆盖 (新增/修改/删除 Hook) | 新增 |
| **PLUGINS Drift** | ❌ 未覆盖 | 覆盖 (插件增减) | 新增 |
| **NEW Agent** | ❌ 未覆盖 | 覆盖 (session.realign NEW 分支) | 新增 |
| **NO_CHANGE** | ❌ 隐含验证 | 显式验证 (幂等性) | 增强 |

*注：SKIPPED_BUSY, FORCE_REALIGN 移至 PR-3 (BUSY 专项)；Agent 无 CMD 字段，漂移由 ENV/HOOKS/PLUGINS 组合触发。*

## §2 DRIFT path 真实状态结果调研

### 2.1 指纹计算与判别 (Source Analysis)
`src/rpc/handlers.rs` 调用 `compute_config_hash` 后对比：
- **NO_CHANGE**: `handlers.rs:467` 命中 hash 一致，返回 `status: NO_CHANGE`。
- **DRIFT 判别**: `handlers.rs:475` 调用 `drift_reason` 识别差异点。
- **REALIGNED**: `handlers.rs:493-520` 执行自毁并调用 `spawn_realign_agent`，返回 `status: REALIGNED` 并记录 `drift_realigned` 事件。

### 2.2 物理副作用要求 (FS Verification)
- **Hooks/Plugins**: `session.realign` 最终会触发 `spawn_realign_agent`，其内部调用 `agent.spawn`。`agent.spawn` 会重新执行 `prepare_home_layout_with_extensions`。
- **PR-2 验证点**: 必须断言 Sandbox 内的 `.claude/hooks/` 等目录确实反映了漂移后的最新物理文件，而不仅仅是 DB 里的 hash 变化。

## §3 NEW 分支真实 RPC Chain 追踪

当 `session.realign` 发现 `ah.toml` 中存在 DB 未记录的 `agent_id` 时：
1. **Handler 分支**: `src/rpc/handlers.rs:457` (else block)。
2. **动作**: 调用 `spawn_realign_agent(..., killed_before_spawn: false)`。
3. **事件序列**:
   - 插入 `agent_spawned` 事件，`payload.reason` 为 `"NEW"` (`src/rpc/handlers.rs:623`)。
   - 状态机变迁：`agents.state` 从不存在到 `SPAWNING` -> `IDLE`。
   - **验证重点**: 测试断最终 IDLE + agent_spawned, 不依赖瞬时 SPAWNING。

## §4 Harness 复用度与扩展

PR-1 的 `Harness` (基于 `dispatch` 模块) 整体架构可复用，但由于需要引入 FS 物理断言及多次 realign helper，复用度约为 **70-85%**。

**建议新增 Helper**:
- `h.query_agent_events(agent_id, type)`: 快速检索特定事件。
- `h.assert_sandbox_file(session_id, agent_id, sub_path)`: 验证物理副作用（如 symlink 是否指向新 hook 路径）。

## §5 现有相关测试 Gap 分析

- `pr4e_up_fingerprint.rs`: 验证了 `realign` 的核心逻辑，但主要关注 hash 计算和 Master 漂移，缺失 Grand Tour 要求的 4 维联合断言（Tmux/Sandbox 检查）。
- `pr4c_hooks_plugins.rs`: 纯单元测试，不涉及 `session.realign` 触发的动态生命周期变更。
- **PR-2 价值**: 首次在“不重置 DB”的长生命周期内验证连续多次漂移（ENV/HOOKS/PLUGINS 混合）的系统稳定性。

## §6 工程量估算 (LOC)

预计新增 `tests/ah_full_e2e_drift.rs` 与 `tests/ah_full_e2e_new.rs` (或合并为 `drift_matrix`)：
- Harness 扩展: ~80 LOC
- DRIFT 矩阵测试 (多种组合漂移): ~400 LOC
- NEW 路径测试: ~100 LOC
- 总计: **500-700 LOC** (与 PR-1 M2 规模相当)。
