# Kiro Requirements: MVP 9 (项目启动器与生命周期回收 / The Launcher & Lifecycle Convergence)

> **文档定位**：本文件是 ccbd-rust 迈向 1.0 正式版的最后一个核心阶段（MVP 9）的官方 R (Requirements) 规格。本阶段的目标是实现从"单点 Agent 操作"到"项目级管控"的跨越，补齐一键启动（Launcher）、环境清理（Reconcile）以及高级任务控制（Cancel），彻底达到并超越旧版 Python ccb 的功能集。

---

## 0. 立项背景与边界共识

### 0.1 为什么必须做这个 MVP（核心驱动）
MVP 8 完成后，ccbd-rust 已经拥有了强大的 L3 调度大脑，能够处理排队、异步等待和流式输出。但在实际工程实践中，用户并不想通过一个个 `ccb spawn` 手动拉起 Agent，也不想在切换项目时手动去 `tmux kill-pane`。

旧 Python ccb 的易用性核心在于：
- **`ccb start`**：读一下 `.ccb/ccb.config`，一键把 4 个 Claude/Codex 分屏拉起来。
- **`ccb kill`**：一键把当前项目的 Daemon 和所有 Agent 干净利落地杀掉。
- **`ccb doctor`**：环境出问题了（如残留 PID 锁、Tmux 假死），一键诊断并修复。

**MVP 9 是 ccbd-rust 的"交付层"补全计划。** 没有它，ccbd-rust 只是一个强大的库/组件；有了它，它才是能替代旧版工具链的产品。

### 0.2 本 MVP 的核心边界
- **一键启动 (Launcher)**：支持基于配置文件（推荐 TOML）的批量启动。
- **布局管理 (Tmux Layout)**：实现自动分屏，不再是单一窗口。
- **生命周期 Reconcile**：Daemon 启动或项目关闭时的残余进程/资源回收。
- **高级 Job 交互**：支持 Job 取消（Cancel）和优先级（Priority）。
- **1.0 CLI 闭环**：补齐 `doctor`, `logs`, `config`, `version` 等所有剩余子命令。

---

## 1. 最小可工作验收标准 (Acceptance Criteria)

MVP 9 验收必须全部通过：

1. **AC1 [Launcher / ccb start]**：在项目根目录下执行 `ccb`（或 `ccb start`），系统能自动寻找 `ccb.toml` 配置，并向全局 `ccbd` 发起批量 Spawn 请求，一次性拉起全家桶。
2. **AC2 [Auto Layout / 自动布局]**：`ccb start` 启动后，Agent 不应散乱在不同 Window，而应根据配置自动完成 Tmux 分屏（如：左边两个面板垂直分，右边一个面板占半屏）。
3. **AC3 [Project Kill / 资源回收]**：执行 `ccb kill`。Daemon 应能根据当前工作目录（CWD）定位到对应的 Session，并递归 SIGKILL 所有关联 Agent 进程，最后关闭 Tmux 窗口。
4. **AC4 [Boot Reconcile / 自愈能力]**：当 `ccbd` 崩溃重启时，它应扫描 DB 中的 `BUSY/IDLE` 状态 Agent，通过 `pidfd_open` 或 `/proc` 校验其真实存活性。若进程已消失，自动将其标记为 `UNKNOWN` 或 `CRASHED` 并回收残留 Pane。
5. **AC5 [Job Control / 任务取消]**：实现 `ccb cancel <job_id>`。对于排队中的 Job 直接取消；对于正在执行的 Job，尝试发送 Ctrl-C (SIGINT) 给 PTY 强制中断。
6. **AC6 [CLI Parity / 完整体验]**：补齐 `ccb doctor`（环境诊断）、`ccb logs`（查看/回溯原始日志）、`ccb config validate`（配置校验）。

---

## 2. 状态机激活范围 (Delta)

### 2.1 Job 状态机扩展
- **CANCELLED**：新增终态。用户主动放弃任务，不再关注其结果。

### 2.2 Agent/Session 生命周期强化
- **SESSION 生命周期**：
    - **ACTIVE**：正常运行。
    - **RECONCILING**：正在进行一致性检查。
    - **CLOSED**：整个项目已停止运行。

---

## 3. R-* 需求切割矩阵 (Scope Definitions)

