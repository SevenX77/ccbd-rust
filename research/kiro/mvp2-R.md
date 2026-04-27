# Kiro Requirements: MVP 2 (隔离长城)

> **文档定位**：本文件是 ccbd-rust MVP 2 阶段的官方 R (Requirements) 规格。本阶段核心目标是建立 Agent 进程的物理安全边界与级联清理机制，确保 L2 调度层具备工业级的进程隔离能力。

---

## 1. 最小可工作验收标准 (Acceptance Criteria)

MVP 2 的核心是验证 Bubblewrap 沙盒的隔离有效性、Systemd 拓扑的级联绑定可靠性以及基于 pidfd 的实时生命周期捕获。当且仅当以下测试场景全部跑通，MVP 2 才算验收合格：

1. **沙盒权限边界测试**：在沙盒内执行 `cat /etc/shadow` 必须**失败**（baseline 不挂 `/etc`，所以失败语义是 ENOENT「No such file or directory」；如果未来 baseline 改为挂 `/etc/resolv.conf` 进入更细粒度的 `/etc` ro-bind，则失败语义变 EACCES「Permission denied」——验收只断言 exit_code 非零 + stderr 含失败提示，不死磕具体 errno 文案）。执行 `ls $HOME/.ssh/` 必须返回 No such file or directory（被 `--dir /home/agent` 替换为虚拟空 HOME）。沙盒外的真实宿主路径既看不见也读不到。
2. **Master 死亡级联测试**：`session.create` 时记录 `master_pid`，Daemon 注册 `pidfd_open(master_pid)`。手动 `kill -9 <master_pid>` 模拟主控崩溃，Daemon 必须在 2 秒内捕获事件并向其管理的所有 Agent 下发 SIGKILL，DB 中关联 Agent 状态全部转 `KILLED` reason=`MASTER_DEATH`（pidfd 唤醒本身亚毫秒，2s 是给 cascade SQL + SIGKILL syscall + tokio 调度链的工程裕度）。
3. **Daemon 死亡级联测试**（分两条子验收）：
   - **AC3a 生产路径**（systemd 托管下的真级联）：在 systemd user service 模式下启动 Daemon（`systemctl --user start ccbd-rust.service`），手动 `kill -9 <ccbd_pid>`，依托 `BindsTo=ccbd-rust.service` + `ccbd-agents.slice` 验证 Agent 进程树被 systemd 自动回收。本条**仅在 CI 的 systemd 集成 job 跑**，不在常规 cargo test 矩阵。
   - **AC3b 开发降级路径**（cargo run 下的 Reconcile 兜底）：直接 `cargo run` 起 Daemon，手动 `kill -9 <ccbd_pid>`。BindsTo 不会触发（Daemon 不在 systemd unit 下），Agent 会变成孤儿进程；下一次启动 Daemon 时通过 `Startup Reconcile` 扫描 active agents 并标 `CRASHED`（reason=`STARTUP_RECONCILE`），保证 SoT 一致。本条在常规集成测试里跑。
4. **Agent 死亡即时捕获测试**：`agent.spawn` 拉起 Agent 后，从沙盒外 `kill -9 <agent_pid>` 杀进程。Daemon 必须通过 `pidfd` 触发亚毫秒级响应，将状态流转为 `CRASHED` 并在 `events` 表插入 `state_change` 记录，`agents.error_code = AGENT_UNEXPECTED_EXIT`。
5. **主动终止测试**：调用新 RPC `agent.kill {agent_id}`，验证 Agent 物理进程被 SIGKILL 杀灭、`agents.state` 转 `KILLED`、`events` 表写入 `state_change` 含 `from→KILLED`。
6. **环境旁路测试**：设置 `CCBD_UNSAFE_NO_SANDBOX=1` 启动，验证系统能跳过 bwrap 直接拉起进程（仅供 CI 调试场景），同时 stderr 输出醒目 WARN 提示当前处于不安全模式。
7. **沙盒缺失硬拒测试**：在未安装 `bubblewrap` 的环境下（用 `PATH=/nonexistent/bin` 模拟）启动 Daemon 且未设 bypass，验证 Daemon 启动期 `check_environment` 直接 fail-closed 退出（`exit code != 0` + stderr 含 `SANDBOX_BWRAP_NOT_FOUND`），UDS 监听器**不**启动，严禁静默回退到无沙盒模式。

