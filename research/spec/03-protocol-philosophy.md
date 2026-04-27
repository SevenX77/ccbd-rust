# 阶段三：协议哲学与核心机制设计 (Protocol & Philosophy)

> **阶段目标**：将阶段二（架构决策）的高层建筑，下沉为数据结构、算法流、状态机流转图及接口契约。输出物必须达到「无二义性（Unambiguous）」的标准，以便在阶段四由 Codex 直接翻译为 Rust 代码。
> 
> **工作流规范**：每个子议题讨论成熟后，其终态结论将反向汇聚至本文件的对应章节。独立讨论将放置于 `research/spec/03-discussion/` 目录下。

## 1. 机制深潜议程设置

根据前期决议的系统复杂度分布，阶段三被拆解为以下 **5 个 S 档（Spec）子议题**。

### S-1：核心状态机与生命周期图谱 (State Machine & Lifecycle)
*   **承接**：A-5（调谐双保险）与议题 1b（反馈闭环）。
*   **讨论要点**：精确定义 Agent 从被操作系统拉起（Spawning）到回收（Killed/Crashed）的完整生命周期。特别要处理 Push 事件（PTY 输出/pidfd）与 Polling 事件（对账循环）在状态流转上的竞争条件。
*   **交付物形式**：`Mermaid` 状态转移图 + 包含前置条件与副作用的状态流转矩阵。
*   **拟定路径**：`research/spec/03-discussion/s-1-state-machine.md`

### S-2：SQLite 权威数据源建模 (SoT Schema Design)
*   **承接**：A-1（持久化）与 C-1（SQLite Schema）。
*   **讨论要点**：设计 `projects`、`sessions`、`agents`、`events`、`evidence` 5 张核心表。定义如何利用版本控制（`state_version`）或事务隔离来实现 A-5 中要求的「基于 CAS 的幂等状态转移」。
*   **交付物形式**：`SQL DDL` 语句 + 关键状态更新的 `SQL` 事务实例。
*   **拟定路径**：`research/spec/03-discussion/s-2-schema.md`

### S-3：JSON-RPC 契约与错误代码树 (RPC API & Error Codes)
*   **承接**：A-2（IPC 协议）、A-7（错误传播）与 C-2/C-3。
*   **讨论要点**：定义 L3（Master）与 L2（ccbd-rust）交互的全部 JSON-RPC 方法签名（入参/出参）。敲定 `AGENT_STUCK`、`SANDBOX_BWRAP_NOT_FOUND` 等具体错误码的继承关系和负载（Payload）结构。
*   **交付物形式**：类似 `TypeScript` 的 Interface 定义（或纯 `JSON Schema`） + 全量错误枚举表。
*   **拟定路径**：`research/spec/03-discussion/s-3-rpc-contract.md`

### S-4：VT100 解析与 Marker 算法 (PTY Processing Algorithm)
*   **承接**：A-3（PTY/VT100 栈）。
*   **讨论要点**：将 A-3 决议的「底部 5 行快路径 + 全局慢路径」转化为具体算法步骤。定义 PTY Chunk 到达时，解析器如何刷新 Screen Buffer，以及如何精准控制引发 `Unknown` 状态的 Timer（计时器）。
*   **交付物形式**：结构化伪代码（Pseudocode） + Timer 触发时序图。
*   **拟定路径**：`research/spec/03-discussion/s-4-pty-algorithm.md`

### S-5：沙盒挂载映射与进程组装 (Sandbox & Process Assembly)
*   **承接**：A-4（沙盒方案）与 A-6（拓扑清理）。
*   **讨论要点**：如何将 `[sandbox]` Provider 契约配置解析并转换为具体的 `bwrap` Shell 调用参数。明确挂载点在 XDG 目录结构下的绝对路径解算规则，以及 `pidfd` 的附加时机。
*   **交付物形式**：配置到命令行的映射逻辑伪代码。
*   **拟定路径**：`research/spec/03-discussion/s-5-sandbox-assembly.md`

---

## 2. 讨论顺序推荐

为了确保前置依赖层层递进，不出现设计死锁，我强烈推荐按以下顺序展开讨论：

1.  **S-1 (状态机) -> S-2 (Schema)**：**（基石阶段）**必须先有逻辑状态的转移图，才能设计出支撑该转移的物理数据库表结构（特别是锁和并发控制字段）。
2.  **S-3 (RPC 契约)**：**（对外契约）**基于已落盘的状态机和 Schema，确立系统向 L3 暴露的 JSON-RPC 接口和错误信息，这是与其他端对接的基础。
3.  **S-4 (PTY 算法) -> S-5 (沙盒组装)**：**（执行末端）**最后处理具体的 I/O 算法和操作系统底层的挂载逻辑，这些属于高内聚模块，不会对上层架构产生副作用。

---

## 3. 防偏航原则 (Spec Guidelines)
在即将展开的任何 S 档讨论中，我们必须遵循以下原则：
- **拒绝抽象**：任何状态改变必须指出对应修改了哪张表的哪个字段；
- **面向失败设计**：每定义一个流程（如解析 PTY），必须同时定义其失败时的后备处理（如触发 `Unknown` 并存储 Evidence）；
- **零黑话**：使用标准的系统编程术语（如 `bind-mount`, `epoll`, `CAS`），不生造词汇。

---

## 阶段三闭合声明

**状态：已完成 (COMPLETED)**

截至目前，阶段三（协议哲学与核心机制设计）的机制深潜已全部收敛，满足完成判据。5 个 S 档子议题已将阶段二的高层架构决策转化为无二义性的底层实现契约：

- **S-1 核心状态机** (`03-discussion/s-1-state-machine.md`)：确立了 6 个状态枚举与基于 CAS 的并发控制矩阵。
- **S-2 数据源建模** (`03-discussion/s-2-schema.md`)：输出包含 STRICT 模式和全量约束的 5 张 SQLite 核心表 DDL 与关键事务流。
- **S-3 RPC 契约** (`03-discussion/s-3-rpc-contract.md`)：完成 9 个 Method 的 JSON-RPC 定义及强类型的 Error Code 错误树分配。
- **S-4 PTY 算法** (`03-discussion/s-4-pty-algorithm.md`)：敲定 Fast/Slow 双扫描路径及 Timer 触发的时序伪代码。
- **S-5 沙盒组装** (`03-discussion/s-5-sandbox-assembly.md`)：产出从挂载解算到 `systemd-run` + `bwrap` 嵌套包装及失败回滚的算法伪代码。

**结论**：系统架构不仅在逻辑和物理层面上完全闭合，且已细化为数据结构与算法流。可以正式由 Master Claude 启动「阶段四：任务拆解」，将这些 Spec 转化为面向 Codex 的原子任务清单。
