# ccbd-rust DESIGN.md v2 设计框架（by Gemini）

> 本文件由 master Claude 在 2026-04-26 调用 Gemini（a2）作为领域分析师产出。Gemini 在 yolo 模式下被边界限制为「只输出文本不写文件」，因此原文输出仅在 a2 tmux pane 的 scrollback 里，本文件由 master Claude 从 pane 内容提取、清理 ASCII 表格 / 缩进 / 装饰线后整合而成。
>
> 调用上下文：调研阶段（Round 1-6）刚收尾，Gemini 在前一轮已完成 by-gemini-deep-v2.md 深度分析（A-G 7 章 + D1-D7 七决策 + verdict 可定稿）；本轮请她在那份分析基础上输出一份 v2 文档骨架，让 master Claude 和 Codex 拿着这个骨架直接下笔。
>
> 输出经过三轮：第一轮 5 章被 token cap 截断在第 5 章 IPC 末尾，第二轮补完 + 追加 6/7/8 章再被截在第 8 章末尾，第三轮补完末句 + 给出 verdict。Gemini 的 verdict：**框架到此结束，8 章已覆盖全部**。

---

## 引言

这是一份为 ccbd-rust 项目量身定制的 DESIGN.md v2 设计框架。该框架基于前 6 轮调研的全部事实语料与候选项目源码分析，严格按照 L2 调度层（全局唯一守护进程）的职责边界进行编排。

本框架的结构被设计为「面向 Rust 工程师的蓝图」，确保一个不熟悉前期 Python 版 ccb 历史包袱的工程师，可以直接根据此框架划分模块、引入依赖并编写代码。

---

## 1. 系统定位与核心概念定义 (System Positioning & Definitions)

本章节向新开发者解释系统的物理边界和核心领域词汇。

- **L2 调度层 (ccbd-rust)**：本项目的物理实体。一个全局唯一运行在后端的 Rust 守护进程（Daemon），负责管理多个不同项目目录下的 LLM Agent 子进程的生命周期与输入输出，不包含任何大语言模型（LLM）的 Prompt 构建或对话逻辑。
- **L1 执行层 (Agent CLI)**：例如 Codex CLI、Gemini CLI、Claude Code。它们是 ccbd-rust 通过子进程拉起的第三方二进制程序，运行在受限的沙盒（Bubblewrap Sandbox）中。
- **Reconciliation Loop (调谐循环)**：ccbd-rust 的核心自我修复机制。它是一个周期性运行的 Rust 异步任务，负责对比「SQLite 数据库中记录的期望状态」与「操作系统进程树 / 文件系统中的实际物理状态」，发现不一致（例如进程意外死亡）时立刻执行清理或重启。

---

## 2. 核心模块边界与代码来源矩阵 (Module Boundaries & Sourcing)

本章节明确哪些代码直接复用开源资产，哪些重新写，防止过度工程。

| 逻辑模块名称 | 对应 Rust 源码路径 | 决策类型 | 来源参考与实现说明 |
|---|---|---|---|
| 伪终端与沙盒封装 (PTY & Sandbox) | `src/pty/pty_session.rs` + `src/sandbox/bwrap.rs` | 直接 Fork | 直接引用 `tamux/crates/amux-daemon/src/pty_session.rs`（提供基于 portable-pty 的跨平台抽象）以及 `tamux/crates/amux-daemon/src/sandbox.rs`（提供对 Linux Bubblewrap 命名空间隔离的封装）。 |
| 事件溯源数据库 (Event Store) | `src/db/events.rs` | 借思路重写 | 借鉴 `overstory/src/mail/` 目录下的 SQLite Mailbox 事件溯源设计思路，但用 Rust 的 rusqlite 库重新实现。用于记录每个 Agent 吐出的 output_chunk 和 completion 事件流。 |
| 健康监测轮询 (Health Polling) | `src/lifecycle/health.rs` | 借思路重写 | 借鉴 `batty/src/team/daemon/health/poll_shim.rs` 中的 Ping/Pong 与探活模型思路，用 Rust 的 `tokio::time::interval` 结合进程 `/proc` 状态检查重新实现。 |
| 权威状态结构 (SoT Schema) | `src/db/schema.rs` | 全自研 | 因为 tamux 的表结构过度绑定其自身的插件生态，我们必须从零设计极简的 SQLite 结构，仅包含 `projects`、`sessions`、`agents`、`events` 四张核心表，强制开启 WAL 模式以支持高并发读写。 |