---

## 2. 状态机激活范围 (State Machine Scope Delta)

MVP 1 已激活 `IDLE / BUSY / CRASHED`。MVP 2 在此基础上**激活 `KILLED`** 状态——它由 `rpc.kill`（来自 `agent.kill` RPC）和 `master.death`（来自 `pidfd_open(master_pid)` 唤醒后的级联清理）两条路径转入，与 `CRASHED` 同为终态但语义不同（`KILLED` 是预期内的主动终止，`CRASHED` 是非预期的自毁）。`SPAWNING` 与 `UNKNOWN` 因仍依赖 vt100 解析能力，**继续 Deferred 至 MVP 3 / MVP 4**。

---

## 3. R-* 需求切割矩阵 (Scope Definitions)

针对 `01-boundaries.md` 中定义的 10 条需求，MVP 2 的演进切割策略如下：

### R-DISPATCH-1: Agent ID 引用稳定性
*   **状态**：🟡 **Partial (部分满足)**
*   **MVP2 语义**：维持 MVP 1 语义。新增 `pidfd` 绑定后，防止了 PID 回绕导致的 ID 误指向，单次生命周期内的稳定性进一步增强。
*   **Carve-out**：跨 Daemon 重启的「无缝接管」（reconnect-to-existing-PID）仍 Deferred，重启后所有 active Agent 仍按 MVP 1 的 Startup Reconcile 强制流转 `CRASHED`。

### R-DISPATCH-2: 显式投递失效通知
*   **状态**：🟢 **In-scope (全量满足)**
*   **MVP2 语义**：用 `pidfd_open` (Linux 5.3+) 取代 MVP 1 的 `Child::wait`。无论是 Agent 自毁、被 OOM Killer 杀掉、还是因 Master 死亡触发的级联清理，Daemon 都能在内核级实时感知，并通过 `events` 表的 `state_change` 让 Caller 通过 `agent.read` 轮询感知，确保事件流中无「状态盲区」。

### R-ISOLATION-1: 物理环境强制隔离
*   **状态**：🟢 **In-scope (全量满足)**
*   **MVP2 语义**：正式激活 `bwrap` 沙盒。Agent 拥有独立的 Mount/User/PID/UTS Namespace，文件系统访问严格收敛到 `/workspace`（外部 Git Repo 只读绑定）和 `/home/agent`（虚拟 HOME），`--unshare-net` 默认拒绝出站网络（除 Provider Profile 通过 `sandbox_overrides` 显式打开）。

### R-RECONCILE-1: 状态唯一事实来源
*   **状态**：🟡 **Partial (部分满足)**
*   **In-scope**：基于 `pidfd` 的事件驱动同步确保 DB 与 OS 进程状态在大多数场景下高频一致。
*   **Carve-out**：每 30 秒一次的幂等全量轮询对账（用于处理 epoll 事件丢失等极端场景）推迟至后续 MVP；`inotify` 监控沙盒目录完整性也推迟。

### R-API-COMPAT-1: 协议破坏性变更约束
*   **状态**：🟢 **In-scope (全量满足)**
*   **MVP2 语义**：`agent.spawn` 新增**可选**参数 `sandbox_overrides`（默认空），新增 `agent.kill` 方法。MVP 1 既有的 4 个 RPC 字段保持向后兼容，无任何 breaking change。

### R-OBSERVABILITY-1: 状态全量可观测
*   **状态**：🔴 **Deferred (完全推迟)**
*   **说明**：仍专注于隔离与拓扑，`system.dump` 接口维持推迟。开发期通过 sqlite3 CLI + `journalctl --user -u ccbd-rust.service` 观测。

### R-RECONNECT-1: 零丢失断线重连
*   **状态**：🟢 **In-scope (全量满足)**
*   **MVP2 语义**：基于 MVP 1 已落地的 `seq_id` 机制。新增的 `state_change → KILLED` 事件同样按 `since_event_id` 顺序拉取。

