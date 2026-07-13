# L2 Scheduler (ah/ahd) 架构地图与模块化解耦草案 (MD1/MD2 Draft)

- **状态**: 草案 (Divergent/Adversarial Input, 待 d1 Claude 设计主笔收敛为正式索引)
- **版本**: v1.0.0
- **日期**: 2026-07-12
- **作者**: Antigravity (设计辩论席)
- **输入来源**: `.kiro/specs/ah-modular-decoupling/requirements.md` + 源码实地审计

---

## 零、 特别论证：`src/agent_io` 的分层归属与去耦设计

在第一阶段的模块大纲论证中，我们针对 `src/agent_io` (包含 `reader.rs`, `registry.rs`, `writer.rs`) 的归属层级进行了深度分析，结论如下：

### 1. 物理层归属：Layer 3 (宿主与执行沙盒物理层)
我们将 `agent_io` 的物理代码主体划分至 **Layer 3 (Execution Substrate)**，理由如下：
* **物理资源管理**：它直接管理底层的 PTY/FIFO 管道创建与轮询监听 (`reader.rs`)、内存中的 TMux 窗格 ID (Pane ID) 与进程 PID 绑定关系 (`registry.rs`)，以及通过 TMux 物理指令向终端注入输入 (`writer.rs`)。这些职责属于纯粹的“宿主操作与物理端子”。
* **反向依赖的架构坏味道**：当前 `agent_io::reader` 直接引入了 `crate::db` 和 `crate::marker`，在读取物理字节的同时触发了数据库修改和 Prompt 判定。这种“物理驱动逻辑”的设计是典型的跨层级强耦合（Layer 3 反向依赖 Layer 4 和 Layer 5）。

### 2. 建议的 MD2 解耦方案 (Sensory Decoupling)
为了遵循“低耦合高内聚”与“第一性原则”，在 MD2 阶段不应当通过“打补丁”维持现状，而应当将 `agent_io` 重构为**纯粹的被动 I/O 驱动**：
* **职责剥离**：`agent_io` 仅负责终端 I/O 数据的读写。它应当暴露出一个无状态的字节流（如 `tokio::sync::mpsc` 通道或 `AsyncRead` 接口），只负责把 TMux Pane 输出 of 物理原始字节向上传递。
* **感知传感器上移**：在 **Layer 5 (Perception)** 层中，引入一个独立的 `PerceptionSensor`（或由 `prompt_handler` 负责）。该传感器负责订阅 `agent_io` 抛出的原始流，在感知层内部使用 `vt100` 解析器和 `MarkerMatcher` 进行匹配，并调用 `db` 进行状态更新。
* **效果**：解耦后，`agent_io` (Layer 3) 向上不依赖任何业务状态机与 Prompt 知识库，成为可独立测试的 PTY 读写夹具。

---

## 一、 五大子系统模块清单 (Architecture Map Index)

### Layer 1: 客户端入口与 IPC 接口层 (Entry & Client Interface)
*负责 L2 命令行接口的解析，以及 Client ↔ Daemon 之间的 IPC 通信生命周期管理。*

*   **二进制：ah (Client CLI)**
    *   *职责*：L2 调度器的用户态/终端态控制工具，负责将开发者的指令打包成 RPC 发送给 daemon。
    *   *路径*：`src/bin/ah.rs`
    *   *关键入口符号*：`main()`
*   **二进制：ahd (Daemon)**
    *   *职责*：L2 调度层的全局唯一常驻守护进程入口，负责初始化 DB、拉起 Orchestrator 以及启动 Unix Domain Socket 监听。
    *   *路径*：`src/bin/ahd.rs`
    *   *关键入口符号*：`main()`
*   **模块：cli (CLI 命令分发)**
    *   *职责*：使用 `clap` 定义 CLI 的所有子命令（如 `up`, `start`, `logs`, `doctor` 等）及环境检查引导。
    *   *路径*：`src/cli/` (包括 `up.rs`, `start.rs`, `setup.rs`, `doctor.rs` 等)
    *   *关键入口符号*：`cli::up::run()`, `cli::service_bootstrap::bootstrap()`, `cli::config::resolve_config()`
