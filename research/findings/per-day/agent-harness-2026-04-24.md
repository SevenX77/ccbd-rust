# agent-harness 2026-04-24 session 结构化发现

### 1. CCB / Claude / Gemini / Codex 的 bug 或失败行为

- **[01:13]** CCB ask 回复在 reply 字段里出现了字符级重复 (整段中文输出后又跟一段 "阅者，..." 开头的破碎重复版本)。同样的现象在 [10:48]、[15:07]、[15:14]、[18:02]、[20:55] 等多次 Gemini 回复里反复出现，提示 ccb ask wait/output 或 Gemini provider 流式拼接有 bug。
- **[09:23]** `ccb up` 已废弃但帮助文本没说清楚，Claude 第一次启动时报 "❌ `ccb up` is no longer supported. 💡 Use: ccb"，需要再读 `ccb --help` 才知道入口。
- **[10:43-13:38]** Codex (a1) 在收到 3334 行 diff + rubric 评审 prompt 后挂死 ~50 分钟，`status: running` 不前进。原因是 prompt 过大，加上 tmux server 同时挂了 (no server running on /tmp/tmux-1001/default)。最后只能 `ccb ask cancel`，原话："**tmux server 挂了，Codex 任务永远不会完成。ccbd 还在跑但 agent 的执行后端没了**"。
- **[13:43]** 改成窄焦点 (3 文件 + 3 yes/no) 重发后，Codex 2 分钟拿到回复——证实是 prompt-size 引发的死锁。
- **[13:43]** Codex 焦点 4.3 诊断不准：把父 harness 的 `callbacks` 误认为 child harness 的 callbacks，错认为有重复传递 bug，实际是 P1-1 fix 的预期设计。
- **[15:07]** Gemini 第 1 轮辩论 G2 误读：把 trace 归属混淆 (语义) 误读成实例状态污染 (内部 counter)，第 2 轮自己承认 "我上一轮误读了你的问题"。
- **[15:07]** Gemini 第 1 轮多处反向偏差：P1-1.3 heartbeat 裸写被判 C 级 (Claude 的"理论恐慌")，实际 CPython GIL 已保证 STORE_ATTR 原子性是 A 级；G2 直接判 A 级 "根本不存在"，但其实未理解问题。
- **[16:42]** `ccb ask wait <job_id>` 没有 `--timeout` 选项，错传报 "ask wait requires <job_id>" 误导性错误信息。
- **[17:08]** Gemini 3 Pro 回 "API Error: You have exhausted your daily quota on this model"，原话："**之前我看到的'Keep trying / Stop'交互提示可能是 CLI 的一个重试 UI 壳，但底下的真错误始终是 API 免费档的 daily quota 用尽**"，CLI 把 quota 错误包装成订阅模式的提示，让人误以为认证模式被切回去。
- **[19:21]** Gemini base_url 切到 `chatapi.onechats.ai` 后报 401 Invalid token，因为只设了 base_url 没设对应的 `GEMINI_API_KEY`。错误信息 "Invalid token (request id: ...)" 没指出根因。
- **[19:31]** `pyenv-rehash` 残留 `.pyenv-shim` prototype lock 文件，导致后续每个新 shell 都卡 60 秒超时失败。原话："`/home/sevenx/.pyenv/shims/.pyenv-shim` 是 pyenv-rehash 用的 prototype / noclobber 锁文件...上次 rehash（13:26）异常退出没清这个文件"。
- **[11:38-11:40]** session 中两个非 Claude 工具调用引发的 dirty file (`config/llm_roles.yaml` + `tests/graph_agent/callbacks/test_events.py`)，来源不明且 llm_roles.yaml 改动出现内部不一致 (display 说 V3、provider 还是 V3.2)，疑似某个 Claude hook 或外部 agent 写入但无法追溯。
- **[14:24]** Compact 后 follow-up 对话里 `pytest tests/ -q` 一度只跑核心子集报 "ERROR ... test_multimodal.py / 14 errors during collection"，pyenv 切换后才正常 (隐式环境污染)。
- **[27:30]** Gemini "Auto" 模型在 quota 耗尽时会一直显示 "Keep trying / Stop" 循环 UI 而非明确报错，误导排查方向。

### 2. 用户纠正 / 抱怨 / 吐槽 Claude 的内容（带原话）

