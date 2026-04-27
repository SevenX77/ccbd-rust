# agent-harness 2026-04-22 Session 结构化清单

> 来源：`/home/sevenx/coding/ccbd-rust/research/sessions/agent-harness/markdown/2026-04-22-session.md`
> 项目：`/home/sevenx/coding/agent-harness`（graph_agent + Skill Studio 设计讨论）
> Session ID：`a825a415-ba1a-47a9-b331-ccb733a6eb96`

---

### 1. CCB / Claude / Gemini / Codex 的 bug 或失败行为

- **[04:48]** Claude 在落盘 plan 前连续触发 `API Error: Stream idle timeout - partial response received`（出现至少 4 次），用户多次回复 "继续"/"又报错了"/"又报错"，最终对话被 API 超时打断，落盘失败（按 plan.md 自身记录第 5 轮）。
  - 用户原话："又报错了" / "又报错"

- **[06:48]** Claude 用 Bash + 双引号 + heredoc 提交 Gemini prompt 时，shell 在参数解析阶段把 `<step>` `<if>` `<else>` `<loop>` `<parallel>` 当成重定向操作符吃掉，导致 Q1 文本到达 HEREDOC 之前就已残缺（"是否引入 / / / / 分支？"）。Claude 自我诊断错根因（一开始以为是 HEREDOC 单引号问题，实际上是反引号在 prompt 里被当成命令替换）。
  - shell 报错原文：``/bin/bash: command substitution: line 61: syntax error near unexpected token `newline'``

- **[06:53]** Gemini CLI 把 prompt 收到了输入框但没自动回车，pane 底部停在 `> [Pasted Text: 69 lines]` 等人按 Enter；`ask gemini` 进程在后台 `do_pol` 轮询等待挂载状态，看起来在工作实际卡死。Claude 检查 `ccb-mounted` 返回空但 Gemini CLI 自身在线 —— 挂载状态短暂脱同步。

- **[07:03]** 用户 `kill 1510044 1510048` 杀外层 shell 后，CCB askd daemon 端 task #2（`20260422-064834-159-1362334-2`）的 `start` 记录没有对应的 `done`，daemon 按 provider 串行化新任务 #3 永久排队。需要重启 askd daemon 才能恢复。

- **[07:07]** 用户重启 askd daemon (pid 1362334) 后 supervisor 自动 respawn 新 daemon (pid 1518190)，但 `ccb-mounted` 一段时间内仍返回 `{"mounted":[]}`，pane title marker 一度丢失，靠 `ccb-ping gemini --autostart` 才确认底层通道已通。

- **[09:08]** 用户更新 CLAUDE.md 后，`.ccb/ccb.config` 改成 `cmd, agent1:codex; agent2:codex, agent3:claude`（Gemini 不在配置里），`ask gemini` 直接报 `unknown agent: gemini`。Claude 没意识到 ccb 已改用抽象代号。

- **[19:35]** 同一类问题再次出现：Claude `ask gemini` 还是 `unknown agent: gemini`；同时本机 `ccb-mounted` / `ccb-ping` 命令直接 `command not found`，整个 CCB 工具集已迁移到 `ccb` 子命令。Claude 摸索后才发现新版用 `a1/a2/a3` 命名（`cmd, a1:codex, a2:gemini, a3:claude`）。

- **[23:33]** Codex 评审输出文件里有奇怪现象：reply 文本被重复了一份且第二份带空格污染（"件套" 单字开头、字间夹空格），疑似 ccb pend 拼接历史的 bug 或 reply trace 重复事件 emit 导致内容拼接两次（log 行 5-9 显示 `completion_item` 多次触发 + `completion_state_updated` 多次）。

- **[全局]** Claude 在 plan.md / Kiro spec 里凭脑子写 SkillManifest（扁平 step）和 CallbackEvent 14 钩子，**完全没核查代码**：现有 SKILL.md 用 `<ref path="nodes/*.md"/>`、`callbacks/base.py` 实际只有 12 个事件 —— Codex 评审才打回来。属于"工程地基里的幻觉引用"自我 bug。

---

### 2. 用户纠正 / 抱怨 / 吐槽 Claude 的内容（带原话）

- **[06:46]** 用户纠正 "PM 改 Markdown 是陷阱" 这套思路，原话："不要用传统软件思维来考虑，我都有copilot了为啥还要表单？也不用担心markdown过于工程化，markdown是为了能够快速输出逻辑严谨的workflow，不用再去写python，因为写python会带来各种各样的工程问题，而我们现在是要把工程问题全部内聚到graph agent核心，是为了可靠性和效率。哪怕引入if/else又怎么样呢，copilot就是engineer assistant，把自然语言翻译成工程语言，甚至pm只是抛出问题，copilot通过规范化的范式解决问题。其他技术问题你和Gemini讨论"
  - 隐含吐槽 Claude 和 Gemini 都陷入"传统软件思维陷阱"

- **[07:49]** 用户对 when 表达式不熟，原话："when是干嘛用的？什么情况Claude code sdk会挂？" —— 暴露 Claude 直接抛技术名词没解释；同条消息里抛挑战："这个产品很像coze和comfy ui，我觉得这类产品最终肯定要变成画布和节点式的展现会更加清晰" 直接挑战 Gemini 上一轮 "React Flow 绝对只读" 的红线。

- **[08:31]** 用户最严厉的方法论批评，原话："这个路线图过于产品化，有点闭门造车的感觉，我需要的p1是能让pm快速干活儿的，哪怕你接个llm聊天框，pm和他聊完，结果自己复制到md文件，然后点击校验运行compiler，告诉pm哪里有问题自己去按照报告去改这样都行。因为我不知道copilot会存在哪些问题，怎样的快速实现是最靠谱的，甚至于我只要点一个按钮就能在这个skill工程目录打开一个Claude codecli终端也行。你能明白我的意思吗？还有一个问题，如果用rust重写整个项目，对于长远来说有没有好处"
  - 明确抱怨 Claude+Gemini "过于产品化 / 闭门造车 / 想完美版本"

- **[23:53]** 用户对 spec fail 的回应，原话："1.项目里的skill就是用来实验的,随便改;2.不同的pm应该要user ID隔离,每个pm 账号下有自己的skill, 互不影响;3.输入输出以及版本管理有讨论过吗?core里面有datamanager和artifactsmanager,我觉得可以把他们融入graph_agent作为标准文件落盘工具;4.你问我的前4个问题我无法回答,问Gemini他是怎么看的;最后一个问题肯定是走重审"
  - 隐含吐槽：第 3 句 "有讨论过吗?" 揭示 Claude 写 spec 完全没想到融入现有 DataManager/ArtifactManager，是漏掉的盲区。
  - 第 4 条 "你问我的前4个问题我无法回答" 反映 Claude 把工程细节问到了用户，违反"前提是 Gemini 能回答的不打扰用户"原则。

- **[隐含贯穿全场]** 用户多次以"其他技术问题你和Gemini讨论"、"问Gemini他是怎么看的"切回，反复提示 Claude 不要把工程决策推给用户。

---

### 3. 用户强意图（带原话）

- **核心哲学（[06:46]）**：SKILL.md 是 Copilot ↔ 引擎的工程 DSL 接口，不是 PM 阅读界面。原话："markdown是为了能够快速输出逻辑严谨的workflow，不用再去写python ... 把工程问题全部内聚到graph agent核心，是为了可靠性和效率"

- **MVP 思维（[08:31]）**：P1 不是完美 Copilot，是"让 PM 快速干活"的最小集合。原话："我需要的p1是能让pm快速干活儿的，哪怕你接个llm聊天框，pm和他聊完，结果自己复制到md文件 ... 甚至于我只要点一个按钮就能在这个skill工程目录打开一个Claude codecli终端也行"

- **Rust 终止（[08:49]）**：原话："按档位A。rust问题终止。落盘文档，走superpower+kiro标准流程"
  - 双重指令：(1) 选 P1 档位 A，(2) Rust 重写不再讨论。

- **走标准流程（[08:49]）**："落盘文档，走superpower+kiro标准流程"

- **Skill 用来实验（[23:53]）**：原话："项目里的skill就是用来实验的,随便改" —— 授权 Claude 做 breaking change（合并 nodes/*.md 进 SKILL.md）。

- **PM 隔离（[23:53]）**：原话："不同的pm应该要user ID隔离,每个pm 账号下有自己的skill, 互不影响" —— 直接定 P1 多租户基线。

- **DataManager 融入（[23:53]）**：原话："core里面有datamanager和artifactsmanager,我觉得可以把他们融入graph_agent作为标准文件落盘工具" —— 给出明确架构方向。

- **重审（[23:53]）**：原话："最后一个问题肯定是走重审" —— Codex fail 后必须 fix→re-submit，不放过。

- **Gemini 审阅（[09:08, 19:35, 23:31]）**：用户三次主动要求 "让Gemini审阅一遍" / "先把kiro spec发给Gemini & codex 看一下,分析一下有什么问题" —— 强烈倾向用 AI 协作做评审而不是自己审阅。

- **审阅前先确认 CCB 状态（[23:28]）**：原话："check一下ccb的状态" —— 多次卡过 ccb 后已经形成"先验状态再发 ask"的肌肉记忆。

---

### 4. 对话中暴露的设计缺陷

- **[06:48]** CCB ask 子命令的 prompt 通道存在 shell quoting 死结：用户/Claude 必须知道 `<>` `\`` 这类 prompt 内字符会被 shell 解析掉。CCB 没有 stdin-only 接口（早期），没在 `ask <provider> <message>` 文档里警告这点。