*   **模块：rpc (IPC 通信核)**
    *   *职责*：定义基于 JSON-RPC 的 Unix Domain Socket 通信管道，管理 RPC Server 路由与客户端桩代码。
    *   *路径*：`src/rpc/` (`mod.rs`, `router.rs`, `handlers/`)
    *   *关键入口符号*：`rpc::router::dispatch_rpc()`, `rpc::Ctx`, `rpc::handlers::handle_spawn_agent()`

---

### Layer 2: 核心调度与生命周期控制层 (Scheduling Core & Lifecycle)
*L2 系统的“心脏”，基于调谐循环（Reconciliation Loop）驱动 Agent 状态机的转换，维持 L1 执行层进程的健康状态。*

*   **模块：orchestrator (全局调度器)**
    *   *职责*：常驻后台任务，定期或被事件唤醒执行 `run_once` 调谐循环。管理作业分发、超时处理及故障熔断。
    *   *路径*：`src/orchestrator/` (`mod.rs`, `pubsub.rs`)
    *   *关键入口符号*：`spawn_orchestrator_task()`, `run_once()`, `pubsub::notify_job_update()`
*   **模块：monitor (进程生命周期监视器)**
    *   *职责*：使用 Linux `pidfd` 系统调用（或跨平台兼容方案）对 Agent 进程进行高精度监控，在进程意外退出时触发回收通知。
    *   *路径*：`src/monitor/` (`mod.rs`, `agent_watch.rs`, `session_watch.rs`, `master_watch.rs`)
    *   *关键入口符号*：`pidfd_open()`, `agent_watch::watch_agent()`, `master_watch::master_watch_patrol_loop()`
*   **模块：master_revival (Master 自愈逻辑)**
    *   *职责*：在监控到 L2 主控会话崩溃或受外部强干扰挂起时，负责无缝拉起并重建 L2 调度环境。
    *   *路径*：`src/master_revival.rs`
    *   *关键入口符号*：`master_revival_loop()`, `revive_master_session()`
*   **模块：master_cutover (Master 主备切换)**
    *   *职责*：负责在多 L2 环境下（若有）或旧守护进程退出后进行主状态机的安全切流，确保同一时间只有一个活跃调度实例。
    *   *路径*：`src/master_cutover.rs`, `src/cli/master_cutover.rs`
    *   *关键入口符号*：`execute_cutover()`

---

### Layer 3: 宿主与执行沙盒物理层 (Execution Substrate & Sandbox)
*处理与 Linux 宿主系统的所有物理交互，包括 PTY、TMux 窗格、Systemd 守护进程单元管理以及文件沙盒校验。*

*   **模块：tmux (TMux 会话驱动)**
    *   *职责*：封装对 `tmux` 命令行的物理调用，创建会话、定位 Pane、物理捕获 Pane 缓冲区。
    *   *路径*：`src/tmux/` (`mod.rs`, `session.rs`, `scope.rs`)
    *   *关键入口符号*：`TmuxServer`, `TmuxPaneId`, `TmuxServer::ensure_session_sync()`, `TmuxServer::capture_pane_sync()`
*   **模块：sandbox (沙盒与隔离验证)**
    *   *职责*：在拉起 Agent 前校验系统沙盒环境（如 Cgroup、物理路径），并在 `Systemd` 用户单元中限制子进程的权限。
    *   *路径*：`src/sandbox/` (`mod.rs`, `systemd.rs`, `path.rs`)
    *   *关键入口符号*：`check_environment()`, `systemd::scope_run()`
*   **模块：platform (OS 平台行为抽象)**
    *   *职责*：抹平 Linux 与 macOS/Windows 的底层进程管理、信号机制差异（如 Linux 下的 `pidfd_open` 与 macOS 的 `kqueue`）。
    *   *路径*：`src/platform/` (包括 `linux/`, `macos/`, `windows/`)
    *   *关键入口符号*：`sys::process`, `ProcessIdentity`, `ProcessWatcher`
