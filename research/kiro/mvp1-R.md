# Kiro Requirements: MVP 1 (I/O 骨架)

> **文档定位**：本文件是 ccbd-rust MVP 1 阶段的官方 R (Requirements) 规格。定义了首个「最小可工作版本」必须满足的需求范围、验收标准，以及严格禁止越界的防偏航红线。

---

## 1. 最小可工作验收标准 (Acceptance Criteria)

MVP 1 的核心是验证 UNIX Domain Socket 通信、SQLite 持久化与裸 PTY 的基础流转。当且仅当以下测试场景全部跑通，MVP 1 才算验收合格：

1. **环境隔离测试**：带 `CCB_ENV=dev` 启动 Daemon，验证 Socket 文件与 SQLite 数据库只生成在 `target/dev_state/` 目录下，不污染 `~/.local/state/ccbd/`。
2. **拓扑创建测试**：通过 UDS 发送 `session.create` 成功返回 `session_id`。发送 `agent.spawn`（Provider 设为 `bash`）成功拉起真实的 bash 进程（裸 PTY）。
3. **I/O 与幂等测试**：
   - 发送 `agent.send` 写入 `echo hello\n` 并携带 `request_id="req-1"`。
   - 再次发送带有 `request_id="req-1"` 的指令，断言数据库没有重复插入 event，进程没有重复执行。
4. **断点拉取测试**：调用 `agent.read` 携带 `since_event_id=0`，能拉取到包含 `hello` 字符串的 `output_chunk` 事件流。
5. **生命周期捕获测试**：在外部通过 `kill -9` 杀掉拉起的 bash 进程，随后调用 `agent.read` 能够在其事件流或状态反馈中观测到 `CRASHED`。

---

## 2. R-* 需求切割矩阵 (Scope Definitions)

针对 `01-boundaries.md` 中定义的全量 10 条需求，MVP 1 的切割策略如下：

### R-DISPATCH-1: Agent ID 引用稳定性
*   **状态**：🟡 **Partial (部分满足)**
*   **MVP1 语义**：L3 必须且仅能通过 `agent_id` 进行 `agent.send` 和 `agent.read` 调用。系统内部隐藏底层 OS PID 或 PTY FD。
*   **Carve-out (剔除项)**：MVP1 仅保证「单 Daemon 生命周期内的 Agent ID 稳定引用」。当 Daemon 重启时，旧的 active agent 会被显式强制流转为 `CRASHED` 状态。跨重启的无缝接管（即 L2 重启后通过扫描 PID 重建 PTY 句柄映射，使得 `agent_id` 依然活跃）推迟至后续 MVP 解决。

### R-DISPATCH-2: 显式投递失效通知
*   **状态**：🟡 **Partial (部分满足)**
*   **In-scope**：Daemon 必须能够捕获子进程的物理死亡（Exit/Signal），并在 SQLite 中将其状态流转为 `CRASHED`，同时在 `events` 表插入 `state_change` 记录。
*   **Carve-out (剔除项)**：MVP 1 不要求使用 `pidfd` (A-5) 实现亚毫秒级探测，允许使用标准的异步 `Child::wait` 捕获；不要求实现长连接的 `session.subscribe` 推送通知，L3 需通过 `agent.read` 轮询感知死亡。

### R-ISOLATION-1: 物理环境强制隔离
*   **状态**：🟡 **Partial (部分满足)**
*   **In-scope**：严格执行 XDG 路径规范与 `CCB_ENV=dev` 的路径路由逻辑。*（Rationale: 环境变量统一使用上游定义的 CCB_ENV=dev，而非 CCBD_ENV，以确保与既有生态测试脚本的兼容性。）*
*   **Carve-out (剔除项)**：不包含任何 `bwrap` 沙盒隔离逻辑（Deferred to MVP 2）。Agent 进程作为裸进程在宿主机的当前 User Namespace 运行。