- **[06:53]** CCB tmux pane 模式存在严重的"投递不等于发送"间隙：text 粘进 Gemini 输入框后必须人按 Enter，才会真正发送；而后台 `ask` 进程默认假设投递成功，开始计时等回复，等到 timeout 了用户才知道卡住。需要主动 `tmux capture-pane` 看 pane 底部是否还有 `[Pasted Text: N lines]`。

- **[07:03]** CCB askd 的串行化队列对"被 kill 的 task"无 garbage collection：外层 shell 死了，daemon 端 task #N 还是 `start` 状态，后续 task #N+1 永远等不到 #N 的 done 信号。需要靠重启 askd daemon 解。这是 daemon 的状态机有 bug。

- **[09:08, 19:35]** CCB 升级到 v6.0.7 后，agent 名字改用项目 `.ccb/ccb.config` 里的抽象代号（`a1/a2/a3`），但旧 ask 客户端报错只说 `unknown agent: gemini`，没提示"试试 a2"或"看 ccb.config"。`ccb-mounted` / `ccb-ping` 等老命令直接 `command not found`，没有迁移提示或兼容层。

- **[plan.md 顶层]** Claude 凭印象写 spec 没核查代码 —— `callbacks/base.py` 实际有 12 个事件，design.md 声称 14 个，凭空多出 `llm_fallback` `validator_start/end` `subgraph_start/end` 等。这反映 Claude 在写 spec 前缺少强制的 "code-grep before claim" 校验流程。