*   **模块：systemd_unit (Systemd 配置生成)**
    *   *职责*：为常驻 Agent 及 `ahd` 动态生成合规的 Systemd User Unit 配置文件，以便利用 `systemd --user` 的强生命周期监视。
    *   *路径*：`src/systemd_unit.rs`
    *   *关键入口符号*：`generate_user_service_unit()`
*   **模块：agent_io (PTY 物理端子 - 详见第零章)**
    *   *职责*：通过 FIFO 和 PTY 驱动 TMux Pane 的标准输入输出，并维护内存中活跃 Pane 绑定的全局映射。
    *   *路径*：`src/agent_io/` (`mod.rs`, `reader.rs`, `registry.rs`, `writer.rs`)
    *   *关键入口符号*：`spawn_agent_io_reader_task()`, `send_text_to_pane()`, `registry::register()`

---

### Layer 4: 数据持久化与 SoT 层 (State & Data Persistence)
*系统的单事实来源（Single Source of Truth），负责数据库连接管理、事务断言以及物理状态目录的隔离维护。*

*   **模块：db (SQLite 事务数据仓)**
    *   *职责*：封装底层 SQLite 交互，驱动 schema 升级，并集中处理跨多表（sessions、agents、jobs）的状态转移事务。
    *   *路径*：`src/db/` (包括 `state_machine.rs`, `schema.rs`, `jobs.rs`, `agents.rs`, `state_machine_assert.rs` 等)
    *   *关键入口符号*：`Db`, `init()`, `state_machine::mark_agent_idle_log_event()`, `state_machine_assert::assert_valid_transition()`
*   **模块：state_layout (状态目录管理)**
    *   *职责*：提供在宿主物理路径下（如 `~/.cache/ah/`）解算会话与 Agent 独立隔离沙盒状态目录的安全规范。
    *   *路径*：`src/state_layout.rs`
    *   *关键入口符号*：`StateLayout`
*   **模块：env (配置环境解算)**
    *   *职责*：读取启动期环境变量并解算底层的全局状态主路径。
    *   *路径*：`src/env.rs`
    *   *关键入口符号*：`resolve_state_dir()`
*   **模块：error (错误定义集)**
    *   *职责*：整个 L2 Scheduler 的强类型自定义错误体系定义。
    *   *路径*：`src/error.rs`
    *   *关键入口符号*：`CcbdError`

---

### Layer 5: 感知拦截与可靠事件层 (Perception & Eventing)
*通过“物理观察”转化为“逻辑事实”的翻译层。监控终端变化，识别 Prompt 拦截决策，并通过 Outbox 保证事件投递的不重不漏。*

*   **模块：prompt_handler (Prompt 拦截与决策)**
    *   *职责*：递归分析终端缓冲区内的 Prompt 并对照 Knowledge Base 决定是否挂起 Agent 进入 `PROMPT_PENDING`；调用 LLM Classifier 完成自动决策与输入注入。
    *   *路径*：`src/prompt_handler/` (包括 `integration.rs`, `runner.rs`, `gating.rs`, `schema.rs` 等)
    *   *关键入口符号*：`scan_prompt_and_apply_outcome()`, `PromptKb`, `PromptScanRequest`
*   **模块：outbox (事务型日志外箱)**
    *   *职责*：实现 Transactional Outbox 机制。通过本地文件落盘（Journal-first）+ 数据库去重台账，确保即使 daemon 闪崩，Hook 状态与任务完成信号也能不重不漏地送达。
    *   *路径*：`src/outbox/` (`mod.rs`, `tests.rs`)
    *   *关键入口符号*：`journal_record()`, `consume_record()`, `cold_scan_dir()`
*   **模块：runtime_events (系统遥测事件快照)**
    *   *职责*：在调度状态机转移时，生成系统当前的拓扑快照事件（Snapshot），写入事件流以便 L3 层追踪。
    *   *路径*：`src/runtime_events.rs`
    *   *关键入口符号*：`RuntimeSnapshot`, `write_state_snapshot()`
