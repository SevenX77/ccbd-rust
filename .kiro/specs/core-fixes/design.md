# Design Document: ccbd-rust Core Fixes

本文档定义了 `ccbd-rust` 核心修复（Daemon 稳定化阶段）的具体物理设计与架构调整方案。

## 1. Session-Level 生命周期管理 (R1 & R4)

### 1.1 Session 命名与隔离
*   **命名规范**: 弃用当前的共享 Session `ccbd-agents` (`src/tmux/mod.rs:15`)。
    *   **Agent**: 改为 **`agent_<agent_id>`**，复用 `ccb.toml` 中的 agent 名。
    *   **Master**: **v3 修订**: 改为 **`master_<project_id>`**。由于 `src/db/schema.rs:12` 仅存 `master_pane_id`，R1 实施时需确保 Master 拥有独立物理 Session 以支撑 1v1 架构。 [证H 影H 置A]
*   **物理层扩展**: 在 `src/tmux/session.rs` 中新增 `pub(crate) fn kill_session_sync(&self, session_name: &str)`，封装 `tmux kill-session -t <name>` 指令，确保物理资源彻底回收。 [证H 影H 置A]
*   **清理路径**: 在 `cleanup_agent_runtime_resources` (`src/agent_io/registry.rs:50`) 中调用上述方法。
*   **Daemon Shutdown**: 修正 `src/bin/ccbd.rs:122-124`，不再 kill `ccbd-agents`，而是遍历 DB 中所有 `ACTIVE` 状态的 agent Session 逐个销毁。 [证H 影H 置A]

### 1.2 物理 PTY 尺寸锁定
*   **实现位置**: `src/tmux/session.rs:55-88` `ensure_session_sync`。
*   **Flag 顺序**: 严格执行 `tmux new-session -d -s <name> -c <cwd> -x 150 -y 60`。
*   **锁定策略**: 紧随其后执行 `tmux set-option -t <name> window-size manual`。 [证H 影H 置A]

### 1.3 BindsTo 与 systemd 联动
*   **统一名称**: 修正 `src/tmux/scope.rs:55` 中的硬编码 `ccbd-rust.service` 为 **`ccbd.service`**，与 `src/sandbox/systemd.rs:29` 对齐。 [证H 影H 置A]
*   **Master Watch**: 维持 `src/monitor/master_watch.rs:7-53` 逻辑，监控 Master PID 退出并触发 `cascade_kill_session_agents`。 [证H 影M 置A]
*   **Daemon 自杀机制**: 提升至 [置A]。
    *   **配置**: `[daemon] auto_shutdown_on_master_exit = true`。
    *   **逻辑**: 当 `master_watch` 捕获到退出信号且 `db` 中 `active_agents` 归零，在 **5s** 宽限期后执行 `system.shutdown`。 [证H 影L 置A]

### 1.4 架构反向与死代码清理
*   **反向理由**: `6739f6a` 的 Grid 布局逻辑在独立 Session 架构下已无物理基础。 [证H 影M 置A]
*   **清理清单**:
    *   移除 `src/rpc/handlers.rs:348-357` 的 `has_layout_hint` 路由。
    *   移除 `src/cli/start.rs:234-307` 的 `split_plan_for_layout` 及相关 RPC 字段（`layout_direction` 等）。
    *   移除 `src/cli/config.rs:73-81` 的 `LayoutConfig::Grid` 变体。
    *   删除 `tests/mvp12_grid_layout.rs` 及 `cli/start.rs:336-429` 中的 Layout 单元测试。 [证H 影M 置A]

---

## 2. 状态机状态空间扩展 (R2)

### 2.1 WAITING_FOR_ACK 状态转换
*   **状态流程**:
    1.  `IDLE` → (RPC `agent.send` / Orchestrator Dispatch) → `WAITING_FOR_ACK`。 [证H 影H 置A]
    2.  `WAITING_FOR_ACK` → (检测到 `is_meaningful_diff` OR 稳定期超时) → `BUSY`。
    3.  `WAITING_FOR_ACK` → (发生 `TmuxCommandFailed` / PID 死亡) → `STUCK` / `CRASHED` (视错误类型回退)。 [证H 影M 置A]