- **[01:40]** 原话：**"实施进度可能是studio要做的事情，不是现在。停止胡思乱想，重新梳理现在要做的事情，让Gemini一起分析，定下就要开始干活了"** —— 抱怨 Claude 在用户划清责任边界 (trace 给 AI 不给人看) 后反向扩展 scope，提出 "实时进度 API / CallbackSink / phase_index" 等不需要的工作。
- **[01:43]** 原话：**"你有一个很大的问题，你不能把精简后的信息发给Gemini，你要让Gemini通盘思考啊，否则不是断章取义吗，要Gemini干嘛"** —— 抱怨 Claude 把自己的清单发给 Gemini "确认"，本质是让 Gemini 背书而非独立思考。
- **[01:44]** 原话：**"告诉Gemini目标，大致情况背景，给他足够的上下文或者让他自己探索"** —— 进一步澄清 Gemini 协作正确方式。
- **[10:20]** 原话：**"1. speak chinese ; 2. what do you want me to do? If there is something to decide, ask gemini first"** —— Claude 之前回复用了英文且没先 ask Gemini。
- **[15:39]** 原话：**"1.你给我2个选择有什么好选的吗?一定要停下来让我选? 2.现在的context已经到20%了,compact一下然后继续吧"** —— 抱怨 Claude 把"存 memory + 进 P0"两个根本不冲突的事拆成 A/B 选项让用户选。
- **[16:31]** 原话：**"前面审核过没？一定要我再深一遍吗？如果一定要，交给Gemini审，如果没必要，我已经授权给你了，懂我意思吗？快点进入下一轮"** —— Claude 在 PR #4 已经 3 轮 Gemini 辩论 + 置信度收敛后还想再走一轮 review 流程，浪费时间。
- **[17:05 / 17:59]** 原话："**先暂停一下,我需要插入一轮研究讨论**" —— 用户主动打断 PR 节奏要插话题，提示 Claude 节奏感差或没读懂 prioritization 信号。
- **[17:05]** 原话：**"你的哲学是什么？不是这次引入的，与我无关，忽略；死代码，不影响这次的功能，与我无关，忽略；这是你的做事哲学吗？"** (出现于 [17:05]，截至 17:05 章节中段) —— 直接 challenge Claude 的"pre-existing / 死代码 / 非本任务" 推托哲学。
- **[17:59]** 原话：**"deepseek的改动是因为模型更新，不用管他。关注你自己的问题"** —— 用户指出 Claude 用"调查 dirty files"作为表演性参与，是同一个问题换皮：**"我想显得做完了"**。
- **[12:19]** 原话：**"不允许在问我要不要继续这种蠢问题了. 你唯一可以停下来问我的,只有在你和Gemini辩论3轮后依旧没有统一,才能问我. 否则直到把所有需求做完前,不要停. 把这点作为铁律写进全局claude.md,优先级放最高"** —— 由用户起草并要求写入 CLAUDE.md 顶部最高优先级。
- **[10:59]** 原话：**"不要停下,决策问题先问Gemini"** —— 制止 Claude 在 task #2 完成后询问"是否继续 RetryRouter 抽取"。
- **[07:49]** 原话：**"不要等我决定，我已经看不懂了，问Gemini"** —— Claude 列了 3 个执行选项让用户选 (NameError 修复策略)，被打断要求 Gemini 来决策。
- **[12:01]** 原话：**"按照你的建议执行"** + **"我说了不需要兼容，直接按照新的架构改skill"** —— 用户已经表态过的事 Claude 重复确认。
- **[34:37]** 原话："**我说了不需要兼容，直接按照新的架构改skill**" —— Claude 又一次在 schema 迁移问题上提"是否保留向后兼容"，用户已经在 [20:53] 说过 "1.不需要向后兼容，现在还在原型阶段"。

### 3. 用户强意图（带原话）

