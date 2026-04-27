# agent-harness 2026-04-21 session findings

来源：`/home/sevenx/coding/ccbd-rust/research/sessions/agent-harness/markdown/2026-04-21-session.md`（5094 行；session ID 38a7c8be-879a-4f98-bc3b-bd00ac919c8f；项目 /Users/sevenx/Documents/coding/agent-harness）。session 中含两段被原样保留的 Claude session 自动 compact summary（长度复制 2 份）+ IMG_0074–IMG_0102 截图还原段。

---

### 1. CCB / Claude / Gemini / Codex 的 bug 或失败行为

- **[08:04]** 用户连发 3 条相同的"执行合并"指令（行 8/24/40 三处时间戳完全一致：`[08:04:44 → 08:04:57]`）。markdown 转录里这条用户消息被复制 3 份，对应 Claude 3 次重复输出 4 个 `TaskCreate(...)` 块，疑似 session 录制层把同一 turn 重复采样。失败的 bash 命令（`Exit code 128 fatal: not a git repository`、`Exit code 1 ls: ... No such file or directory`、`fatal: pathspec ... did not match any files`）也被原样复制 3 份。

- **[12:25]** Claude 在 git 提交流程中第一次 `git add` 路径写错：`src/core/graph_agent/skills/__init__.py` 文件不存在，命令被 git 拒绝（`Exit code 128 fatal: pathspec '.../skills/__init__.py' did not match any files`）。Claude 直接重发去掉该路径的 `git add`，没有先 `find`/`ls` 验证。

- **[18:58]** Claude session 自动 compact 触发后，`This session is being continued from a previous conversation that ran out of context...` summary 块在 markdown 里被复制 3 份（行 2876、3109、3342），完全相同的 Summary（item 1–9）连续出现 3 次，每次后面都跟着同样的 29 张截图 Read 序列。说明 session 录制 / continue 流程把 compact 后的恢复消息重复注入。

- **[19:06]** Claude 调用 `Write()` 工具时连续 3 次抛 `InputValidationError: Write failed due to the following issues: The required parameter 'file_path' is missing / The required parameter 'content' is missing`（行 3724–3740）。紧接着 `Edit()` 也抛同样错（参数全空），`Bash()` 也抛同样错（行 4033–4041）。Claude 内部 tool dispatcher 在某个路径上走到了"调用工具但不传任何参数"的状态，多次重试都不带参数。

- **[19:06+]** Claude 在落盘截图原文阶段反复触发 `⚠️ API Error: Stream idle timeout - partial response received`（出现 4 次：行 4499、4528、4540、5046）和 `⚠️ Request timed out`（行 4507）。每次超时后，要么是用户用"又报错了 / 又报错"催促重试，要么是 Claude 自己收到部分响应后中断。最终是用 Python heredoc 脚本 (`python3 << 'PYEOF'`) 绕过 Write 工具直接落盘才成功。

- **[19:07]** Claude 用 IMG_0074–IMG_0102 共 29 张截图 reconstruct 计划。markdown 显示 Read 调用序列里出现 IMG_0080/0081/0082/0089/0090/0091/0082/0083 等多次重复读取（行 3554–3597），疑似有重试或并发 race 导致同一截图被读多遍。

- **[19:07]** Claude 第一次试图 Read `/Users/sevenx/Documents/coding/agent-harness/temp/graph_agent/plan.md`，得到 `File does not exist. Note: your current working directory is /Users/sevenx/Documents/coding/agent-harness.`（行 3602）。这其实是 Claude 自己幻觉的路径——用户提的是 `temp/plan screenshot/`，不是 `temp/graph_agent/plan.md`。Claude 后续创建了这个不存在的目标，又在用户复述需求后才发现真正要写的是 `temp/plan.md`。

---

### 2. 用户纠正 / 抱怨 / 吐槽 Claude 的内容（带原话）

- **[compact summary 中转述]** 用户在更早一次对话里纠正过 Claude 写错项目位置：`"我要你把需求写到这个项目里面啊（agent-harness）"`（compact summary "Errors and Fixes" 第 6 条）。Claude 之前把 native-skill-wrapper 的 design.md 错写到 AI-story-forge 项目下。

- **[compact summary 中转述]** 用户曾让 Claude 聚焦：`"你不用管测试的东西，你的任务聚焦在把 graph_agent 修到没有 bug"`（行 3268、3501、3268 重复 3 次）。这是对 Claude 之前自作主张去碰测试代码的纠正。

- **[19:07 第三轮截图原文]** 用户对 Claude 的方案表示部分看不懂：`"你的分析和建议都很好（很高我也看不懂），你想要讨论的我只能回答：一定是 web 应用，因为你子进程是什么原理？sdk 效果哪个好？sdk 是把 Claudecode 完整功能接进来？帮我科普一下"`（行 4398，第三轮截图还原）。这是用户明确表态：Claude 给的技术细节超出他的判断能力，需要先科普再讨论。