*   **并发控制 (RPC 互斥)**: **v3 修订**: 在 `handle_agent_send` 入口处严格复用 `src/rpc/handlers.rs:868` 的状态检查逻辑。若 Agent 处于 `WAITING_FOR_ACK` 态，第二个并发 `agent.send` 必须被立即拒绝并返回 `BUSY`，严禁进入 `send_text_to_pane` 路径，确保状态机单一链路执行。 [证H 影H 置A]

### 2.2 视觉确认逻辑 (ACK 落地)
*   **Polling 修正**: 由于 `PaneDiffWatcher` (`src/pane_diff/mod.rs:9`) 的 30s 间隔过大，ACK 确认将采用以下方案： [证H 影H 置A]
    *   **复用并增强 `spawn_new_capture_seed`**: 位于 `src/rpc/handlers.rs:1010`。
    *   **改动**: 将轮询间隔从 100ms 降至 **50ms**，并将退出条件由简单的“非 baseline 前缀”扩展为 **`is_meaningful_diff`** (复用 `src/pane_diff/mod.rs:151`) 与 **`stability_ms`** 复合判定。
    *   **整合**: 该后台任务完成后，直接在回调中执行 `update_agent_state(... "BUSY")`。 [证H 影H 置A]

### 2.3 决策解决 (Uncertain Areas)
*   **不确定区域 #3 (L3 Assert)**: `src/db/state_machine_assert.rs` 中的 L3 证据断言应支持从 `WAITING_FOR_ACK` 跳过。理由：L3 证据具有更高权威性，可直接覆盖物理层的 ACK 等待。 [证M 影M 置A]
*   **不确定区域 #4 (Job Schema)**: 无需 Schema Migration。`db/jobs.rs` 的 `status` 为 `TEXT`，Rust 代码增加对新状态的适配即可。 [证H 影L 置A]

### 2.4 State Guard Audit Table
针对全仓硬编码的 State List 进行 `WAITING_FOR_ACK` 兼容性适配： [证H 影H 置A]

| file:line | 硬编码原状 | 适配决策 |
|---|---|---|
| `src/db/state_machine.rs:56` | `IN ('SPAWNING', 'BUSY')` | 加入 `WAITING_FOR_ACK`，允许 Marker 直接 Match |
| `src/db/state_machine.rs:128` | `state = 'BUSY'` | `WAITING_FOR_ACK` 超时应触发 `STUCK` |
| `src/db/state_machine.rs:194` | `IN ('SPAWNING', 'BUSY')` | 加入 `WAITING_FOR_ACK`，超时转 `UNKNOWN` |
| `src/db/jobs.rs:196` | `IN ('IDLE', 'UNKNOWN')` | 维持原状，排除 `WAITING_FOR_ACK` (防抢跑) |
| `src/db/system.rs:413` | `IN ('SPAWNING', 'BUSY', 'IDLE')` | 加入 `WAITING_FOR_ACK` (Recovery Scan) |
| `src/db/system.rs:549` | `IN ('SPAWNING', 'BUSY', 'IDLE')` | 加入 `WAITING_FOR_ACK` (Crash Recovery) |
| `src/rpc/handlers.rs:786` | reply "BUSY" | 改为根据转换结果返回 "BUSY" 或 "WAITING_FOR_ACK" |
| `src/rpc/handlers.rs:868` | `if state != "IDLE"` | 维持原状，L3 Assert 前必须物理 IDLE |

---

## 3. 路径与挂载绝对校准 (R3)

### 3.1 absolute_path 传导链
*   **结构扩展**: 为 `src/db/schema.rs:86-92` 中的 `Session` struct 增加 `absolute_path: String` 字段。 [证H 影H 置A]
*   **SQL 适配**: 修正 `src/db/sessions.rs:78-92` 的 `query_session_by_id_sync`，通过 `JOIN projects ON sessions.project_id = projects.id` 补齐路径字段。
*   **物理下发**:
    1.  **Master**: 修正 `src/rpc/handlers.rs:146`，改用 `session.absolute_path` 作为 `master_cwd`。 [证H 影H 置A]
    2.  **Agent**: 在 `handle_agent_spawn` 中，不再传递 `session_dir` (sandbox 路径) 给 tmux `-c`，统一改为 `session.absolute_path`。 [证H 影H 置A]

