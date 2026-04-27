# 04：MVP 增量路线图 (MVP Roadmap)

> **设计哲学**：遵循标准 Kiro 流程的增量实施原则。将 ccbd-rust 从全量规格（01/02/03）垂直切割为四个可独立验证的 MVP 阶段。每个阶段必须产出属于该阶段的 **R (Requirement) / D (Design) / T (Tasks)** 文档，严禁跨阶段提前实施。

## 1. MVP 阶段划分与里程碑

### MVP 1：I/O 骨架 (The Skeleton)
*   **目标**：确立 ccbd-rust 守护进程与主控（L3）之间的基本生命周期闭环。
*   **核心范围**：
    - **RPC**: `session.create`, `agent.spawn`, `agent.send`, `agent.read` (基础版)。
    - **状态机**: 简化为 `IDLE`, `BUSY`, `CRASHED`（不含 vt100 解析）。
    - **持久化**: SQLite 基础表结构（仅限 `projects`, `sessions`, `agents`, `events`）。
    - **I/O**: 裸 PTY 读写，不含沙盒。
*   **最小可工作定义**：Master 启动 Daemon，成功拉起一个交互式 `bash`，发送 `ls` 指令并能在 1 秒内通过 RPC 获取到输出内容。
*   **关联 Spec 引用**：S-2 (Schema 基础), S-3 (RPC 前 4 个方法)。

### MVP 2：隔离长城 (The Fortress)
*   **目标**：实施物理安全隔离与资源收割机制。
*   **核心范围**：
    - **沙盒**: `bwrap` 默认 baseline 参数集实现，挂载 XDG 路径。
    - **拓扑**: `systemd-run --scope` 包装，`pidfd_open` 监控 Master 与 Agent 死亡。
*   **最小可工作定义**：Agent 被拉起后，在沙盒内无法读取 `/etc/shadow` 或用户根目录；Kill Daemon 后，所有 Agent 被 Systemd 级联杀掉。
*   **关联 Spec 引用**：A-4 (Sandbox), A-6 (Topology), S-5 (Assembly)。

### MVP 3：语义感知 (The Retina)
*   **目标**：让 L2 具备「阅读」终端内容并识别 Marker 的能力。
*   **核心范围**：
    - **PTY 算法**: 引入 `vt100` 解析器，实施「底部 5 行优先」匹配算法。
    - **状态机**: 激活 `BUSY` 状态到 `IDLE(Matched)` 的确定性流转。
    - **计时器**: `MarkerTimer` 实时重置逻辑。
*   **最小可工作定义**：发送一个命令给 Gemini CLI，Daemon 能在 `✦` 出现瞬间自动将状态从 `BUSY` 切回 `IDLE`。
*   **关联 Spec 引用**：A-3 (VT100), S-1 (State Machine), S-4 (Algorithm)。

### MVP 4：反思闭环 (The Feedback)
*   **目标**：全量达成 R-STATE-FALLBACK-LOOP 闭环。
*   **核心范围**：
    - **异常处理**: `UNKNOWN` 状态逻辑分支全线贯通。
    - **持久化**: `evidence` 表激活，支持 PTY 快照 Dump。
    - **RPC**: `agent.assert_state`, `agent.discard_evidence`。
*   **最小可工作定义**：模拟一个 Marker 规则变更，Daemon 自动进入 `UNKNOWN` 状态，Master 收到通知并查询 Evidence 成功。
*   **关联 Spec 引用**：议题 1b (Feedback Loop), S-3 (逃生舱 API)。

---

## 2. 演进与对齐规则

### 2.1 Kiro 文档落盘模式
每个 MVP 阶段启动前，由 Master Claude 带领 Gemini 针对该 MVP 的 Scope，从既有的 01/02/03 全量 Spec 中**提取并固化**出一份当前阶段的 `R/D/T`：
- **`research/kiro/mvpX-R.md`**: 明确该阶段必须满足的需求编号（如 R-DISPATCH-1）。
- **`research/kiro/mvpX-D.md`**: 摘录该阶段涉及的 S-Spec 伪代码和 Schema 字段。
- **`research/kiro/mvpX-T.md`**: Codex 的原子执行任务书。

### 2.2 防偏航原则
- **禁止「就手写了」**：Codex 在实施 MVP 1 时，即使发现 S-5 的沙盒代码很好写，也**禁止**提前编写。必须保持代码库在每个里程碑的纯净度。
- **配置先行**：MVP 1 虽然没有全量 Schema，但必须一开始就按照 S-2 设计的字段命名，防止后期重构数据库导致的迁移成本。

---

## 3. 推荐顺序与当前位置

**当前位置**：MVP 0 (架构闭合)。
**下一步计划**：启动 **MVP 1 (I/O 骨架)** 的 Kiro R/D/T 编写。