---

## 3. 关键风险与解决方案架构 (Key Risks & Solutions Architecture)

本章节显式接住前置调研中确定的 A/B/C 三类风险，并给出物理级别的解决路径。

### A 类风险（必解）：投递可靠性与 ACK 机制

- **痛点背景**：旧版 CCB 依赖 `tmux paste-buffer -p` 结合硬编码的 sleep 进行盲投（Fire-and-forget）。这导致了 Bug X（斜杠命令被当作普通文本粘贴而不被执行）和 Bug Y（丢失完成状态）。
- **决策路径**：放弃 Hook 回调，选择「vt100 Activity Markers 解析」结合「纯 Keystroke 模拟」。
  - **为什么不选 Hook**：根据 Bug Y 报告，Gemini 等 CLI 的 Hook 触发存在 4 秒以上的物理延迟，且容易与内部状态锁产生竞态。Hook 是应用层补充，不能作为基础 I/O 的确认协议。
  - **实现模块**：`src/pty/stream_parser.rs`。
  - **处理思路**：
    1. ccbd-rust 废弃 tmux paste，向 PTY 的标准输入（stdin）逐字节写入命令（Keystroke 模式），确保 `/clear` 等特殊前缀能触发 Agent CLI 原生的快捷键解析机制。
    2. ccbd-rust 持有 PTY 的标准输出（stdout）读取端，使用 vt100 库在内存中维护一个虚拟终端屏幕。
    3. 持续解析屏幕底部的活动标记（Activity Markers，如 `Thinking...` 或 `✦`），只有当终端内容出现明确的「空闲输入提示符（Prompt Ready Marker）」时，才向 SQLite 的事件表写入一条 `DeliveryAck` 记录，完成闭环。

### B 类风险（应解）：启动竞态与主控意外退出的资源回收

- **痛点背景**：Agent CLI 进程拉起后，到其完全初始化并能处理输入之间，存在一段响应真空期（Settle Window）。此外，如果 Master Claude（主控进程）被意外 SIGKILL，其衍生的 Agent 子进程会变成僵尸节点。
- **实现模块**：`src/lifecycle/startup.rs` 与 `src/reconcile/janitor.rs`。
- **处理思路**：
  1. **启动竞态消除**：在 `agents` 表中引入特定提供商（Provider）的参数化 `settle_window_ms`。子进程 spawn 成功后状态记为 `spawning`，仅当 vt100 解析器首次捕获到该 CLI 的欢迎横幅（Welcome Banner）后，状态才流转为 `running`，此时方可接收任务。
  2. **Master 崩溃回收**：L3 编排层在调用 `session.create` RPC 接口时，必须传入 `master_pid`。ccbd-rust 的调谐循环（Reconciliation Loop）会利用 `pidfd_open`（或轮询 `/proc/<master_pid>`）监控该主控进程的存活状态。一旦主控消失，调谐循环立即级联发送 SIGKILL 给关联的所有 Agent 子进程，并清理沙盒目录。

### C 类风险（监控）：Bubblewrap 沙盒的 I/O 性能损耗

- **痛点背景**：Bubblewrap (bwrap) 需要进行大量的 bind-mount（将真实目录只读映射到虚拟文件系统中），在高频拉起 Agent 时可能会加剧磁盘 I/O 负担，导致启动延迟。
- **处理思路**：在 v2 架构中不改变其硬隔离的实现策略，但在 `src/sandbox/bwrap.rs` 中埋点记录挂载执行的耗时（毫秒级）。通过 RPC 接口的 `system.health` 暴露沙盒启动的 P95 延迟指标，供后续 Phase 3 阶段评估是否需要引入常驻沙盒池（Sandbox Pool）。

---

## 4. 状态机与生命周期流转 (State Machine & Lifecycle)

本章节定义 Agent 进程在 ccbd-rust 内部的生命周期，以及 SQLite 是如何作为唯一事实来源（Source of Truth）的。

