# agent-harness 2026-04-20 Session Findings

### 1. CCB / Claude / Gemini / Codex 的 bug 或失败行为

- **[19:46]** Codex 在比较 temp/graph_agent 与 src/core/graph_agent 时，把 __pycache__ 中的 .pyc 文件（编译时间戳不同）单独列出来"解释为什么不同"，属于无意义噪声，本应直接过滤。
- **[20:35]** `python3 -m graph_agent.runner` 命令报 `No module named graph_agent.runner`——pyproject.toml 里 `[project.scripts]` 写的是 `graph_agent.core.runner:main`，而 `__main__.py` 也指 `core.runner`，但用户/Claude 按 README 风格调 `graph_agent.runner` 直接失败。文档/入口路径不一致是 graph_agent 自身的发布问题。
- **[20:35]** 运行日志里反复出现 `[Bridge] Tool input JSON parse failed in segment: Expecting property name enclosed in double quotes: line 1 column 2 (char 1)`——graph_agent 的 callback_bridge 解析工具输入 JSON 失败但仍能往下走，属于 silent recoverable bug，未上报到结果层。
- **[20:35]** `[Harness] Auto-checkpointer failed, running without: cannot import name 'override' from 'typing'` —— graph_agent 在 Python 3.10 下 checkpoint 自动降级，触发警告但 README/pyproject 没说明 3.12+ 的硬约束在 checkpoint 行为上的具体后果。
- **[20:35]** `[ReasoningPatch] OpenAI SDK v2.21.0 detected — skipping model_config patch (only tested with v1.x). reasoning_content may not be preserved.` —— graph_agent 与 openai SDK v2 不兼容的已知降级路径，靠 warning 提示但没有 fix。
- **[21:22]** 用户转述：DeerFlow 原生 task_tool 在 Sandbox 里用 `run_skill()` 独立测试某 Node 时崩溃 `AttributeError: 'NoneType' object has no attribute 'get'`（task_tool.py:88 `runtime.context.get("thread_id")`），原因是 `runtime.context` 为 None；后续 grep 发现总共 7 个 deerflow 文件有同类裸 .get/[] 访问。
- **[21:37]** Bash heredoc 的多个 cp 命令一次性写多行未挂 `&& echo done` 时，输出顺序混乱难判断成功；后改为分步骤跑。（操作小颠簸而非真 bug）
- **[23:13]** Claude 调用 `kiro:spec-design` 时，cwd 是 agent-harness，但用户给的 path 是 AI-story-forge 的 spec 路径——Claude 直接把设计写进 AI-story-forge 而不是 agent-harness，体现 kiro 命令对"目标 spec 跨项目"场景没有保护，Claude 也没主动确认目标项目。
- **[23:41]** `kiro:spec-init` 在 agent-harness 内运行，发现 `.kiro/settings/templates/specs/` 不存在，是因为 agent-harness 从未跑过 `/kiro:steering`；spec-init 应该在缺失模板时给出更清晰的引导而不是直接 Glob 0 命中后报"No files found"。

### 2. 用户纠正 / 抱怨 / 吐槽 Claude 的内容（带原话）

- **[20:52]** 用户中断 Claude 的 Agent 工具调用：原话"[Request interrupted by user for tool use]"，因为 Claude 直接派 sub-agent 去 explore，用户希望先看 Claude 自己的判断再让 Gemini 评判。紧接着原话"你来判断一下合不合适，可以让Gemini也来参与评判一下"——纠正 Claude "不要默认派 subagent，要先自己判断"。
- **[21:29]** 用户原话："你不用管测试的东西，你的任务聚焦在把graph_agent修到没有bug" ——Claude 在追问"sandbox 测试目录的 ThreadPoolExecutor 是否要替换"时被打断，用户明确把任务范围收窄。
- **[23:40]** 用户原话："你当我傻啊，你自己看看有没有.kiro？" ——前一回合 Claude 解释 .kiro 是隐藏目录、教用户怎么显示隐藏文件，但用户的真实问题不是"看不见"而是"我让你写到 agent-harness 里你写到 AI-story-forge 去了"。Claude 没听懂用户实际的诉求，被反讽。
- **[23:41]** 用户原话："我要你吧需求写到这个项目里面啊。。。" ——明确指出 Claude 写错了项目，连续两次技术性回答没击中诉求让用户烦躁（"。。。" 语气）。

