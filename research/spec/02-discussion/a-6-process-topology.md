# A-6 议题辩论：进程拓扑与级联清理

## Round 1 - Master Claude 立场

**决策提议**：采用 **单 Daemon + systemd-run transient unit + pidfd_open + 调谐循环** 的协同架构。放弃 Supervisor 子进程模型。

### 正方推理
1. **直接父子进程模型**：Agent 作为 Daemon 的直接子进程（Child），通过 `pidfd_open` 监控，退出时（SIGCHLD 或 EPOLLIN）触发亚毫秒级响应，零 IPC 开销。
2. **借力 Systemd 生态**：使用 `systemd-run --user --scope` 接管 Cgroup 资源隔离与收割，复用成熟基础设施。
3. **Master 崩溃亚毫秒回收**：L2 Daemon 利用 `pidfd_open(master_pid)` 监听调用方主控。主控 OOM 时，Daemon 立即执行级联清理。
4. **L2 崩溃双保险兜底**：通过 `BindsTo=ccbd-agents.slice`，当 Daemon 崩溃时 Systemd 会自动杀死所有关联 Agent。同时，Daemon 重启后的 `Startup Reconcile` 负责扫尾残留。

### 反方立场（Daemon + 多个 Supervisor）
- **隔离故障域**：Supervisor 崩溃不影响主 Daemon。
- **代价**：两层 Watchdog 嵌套复杂度爆炸；需要发明一套 Supervisor 与 Daemon 之间的 IPC 状态同步协议；NIH 重新发明 Cgroup 管理逻辑。

---

## Round 2 - Gemini 评估

### 1. 架构简化与 Systemd 绑定
- **决议**：全面倒向 **Linux First, Systemd Native**。对于 macOS 等非 Systemd 环境，采取功能降级（Degraded Mode），仅依赖 `Startup Reconcile` 兜底，放弃 Cgroup 资源配额限制。

### 2. --scope 与生命周期绑定
- **决议**：坚决使用 `systemd-run --scope`。
- **理由**：这保持了进程树上的直接父子关系，Daemon 才能对 Agent 调用 `waitpid` 或捕获 `SIGCHLD`。这是获取准确退出码（Exit Status）和毫秒级响应的基础。

### 3. 旧内核 Fallback 机制
- **决议**：对不支持 `pidfd_open` 的环境，使用 `tokio::signal::unix::SignalKind::child()` 配合轮询 `try_wait` 捕获死亡。

### 4. 主从绑定模型 (Master Session)
- **决议**：**禁止 Master 所有权转移**。Agent 的生命周期与 `master_pid` 强绑定。若切换主控，必须执行 Kill & Respawn，并通过 `since_event_id` 和复用沙盒目录恢复状态，保持 L2 逻辑极简。

### 5. Cgroup 默认资源基线
- **决议**：提供 Per-Provider 配置项，基础安全基线为：`MemoryMax=2G`，`TasksMax=1024`（防止 Node 进程树耗尽 PID），`CPUWeight=normal`。

---

## 最终决议

**决策结果**：采用 **单 Daemon + Systemd `--scope` 协同拓扑**，拒绝引入 Supervisor 中间层。

- **核心机制**：
  1. Daemon 直接 fork 进程，外挂 `systemd-run --scope` 获取隔离环境。
  2. Master 监控基于 `pidfd_open(master_pid)` 触发级联 SIGTERM。
  3. L2 崩溃保护基于 `BindsTo=ccbd-agents.slice` + `Startup Reconcile` 双保险对账。
  4. Master 所有权不可转移，保证状态机单向流通。

**决议日期**：2026-04-26
**达成方式**：Master Claude 提议，Gemini 补强（macOS 降级声明、TasksMax 安全线、拒绝转移）并确认。
