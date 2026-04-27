# 阶段二：架构决策追踪议程

## 阶段目标 + 完成判据（重述）
- **目标**：对 8 章框架中的技术选择进行深度背书，确保 Codex 在实施时理解「为什么」而非仅仅是「做什么」，防止架构随实施漂移。
- **完成判据**：所有 A 档议题均完成「正反辩论」并形成决议文档；B 档议题完成批量理由说明。

## A 档决策（独立议题深度辩论）

- **A-1: 持久化方案 (SQLite + WAL vs Others)**
  - *辩论点*：SQLite 带来的 ACID 保证对 `R-RECONNECT-1`（断线重连）至关重要，但其 C-FFI 依赖是否比 pure Rust 的 sled 更有价值？如果选 JSON 文件，如何保证 `R-RECONCILE-1` 的对账幂等性？
- **A-2: IPC 协议与序列化 (UDS + JSON-RPC vs gRPC/HTTP)**
  - *辩论点*：JSON-RPC 2.0 在 JS/Python 端的低侵入性 vs gRPC 的强类型 IDL。在 `R-API-COMPAT-1` 约束下，哪种协议更易于在不破坏兼容性的情况下演进？
- **A-3: PTY/VT100 确认闭环栈 (portable-pty + vt100 vs Custom)**
  - *辩论点*：直接调用 `nix::pty` 的极简主义 vs `portable-pty` 的成熟封装。这是解决「斜杠命令被吞」风险的核心，必须辩论其对 Activity Markers 捕捉的准确性。
- **A-4: 沙盒方案 (Bubblewrap vs unshare/firejail)**
  - *辩论点*：Bwrap 的跨发行版兼容性 vs 直接调用 Linux Namespace。如果选反方，如何在非 root 环境下优雅处理 bind-mount 的安全复杂性？
- **A-5: 状态调谐策略 (Polling vs Event-driven/pidfd)**
  - *辩论点*：纯 Polling 的高延迟 vs `pidfd_open` (Linux 5.3+) 的亚毫秒级响应。为满足「双保险」原则，是否应采用「Push 触发响应，Polling 兜底对账」的混合模型？
- **A-6: 进程拓扑与级联清理 (Watchdog Model)**
  - *辩论点*：Master 崩溃后 L2 如何绝对可靠地清理 L1 资源？如果 L2 自己崩溃了，谁来清理 L1？
- **A-7: 错误传播与结构化 Code 设计**
  - *辩论点*：如何将 PTY 层的系统错误映射为 L3 可理解的结构化代码（如 `AGENT_STUCK`）？这是满足 `R-ERROR-CODES-1` 的关键。

## B 档决策（批量背书，无需独立议题）

- **B-1: 异步运行时 (tokio)** -- 理由：Rust 异步生态的事实标准，周边集成最广，无合理反方。
- **B-2: 日志体系 (tracing + tracing-subscriber)** -- 理由：支持结构化 Span 绑定，是追踪并发 Agent 状态漂移的最佳工具。
- **B-3: 序列化/配置格式 (serde + toml/json)** -- 理由：Rust 生态默认选择，性能与 ergonomics 兼顾。
- **B-4: 单二进制多模式 (Single Binary)** -- 理由：议题 3 已决议，简化分发且减少跨进程符号冲突。
- **B-5: 测试 Mock 策略 (mock_agent.sh)** -- 理由：8 章框架已定，通过 Shell 模拟外部环境是解耦 OAuth 等重型依赖的最廉价方案。
- **B-6: 路径解析规范 (directories-next/ProjectDirs)** -- 理由：遵循 XDG 规范，满足 `R-ISOLATION-1` 要求的生产/开发环境隔离。

## C 档决策（延迟到阶段三机制深潜）

- **C-1: SQLite 具体 Schema 定义**
- **C-2: 具体 RPC 方法签名与字段**
- **C-3: 具体错误枚举变体**

## 阶段二讨论顺序推荐
1. **A-2 (IPC)** -> **A-7 (Error Code)**：契约先行。
2. **A-1 (Persistence)** -> **A-6 (Topology)**：骨架先行。
3. **A-3 (PTY)** -> **A-5 (Reconciliation)**：I/O 攻坚。
4. **A-4 (Sandbox)**：安全固化。

---

## A-2 决议：IPC 协议与序列化方案

**决策内容**：采用 **Unix Domain Socket (UDS) + Newline-delimited JSON-RPC 2.0**。通过 JSON-RPC Notifications 实现服务器向客户端的异步事件推送（如输出流、状态变更）。

