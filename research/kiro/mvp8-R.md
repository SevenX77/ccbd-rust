# Kiro Requirements: MVP 8 (任务编排与信箱内核 / The Mailbox & Orchestration Pivot)

> **文档定位**：本文件是 ccbd-rust 迈入 L3（编排层）门槛的标志性阶段（MVP 8）的官方 R (Requirements) 规格。本阶段将系统从"只懂读写和正则的 L2 监工"升级为"拥有任务队列和完成通知的 L3 大脑"，接管旧 Python ccb 的 Mailbox 核心逻辑。

---

## 0. 立项背景与边界共识

### 0.1 为什么必须做这个 MVP（核心驱动）
MVP1-7 打造了工业级的 L2 调度底座（安全隔离、TUI 兼容、现场保真、多行无损）。但从用户的宏观视角看，系统仍然无法使用，因为目前只能发送最原始的 `agent.send`（且必须自己处理并发失败和等待轮询）。

旧 Python ccb 之所以好用，是因为它提供了高维度的用户 CLI 接口：
- `ccb ask <agent> <msg>`：提交任务，排队执行，拿到 `job_id` 即可撒手。
- `ccb pend <job_id>`：阻塞等待并获取 Agent 最终回答。
- `ccb watch <agent>`：像流媒体一样实时观看背后 Agent 的思考过程。

用户下达了明确指令："替代现在的 ccb"。**如果缺少异步队列管理和 Job 状态流转机制，ccbd-rust 就只能是个半成品 Daemon。** MVP8 的使命就是在 Rust 内部长出 L3 调度大脑，承载旧版本 `lib/mailbox_kernel/` 的职责，彻底淘汰旧版的队列管理系统。

### 0.2 本 MVP **不做**的事（留给 MVP9）
- **Launcher / 一键起全家桶**：`ccb start` （自动根据 TOML 唤起多 Agent、切分布局 4-pane 等）仍推迟。本阶段测试时依然假定 Agent 已通过 `ccbd` 被手动 Spawn。
- **多项目 Reconcile 管控**：全局扫描所有沙盒清理、项目维度的隔离暂不在本期深化，只做 Agent/Job 粒度的生命周期。
- **高级 Job 控制**：Job 取消（Cancel）、优先级插队（Priority）等，不在本 MVP 处理（仅做 FIFO 顺序投递）。

### 0.3 与上下游 MVP 的关系
- **承上（MVP7）**：强依赖 MVP7 对 IDLE 状态精准的识别能力。只有准确判定了 IDLE，Job Queue 才敢将排队中的下一个任务 `DISPATCH` 给 Agent。
- **启下（MVP9）**：MVP8 完成后，核心操作语义（L1-L3）全部闭环。MVP9 只需补齐周边辅助脚本，即可宣布 ccbd-rust 1.0 发布。

---

## 1. 最小可工作验收标准 (Acceptance Criteria)

MVP 8 验收必须全部通过：

1. **AC1 [Job 提交与排队]**：新增 `ccb ask <agent_id> "<message>"` 命令及对应的 RPC `job.submit`。Daemon 接收后，立刻返回持久化的 `job_id`，且不阻塞主 Tokio 事件循环。当目标 Agent 处于非 IDLE 状态时，Job 进入 `QUEUED` 状态。
2. **AC2 [顺序调度 / Serial-Per-Agent]**：实现基于 SQLite 锁或 Tokio 锁的单 Agent 串行调度器。当 Agent 转移至 IDLE 状态时，调度器自动从队列中拉取最早的一个 `QUEUED` 任务进行投递（转为 `DISPATCHED` 状态），彻底消灭多主控并发调用的 Race Condition。
3. **AC3 [结果等待与回复]**：新增 `ccb pend <job_id>` 命令及对应的 RPC `job.wait`。主控可以阻塞调用此接口（采用长轮询或 Server-Push 通知），直到该任务执行完毕（Agent 再次回到 IDLE），并完整返回该阶段 Agent 打印的所有新内容（The Reply）。
4. **AC4 [流式观测 / Watch]**：新增 `ccb watch <agent_id>` 命令，利用 Server-Push (SSE / Stream / 长轮询拉取) 持续阻塞输出目标 Agent 的 `output_chunk`，并在 Agent 状态发生变更时输出标记，做到旧版 ccb watch 的同等体验。
5. **AC5 [持久化恢复]**：Daemon 意外崩溃重启后，尚未执行（QUEUED）的 Job 必须保留在队列中，等待 Agent 重新启动并恢复至 IDLE 后继续执行。
6. **AC6 [真实现场全绿]**：在 playground 真实环境中，手动 `spawn` 真实 Codex，然后在另一终端执行 `ccb ask a1 "write a python hello world" && ccb pend <job_id>`，能够完美闭环拿到代码。