*   **模块：marker (Prompt 边界匹配器)**
    *   *职责*：通过正则或稳定时钟，判定终端缓冲区是否存在需要拦截的 Prompt 特征字符。
    *   *路径*：`src/marker/` (`mod.rs`, `registry.rs`)
    *   *关键入口符号*：`MarkerMatcher`, `parser_registry`
*   **模块：completion (日志型完成观测器)**
    *   *职责*：针对非交互式运行的 Agent，后台读取并解析其输出日志（如 `.claude` 文件的末尾），识别其 Turn 的结束点并改写状态。
    *   *路径*：`src/completion/` (`monitor.rs`, `reader.rs`, `parser.rs`, `registry.rs`)
    *   *关键入口符号*：`run_log_monitor_tick()`, `LOG_MONITORS`
*   **模块：pane_diff (物理屏幕差分观察)**
    *   *职责*：分析 tmux 窗格字面内容随时间的增长率与变化率，检测卡死状态（STUCK）以及 Spinner 思考状态。
    *   *路径*：`src/pane_diff/` (`mod.rs`, `watcher.rs`)
    *   *关键入口符号*：`pane_diff_watcher_loop()`, `AgentDiffState`

---

## 二、 能力 → 模块 Owner 映射表 (Capability Map)

本映射表用于帮助开发者在新增特性或排查故障时，迅速定位 Owner 模块，避免盲目 grep：