### 3.2 bwrap 与沙盒增强
*   **强制 Chdir**: 在 `src/sandbox/bwrap.rs` 参数流中加入 `--chdir /workspace`。 [证H 影H 置A]
*   **安全隔离清单**: [证H 影H 置A]
    *   **排除规则**: 默认禁止 bind `$HOME`，改为通过物化逻辑生成的 `/home/agent`。
    *   **只读绑定**: `.git` 目录默认执行 `--ro-bind`。
    *   **配置扩展**: `ccb.toml` 增加 `[sandbox] additional_ro_binds = []` 以支持用户自定义挂载点。 [证M 影L 置A]

### 3.3 决策解决 (Uncertain Areas)
*   **不确定区域 #1 (HOME 物化)**: **v3 修订**: 物化业务逻辑本身维持现状（Out-of-Scope），但 `src/provider/home_layout.rs:33` 的形参重命名（`project_root` -> `sandbox_dir`）被判定为 **R3 范围内的 In-Scope Cleanup 任务**，以消除代码层面的语义误导。 [证H 影L 置A]

---

## 4. 配置模板与 CLI 兼容性 (R4)

### 4.1 ccb.toml 推荐配置
```toml
[master]
cmd = "claude --dangerously-skip-permissions --continue /remote-control"
enabled = true
```
*   **语义与分词**: `sh -lc` 模式支持引号内的复杂参数。
*   **Claude Slash Command**: `/remote-control` 虽为 slash 指令，但在 `claude-code` CLI 中支持作为 argv 传入直接启动。若未来版本变化，将通过 `agent_io/writer.rs` 的 keystroke 路径补足（见 4.2）。 [证M 影L 置A]

### 4.2 CLI 交互适配
*   **ccb-rust attach**: mvp15 commit `957dbf5` 已实现。在 R1 改名后，`ccb-rust attach <agent_id>` 逻辑应映射至 `tmux attach -t agent_<agent_id>`。 [证H 影M 置A]
*   **不确定区域 #2 (Paste-buffer 风险)**: **v3 修订**: **Deferred**。确认 Bug X（Paste-buffer 导致的特殊字符截断）风险依然存在，单 Session 隔离仅降低了跨 Agent 干扰，未解决 PTY 注入根因。此项任务正式延后至后续专向 Spec，本次不改动 `agent_io/writer.rs`。 [证M 影H 置A]

### 4.3 Onboarding & Migration
*   **旧配置兼容**: Daemon 启动时应检查旧版 `ccb.toml`。若 `master.cmd` 为空，自动填入默认 `claude`。
*   **孤儿迁移**: 本次更新后，旧的共享 Session `ccbd-agents` 将不再被新代码管理，建议通过 `doctor` 提示用户手动清理一次。 [证L 影L 置A]

---

## 5. 跨 Requirement 耦合与实施顺序

### 5.1 耦合分析表
| 耦合对 | 影响描述 | 协同决策 |
|---|---|---|
| **R1 × R2** | 1-Session 隔离使得 PTY 尺寸锁生效，为 R2 的视觉确认提供稳定基座。 | `spawn_new_capture_seed` 必须适配新 Session 命名。 |
| **R1 × R3** | R1 的 Session 目录不再作为 tmux CWD，必须强制从 R3 的传导链获取。 | `ensure_session` 与 `spawn_window` 统一使用 `absolute_path`。 |
| **R1 × R4** | `ccb-rust attach` 语义需根据 R1 的新 Session 命名规则重新映射。 | 统一使用 `agent_<agent_id>` 作为物理标识符。 |
| **R2 × R3** | `agent.spawn` 路径同时受新状态机与新 CWD 下发影响。 | 明确 `session_dir` (sandbox) 仅限 PTY 内部，物理 cwd 走 R3。 |
| **R2 × R4** | `ccb.toml` 启动命令的复杂性可能导致 ACK 时间窗口波动。 | R2 需保持 500ms 强制窗口作为兜底。 |
| **R3 × R4** | Master 启动参数透传后，其工作目录校准是保证 `--continue` 生效的前提。 | 必须保证 Master CWD 先于指令执行校准完成。 |

### 5.2 推荐实施顺序
1.  **Phase 1 (Isolation)**: R1 (Session 隔离) + R3 (路径校准)。解决最底层的物理错误。
2.  **Phase 2 (State Machine)**: R2 (WAITING_FOR_ACK)。解决状态机假死。
3.  **Phase 3 (Config)**: R4 (ccb.toml 更新) + `ccb-rust attach` 适配。 [证H 影M 置A]