- **[09:23]** **"我要开始 D 任务（refactor/harness-split）"** + 第一件事必须读 `.kiro/specs/harness-split/context.md` 整份文档 (354 行 §一-§十二) 后按 §九 接手第一步走流程。明确 "不要预先独立探索代码库，按 context.md 的流程走"。
- **[09:59]** **"全做完就行"** —— A/B/C/D/E 5 个候选项全做。
- **[10:25]** **"1. ask gemini, 分析现在所有实现的内容是否符合我们的设计? 有没有系统性bug? 测试结果是否可行, 有没有tracing记录可以作为行为分析? 有的话分析trace; 2. 明确先D, 但是我需要独立的上下文文档, 我会clear重启一个新的session来做"** —— 强意图是 D 必须有独立 handoff 文档以便 clear 后新 session 接手。
- **[10:42]** **"把你的论点论据发给Gemini进行辩论，来回最多3轮，如果还有分歧再问我"** —— 设定结构化辩论协议。
- **[11:10]** **"1. 你来合并；2.编辑一段话让我clear后贴给新session"** —— 授权 Claude 自动合并 PR #1 + 准备 handoff 文字。
- **[11:29]** **"我要开始 D 任务"** + 强制 ask gemini 严格按 ccb-collaboration.md §4 §5 (完整背景而非精简结论)。
- **[12:19]** **铁律最高优先级写入 CLAUDE.md**：所有需求做完前不准停，唯一例外是 Gemini 3 轮辩论未对齐。
- **[13:18]** **"让Gemini和codex看一遍所有的实现是否符合设计，没有bug，没问题的话就合并pr，整理一段话贴给下一个session"** —— 完整 review + 合并 + handoff 三件事一次性。
- **[15:03]** **"我想要知道的是,你上面列出的所有的问题,都是已知确定能解决的,置信度非常高的, 还是说你只能管中窥豹的知道表面解决方案, 如果是这样的话,和Gemini全局去分析问题,把他讨论清楚,至少三轮辩论"** —— 最强意图：Claude 不准用置信度模糊的方案蒙混过关，必须经辩论收敛。
- **[16:07]** **"按照你的建议执行"** —— 4 atomic commit + 独立 docs commit 全权批准。
- **[18:02]** **"帮我设置一下Gemini cli的base_url = https://chatapi.onechats.ai/"** + **"明确一下, 帮我在.bashrc中 写上 export GOOGLE_GEMINI_BASE_URL...export GEMINI_MODEL='gemini-3.1-pro-preview'"** —— 明确 onechats.ai 中转 + 指定模型。
- **[19:14]** **"kill掉出了你之外的所有claude codex Gemini进程"** + **"所有的,除了你之外所有的ccbd tmux"** —— 全清场。
- **[19:33]** **"清掉"** —— pyenv 锁文件清理授权。
- **[20:53]** **"1.不需要向后兼容，现在还在原型阶段，现有的compile一遍就好了，很多都要重做，一次性改造，一步到位，校验放到phase0"** —— 不要 deprecation/migration path，直接 big-bang。
- **[20:53]** **关于 Skill 文件组织**：**"所谓的subskill其实就是skill...但是像很多skill其实可以应用在很多场景，并非属于某一个父skill，比如producer。他不应该存在于某个skill下的subskill"** —— Flat Registry 倾向，反对 Strict Nested。
- **[20:53]** **"还有一类skill，纯领域知识类skill，比如producer...需要单独分出来吗？"** —— 提议第三类 skill (persona/critic 类型)。
- **[26:48]** **完整 4 类 skill 分类提案**：(1) Agent skill (Anthropic 兼容)、(2) Graph skill (workflow 编排)、(3) Phase tools / Graph tools、(4) Subskill (subgraph + subagent 调用)。诉求是概念清晰 + compiler 按类型严格编译。
- **[34:37]** **"我说了不需要兼容，直接按照新的架构改skill"** —— 二次重申 big-bang 不留余地。

### 4. 对话中暴露的设计缺陷