### 3. 用户强意图（带原话）

- **[19:54]** "合并，然后在这个项目不改graph agent能将他跑起来吗？" ——决定把 temp 版 resolver.py 合到 production，并要求验证不修改 graph_agent 的可运行性。
- **[20:35]** "随便试一个" ——授权 Claude 选 skill 跑端到端验证（被理解为 text-segmentation）。
- **[20:52]** "我希望把这两者也合并到graph_agent里面作为内置功能" ——内化意图：把 ArtifactManager + DataManager 合进 graph_agent 作为内置；后被 Gemini + Claude 共同否决了 DataManager。
- **[21:24]** "把这个需求一并加入" ——让 Claude 把"修 task_tool NoneType crash + 把手写 ThreadPoolExecutor 替换回原生"两件事并入当前任务。
- **[21:29]** "你不用管测试的东西，你的任务聚焦在把graph_agent修到没有bug" ——任务范围明确收窄到 graph_agent 本体。
- **[21:37]** "把这个项目配置一下" （指 0_claude-code-starter-kit）——要求把那个 starter-kit 配置成可用的 .claude/ 部署样板。
- **[22:17]** "1.要的" ——明确要把 kiro 命令复制到全局 ~/.claude/commands/。
- **[23:13]** "用kiro写一下这个需求" ——用 kiro 流程为 native-skill-wrapper 生成正式 design.md。
- **[23:41]** "直接创建" ——跳过 /kiro:steering 直接创建 spec。
- **[00:10]** "现在要讲两个graph_agent合并到一起，根据你得到的信息分析应该怎么做" ——明确转入合并策略制定阶段。
- **[00:10]** "看下说的有没有道理，然后让Gemini也评判一下你给的修改, 是否靠谱？" ——要求把"用户分析 + Claude 合并方案"一并送 Gemini 做交叉评判。

### 4. 对话中暴露的设计缺陷

- **[20:35]** `python -m graph_agent.runner` 与 `python -m graph_agent`、`graph-agent` 三种入口存在不一致；只有 `graph-agent` 脚本和 `python -m graph_agent` 可用，文档里 docstring 却写 `python -m graph_agent.runner`——入口元数据散落多处且不一致。
- **[20:35]** `callback_bridge` 在 LLM 给出非标准 JSON 的工具调用时，反复 WARNING 但没有计入 metrics 或回报到 result——属于"silent recoverable failure"，违反用户要求的 logging 原则（每个降级必须可观测）。
- **[20:52]** ArtifactManager 公共签名里硬塞 `project_id` 参数，但 graph_agent 是跨项目通用框架——这是已发现的耦合点，本来就不该在通用模块里出现"宿主项目 ID"。
- **[20:53]** DataManager 同时绑了"`story_forge.core.config` 类导入" + "`config/pipeline.yaml` 路径硬编码"两个宿主依赖，且这两个依赖都用 `Path(__file__).parent.parent.parent.parent` 这种向上跳目录的方式定位——硬编码相对路径放在宿主项目目录结构里，移植即崩溃。
- **[21:22]** DeerFlow `task_tool` 设计假设 `runtime.context` 总是非 None，但 graph_agent 暴露的 `run_skill()` 入口又在 sandbox/独立测试场景下不构造完整 LangGraph runtime——两层接口对 runtime 完整性的预设不一致，导致独立测试场景必崩。
- **[21:24]** 用户为了规避 task_tool 崩溃，写了手写 ThreadPoolExecutor 子任务并发 ——这个绕过本身就反映了 graph_agent 在"声明式并发"上没有可靠原生方案，开发者被迫造轮子。
- **[23:13]** Claude 在 cwd=agent-harness 时执行 `kiro:spec-design`，但用户给的目标 path 是 AI-story-forge——`kiro:spec-design` 在多项目场景下没有"目标 spec 应在哪个项目"的明确归属判断，跟 cwd 解耦不彻底。
- **[23:41]** `.kiro/settings/templates/specs/` 在 agent-harness 中缺失，要靠 Claude 手动从 AI-story-forge cp 一份过来——starter-kit 没把 kiro 模板纳入项目初始化标准流程，每个新项目都要重做一次。
- **[23:18]** SF (AI-story-forge) 反向修改了 graph_agent/deerflow/ 7 个文件，删除了 `(runtime.context or {})` 防御——违反"deerflow 是受保护区，永远不碰"的项目规则；说明项目当前没有 enforcement 机制（没有 protected_zone hook 在 SF 项目里生效，或开发者绕过了 hook）。
- **[23:18]** 两个项目的 graph_agent 各自演进，SF 多出 3 套新特性（Sub-skill / MD Parser / V2 Schema Tag），但 spec 文档只有 1 个完整四件套（pipeline-tool-call-refactor），native-skill-wrapper 只有 design.md，ToolMetrics 日志和 deerflow 改动完全没有 spec——spec-driven 流程在多项目并行开发中被绕过。
- **[23:18]** AH 的 `tasks.md` 里 §A.1（md-patch 迁移到 builtin）、§C.1（compiler 识别 schema tag）已实际完成但仍标 "待实现"——tasks 状态与代码状态脱钩，没有自动同步机制。