| Req ID | Description | MVP 1-8 状态 | MVP 9 更新状态 | 备注 |
|---|---|---|---|---|
| **R-LAUNCH-1** | 配置驱动批量启动 | ⚪ N/A | 🟢 **Full** | 支持 `ccb.toml` 格式，支持 `ccb start` |
| **R-LAYOUT-1** | Tmux 自动分屏逻辑 | 🟡 Partial | 🟢 **Full** | 从单一 Window 扩展为灵活的 Split Layout |
| **R-RECON-1** | 僵尸进程与孤儿资源回收 | ⚪ N/A | 🟢 **Full** | 解决 Daemon 重启后的状态不一致问题 |
| **R-JOB-2** | 任务取消与控制 | ⚪ N/A | 🟢 **Full** | `job.cancel` 接口及 SIGINT 注入 |
| **R-DIAG-1** | 诊断工具 (ccb doctor) | ⚪ N/A | 🟢 **Full** | 提供环境检查与自修复建议 |
| **R-CLI-3** | 1.0 CLI 语义完整性 | 🟡 Partial | 🟢 **Full** | 补齐 `logs`, `version`, `config` |

---

## 4. 范围分阶段（实施视角）

### G9.0：Project Launcher (配置与批量启动)
- 定义 `ccb.toml` 格式（兼容旧版 `ccb.config` 语义，改用 TOML）。
- 实现 CLI 端的配置解析与 `ccbd` 的批量任务分发。
- **Checkpoint**：`ccb start` 能一次性拉起多个配置好的 Agent。

### G9.1：Lifecycle & Reconcile (生命周期与自愈)
- 实现 `ccb kill` RPC，支持 Session 维度的级联删除。
- 在 Daemon `main` 函数增加 `reconcile_all_sessions` 逻辑。
- 强化 `evidence` 与资源清理的绑定，确保 sandbox 目录在 Agent 彻底结束后能被清理。
- **Checkpoint**：非法杀掉 Agent 进程后，Daemon 重起能自动识别。

### G9.2：Layout & Job Interactivity (布局与交互)
- 引入 `src/tmux/layout.rs`。实现经典的 "4-pane grid" 布局算法。
- 实现 `job.cancel`。在 PTY 中注入中断字符（`0x03`）。
- **Checkpoint**：启动 4 个 Agent 能在同一个 Tmux Window 里整齐排列。

### G9.3：Final Polish & Doctor (收尾与诊断)
- 实现 `ccb doctor`。
- 实现 `ccb logs` (基于 `events` 表的原始流回溯)。
- **Checkpoint**：所有测试用例全绿，`ccb --help` 展示完整命令集。

---

## 5. 跟前后 MVP 的接口约束

- **Config 文件**：不再使用 `.ccb/ccb.config` (JSON)，统一使用项目根目录下的 `ccb.toml` (TOML)。
- **RPC 变更**：
    - `job.cancel(job_id)` -> `void`
    - `session.kill(session_id, force)` -> `void`
    - `session.list()` -> `vec<SessionSummary>`
- **TMUX 命名空间**：一个 Session 对应一个 Tmux Window (Name = `ccb:<project_id>`)。

---

## 6. 核心架构决断 (Architectural Decisions / Open Questions)

### 决断 1：配置格式选择
**推荐选择：TOML**
- 理由：Rust 生态对 TOML 有极佳支持（serde_toml）。TOML 相比 JSON 更适合人类编辑，且比 YAML 语义更严谨。

### 决断 2：Tmux 布局策略
**推荐选择：固定 Grid + 自定义 Plan**
- 优先实现几种标准布局：`Single`, `Grid` (2x2), `Stacked` (Vertical list)。
- 不要在 MVP 9 尝试实现无限嵌套的分屏算法，保持简单。

### 决断 3：`ccb kill` 的彻底性
**推荐选择：双阶段清理 (Graceful -> Force)**
- `ccb kill` 首先尝试通过 RPC 正常关闭 Agent（发送停止信号）。
- `ccb kill -f` 直接通过 PID 强制杀掉进程并强行剥离 Tmux 窗口。

### 决断 4：孤儿状态的处理策略
**推荐选择：DB 为准 + 现场校验**
- Daemon 启动时以 SQLite `agents` 表中未结束的记录为索引。
- 对每个记录，通过 `kill(pid, 0)` 探测。若进程已死，立即触发状态机降级。

### 决断 5：`ccb logs` 的实现
**推荐选择：Event Sourcing 回溯**
- `ccb logs <agent>` 不是去读一个 `.log` 文件，而是调用 `agent.read(since_event_id=0)` 获取所有 `output_chunk` 并打印。这体现了 Event Sourcing 的架构优势：日志即数据。