- **状态枚举定义**：
  - `Spawning`：进程已由 OS 拉起，正在等待 vt100 捕获初始化完成标记。
  - `Running`：就绪状态，可接受 stdin 输入。
  - `Busy`：已接受输入，vt100 正在持续捕获 Thinking 或生成输出的标记。
  - `Stuck`：超过 5 分钟没有任何标准输出流量变更（触发异常事件推送）。
  - `Crashed`：进程退出码非 0，或调谐循环发现 `/proc` 中进程已消失但 SQLite 记录为存活。
- **对账法则 (Reconciliation Rule)**：文件系统的增删改（例如为 Agent 创建专属历史记录目录）必须发生在 SQLite 事务 BEGIN 与 COMMIT 之间。如果文件操作失败，事务回滚；如果进程实际死掉但数据库未更新，依赖下一次调谐循环自动修正。对比现有 Python 版将状态分散在 4 个不同维度的乱象，Rust 版严格遵循「数据库先写日志（WAL）再执行物理操作」的纪律。

---

## 5. 跨进程通信与 JSON-RPC 契约 (IPC & Protocol Contracts)

本章节规定主控（L3）如何与 ccbd-rust（L2）通信，替代掉以前不可靠的命令行直接调用。

- **通信媒介**：单一 Unix Domain Socket (UDS)，默认路径为 `~/.local/state/ccbd/ccbd.sock`。杜绝不同项目路径下启动多个隔离 Daemon 导致的上下文串台。
- **核心协议**：基于换行符分隔的 JSON-RPC 2.0 协议（Newline-delimited JSON-RPC）。
- **关键数据流示例**：
  1. **发送指令 (Push)**：L3 发起 `{"method": "agent.send", "params": {"agent_id": "a2", "text": "/clear\n"}}`。ccbd-rust 接收后，将其转化为 Keystroke 写入 a2 的 PTY stdin。
  2. **获取结果 (Pull)**：L3 通过轮询 `{"method": "agent.read", "params": {"agent_id": "a2", "since_event_id": 105}}`，批量获取增量的 output_chunk 事件，并在遇到标志位 `terminal: true` 时判定当前任务的物理闭环。
  3. **异常推送 (Notification)**：如果 Agent 超过阈值无响应，ccbd-rust 主动向 UDS（Unix Domain Socket）广播 `{"method": "agent.stuck", "params": {"agent_id": "a2"}}`。
     - **L3 处置契约**：L3 接收到该通知后，ccbd-rust 层面不采取任何自动重启操作（维持 L2 调度层「只汇报不决策」的职责隔离）。L3 可以决定发送 `{"method": "agent.kill"}` 介入救援并清理死进程，随后调用 `agent.spawn` 携带调整后的 Prompt 重启 Agent；或者直接将 stuck 状态抛出给前端，等待用户手工判定。
  4. **长连接与事件订阅机制 (Event Subscription)**：考虑到 UDS 是双向持久连接，协议必须支持 L3 发起 `{"method": "session.subscribe", "params": {"session_id": "xxx"}}`。ccbd-rust 在收到订阅后，将该 Agent 产生的所有 output_chunk 和 completion 事件以 JSON 流形式主动 Push 给该 Socket 连接，替代低效的轮询（Polling）读取。
  5. **安全与鉴权机制 (Security & Auth)**：由于 Socket 固定监听在本地 `~/.local/state/ccbd/ccbd.sock`，我们将完全依赖 Linux 操作系统的文件权限（File Permissions，例如 `chmod 0600`）来隔离外部访问。JSON-RPC 协议层内部不再引入复杂的 Token 鉴权机制，直接信任任何能够写入该 Unix Socket 的本地进程。

---

## 架构师判定：框架是否足够？

作为资深架构师，Gemini 判断前面 5 个章节不足以让一个完全不熟悉该项目的 Rust 工程师安全地下笔写代码。

前面 5 章解决了「做什么」和「怎么做」的业务逻辑，但在 AI Vibecoding（依靠大模型编写代码）的背景下，工程师（包含 Codex 等辅助 Agent）最容易在状态重置、日志排查和外部依赖测试这三个工程切面上把系统改挂。因此，必须追加以下三个工程标准章节，整个 v2 框架才算真正闭环。