### 5. 决策转折点

- **[19:46]** Codex 给出 diff 结果（仅 resolver.py 不同）→ 用户决定合并并继续验证可运行性。这个轻量 diff 让"先合再跑"路径成立，否则要先做更大改动评估。
- **[20:35]** text-segmentation skill 跑通（101s, 3 phases, in=37770/out=5454）→ 验证 graph_agent 在 agent-harness 这个项目里"零修改可运行"，奠定后续把它视作稳定子模块的判断。
- **[20:53]** Gemini + Claude 共同结论"ArtifactManager 可合并（去业务化）+ DataManager 不能合并（硬依赖宿主）" → 用户后续没有强行让 DataManager 进入 graph_agent，遵从了架构判断，体现 Gemini 在领域专家角色的有效性。
- **[21:31]** 用户"你不用管测试的东西，聚焦把 graph_agent 修到没 bug" → Claude 立刻收窄范围，对 graph_agent 内部所有 7 处 `runtime.context` 裸访问统一加 `(or {}).get()` 防御。这是从"探索 -> 修复"的决定性转折。
- **[21:56]** `present_file_tool.py` 那一处不修（已有 `if not thread_id: raise ValueError`，故意行为）→ Claude 没有盲目"全部统一加防御"，识别出语义意图，是好的 case-by-case 判断。
- **[22:17]** 用户决定把 kiro 命令安装到全局 `~/.claude/commands/kiro/` → 把 starter-kit 的 kiro 流程从"项目级"提升到"用户级默认能力"，影响后续所有项目可直接 `/kiro:*`。
- **[23:13]** 用 kiro 流程写 native-skill-wrapper design.md（而不是直接交付实现）→ 把"特性需求"正式纳入 spec-driven 流程，但同时暴露了 cross-project 路径混乱的问题。
- **[00:10]** 用户提出"两个 graph_agent 要合并到一起" → 把之前两条并行的演化路径（AH 修 deerflow 防御 / SF 加新特性）正式合并为一条主线决策。
- **[00:13]** Gemini 评判明确支持"立即丢弃 SF 对 deerflow 的违规修改，迁移 3 个新特性到 AH" → 形成最终合并策略共识。这是用 Gemini 做交叉评判后用户接受的决定性转折，也证明了"领域问题先过 Gemini 不直接交用户"这条规则在本 session 里被遵守。

---

核心主题：以"graph_agent 框架在 agent-harness 项目能否零修改运行"为入口，逐步暴露 deerflow `runtime.context` 防御缺失、跨项目 graph_agent 演化分叉（AH 加防御 vs SF 删防御 + 加新特性）、ArtifactManager/DataManager 是否能合入框架等架构边界问题，最终通过 Gemini 交叉评判形成"以 AH 为主线，丢弃 SF 对 deerflow 源码的违规修改、迁移 SF 的 Sub-skill / MD Parser / V2 Schema Tag 三套新特性"的合并策略。