### R-RECONCILE-1: 状态唯一事实来源
*   **状态**：🟡 **Partial (部分满足)**
*   **In-scope**：SQLite 的 `agents` 表是记录 Agent 状态的唯一事实来源。状态机必须正确记录 `IDLE` 和 `CRASHED`。
*   **Carve-out (剔除项)**：由于没有 `vt100` 解析器（Deferred to MVP 3），状态无法从 `BUSY` 自动恢复为 `IDLE`。MVP 1 允许 `agent.send` 绕过状态锁持续写入；不包含每 30 秒定期对比 `/proc` 的完整调谐循环。

### R-API-COMPAT-1: 协议破坏性变更约束
*   **状态**：🟢 **In-scope (全量满足)**
*   **MVP1 语义**：暴露的 RPC 接口（`session.create`, `agent.spawn`, `agent.send`, `agent.read`）必须严格遵循 S-3 决议的 JSON-RPC 2.0 封套标准，不允许随意增删必要字段。

### R-OBSERVABILITY-1: 状态全量可观测
*   **状态**：🔴 **Deferred (完全推迟)**
*   **说明**：MVP 1 专注于基础 I/O，`system.dump` RPC 接口不在本阶段开发范围内。开发阶段直接通过 sqlite3 CLI 观测 `target/dev_state/ccbd.sqlite`。

### R-RECONNECT-1: 零丢失断线重连
*   **状态**：🟢 **In-scope (全量满足)**
*   **MVP1 语义**：`agent.read` 必须严格按照 S-2 设计的 `seq_id` 机制实现 `since_event_id` 拉取。必须能准确返回 `output_chunk` 事件。

### R-IDEMPOTENCY-1: 选填投递幂等性
*   **状态**：🟢 **In-scope (全量满足)**
*   **MVP1 语义**：`agent.send` 必须接收可选的 `request_id`，并在写入 SQLite `events` 表时触发 UNIQUE 约束检测，实现严格的单次写入。

### R-ERROR-CODES-1: 结构化错误处理
*   **状态**：🟡 **Partial (部分满足)**
*   **In-scope**：错误返回必须包裹在 JSON-RPC 的 `code: -32000` 格式下，并包含明确的 `error_code`（如 `IPC_INVALID_REQUEST`, `AGENT_NOT_FOUND`）。内部模块使用 `thiserror`。
*   **Carve-out (剔除项)**：不实现与 VT100、沙盒相关的特定错误码（如 `PTY_MARKER_TIMEOUT`, `SANDBOX_BWRAP_NOT_FOUND`）。

### R-STATE-FALLBACK-LOOP: 状态识别异常闭环
*   **状态**：🔴 **Deferred (完全推迟)**
*   **说明**：不包含 `vt100`，不包含 `UNKNOWN` 状态，不操作 `evidence` 表，不实现 `agent.assert_state`。全量延迟至 MVP 4。

---

## 3. 严格禁止的越界行为 (Anti-goals / 防偏航)

为了防止过度工程导致 MVP 1 迟迟无法跑通，实施者在编写代码时**必须克制**，严禁触碰以下领域：

1. **禁止组装沙盒与越权包装**：在实现 `agent.spawn` 时，禁止组装 `bubblewrap`, `systemd`, `pidfd`；必须通过 `portable-pty` 拉起裸 PTY 进程，严禁使用普通 `std::process::Command` 导致丢失 tty 环境。
2. **禁止引入 `vt100` 解析逻辑**：PTY 的输出字节流直接被封装为 `output_chunk` 事件写入 DB。禁止维护任何 200x200 的内存屏幕，禁止引入匹配正则表达式。
3. **禁止开发 `evidence` 表相关的 DAO 逻辑**：哪怕看到 S-2 的 schema 中有定义，MVP 1 的 Rust 代码也不应生成任何对 `evidence` 表的插入逻辑。
4. **禁止实现 MVP 1 核心范围之外的 RPC**：如 `session.subscribe`, `system.dump`, `agent.kill`, `agent.assert_state` 等，一律留待后续迭代。