---

## 6. 可观测性与日志体系 (Observability & Logging)

本章节定义系统在后台作为守护进程运行时，如何留下足以追踪复杂并发 Bug 的审计线索。

- **日志库选择**：全量引入 `tracing` 和 `tracing-subscriber` 库来代替标准输出打印（`println!`）。
- **上下文绑定（Context Span）**：规定任何针对 Agent 状态转移（如 Spawn、Kill、检测到 Stuck）的日志，必须携带 `session_id` 和 `agent_id` 作为结构化字段（Span Attributes）。这是为了防止在处理多个并发 Session 时，日志行出现串台，导致无法追踪特定 Agent 的死亡原因。
- **物理落盘**：ccbd-rust 的错误与追踪日志必须自动滚动写入 `~/.local/state/ccbd/ccbd.stderr.log`。

---

## 7. 测试策略与 Mock 机制 (Testing Strategy & Mocking)

本章节定义工程师如何通过测试网闸，这也是基于 `docs/DESIGN.md` (v1 草稿) 第 6.4 节 "AI 自主测试边界" 必须落地的强制规范。

- **隔离真实 LLM API**：在任何本地自动化测试（`cargo test`）中，绝对禁止通过 `Command::new("gemini")` 去拉起真实的 Gemini 或 Codex 命令行。因为真实的 CLI 依赖复杂的身份验证（OAuth），容易触发超时或风控，导致测试结果 flaky（时好时坏）。
- **夹具实现 (Test Fixtures)**：工程师必须首先在 `tests/mock_agent/mock_agent.sh` 路径下实现一个模拟 Bash 脚本。该脚本需满足：监听标准输入，收到 "hello" 时延迟 100 毫秒输出 "world"，收到 "stuck" 时模拟死循环卡死。
- **测试分层**：
  1. **数据库层**：SQLite CRUD 测试必须使用内存模式（`rusqlite::Connection::open_in_memory()`），保证测试的纯净与高速。
  2. **集成调度层**：让 ccbd-rust 启动 `mock_agent.sh`，通过 UDS 下发指令并断言其能够正确触发 `DeliveryAck` 机制，以及能在 stuck 场景下被调谐循环（Reconciliation Loop）正确回收。

---

## 8. 构建部署与环境隔离 (Build, Deployment & Env Isolation)

本章节解决开发者在修改代码时不污染用户真实使用状态的物理隔离问题。

- **路径解析规范**：必须使用 `directories::ProjectDirs` 获取符合 XDG 规范的文件路径，杜绝在代码中硬编码绝对路径。
- **开发模式 (Dev Mode)**：工程师必须实现一套基于环境变量 `CCB_ENV` 的路径路由逻辑。
  - 当 `CCB_ENV=dev` 时，SQLite 数据库（`ccbd.sqlite`）、Socket 套接字（`ccbd.sock`）以及沙盒工作区，全部映射到 Rust 项目内的 `target/dev_state/` 目录下。
  - **收益**：如果代码导致状态机崩溃或数据库锁死，开发者只需要执行 `cargo clean` 即可一键销毁所有错误状态，完全不会干扰 L3 编排层当前正在使用的 `~/.local/state/ccbd/` 生产环境及其中记录的真实会话状态。

---

## 框架完整性 verdict

**框架到此结束，8 章已覆盖全部。**

解释——前 8 个章节构成了一个逻辑闭环：

1. 边界与定义（章 1）+ 模块重用划分（章 2）确定了系统要「做什么」以及「用什么代码做」。
2. 风险攻克（章 3）+ 核心状态机（章 4）+ 通信协议（章 5）解决了系统「怎么在分布式环境下安全运行」的核心难点（ACK 确认、竞态处理、数据源单一化）。
3. 可观测性（章 6）+ 测试策略（章 7）+ 环境隔离（章 8）提供了「怎么修 Bug」和「怎么安全验证」的工程脚手架。

对于一个专注系统底层进程管理的 Rust 守护进程而言，这 8 个维度已经穷尽了从设计、开发、测试到部署的全部必要技术上下文。