---

## 2. 状态机激活范围 (Delta)

本次新增 **Job 状态机（二阶状态机）**，它挂载于核心 Agent 状态机之上。
- **QUEUED**：任务已接纳，等待调度器取件。
- **DISPATCHED**：任务已被送入 PTY（底层调用 `agent.send`），此时 Agent 状态通常变为 BUSY。
- **COMPLETED**：Agent 完成该任务的输出并回归 IDLE，结果已归档。
- **FAILED**：发送失败，或 Agent 在任务执行中转为了 UNKNOWN / CRASHED 状态。

---

## 3. R-* 需求切割矩阵更新 (Scope Definitions)

| Req ID | Description | MVP 1-7 状态 | MVP 8 更新状态 | 备注 |
|---|---|---|---|---|
| **R-MAILBOX-1** | 异步信箱排队机制 | ⚪ N/A | 🟢 **Full** | 新增：实现 Serial-per-agent 队列调度 |
| **R-CLI-2** | L3 编排级命令行 | 🟡 Partial | 🟢 **Full** | 从仅有 `ping/ps` 扩展出 `ask/pend/watch` 业务命令 |
| **R-RPC-STREAM-1** | 实时输出流协议 | ⚪ N/A | 🟢 **Full** | 解决 `watch` 轮询耗能，建立有效的长连接返回通道 |
| R-STATE-* | Agent 核心状态机 | 🟢 Full | 🟢 Full | 保持不动，作为调度信号源 |

---

## 4. 范围分阶段（实施视角）

为管控复杂度，实施分为三个递进的物理阶段。

### G8.0：Database Extension (数据模型扩充)
- 创建 `jobs` 表用于持久化 Mailbox。
- 扩充 `rpc/handlers.rs` 支持 `job.submit`（非阻塞落库）。
- **安全检查点**：`ccb ask` 能执行并返回 `job_id`，数据库中能看到排队记录。

### G8.1：The Orchestrator Loop (大脑运转)
- 创建 `src/orchestrator/mod.rs`（或类似命名），引入常驻内存的调度循环 / 事件触发器。
- 当 `agent` 变更为 `IDLE` 时，调度器取出最老的 `QUEUED` 任务执行 `agent.send`。
- 处理 Agent CRASHED 时 Job 转 `FAILED` 的清理逻辑。
- **安全检查点**：提交多个 `ccb ask`，能观察到 Agent 空闲时自动按顺序被投递（Serial-per-agent）。

### G8.2：Synchronous Feedback (观测闭环)
- 实现 `job.wait` (长轮询或基于 Tokio `watch` / `Notify` 唤醒机制)。
- 实现 `ccb pend` 与 `ccb watch`。
- **安全检查点**：`ccb watch` 启动后无延迟看到输出，`ccb pend` 能精确剥离出该 Job 产生的 stdout Reply。

---

## 5. 跟前后 MVP 的接口约束