### R-IDEMPOTENCY-1: 选填投递幂等性
*   **状态**：🟢 **In-scope (全量满足)**
*   **MVP2 语义**：维持 MVP 1 已实现的 UNIQUE 约束机制，`agent.kill` 不引入新的幂等性要求（同一 agent 的重复 kill 视为无副作用，但需返回 `AGENT_NOT_FOUND` 给已 KILLED/CRASHED 的目标）。

### R-ERROR-CODES-1: 结构化错误处理
*   **状态**：🟡 **Partial (部分满足)**
*   **In-scope**：实现 `SANDBOX_BWRAP_NOT_FOUND` / `SANDBOX_USER_NS_DISABLED` / `SANDBOX_MOUNT_FAILED` 三个 SANDBOX 系列错误码、`AGENT_UNEXPECTED_EXIT`（pidfd 触发的非预期退出）、以及 `ENVIRONMENT_NOT_SUPPORTED`（systemd-run 缺失或非 Linux 平台启动期硬拒，独立于 SANDBOX_* 系列以保留语义清晰度）。
*   **Carve-out**：`PTY_MARKER_TIMEOUT` / `AGENT_SPAWN_TIMEOUT` 等 VT100 相关错误码继续 Deferred 至 MVP 3。

### R-STATE-FALLBACK-LOOP: 状态识别异常闭环
*   **状态**：🔴 **Deferred (完全推迟)**
*   **说明**：不包含 `vt100`、不包含 `UNKNOWN` 状态、不操作 `evidence` 表、不实现 `agent.assert_state` / `agent.discard_evidence`。全量延迟至 MVP 4。

---

## 4. 严格禁止的越界行为 (Anti-goals / 防偏航)

为了防止过度工程导致 MVP 2 迟迟无法跑通，实施者必须**克制**，严禁触碰以下领域：

1. **禁止引入 `vt100` 解析**：本阶段依然不允许对 PTY 字节流做任何内容解析，即使是为了「探测启动成功」。`SPAWNING` 状态保持 Deferred，新拉起的 Agent 默认直接落 `IDLE`（继承 MVP 1 行为）。
2. **禁止开发 `evidence` 表 DAO 与 `UNKNOWN` 状态**：哪怕看到 S-2 的 schema 中有 `evidence` 表定义、S-1 状态机有 `UNKNOWN` 节点，MVP 2 的 Rust 代码不应生成任何对 `evidence` 表的写入逻辑或 `UNKNOWN` 状态枚举。
3. **禁止实现 Provider Profile 自动加载**：沙盒参数组装采用硬编码 baseline + `agent.spawn` 入参 `sandbox_overrides`。**禁止**开发 `.ccb/providers/*.toml` 的 `[sandbox]` 节解析引擎，profile 自动加载推迟至后续 MVP。
4. **禁止实现 `session.subscribe` 长连接推送**：虽然 `pidfd` 提供了实时通知能力，本阶段仍要求 L3 通过 `agent.read` 轮询 `state_change` 事件感知 Agent 死亡。Server-Push Notification 全量推迟。
5. **禁止跨平台兼容代码**：MVP 2 强依赖 Linux 5.3+ 的 `pidfd_open` 与 systemd `--user --scope`。在 macOS / 非 systemd 环境下，允许 `cargo build` 编译通过但运行时直接 panic 退出并提示 "MVP2 requires Linux + systemd"。`HealthProvider` trait 抽象与 macOS polling 降级路径推迟。
6. **禁止在 `agents` 表持久化 sandbox_path**：沙盒目录 `~/.local/state/ccbd/sandboxes/<agent_id>/` 必须由 XDG 解算函数动态构造，禁止在 schema 中新增 `sandbox_path` 字段。
7. **禁止实现 `inotify` 监控**：A-5 决议中提到的「沙盒根目录被外部删除时强杀 Agent」推迟至后续 MVP，本阶段仅依赖 `pidfd` 探测进程死亡。
