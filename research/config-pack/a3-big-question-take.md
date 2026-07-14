# 填补“架构良心”深层设计问题与工作小组边界判定 (a3-big-question-take.md)

本文件由 a3 worker (explore/深度调研+设计角色) 独立撰写。对照用户 2026-07-06 消息的六条更正与目标澄清（以 [user-reframe-2026-07-06.md](file:///home/sevenx/coding/ccbd-rust/research/config-pack/user-reframe-2026-07-06.md) 为准），独立攻克核心大问题：**“谁/什么来填补用户作为非资深工程师的架构经验缺口，以盯防跨功能切面的工程架构漂移？”**

本报告拒绝罗列骑墙选项，直接给出 worker 的独立判断，并剖析既有设计盲区。

---

## 一、 我对大问题的独立判断：架构良心由什么承载

**我的核心判断**：
> **“架构良心”绝对不能仅承载于单一的、活在对话上下文里的 “Architect Agent” 角色；它必须承载于一个由「显式契约声明（Living Schema）」与「硬性静态分析门禁（SCS Gates）」组成的自动化机制，并配合「人机架构后果对齐（Semantic Handoff）」共同实现。**

在 Vibe Coding 或快速迭代中，软件架构之所以迅速崩塌，核心在于：**人类在设计时存在“经验盲区”，而 AI 在实施时存在“局部优化惯性”**（如用户指出的：a 出问题改 a，不深究 a/b 共用的底座 c，留下冲突）。
要守住架构良心，必须有三层实体承载：

1. **显式契约声明 (Living Schema - 静态资产)**：
   将系统的事件订阅模式、数据模型（Schema）、API 接口定义为强类型、机器可读的声明文件（如 Protocol Buffers、OpenAPI Spec 或 Rust 强类型 Trait/Struct）。它们是代码库的一部分，不是自然语言文档。
2. **硬性静态分析门禁 (SCS Gates - 物理守护者)**：
   将“解耦边界”和“契约一致性”转化为 CI 门禁中的静态检查脚本。例如，禁止 `apps/studio` 直接 import `packages/graph-agent` 的内部非公开模块；若新改动导致公共数据模型 `c` 的 schema 发生非向后兼容改变，编译或静态分析器必须直接报错，阻止 PR 合并。
3. **人机架构后果对齐 (Semantic Handoff - 人机接口)**：
   系统在读取产品 spec 后，必须通过 AST (抽象语法树) 静态反推其改动范围，自动向非技术 PM（用户）提示其“产品行为”背后的**架构级工程后果**（例如：“此改动要求在 backend 库新增 QiniuProgress 字段，并与 S3Progress 共享同一条数据流。这会导致存储格式发生改变。你是否同意该数据一致性设计？”）。用户不需要懂技术实现，但需要被告知并授权“产品决策的架构代价”。

---

## 二、 哪部分是 SCS 级 / 哪部分 master+workers 能承接

我们必须在 SCS (Spec Coding System) 大系统与 Master+Workers 组内工作流之间切出一条极为锋利的边界：

### 1. 什么是天然属于 SCS (Spec Coding System) 级别的职责？
* **全局架构骨架提取与冲突检测**：
  在每个 Feature 开发周期启动前，SCS 必须执行全局静态分析，生成当前的“架构底座拓扑”，并与新的 Product Spec 进行语义差异对比。
  * *示例*：当 Spec 要求“添加 QiniuProgress 状态”时，SCS 需要检测出 `storage.rs` 中已有类似的 `S3Progress` 通道，主动向 PM 报警“检测到重复的数据通道设计”，避免“按下葫芦起了瓢”和重复造轮子。
* **主动生成架构变更 spec（替用户填补盲区）**：
  因为非资深工程师（用户）无法自己规划模块，**SCS 必须具备从产品 Spec 自动反推模块边界改动的能力**。它必须输出技术工单（SCS Spec），规划好本次改动需要落在哪几个模块（如：1. 修改 gateway 接口，2. 在 engine 补测试，3. 在 backend 改 adapter，4. 前端渲染），而不能让 Master 小组进入代码后“摸着石头过河、遇到边界绕着走”。
  * *自动化难点*：难以单纯依靠大模型对代码的自然语言理解。
  * *解决方向*：必须依赖**形式化的软件架构定义模型**（SCS 可以通过查询 AST 依赖图、API 路由注册表等硬性数据源来辅助推理）。

### 2. 什么是 Master+Workers 小组内部能够承接的？
* **局部契约与边界的严格执行**：
  一旦 SCS 划定了局部工单（如：“修改 gateway adapter `src/core/adapters/qiniu.rs`，实现契约 A，并编写对应的 TDD 失败用例”），小组即可在被锁死的工作空间内自驱。
* **TDD 循环与功能完备性验证**：
  编写 failing tests，写实现，跑 CI Gates。Worker 可以极其严谨地执行这些机械性的闭环任务。
* **局部代码层面的 First-principles 审核**：
  Master 在审查 Worker 的 PR 时，能够基于当前 worktree 的 diff 审计“是否只是打补丁/try-except 绕过，还是真正修改了逻辑所在的模块层”。这种局部代码健康度是 Master 可以审计的。

---

## 三、 捅穿 Naive 答案的漏洞

> **“就加一个 Architect-Reviewer Agent 站在高处审阅 PR 不就行了？”** —— 这是一个充满漏洞的幻想，在实际工程中必定会由于以下三个漏洞而被击穿：

1. **注意力漂移与上下文容量限制（Forgetfulness Limit）**：
   随着项目（如 `agent-harness`）复杂度从 MVP1 扩张到多模块、前后台混合，代码库的 Token 数量将远远超出单个 Agent 在单次 Session 里的核心注意力范围。Reviewer Agent 在看具体 PR 差异时，只能进行“局部局部审计”。它会觉得“这个 API 改动看起来很清晰，可以通过”，但它会**完全遗忘三周前在另一个模块里定下的命名规范或共享 Schema `c` 的不变性约束**。
2. **缺乏物理维度的强制惩罚手段（The Path of Least Resistance）**：
   Reviewer Agent 是被动的、非硬性的。在“只要功能能跑通，PM 急着上线前端”的交付压力下，面对 Worker 提交的略带瑕疵但测试全绿的 workaround 代码，Reviewer Agent 缺乏强力的工具去拒绝。它没有编译器那样的底气，它的判断只是自然语言建议，极易在人类和 Worker 模型的妥协中被忽略。
3. **“盲人摸象”的恶性循环（The Echo Chamber）**：
   如果 PM 自身在系统架构上存在盲区（例如：不知道 Tauri 应该跟数据库解耦），PM 给出的 Feature Spec 就会包含结构性的架构错误（例如直接在前端写 SQL 逻辑）。Reviewer Agent 如果以 PM 的 Spec 为真理源，它只会确认“Worker 写的代码确实完美实现了 Spec 中的 SQL 逻辑，Review 合格”，从而把一个结构性缺陷当成正确交付合并进 `main`，加速项目的腐烂。

---

## 四、 小组在大框架里的位置与边界

### 1. 大框架（SCS 契约与守护层）
* **大框架是“规则的物化注册表”**：
  在代码库中维护一份 `.ah/architecture.json` 或一系列强 Schema 文件（如 OpenAPI yaml, protobuf）。
* **大框架是“无情的门禁”**：
  在 GitHub CI 门禁中加入静态架构校验工具（如架构依赖检查、Data Schema 兼容性 Diff 工具）。
* **SCS 负责开工前的工单规划**：
  SCS 先于小组启动。它接收人类 PM 的需求，查询代码资产，产出“架构变更 work order”，划分好模块边界。

### 2. 工作小组（Master+Workers 实施工厂）
* **定位**：
  小组是 **SCS 契约的忠实执行者与局部质量把关人**。
* **输入**：
  SCS 规划好的“模块工单 + Schema 契约”。
* **工作空间**：
  严格限制在专属的 Git Worktree 中。
* **边界**：
  * **Master+Workers 只能在 SCS 给定的“局部沙箱”内编程**。
  * 如果 Worker 试图在实现中修改公共库以偷懒，或者破坏了模块解耦边界，它在本地跑 CI 门禁或推送到 PR 时，大框架的静态门禁会**无条件直接拒收**。
  * 小组不需要，也不应该承担“跨功能全局一致性”的宏观重任，他们只需要在 SCS 给定的战壕里，把单兵战术质量（TDD、物理验证、视觉看齐设计）做到极致。

---

## 五、 小组内实现手段的初步映射 (Skill / Hooks / MCP / 规范守护)

针对用户消息六提出的“仅靠 `CLAUDE.md` 和 memory 太脆、易失焦”问题，我们需要将协作规矩从“嘴上说说”硬化为以下具体工具：

### 1. 沉淀为 Skill (工具化常用命令)
* **`verify-ui-spec` Skill**：
  将 `FRONTEND_UI_SPEC.md` 中关于圆角、间距、shadcn/ui 使用的静态检查，做成一个 CLI 分析 Skill。当 Worker 编写完 CSS/Tailwind 时，自动扫码，发现 hex 颜色或非语义化 token 直接报错。
* **`tdd-validator` Skill**：
  提供一个分析工具，读取当前 worktree 的 git history，验证是否存在“先提交 failing test，再提交 production code”的轨迹，或者至少在 `tests/` 下有新加的用例，防止 Worker 蒙混过关。

### 2. 用 Hooks 保证运行期可靠性
* **Git Pre-commit Hook**：
  在人类或 Worker 提交代码前，自动运行 `ruff check`、`mypy` 与本地轻量测试。
* **Worktree Boundary Hook**：
  限制 Worker 只能在 `.worktrees/task-*` 目录下执行写操作，严禁越权污染主仓根目录或其它并发任务的 worktree。
* **`ah` Daemon Hooks**：
  利用现有的 `UserPromptSubmit`（切换 BUSY 状态）和 `Stop`（切换 IDLE 状态）硬钩子，在事件广播中加入 **Sandbox-Sanity 检测**。如果检测到 Worker 的 `CLAUDE_CONFIG_DIR` 被设为了相对路径，立即拦截并触发异常警报，拒绝派单，杜绝僵尸进程。

### 3. 用 MCP Server 提供权威真理源与上下文检索
* **Architecture MCP Server（核心）**：
  Worker 在编写代码前，禁止其在 195MB 的大文件或数百个源码文件里盲目 grep。Worker 必须调用自定义的 Architecture MCP 接口：
  * `get_module_interfaces(module_name)` -> 返回该模块导出的强契约接口及类型定义；
  * `get_active_schemas()` -> 从数据库/API 定义中返回所有活跃的事件与数据结构规范。
  这确保了 Worker 拿到的是无幻觉的、绝对准确的底层架构契约。
* **Git State MCP Server**：
  提供工作区与 worktree 并发状态的检索，协助 Master 调度时精确判定哪些文件正在被其它 worktree 独占，从而在调度层避免并发冲突。