- **[19:07 第五轮截图原文]** 用户对反复 API 超时不耐烦：`"又报错了"`（行 4530）和 `"又报错"`（行 4536），连续两次催促，反映 Claude 在 stream idle timeout 后没有立即降级到非流式落盘方式。

---

### 3. 用户强意图（带原话）

- **[12:25]** 创建 GitHub 远程仓库并推送：`"还没有远程git仓库，帮我在远程创建git仓库，然后commit push"`（行 2337）。

- **[18:55]** 还原对话计划框架：`"'/Users/sevenx/Documents/coding/agent-harness/temp/plan screenshot'根据这个对话截图，还原整个对话中聊到的计划框架"`（行 2841）。

- **[19:06]** 不只要框架，要原文：`"我希望你把截图中的对话完整内容也记录下来"`（行 3647 / 3659 重复 2 次）。这是对 Claude 第一次只落了"提炼框架"不够的明确升级要求。

- **[第三轮截图还原 19:07]** 一定要 web 应用：`"一定是 web 应用"`（行 4398）。这是用户对 Studio 形态的 hard 拍板，封掉 VS Code 扩展选项。

- **[第三轮截图还原 19:07]** 要 SDK 科普：`"sdk 效果哪个好？sdk 是把 Claudecode 完整功能接进来？帮我科普一下"`（行 4398）。这是对 SDK vs 子进程黑话不懂的明确求科普诉求。

- **[第三轮截图还原 19:07]** 意图偏离检测要 LLM judge：`"意图偏离检测是否要 llm 做 judge？是的，但是你刚刚说的 plan checklist 也应该作为 llm 分析的一部分"`（行 4398）。

- **[第三轮截图还原 19:07]** 立刻落盘：`"我希望你快速落盘文档，这个项目有没有 super power 和 kiro？"`（行 4400）。这是要求"立刻产出文件"+"顺便确认存量工具"的合并意图。

- **[第五轮截图还原 19:07]** 多 agent battle 协议：`"1. Gemini cli 做辅助分析，用 claude+ccb 协议来调用（看一下库里面有没有 ccb 协议），Claude 做业务逻辑分析建议。claude code 与 Gemini battle 出最终结果（3 轮，battle 不出的问题交给用户）"`（行 4505）。这是用户最早提出"3 轮 battle / 升级用户"协议的原话——后来在 ~/.claude/CLAUDE.md "铁律" 里变成"和 Gemini 就同一个具体决策辩论满 3 轮后依旧没对齐"那条全局规则。

- **[第五轮截图还原 19:07]** 模型选择要细粒度：`"不单单是三档 role，还有独立模型指定，role 会有有不同模型优先级 fallback，而独立模型指定是确定性的测试某个模型"`（行 4505）。

---

### 4. 对话中暴露的设计缺陷

- **graph_agent 框架本身的设计债（Claude 在分析报告里列）**：
  - `core/harness.py` 952 行（行 26、4100、4618），违反全局 code-style "Max file length: 300 lines"。
  - `deerflow/` 与 `core/parser.py` 有两套并行的 SKILL.md 校验，CHANGELOG 自称"by design"，但 Claude 评注 `"一但规则不同就是债，不是设计"`（行 3799）。
  - 14 个 CallbackEvent 钩子未类型化（仅靠 dict + 字符串字段），是 Studio 前端稳定消费事件流的根本卡点。Claude 评：`"不做这个，Studio 永远在追 bug"`（行 3795）。
  - `docs/graph_agent_docs/` 与 `src/core/graph_agent/docs/` 两套完全重复的 9 篇文档（行 4107、4625）。
  - `skills/builtin/script/patch_tools.py` 与 `skills/builtin/md-patch/script/patch_tools.py` 内容疑似重复（行 4109、4627）。
  - `skills/builtin/md-patch/script/__pycache__/*.pyc` 被签入 git（行 4108、4626）。
  - `2026-04-08-...ai-narrated-recap.txt`（1230 行 session 录）被签进仓库（行 4106、4624），疑似当时合并时一股脑 `git add .` 顺手带进。
  - 多模态工具（generate_video / synthesize_speech / understand_video）无单测（行 4102、4620）。

- **runtime.context NoneType crash 暴露的根因（compact summary 转述）**：在 standalone 调 `run_skill()` 时 `GraphAgentHarness` 没完整初始化，`task_tool.py:88` 直接 `runtime.context["thread_id"]` 会爆 `TypeError`。修复用 `(runtime.context or {}).get("thread_id")` 模式打了 8 个文件。这不是 8 个独立 bug，是 deerflow 整体没把 `runtime.context` 当 Optional 处理的设计缺陷。

- **Python 3.10 vs 3.12 兼容裂缝**：`md_to_json.py` 用了 PEP 695 generics `def diagnose[T: BaseModel]`（3.12+ 语法），在 3.10 直接 SyntaxError。修复成 `_T = TypeVar("_T", bound=BaseModel)` 老语法。SF 项目的 3.12 跟 AH 项目的 3.10 没明确 pin 死。

