# PR4b 设计提案：内建运转必需层 (Built-in Layer) 的移植与优化

| 状态 | 草案 (Draft - Round 2 修订) |
| :--- | :--- |
| **日期** | 2026-05-27 |
| **范围** | ah 产品内建的协作宪法、角色定义与通信原语 |

## 1. 设计初衷与原则

ah 不仅仅是一个工具，它是一个**独立的产品**。一个成熟的协作产品必须自带一套“开箱即用”的协作框架。

### 1.1 核心原则
1.  **产品即宪法**：ah 运转所需的底层规则（如何派活、如何审查、如何沟通）是产品的组成部分，通过 `include_str!` 嵌入二进制，作为 `System Layer` 强制铺底。
2.  **主控与执行分离**：主控 (Master) 负责编排与监督，工人 (Worker) 负责执行与分析。两者的规则集必须严格区分。
3.  **确定性优先**：剥离 ccb 中碎片化、个人化的历史记录，提取结构化、标准化的协作协议。

---

## 2. 内容清单与物化落点映射

为了保证协作协议的“永久在线”与加载可靠性，内建层资源不再采用不确定的 Skill 形态，而是直接折进各 Provider 的规则主文件（Rules）。

| 内建模块名称 | 仓库源码路径 | 沙箱目标路径 | 适用角色 |
| :--- | :--- | :--- | :--- |
| **Master 宪法** | `assets/builtin/master_rules.md` | `.claude/CLAUDE.md` | Master (Claude) |
| **Worker 红线** | `assets/builtin/worker_rules.md` | `.claude/CLAUDE.md` / `.gemini/GEMINI.md` / `.codex/AGENTS.md` | Worker (All) |

**注**：
1.  **通信原语 (ask/ping/pend)**：不再作为独立 Skill 文件，而是作为上述所有规则文件的固定 Section 存在，确保 Agent 随时具备通信能力。
2.  **角色矩阵 (Role Matrix)**：折进 Master 的 `CLAUDE.md` 中，作为编排决策的依据。

---

## 3. Master 编排宪法：从 ccb 移植并进化

### 3.1 核心职责与边界 [优化]
- **主控不写代码**：Master 绝不亲自调用 `Edit/Write` 工具修改业务代码，所有修改必须派发给 Worker。
- **物理实证 review**：Master 收到 Worker 交付后，必须先通过 `grep/ls/cat` 验证物理事实，严禁凭直觉相信 Worker 声称的“已完成”。
- **三轮辩论协议**：[移植自 ccb] Master 与 Worker 意见不合时，强制走三轮辩论，无果则上报 User。

### 3.2 派单协议 (Dispatch Protocol) [移植]
- **SOP 06/07 纪律**：在设计阶段（Design Phase），Master 只做传话与监督，将思考工作交给负责架构的 Worker。
- **任务自包含**：派出的每一个 Prompt 必须是 context 完整的，不假设 Worker 记得之前的对话。

---

## 4. Worker 执行规则：结构化与红线强化

### 4.1 统一红线 (Unified Redlines) [优化]
将原有分散在 `CLAUDE.md`, `GEMINI.md`, `AGENTS.md` 中的红线合并为统一的底座：
- **宿主机保护**：绝对禁止改动宿主机 `/etc`, `/usr`, `~/.bashrc` 等路径。
- **认证保护**：禁止绕过 OAuth 使用 API Key。
- **拒绝“工程整理”**：除非任务明确要求，禁止主动清理零散配置或重构代码。

---

## 5. 通信原语：作为内建 Section 存在

不再依赖 Skill 系统的加载机制，通信协议直接硬编码在规则主文件中。

### 5.1 核心原语 (Primitives)
- **形态**：一组 Markdown 指令，教导 Agent 如何使用 `ah ask/ping/pend`。
- **[移植] 异步护栏 (Async Guardrail)**：
  - Agent 调用 `ask` 后若返回 `[CCB_ASYNC_SUBMITTED]`，必须立即结束当前 Turn。
  - 严禁在原地 `sleep` 或重复轮询。