**核心背书**：
1. **调试高友好**：支持 `socat` 等原生工具直接观测二进制流，对 L2 守护进程排障至关重要。
2. **极简集成**：Caller 端（L3/IDE）无需 Protobuf 等重型工具链，保持跨语言接入的低门槛。
3. **架构前瞻**：原生支持 UDS 辅助数据（SCM_RIGHTS），为未来直传 PTY FD 等底层优化预留空间。

**决议日期**：2026-04-26
达成方式：Master Claude 提议，Gemini 补强（FD Passing/Framing）并确认。详见 `research/spec/02-discussion/a-2-ipc-protocol.md`。

---

## A-7 决议：错误传播与结构化 Code 设计

**决策内容**：采用「两层模型」。内部 Module 强制使用 `thiserror` 定义强类型枚举；RPC 边界统一映射为结构化 JSON-RPC 错误。

**核心背书**：
1. **R-ERROR-CODES-1 达成**：通过 `error_code` 字符串枚举（如 `PTY_MARKER_TIMEOUT`）实现机器可读的错误逻辑分发。
2. **零歧义演进**：错误码采用 `MODULE_REASON` 命名规范，遵循「只增不减」原则，确保 L3 逻辑在后端升级时的稳定性。
3. **强类型工程质量**：利用 Rust `thiserror` 强制实施者在开发期面对所有错误路径，禁止使用 `anyhow` 兜底。

**决议日期**：2026-04-26
**达成方式**：Master Claude 提议，Gemini 细化 6 大类目结构与命名规范并确认。详见 `research/spec/02-discussion/a-7-error-propagation.md`。

---

## A-1 决议：持久化方案与数据库选型

**决策内容**：采用 **SQLite + WAL 模式（rusqlite bundled）**。

**核心背书**：
1. **R-RECONCILE-1 刚需**：SQLite 提供的 ACID 事务与 SQL JOIN 表达力，是确保文件系统状态与内存状态严格对账的唯一成熟基石。
2. **极高吞吐下的安全性**：通过 `PRAGMA synchronous=NORMAL;`，在保障进程级崩溃零损坏的前提下，完美消化每秒数十次的 PTY Chunk 写入。
3. **单二进制纯净分发**：利用 `rusqlite` 的 `bundled` 特性，屏蔽 C-FFI 跨平台依赖痛点，搭配 `include_str!` 嵌入式 Migration，符合 B-4 部署规约。
4. **Mock 测试友好**：原生支持 `:memory:` 模式，为高频 TDD 测试提供毫秒级隔离环境。

**决议日期**：2026-04-26
**达成方式**：Master Claude 提议，Gemini 补强（执行模型防阻塞/PRAGMA 确立/容量论证）并确认。详见 `research/spec/02-discussion/a-1-persistence.md`。

---

## A-6 决议：进程拓扑与级联清理

**决策内容**：采用 **单 Daemon + Systemd `--scope` 协同拓扑**，拒绝引入 Supervisor 子进程层。全面确立「Linux 优先，macOS 降级」的环境假设。

**核心背书**：
1. **零 IPC 开销**：Agent 维持为 Daemon 的直接子进程（Child），避免了双层 Watchdog 带来的状态撕裂与 IPC 协议负担。
2. **底层信号监听**：结合 `pidfd_open` 与 `--scope`，Daemon 能直接捕获 Master 与 Agent 的内核级状态变更，实现亚毫秒级的异常响应。
3. **彻底的级联防护**：依托 `ccbd-agents.slice` 的 Systemd BindsTo 机制，辅以 SQLite 的 Startup Reconcile 对账，构建出 L2 崩溃情况下的无死角资源回收网。
4. **不可变所有权**：禁止 Master 所有权转移，强制通过 Respawn+状态恢复进行交接，极大简化了内部状态机。

**决议日期**：2026-04-26
**达成方式**：Master Claude 提议，Gemini 补强（环境降级声明、TasksMax 调优、禁止重分配）并确认。详见 `research/spec/02-discussion/a-6-process-topology.md`。

---

## A-3 决议：PTY/VT100 确认闭环栈

**决策内容**：采用 **portable-pty (tamux fork) + vt100 (doy/vt100)** 组合栈。在内存中维护 **200x200** 虚拟屏幕，执行「实时解析 + 底部行优先匹配」策略。