- **[design.md vs SKILL.md]** Claude 设计的 SkillManifest（扁平 `phases: list[PhaseConfig]`）和现有 5 个 skill 的实际结构（`<node><ref path="nodes/*.md"/></node>` 外部引用）完全不兼容。spec 里的 phase 概念跟代码里的 node+ref 双层结构是平行的两个心智模型。

- **[design.md Non-Goal vs R4]** "不改 DeerFlow 源码" Non-Goal 和 R4 "Prompt Capture 必须在 DeerFlow LLM 调用点埋点" 在技术上互斥。Claude 没识别出这个矛盾，Codex 才点出。

- **[research.md D2 vs R2 AC2]** R2 AC2 要求 AST 反向序列化"字节级幂等"，research.md D2 自承"换行/空白要和 Claude Code CLI 手写风格对齐否则 diff 噪声"——字节级幂等与人工手写风格本质冲突，目标不可达成。

- **[R7 AC5 vs R11 / D6]** R7 AC5 要求 "同时支持 gemini CLI 作为备选"，R11 和 research.md D6 又明确否决 Gemini CLI 作为 Copilot fallback。同一份 spec 自相矛盾。

- **[全局协作]** Claude 把工程决策（SkillManifest 怎么调和 ref / 事件清单怎么重写 / Prompt Capture 怎么解 / P1.5 阈值时机）打包问用户，用户回复 "前4个问题我无法回答,问Gemini" —— Claude 没遵守 "Pre-Escalation Self-Check"（先过 Gemini 才升级用户）。

- **[全局协作]** Claude 也没遵守 ccb-collaboration.md 的"主控内部分派默认 `--wait`"原则，前几轮全部用 async + `Gemini processing...` 模式，导致 prompt 损坏后才发现，浪费多次往返。

- **[Gemini 输出]** Gemini reply 多次出现"重影"现象（同一段中文被复述两份，第二份字间夹空格），疑似 ccb 流式拼接时把 `completion_item` 多次事件累加。这会让大文本评审时下游解析麻烦。

- **[审阅工作流]** Plan/Code Review framework 要求 reviewer=Codex，但 Claude 第一反应是用 Gemini 评审，靠用户引导才补上 Codex 的双评审。说明 framework 在 master Claude 视角不够显眼。

---

### 5. 决策转折点

- **[04:48]** 启动：用户要求"看一下有没有一个叫plan.md的文件" → 进入对 plan.md 的深度评估流。Claude 决定先 ask Gemini 做架构分析，进入"主控内部分派"模式。