- **md_to_json.py 重复定义**：`_T = TypeVar("_T", bound=BaseModel)` 在文件里出现两次，`from typing import TypeVar` 也重复 import 两次（compact summary "Errors and Fixes" 第 3、4 条）。说明从 SF 复制到 AH 时是粘贴拼接，没去重。

- **CCB `.ccb/` 没进 .gitignore**：Claude 落盘文档时发现 `.ccb/` 目录被签入 commit（`new file: .ccb/.claude-session` 等 4 个文件出现在 git status，行 2482–2485），这是本地 runtime 元数据不该进版本控制。这个问题在 ~/.claude/CLAUDE.md 全局规则里后来才补丁解决（fork 加 `.gitignore`）。

- **session 录制层的重复采样**：同一时间戳 `[08:04:44 → 08:04:57]` 的内容被复制 3 份（行 6、22、38），同样的 compact summary 也被复制 3 份。这是 markdown 转录脚本（不是 Claude 本身）的去重逻辑缺陷——按时间戳作 key 时没合并相邻重复段。这个问题影响所有用此 markdown 转录跑下游分析的工具。

- **Claude 工具调用时无参数**：行 3724–3740、4009–4014、4033–4041 三段都是 Claude 调 Write/Edit/Bash 时**完全不传任何参数**就触发 InputValidationError。说明 Claude 内部某个推理路径下"想用工具但不知道传什么"，没 fallback 到先 echo 思路再调工具的策略。

---

### 5. 决策转折点

- **[12:25]** 从"分析合并方案"切换到"执行合并"——用户单句 `执行合并`（行 8）触发 4 个 TaskCreate 任务，把之前 Claude+Gemini 多轮辩论得出的方案（AH 主仓 + SF 三特性合并 + 保留 deerflow 防御性 null check）落地为代码改动。

- **[12:25]** 决定项目主仓位置：用户 `还没有远程git仓库，帮我在远程创建git仓库`（行 2337）→ Claude 执行 `gh repo create agent-harness --public --source=. --remote=origin`，最终落到 `https://github.com/SevenX77/agent-harness`。这是 agent-harness 项目从 SF 子目录正式独立成 GitHub 仓库的时间点。

- **[18:58 compact 后]** 任务焦点从"代码合并"转到"还原计划框架"。session 在 14:39 因 login 中断 4 小时（`Login interrupted` 行 2834），18:55 用户回来贴截图目录，18:58 触发自动 compact，从此进入"读 29 张截图 → 重构计划框架"模式。

- **[19:06]** 用户升级要求："只要框架"→"要原文"。`"我希望你把截图中的对话完整内容也记录下来"`（行 3647）让 Claude 必须重读所有截图、按截图原文（含用户原话和 Claude 完整回复）逐条还原，而不只是提炼出方案表格。

- **[第三轮截图 19:07]** Studio 形态拍板：用户 `"一定是 web 应用"`（行 4398）让 Claude 之前列的"web 应用 vs VS Code 扩展"取舍直接消解，VS Code 扩展方向被砍。

- **[第四轮截图 19:07]** Copilot 集成方案拍板：Claude 在科普 SDK vs 子进程后给出选型结论 `"Studio 用 SDK"`（行 4479），理由是 web 服务化 + 程序化拦截工具调用做 diff + 每个 PM 独立权限沙箱。子进程降级为 fallback。这是 Skill Studio Copilot 架构的核心决定。

- **[第五轮截图 19:07]** Claude+CCB+Gemini 协议雏形：用户在落盘期间提出 `"Gemini cli 做辅助分析，用 claude+ccb 协议来调用 ... claude code 与 Gemini battle 出最终结果（3 轮，battle 不出的问题交给用户）"`（行 4505）。这是 sevenx 后来在 `~/.claude/CLAUDE.md` 顶部"铁律"那条规则、以及 `~/.claude/rules/ccb-collaboration.md` "Decision Escalation 协议"那一节的雏形——3 轮 battle、未对齐才升级用户。

- **[19:07]** 落盘失败导致工具升级：API stream idle timeout 反复失败 4 次后，Claude 放弃 Write/Edit 工具，改用 `Bash(python3 << 'PYEOF' ... PYEOF)` 用 Python heredoc 直接落盘，最终成功（行 5079 输出 `done`）。这是从"用框架工具"降级到"用 shell 兜底"的转折，也是后来 sevenx 在 ~/.claude/rules/ 写各种 fallback 纪律的经验来源之一。

---

## 核心主题

agent-harness 在 2026-04-21 完成从 AI-story-forge 子目录到独立 GitHub 仓库的剥离 + 三大特性合并入库，紧接着在与 PM 视角讨论"graph_agent 下一步迭代"时，整理出 Skill Studio 前端方案、Claude+Gemini+CCB 多 agent 协作协议雏形、和 SDK vs 子进程的 Copilot 选型结论；过程中暴露 graph_agent 框架本身的多个设计债（952 行 harness、双轨校验、CallbackEvent 未类型化）和 Claude 工具调用层的稳定性问题（API stream idle timeout、空参数 InputValidationError、session 录制重复采样）。