| 能力 (Capability) | 对应职责 (Responsibility) | 负责模块 (Owner Module) | 核心路径 |
| :--- | :--- | :--- | :--- |
| **持久化与断言** | 对 session、agent、job 的一切状态转移事务、锁机制与断言约束 | `db::state_machine` | [state_machine.rs](file:///home/sevenx/coding/ccbd-rust/src/db/state_machine.rs) |
| **物理进程监控** | 监听宿主进程的真实 PID 退出、发送物理 SIGKILL 信号 | `monitor` / `platform` | [monitor/mod.rs](file:///home/sevenx/coding/ccbd-rust/src/monitor/mod.rs) |
| **物理终端注入** | 向 TMux 窗格注入 keystroke/文本，清除会话遗留 pane | `agent_io::writer` / `tmux` | [agent_io/writer.rs](file:///home/sevenx/coding/ccbd-rust/src/agent_io/writer.rs) |
| **环境沙盒检验** | 校验 cgroup 挂载状态、衍生出不受 cgroup 限制 of 无沙盒启动态 | `sandbox` | [sandbox/mod.rs](file:///home/sevenx/coding/ccbd-rust/src/sandbox/mod.rs) |
| **Systemd 托管** | 生成 systemd transient unit 配置，托管守护运行 | `systemd_unit` | [systemd_unit.rs](file:///home/sevenx/coding/ccbd-rust/src/systemd_unit.rs) |
| **感知：Prompt 扫描** | 物理读取 TMux Buffer 匹配 Prompt 特征与匹配过滤 | `prompt_handler` | [prompt_handler/integration.rs](file:///home/sevenx/coding/ccbd-rust/src/prompt_handler/integration.rs) |
| **感知：卡死监控** | 定期对比 Pane 缓冲区差分，上报卡死事件 | `pane_diff` | [pane_diff/mod.rs](file:///home/sevenx/coding/ccbd-rust/src/pane_diff/mod.rs) |
| **感知：日志完成** | 轮询 L1 执行层本地日志文件，捕捉执行完毕信号 | `completion` | [completion/monitor.rs](file:///home/sevenx/coding/ccbd-rust/src/completion/monitor.rs) |
| **事件投递保障** | 本地日志写盘 (Journal-first) 与消费去重 (JC-1) | `outbox` | [outbox/mod.rs](file:///home/sevenx/coding/ccbd-rust/src/outbox/mod.rs) |
| **主备自愈** | 侦测并自动重建损坏的 Master Session 会话 | `master_revival` | [master_revival.rs](file:///home/sevenx/coding/ccbd-rust/src/master_revival.rs) |

---

## 三、 MD2 模块化解耦建议 (Decoupling Recommendations)

根据“不打补丁、可推倒重写、低耦合高内聚”的第一性原则，我们针对该架构地图提出以下解耦边界重塑建议：

### 1. 消除数据层的全局穿透 (Strict Repository Pattern)
*   **现状痛点**：目前 `orchestrator`, `completion::monitor`, `agent_io::reader` 等多个顶层模块直接持有 `Db` 的 clone，并在其内部自由地拼装 `rusqlite::params!`、调用 `db::state_machine` 函数。这导致数据持久化逻辑深深散落在各个分层。
*   **解耦策略**：
    *   在 Layer 4 中抽象出一个**仓储接口（Repository Traits）**，例如 `AgentStore` 与 `JobStore`。
    *   外部模块只能调用诸如 `store.mark_idle(agent_id)` 的领域方法，隐藏底层的 SQL/rusqlite 事务及 DDL 细节。任何跨模块的数据读取和断言均必须在数据层闭环。
    *   *收益*：若未来需要将 SQLite 替换为其他引擎或升级 schema，仅需重写 Repository 实现，无需改动任何调度器逻辑。

### 2. 剥离 `agent_io` 的业务感知逻辑 (Passivate Terminal I/O)
*   **现状痛点**：`agent_io::reader` 应当属于 Layer 3，但它却硬性引用了 Layer 5 的 `MarkerMatcher` 以及 Layer 4 的 `Db`。这导致我们无法在不拉起数据库和加载 matcher 配置的情况下独立测试 PTY 数据的读取稳定性。
*   **解耦策略**：
    *   将 `agent_io::reader` 重构为一个纯粹的 **字节管道发送器**。它仅将 FIFO 中轮询出的裸字节写入一个 Tokio 广播信道 (`tokio::sync::broadcast::Sender<Vec<u8>>`)。
    *   在 Layer 5 中，新增一个名为 `PerceptionStreamProcessor` 的常驻逻辑。它订阅上述广播信道，并在拿到数据后运行 `vt100` 解析与 `MarkerMatcher` 拦截，最后再调用 `db` 状态转移。
    *   *收益*：解耦合后，Layer 3 不再具有任何“业务意识”，甚至不知道 L2 主调度器的存在。

### 3. 解除 Orchestrator 与数据层之间的环形依赖 (Outbox Event-Driven Tick)
*   **现状痛点**：当前存在环形依赖链：`orchestrator` 执行 Tick $\to$ 改变状态机（调用 `db::state_machine`）$\to$ 状态机完成事务后调用 `orchestrator::pubsub::notify` $\to$ 重新唤醒 `orchestrator` 的 Tick 循环。
*   **解耦策略**：
    *   **Orchestrator 被动化**：利用现有的 Layer 5 `outbox` 模块作为唯一的控制中枢。
    *   当 `db::state_machine` 修改状态时，其仅在一个原子事务中将对应的快照事件插入 `outbox` 数据库表。
    *   `outbox` 独立的消费轮询器监听到新事件时，再通过 `WAKER` 触发 `orchestrator` 的 Tick。
    *   *收益*：消除了强耦合的直接函数回调链，使得状态改变与调度决策异步化。

### 4. 彻底划清 CLI 与 Daemon 编译边界 (Binary Decoupling)
*   **现状痛点**：`ah` (CLI) 和 `ahd` (Daemon) 共享同一个巨大的 `lib.rs` 依赖集，导致 CLI 包体积过大且包含大量仅用于守护进程的第三方库（如 `rusqlite` bundled、系统级的 `nix` 依赖等）。
*   **解耦策略**：
    *   在 `Cargo.toml` 中将项目划分为 **Cargo Workspace**。
    *   将 L2 的宿主接口/客户端抽象为一个独立的轻量级 Crate (`ah-client`)，其唯一的依赖是 `tokio::net` 与 `serde_json`（用于发起 IPC）。
    *   将调度、持久化及感知引擎留在 `ahd-daemon` 中。
    *   *收益*：极致提高 CLI 的分发和引导速度，防止 Daemon 的内部变化泄露给客户端二进制。