**核心背书**：
1. **语义级状态感知**：通过完整 VT100 状态机而非字节流匹配，彻底解决 ANSI Escape 序列对 Marker 识别的干扰，确保 L2 能正确理解 Modern TUI 的视觉布局。
2. **极致响应速度**：利用 Rust 解析器的高性能实现 Real-time 逐块解析，配合底部 5 行快路径匹配，为 L3 提供亚秒级的「输入就绪」反馈。
3. **工程复用与风险规避**：直接 Fork `tamux` 的 PTY 封装，规避了 Linux 沙箱环境下 FD 处理与信号同步的复杂性，显著降低 MVP 阶段的 I/O 不稳定性。
4. **闭环兜底支撑**：该栈输出作为 `R-STATE-FALLBACK-LOOP` 闭环的核心数据源，在匹配失败时提供高保真的 Evidence Dump 以供后续规则固化。

**决议日期**：2026-04-26
**达成方式**：Master Claude 提议，Gemini 补强（Screen 尺寸论证/解析触发时机/混合扫描策略）并确认。详见 `research/spec/02-discussion/a-3-pty-vt100.md`。

---

## A-5 决议：状态调谐策略与双保险模型

**决策内容**：采用 **Push 触发响应（pidfd + inotify + epoll）+ Polling 幂等对账（30s）** 的混合模型。macOS 环境下自动退化为 **1Hz 高频 Polling**。

**核心背书**：
1. **R-DISPATCH-2 达成**：利用 `pidfd_open` (Linux 5.3+) 捕获内核级进程死亡信号，实现亚毫秒级的故障探测与 Notification 推送，彻底消除「死等僵尸进程」的逻辑空转。
2. **高保真 Unknown 判定**：通过 Push 驱动的实时 `MarkerTimer`，在 Agent 响应超时瞬间立即触发 `Unknown` 状态，确保议题 1b 的 Evidence Dump 能准确捕捉第一现场。
3. **最终一致性保障**：30s 间隔的对账循环作为「双保险」，不依赖外部信号，通过直接对比数据库状态与 `/proc` 物理真实性，强制修正任何因系统竞态导致的逻辑偏差。
4. **状态机事务化**：通过 SQLite 的原子更新语句实现跨事件源的幂等状态转移，确保清理逻辑（如信号下发、目录回收）在任何并发场景下均「有且仅有一次」成功执行。

**决议日期**：2026-04-26
**达成方式**：Master Claude 提议，Gemini 补强（CAS 状态转移/macOS 退化策略/Timer 驱动模型）并确认。详见 `research/spec/02-discussion/a-5-reconciliation.md`。

---

## A-4 决议：沙盒方案 (Bubblewrap)

**决策内容**：采用 **Bubblewrap (`bwrap`) CLI 调用** 作为唯一沙盒后端。拒绝静默降级，允许通过环境变量显式绕过。支持基于 Provider 契约的沙盒配置动态挂载。

**核心背书**：
1. **安全与工程的极致平衡**：放弃自研 `unshare(2)` 的重复劳动与高安全风险，直接复用 Linux 桌面生态（Flatpak）的标准化组件，实现最低成本的工业级 Namespace 隔离。
2. **严防安全静默失效**：启动期强制前置检查 `bwrap` 二进制。当环境缺失特性时，宁可报错退出也绝不回退到无沙盒模式，仅允许 `CCBD_UNSAFE_NO_SANDBOX=1` 作为 CI 场景的高危 Bypass 逃生舱。
3. **最小权限细粒度**：摒弃粗放的全局 `.ccb` 挂载，转为依托 L2 动态参数组装与 Provider 契约（`[sandbox]` 配置），实现精准到目录级的只读绑定与网络隔离控制。
4. **统一错误语义**：启动期及运行期的沙盒异常，将严格依照 A-7 错误传播模型，映射为 `SANDBOX_BWRAP_NOT_FOUND` 等强类型错误反馈至 L3。

**决议日期**：2026-04-26
**达成方式**：Master Claude 提议，Gemini 补强（拒绝 Fallback/优化 Baseline/错误映射）并确认。详见 `research/spec/02-discussion/a-4-sandbox.md`。

---

## 阶段二闭合声明

**状态：已完成 (COMPLETED)**

截至 2026-04-26，阶段二的架构决策已全部收敛：
- **7 项 A 档独立议题**（A-1 Persistence, A-2 IPC, A-3 PTY/VT100, A-4 Sandbox, A-5 Reconciliation, A-6 Topology, A-7 Error Codes）**已全部完成「正反辩论」并形成决议文档**。
- **6 项 B 档议题**（异步运行时、日志、序列化、二进制分发、测试 Mock、路径规范）**已完成批量背书**。
- **3 项 C 档议题**（具体 Schema、RPC 签名、错误枚举变体）**已确认延迟至阶段三实施期间进行动态细化**。

**结论**：满足阶段二完成判据。当前的决议集已经为 Codex 进入阶段三（代码生成）提供了充分、坚实且无歧义的架构蓝图与逻辑约束。