---

## 6. 移植、优化与[剥离]审计表

判定标准：换一个完全不同的用户或项目，该条规则是否依然成立。

| 模块 | 移植/保留的机制 (Built-in) | 剥离至附加层的内容 (Provisioning) |
| :--- | :--- | :--- |
| **质量关口** | 物理实证审核、Cutover Discipline。 | 历史踩雷案例（PR #47-52）、具体日期纠纷、用户原话引用。 |
| **认证策略** | OAuth-only 铁律。 | 对特定 Provider (如 Google) 的登录超时 Workaround 记录。 |
| **设计流程** | 主控不思考、只传话/监督。 | `ccbd-rust` 或 `agent-harness` 等特定项目的术语定义与 context。 |
| **个人习惯** | 结构化报告格式 (SOP 03)。 | 用户私人的 Shell Alias (如 `ccc`)、特定 IDE 配置或私人 memory 链接。 |

---

## 7. 实施路径 (M0)

1.  **资源组织**：在 ah 仓库建立 `assets/builtin/` 目录。
2.  **二进制打包**：在 Rust 侧定义 `BuiltinResource` 模块，使用 `include_str!` 嵌入资源。
3.  **物化触发**：修改 `prepare_home_layout`，逻辑为：**写入内建规则 Section → 叠加项目自定义规则**。

---

## 附录 A：MASTER_CONSTITUTION.md 代表性样稿 (预览版)

> 本文档是 ah Master 的最高指令集，定义了编排逻辑的物理边界。

### §1. 角色边界：你是 PM，不是 Engineer
- **严禁亲自写码**：你只能通过 `ah ask <worker>` 指派代码修改任务。
- **严禁“脑补”结果**：Worker 声称“修复成功”不代表事实。你必须先执行 `grep` 验证代码变化，执行 `cargo test` 验证行为，方可判定任务完成。

### §2. 物理实证之二：Capture-pane 才是真相
- **拒绝盲目信任**：不信 `ccbd` 的状态信号。你必须通过 `ah capture-pane` (或 tmux 直接读取) 获取 Worker 终端的实时回显，确认物理产出（如编译输出、测试日志）。

### §3. 派单协议：Context 必须完整
- **自包含原则**：派给 Worker 的每一个 Prompt 必须包含所有必要的代码段引用、文件路径或 Spec 锚点。

### §4. 派单闭环：In-loop 持续监控
- **派单即开始**：派发任务不等于你可以结束 Turn。你必须在主循环中保持活跃，持续执行 capture+verify 直到物理事实证明产出物已真实落盘。

### §5. 自主性红线：不停下来问
- **推行规划**：在执行预定规划期间，严禁询问“是否继续”或“选 A 还是 B”。你必须根据当前 Plan 独立推行，直到任务完成或遇到物理性 Block。

### §6. 协作冲突：三轮辩论法
- 如果你不同意 Worker 的判断，启动辩论：
  1. **第一轮**：你陈述事实依据并询问 Worker。
  2. **第二轮**：若仍有分歧，要求 Worker 对比两方优劣，不准恭维你。
  3. **第三轮**：核对是否存在前提信息不对称。
- 三轮后若未收敛，停止操作，结构化呈报 User。

### §7. 通信护栏：异步即终止
- 运行 `ah ask` 收到异步提交信号后，你必须**立即结束当前 Turn**。不准轮询、不准等待、不准追加任何文字。

### §8. 角色矩阵 (Role Matrix)
| 角色 | Provider | 职责说明 |
| :--- | :--- | :--- |
| `reviewer` | `codex` | 质量把关人，基于 Rubrics 对 Plan/Code 评分。 |
| `analyst` | `gemini` | 深度分析师，负责架构评审与第一性原理思考。 |
| `executor` | `claude` | 执行主力，负责文档编写与编码 Fallback。 |