- **JSON-RPC schema**：绝不破坏原有协议。在原有的 `agent.*` 命名空间外，新增 `job.*` (如 `job.submit`, `job.wait`) 和 `agent.watch` 协议。
- **SQLite 范式**：原 `events`, `agents`, `sessions`, `evidence` 表结构不变。新增 `jobs` 表，通过外键 `agent_id` 关联。
- **事务边界**：MVP5 / MVP6 打造的 DB Async Wrapper 边界及 Tmux 集成方式不容任何破坏。调度器只能使用已提供的底层安全函数（如 `send_text_to_pane`）。

---

## 6. 核心架构决断 (Architectural Decisions / Open Questions)

此部分直接定调，消除架构实施阶段的不确定性。

### 决断 1：Job 状态机流转设计
**推荐设计：5 态 (QUEUED → DISPATCHED → COMPLETED / FAILED / CANCELLED)**
- 不增加 `TIMEOUT` 状态。在 ccbd-rust 的设计哲学里，超时（Marker Timeout）是 Agent 的核心状态（转为 `UNKNOWN`）。当 Agent 进入 `UNKNOWN` 状态时，调度器会捕获此事件，并将当前正在 `DISPATCHED` 的 Job 标记为 `FAILED`（并在 Payload 附带原因）。这是最清晰的职责解耦：底层管连接韧性，调度层管任务归属。

### 决断 2：Mailbox 持久化方案 (Table Design)
**推荐设计：新建独立的 `jobs` 表。**
- 不能复用 `events` 表。`events` 表是 Immutable Append-Only 日志流水（Event Sourcing）。
- Job 是一种 Entity，拥有生命周期（Status Update），且需要存储 `job_id`、`agent_id`、`prompt`、`reply_text`、`submitted_at` 等维度信息，混用会破坏 schema。

### 决断 3：job_id 生成策略
**推荐设计：UUID v4 (带有前缀如 `job_`)**
- 避免使用 Sequential ID 导致分布式或多主控发号竞争时序泄露。UUID 是标准、去中心化且兼容当前 `sess_` 前缀范式的选择。

### 决断 4：`ccb ask` 的执行语义
**推荐设计：异步（Async），立即返回 `job_id`。**
- CLI 命令 `ccb ask` 作为投递者，只需返回 `job_id` 然后 `exit(0)`。如果用户需要等待结果，必须由另一个工具或参数支撑：设计为 `ccb ask <agent> <msg> --wait`（内部其实是按序调了 `job.submit` 再立即调 `job.wait`），与旧版本习惯保持完全一致。

### 决断 5：多主控串行队列的并发安全 (Queue Policy)
**推荐设计：利用 SQLite 行级锁 (`SELECT ... FOR UPDATE` 替代机制) 结合单点 Orchestrator**
- 鉴于 `ccbd` 是单实例 Daemon，最佳做法是在 Daemon 内部启动一个单一的轻量级 Async Task (如 `spawn_orchestrator_loop`)。
- 当接收到任务，或者底层 `mark_agent_idle_matched` 执行完毕时，发送一个信号（`tokio::sync::mpsc` 或者 `Notify`）通知 Orchestrator。
- Orchestrator 在一个串行的执行线程里，通过 SQLite CAS 获取队头，进行 `agent.send` 投递，杜绝数据竞态。不要用粗粒度的 Tokio Mutex 阻塞并发进入的 RPC 请求。

### 决断 6：Job_completed 的主控通知机制
**推荐设计：JSON-RPC 接口内的 Long-Polling 降维打击。**
- 在 Daemon 内部维护一个基于 `tokio::sync::broadcast` 或 `Notify` 的内存订阅中心。
- RPC Handler `job.wait` 被调用时，Handler 将自身挂起（`.await` 等待 channel）。
- 当 Agent 状态跳变导致 Job 转为 `COMPLETED`，Orchestrator 广播该事件。
- Handler 被唤醒，去数据库查询 Reply 内容，最后作为标准 JSON-RPC Response 返回。
- **理由**：这避免了引入复杂的 WebSocket (SSE) 协议解析，保持 JSON-RPC UDS 在工具链上的极简与穿透性。同理，`ccb watch` 也可以通过类似方式实现。