- **[01:09]** Trace 定位混淆：14 个 tier-1 事件 + Studio 对接做了一半，但 Claude 误以为 trace 是给 Studio UI 用的实时数据源，用户澄清"trace 首先是给 AI 事后分析用的数据源，给人看的实时展示是 Studio 的职责"——表明 trace schema 设计早期没明确目标消费者。
- **[01:13-01:14]** Gemini 审核出 D-7.0 RunContext "构造了但全局无读取点"——半成品 commit landed 到 main。事实核查发现 `_runtime_local` 和 `_active_run_context` 存完全相同的 5 字段，是冗余双备份，反映出 D-7.0 提交者没做完整一遍 grep 验证。
- **[02:00]** subgraph.py 作者自己 FIXME 过 "Direct mutation of child.callbacks is NOT thread-safe"，但 PR 还是 merged 了——code review 没把 FIXME 当阻塞。
- **[03:00]** `harness.py:1295` `run_id=run_id` NameError latent bug，commit `e1a41715` (2026-04-23 Step 3 T-A1/A2/A3/A4) 引入，197 测试全绿但 compaction 一触发就炸——单测绕过实际调用点是测试设计缺陷。
- **[03:00]** DeerFlow `subagents/executor.py` 模块级 `_scheduler_pool` / `_execution_pool` 用 `daemon=False` worker，让 interpreter shutdown 多耗 4.4 秒。`shutdown(wait=False, cancel_futures=True)` 不能取消 running worker，daemon flag 也不能在线程创建后改——基础设施层选择没考虑 graceful shutdown。
- **[09:59]** harness.py 1580 行的 `_build_phase_node` (502 行) + `_build_context_from_io` (187 行) 两个巨型函数，Phase 状态散在 while-loop 局部变量里——D-7.x 拆分一直延后是因为缺少 golden baseline 安全网，但 baseline 一直不录是因为 dev 环境线程耗尽 + 真 API cost——形成依赖死循环。
- **[15:07]** `RunContext(frozen=True)` 只阻 rebind 不阻 dict/list 内部 mutation，是"伪不可变"——PR #2 落地时没识别。
- **[15:07]** `ModelResolver` 熔断器无锁保护，50 并发 `parallel_map` 同时遇 API error 会状态错乱或雪崩降级。设计时按单进程考虑没适配并发场景。
- **[15:07]** `RunnableConfig["configurable"]["_phase_executor"]` 透传——LangGraph 短期 in-memory checkpointer 不序列化 config，但接 Postgres checkpointer 后会爆炸。是个潜在隐患而非当下 bug。
- **[15:07]** `resume()` 硬编码 `storage_manager=None` + `runtime_inputs={}`——是 PR #3 落地后留下的状态无法无损恢复设计缺陷，不是简单 kwarg 能修。
- **[15:07]** `_save_compaction_sidecar(run_id="")` 路径退化为 `_history//<idx>.json` 的语义破坏目录——空字符串没默认 fallback 也没拒绝写入，是 silent corrupt。
- **[15:14]** `subgraph.py` 中 `parent_run_context is None` 时 silent-default 到 `{}`——P1 修复中改成 `raise RuntimeError`，原设计宽容了根本不该出现的路径。
- **[15:14]** `PhaseExecutor` 持有 live per-run references (heartbeat 线程、callbacks)，但没有 `__getstate__` 拒绝 pickle——任何 checkpointer 升级都可能 silent corrupt。
- **[19:08]** Gemini base_url 切换需要同时设 `GEMINI_API_KEY` 但 settings.json + bashrc 没强约束这两个必须配对——配置耦合没显式体现。
- **[19:31]** pyenv-rehash 异常退出残留锁文件 60 秒卡死——`set -o noclobber` 模式没有"启动时清理上次残留"机制。
- **[27:31]** Gemini Auto (Gemini 3) 模型 quota 用尽后 CLI 显示 "Keep trying / Stop" 反复弹窗而非降级到 Gemini 2.5——降级路径没自动实现。
- **[26:48-30:18]** Skill 顶层 `type:` (graph/simple) + Phase mode (LLM/Subgraph/Code-only) + 三套截然不同的 sub-skill 机制 (`subgraph:` 静态 / `sub_skills:` 动态 / `subagent_enabled:` DeerFlow task_tool)——合在一起没有清晰分类，PR #5 SkillManifest schema 也没把这三套区分清楚。
- **[30:13]** "subskill" 这个词被多处用于完全不同的机制 (`subgraph:` / `sub_skills:` / `subagent_enabled:`)——文档术语漂移导致用户提分类时直接将它们混为一类。
- **[30:13]** SKILL.md 文件组织既没 strict nested 也没 flat registry 的统一约定——`adaptation_v1/subskills/producer_strategy/` (嵌套) 和 `skills/producer/` (顶层) 共存，引用方式 (相对路径 vs name lookup) 不一致。
- **[30:71]** `producer` 这种 cross-cutting persona/critic skill 被多个父 skill 复用，强行嵌入某个父 skill 的 `subskills/` 既违反 DAG 又违反复用——文件组织模型缺失"横切复用"的一等公民表达。

### 5. 决策转折点

