# ccbd-rust Spec-Driven Design 工作流程

> **项目铁律**：本项目严格遵循 `superpower` + `Kiro` 的 spec-coding 流程。在 `research/spec/` 目录下的对应阶段文件未获得 Master Claude 终审通过前，禁止 Codex 进入 `src/` 编写任何业务逻辑代码。

---

## 1. 流程总览：Kiro 三段式驱动架构

本流程从当前的「8 章框架」出发，通过四个阶段将抽象架构物化为具体的执行指令：

1.  **阶段一：宏观对齐 (Requirements)** -> 明确边界与定位。
2.  **阶段二：决策固化 (Design)** -> 详述功能实现的「为什么」。
3.  **阶段三：机制深潜 (Spec)** -> 解决实现哲学与核心协议。
4.  **阶段四：任务分解 (Tasks)** -> 产出可被 Codex 直接执行的任务清单。

---

## 2. 阶段详解

### 阶段一：宏观定位与边界对齐 (Boundary & Positioning)
*   **目标**：解决用户反馈第 1 点，明确 ccbd-rust 在工具链中的生态位。
*   **讨论重点**：ccbd-rust 是作为「纯粹的物理 I/O 调度器」还是「带部分逻辑的 Agent 管理器」？
*   **交付物**：`research/spec/01-boundaries.md`
*   **完成判据**：明确定义出 ccbd-rust **「绝对不做之事」**（如：不解析 Prompt 内容、不处理多轮对话上下文）。

### 阶段二：架构决策追踪 (Architecture & Decision Mapping)
*   **目标**：解决用户反馈第 3 点，通过详述决策原因防止实施漂移。
*   **方法论**：参照 `superpowers/writing-plans` 的深度模式，对 8 章框架中的每一项技术选择进行「正反辩论」。
*   **讨论重点**：为什么选 SQLite 而不是文件系统？为什么选 UDS 而不是 HTTP？
*   **交付物**：`research/spec/02-architecture-decisions.md`
*   **完成判据**：每个核心模块（如 `src/pty`）均有对应的「决策背书」，确保 Codex 在写代码时知道「为什么这么写」。

### 阶段三：协议哲学与核心机制设计 (Protocol & Philosophy)
*   **目标**：解决用户反馈第 4 点，处理「被动 Hook vs 主动轮询」的实现哲学。
*   **讨论重点**：
    -   **双保险机制**：设计「被动事件驱动（低延迟）」+「主动状态调谐（高可靠）」的结合模型。
    -   成本控制：定义轮询频率的自适应算法（Idle 状态降低频率，Active 状态提升频率）。
*   **交付物**：`research/spec/03-protocol-philosophy.md`
*   **完成判据**：输出完整的伪终端（PTY）解析协议细节与状态机转移图（mermaid 格式）。

### 阶段四：任务级细化与验证规划 (Task Decomposition & TDD)
*   **目标**：将设计转化为 `Kiro` 风格的原子任务，整合 `superpowers/test-driven-development`。
*   **讨论重点**：定义每一个 Task 的单元测试或集成测试路径。
*   **交付物**：`research/spec/04-task-list.md`
*   **完成判据**：产出一份包含 `[ ]` 勾选框的任务清单，每个任务必须包含：`描述` + `涉及文件` + `验证方法`。

---

## 3. 角色分工

| 角色 | 主导阶段 | 职责描述 |
|---|---|---|
| **Master Claude (主控)** | 阶段一、四 | 负责需求拍板、任务分发、终审验收。 |
| **Gemini (分析师)** | 阶段二、三 | 负责架构评审、技术辩论、撰写 Spec 细节、发现盲区。 |
| **Codex (实施者)** | 阶段四（参与） | 参与任务可行性评估，随后根据 `04-task-list.md` 执行编码。 |

---

## 4. 防漂移铁律 (Implementation Guardrails)

1.  **Spec 优先**：Codex 发现代码实现与 `research/spec/` 冲突时，必须停止并询问，不得擅自修改 Spec 以顺应代码。
2.  **溯源要求**：在 `src/` 的核心函数注释中，必须引用 `research/spec/` 中的决策编号（例如：`// See spec/02-decisions#D3-SQLITE-WAL`）。
3.  **双保险准则**：对于所有 I/O 确认逻辑，代码必须同时包含「事件处理句柄」与「调谐巡检逻辑」。

---

## 5. 接下来的一步 (Immediate Action)

请 Master Claude 启动 **阶段一：宏观边界对齐** 的讨论。
建议讨论议题：*「ccbd-rust 接收到 Agent CLI 吐出的特定错误（如 API Quota Exceeded）时，应该是在 L2 层解析并报错，还是原样透传给 L3 处理？」*