- **[06:46]** **核心哲学纠正（最大转折）**：用户拒绝 "PM 改 Markdown 是陷阱" 的论点，重定义 SKILL.md 为 "Copilot ↔ 引擎的工程 DSL"。Claude 和 Gemini 上一轮的多个论点（"If/Else 是临界点"、"用表单妥协"、"DSL 警戒线为人类可读性"）全部被推翻。落地动作：写入 `.claude/projects/.../memory/feedback_markdown_as_engineering_dsl.md`。

- **[07:22]** Gemini 第二轮全面对齐新哲学，撤回 "PM Markdown 编辑" 担忧，强化 "React Flow 只读"，重定义 DSL 警戒线为 "Copilot 单轮 context 内稳定 diff 极限"。提出 P0.5 Copilot Core 必须先于 P1。

- **[07:49]** 用户挑战 "画布 vs 文本 DSL" 红线，引用 Coze / ComfyUI。Claude 把它升级为"真正的架构分歧"打包问 Gemini。

- **[07:54]** Gemini 第三轮提出 **"Topology vs Content 分工 + AST 单一事实源"** 双解，部分撤回上一轮"React Flow 绝对只读"。引入 "Pydantic ↔ SKILL.md 反向序列化" 作为 P0 必做工程量。胖节点 vs 瘦节点二分法精准解释 graph_agent 不能照搬 Coze。

- **[08:31]** **MVP 转折**：用户严厉批评 Claude+Gemini 的 "P0.5 Copilot Core / Golden 50 / 完整 SDK 集成" 路线图为"过于产品化、闭门造车"，要求 P1 退化到 "档位 A：Lint+Run+Open CLI 三按钮"，借 Claude Code CLI 当 Copilot。原 P0.5 推到 P2。

- **[08:49]** Rust 重写 / 路线图 / 落盘流程一并拍板。Claude 立即按 Kiro+Superpowers 标准流程并行起 5 个 task 落盘。

- **[09:00]** Kiro spec 四件套 + Superpowers plan + plan.md v2 摘要全部落盘完成（共 1846 行），Claude 准备开始 P0 实施。

- **[09:08]** 用户要求 "让Gemini审阅一遍" → 触发 CCB agent 名字 unknown 故障，session 早期版的 askd 还在用 provider 名字。中断到 19:35 用户重新触发审阅。

- **[19:35]** CCB 已升级 v6.0.7，发现 `a2:gemini` 抽象命名。Gemini 审阅请求成功投递。

- **[23:31]** **双评审决策**：用户要求 "kiro spec发给Gemini & codex 看一下" → Claude 并行投递两个 ask --wait（实际跑成 background ID + 通知机制）。Codex 8 分钟后回 fail，Gemini 4 分钟后回 conditional_pass。

- **[23:39]** **Codex Verdict: fail (5.2/10)**：揭出 2 Blocker（Prompt Capture vs 不改 DeerFlow / SkillManifest vs 现有 SKILL.md 结构）+ 3 Major + 4 矛盾 + 10 missing items。Claude 立即代码核查确认两个 Blocker 都属实。

- **[23:53]** 用户对 6 个修订疑问的 4 项回答 "我无法回答,问Gemini"，把 4 个工程决策推回 Gemini-Claude 闭环 + 透露 DataManager/ArtifactManager 应融入。明确 "走重审"（fix→re-submit）。

- **[23:55]** Gemini 第四轮对 6 问全部表态：扁平 inline、12+2 事件、TracingClientProxy、阈值最后一周校准、`workspaces/<user_id>/skills/`、StorageManager 内置化。给出 Top 5 必改项。Session 在准备开始修订 spec → 重审 Codex 时结束。

---

## 核心主题

**这一天用户三次拽 Claude+Gemini 出"产品经理病/工程师洁癖/凭脑子写 spec"的坑——把 SKILL.md 重定义为机器级工程 DSL（推翻"PM 可读性"包袱）、把 P1 拉回 MVP "三按钮 + 借 Claude Code CLI 当 Copilot"（推翻"完美 Copilot Core"路线图）、把 spec 打回重审（推翻"凭印象写 14 个事件 + 扁平 step"的代码幻觉）；同期 CCB 在 ask 通道、tmux 投递、daemon 串行化、版本迁移命名、reply 重影等多个面暴露稳定性短板，主控 Claude 的 ask 协作纪律（先过 Gemini 再问用户、--wait 默认、code-grep before claim）反复失守。**