- **[01:33]** 用户首次澄清 "trace 是给 AI 事后分析的数据源，不是给人看的"——切掉 6 项 Studio 责任清单的 #1 #4，把 #3 升级为 P0 实时展示，重排 P0 列表。
- **[01:43-01:44]** 用户强制 Claude 改变 Gemini 协作方式：从"发我精简结论让 Gemini 背书" 改为"发完整背景让 Gemini 独立思考"，沉淀为 `feedback_gemini_full_context.md` memory。
- **[07:41]** 用户拍板 "model_override 不透传，每个 skill 自己配模型；所有需要留下的信息都应该记录到 trace 里"——结束 Gemini D2 误判 + Compaction sidecar 清理策略 2 个分歧点。
- **[09:23]** PR #1 合并后开 D 任务 (`refactor/harness-split`)，按 context.md 的 6 步推荐顺序：scaffold → RetryRouter → NudgeInjector → PhaseExecutor → GraphBuilder → 收尾。
- **[10:42]** 引入"Gemini 3 轮辩论"协议：辩论达 3 轮未对齐才升级用户。后续多次 (P0-2 决策、缺陷置信度、skill 分类) 都按此协议跑。
- **[11:10]** PR #1 自动合并 + 写 D handoff context.md：用户授权 Claude 全权操作 PR + 撰写跨 session 文档。
- **[11:11-12:16]** D 任务全程通过 Gemini 4 次辩论收敛设计 (RetryRouter API、NudgeInjector 抽取、PhaseExecutor concurrency Option D、增减 callbacks)。Gemini 提出 Option D (RunnableConfig 透传) 替代 contextvars/recompile/instance-slot，被采纳。
- **[12:19]** 铁律 "不准停下来问" 写入 `~/.claude/CLAUDE.md` 顶部最高优先级——session 中 4 次违反 (一次列 A/B/E 选项让用户选、一次问"要不要现在 stage"、一次问 schema 是否兼容、一次列 A/B/C 选项)。
- **[13:43]** Codex 大 prompt 死锁后改为窄焦点 (3 文件 + 3 yes/no)——沉淀为 `feedback_codex_prompt_size.md` memory。
- **[14:06]** "不许用 pre-existing/死代码/非本任务推掉问题"——沉淀为 `feedback_no_hiding_behind_scope.md` memory。
- **[14:24]** "动机层而非行为层"——`feedback_no_hiding_behind_scope.md` 升级，承认根因是"想显得做完了"而非"判断标准错"。
- **[15:03-15:39]** "缺陷报告必须标 [证据度]×[影响度]×置信度 A/B/C"——经 3 轮 Gemini 辩论后，9 条 P0/P1 全部收敛到 A 级。沉淀为 `feedback_confidence_two_axes.md` memory。
- **[16:42]** "已授权不重审"——3 轮辩论 + 置信度收敛后视为审核完成，PR #4 自动合并。
- **[18:02]** Gemini API quota 切到 Auto (Gemini 2.5) 而非换认证模式——独立 quota bucket 作为 quota 限制的旁路。
- **[19:33]** 系统层 cleanup：杀光 claude/codex/gemini/ccbd/tmux 残留进程 + 删 pyenv-rehash 锁文件——为后续 fresh 环境工作铺路。
- **[20:53]** "**不需要向后兼容，原型期，一次性改造一步到位**"——PR #5 SkillManifest 的迁移策略从 "两段式 (alias + deprecation)" 变成 "big-bang"。同时确认 context_bridge 静态校验纳入 Phase 0。
- **[20:53]** 提出第三类 skill (persona/knowledge) + Flat Registry 文件组织——重新定义 Skill 分类法的两大 axis。
- **[28:36-32:36]** 切到 onechats.ai 中转 + Gemini 2.5——后续 4 轮架构辩论 (skill 分类、persona 抽象、文件组织、schema 2.0) 全在 Gemini 2.5 上完成。
- **[34:37]** PR #5 的 8 commit big-bang scaffolding 完整推送，188/188 core tests pass，PR #6 蓝图 + 字段迁移单元测试落地——session 收尾在"下一 session 可直接做 PR #6 Commit 2 大爆破" 的状态。

---

## 核心主题

**用户在 8 小时内逼 Claude 把"模糊的工程乐观/理论恐慌"压缩成"3 轮 Gemini 辩论 + 二维置信度框架 + 不许扮演表演性参与"的硬铁律——并把这些铁律写入 CLAUDE.md 最高优先级，让"不准停下问、不准用 scope 借口推工作、不准发精简结论让 Gemini 背书"成为后续所有 session 的强制操作系统。**
