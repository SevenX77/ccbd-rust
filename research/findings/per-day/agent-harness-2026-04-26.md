# agent-harness 2026-04-26 session 结构化提炼

### 1. CCB / Claude / Gemini / Codex 的 bug 或失败行为

- [10:57] CCB `ccb ps a2` 报错 `ps does not accept extra arguments: ['a2']`，但文档/常识里 ps 应该能按 agent 过滤；CLI 缺少子命令参数支持。
- [10:58] Gemini pane 弹出 "A potential loop was detected" 弹窗阻塞所有交互；ccbd 协议包装层无法识别/dismiss 该弹窗，每次新 session 首句必触发，必须人工 send-keys 选 2 才能继续。
- [10:58] `command ccb ask --wait --timeout 30 a2 "/new"` 超时失败：`/new` 是 provider 命令而非消息，但 ccb ask 没区分，把它当成 prompt 提交后 job 卡死。
- [10:58] `autonew gemini` 报 `[ERROR] No active gemini session found for this project.`——autonew skill 不认 ccbd 管理的 session（pane_registry_runtime 跟 ccbd 不互通）。
- [11:01] Gemini SIGTERM 被吃掉无效，要 SIGKILL（kill -9）才能重启，pane respawn 后又重新弹 loop detection。
- [11:02] Gemini ping 一句简单话 thinking 2m+ 不返；后续 plan-review 跑 14min+ memory 545MB 还在涨，明显 stuck；多次被 ccbd `project_shutdown` kill（generation 40→42→43）。
- [11:02] `ccb ask --wait` 多次 wait timed out 但 job 实际仍 running，错误处理缺乏一致语义。
- [11:09] Sleep 25 秒被 Bash tool 阻断："Blocked: sleep 25 followed by..."——tool 限制无法直接长 sleep 等条件，需走 Monitor/run_in_background。
- [11:24] Gemini job_5a27b55a9000 跑 14min 没回，ccbd 把它 `project_shutdown` 强 kill；Claude 在每次重提前都不知道 ccbd 会 kill，导致 prompt 浪费。
- [17:42] Codex 三次连续 `status: completed` + `completion_reason: task_complete`，但 `reply:` 字段是空字符串——伪完成。`/new` reset 也无效。
- [17:44] Gemini `wait timed out` 后 job 仍在 running，cancel 后才结束。
- [01:04] Codex job 直接 `failed` + `completion_reason: pane_dead`；pane 内全是 `ERROR: No saved session found with ID 019db764-5633-...` 死循环 retry 一个失效 session ID。stale ID 写死在 `.ccb/.codex-a1-session` 的 `start_cmd` + `runtime.json` 的 `session_ref`，每次 pane 重建都用旧 ID。
- [01:32] kill 单 pane → ccbd 重建新 pane %5，但仍用同样 stale session ID 死循环。
- [01:51] `command ccb` 起来后 ccbd 在 `mounted ↔ stopping ↔ starting` 之间反复抖动，必须 `ccb kill -f` 硬重启才稳。
- [01:51] Gemini pane 显示 `API_KEY_INVALID` from googleapis.com——之前所有"Gemini thinking 14min"的根因可能就是 API key 失效，被错误归因为模型/CCB 协议问题。
- [02:16] `ccb ps` 一刻显示 mounted/idle，几秒后 tmux server 消失 + ccbd 变 unmounted，状态机不一致。

### 2. 用户纠正 / 抱怨 / 吐槽 Claude 的内容

- [01:32] 用户："kill，快"——Claude 上一条把 3 个选项摆给用户征询同意（"我倾向 1...要不要我执行?"），用户用最短指令打断这种征询性犹豫。

### 3. 用户强意图（带原话）

- [10:57] "Gemini可以重启吗？查看一下连通性，清空context ， 联通的话就继续任务"——单条核心指令，确认 Gemini 通后接续 PR #7 工作。
- [01:01] "让Gemini做一次全局的评测，现在的代码实施情况与我们的设计对比差异，进度怎么样，哪些需要调整的；让codex全局审核一下所有代码有什么bug有什么风险，有什么不够专业，不够工程化的"——明确要并行的双 agent 全局评测。
- [01:29] "确认一下ccb pane的状态"——直接命令式，要事实状态而不是 Claude 的解读。
- [01:51] "重启一下Gemini试试"——简短指令推动 Claude 行动。
- [01:32] "kill，快"——见上。

### 4. 对话中暴露的设计缺陷

- ccb session 持久化文件（`.ccb/.codex-a1-session` + `agents/a1/runtime.json`）写死 codex CLI 的 `resume <session-id>`；当那个 session ID 在 codex 后端被清理后，pane 重建仍用 stale ID 死循环 retry，没有"找不到就 create new"的 fallback。
- `ccb ps` 把 pane status (alive) 当成 provider status；codex 进程实际在死循环出错时 pane 仍 alive，导致 Claude/用户被假象误导（state=idle / queue=0 但 agent 完全没工作）。
- Codex job 状态机能产出 `status: completed` + `completion_reason: task_complete` + 空 reply 的"伪完成"，没有"reply 为空 = 实际未完成"的健全性校验。
- `/new` 命令既能当 ccb ask 的消息（被当 prompt 提交，job 卡死），又能通过 autonew 直接 send-keys 到 pane，两种语义不区分容易踩坑；autonew 的 pane registry 又跟 ccbd 的 session 注册不打通。
- ccbd 状态机会反复 `mounted ↔ stopping ↔ starting` 抖动，没有 backoff/quiesce 机制；用户和 Claude 都只能靠 `kill -f` 硬重启绕开。
- Gemini API key 失效不暴露成 ccbd 健康指标——pane 看起来 alive、job 状态 running、claudeforce 能等到 `wait timed out`，根因要靠手动 capture-pane 才能从 splash + 错误堆栈里挖出来。整个观测链路对"auth 层挂"不可见。
- Gemini-cli 的 "loop detection" 弹窗 + Gemini 模型的 "thinking 14min memory ballooning" 在外观上跟 "Gemini 慢" 无法区分，被错误地写成"Gemini 不可靠 - 模型问题" memory，实际可能是 API key/弹窗/ccbd 协议三者叠加。
- PR #5 累积 41 commits，覆盖 Phase 0 + PR #6 big-bang + PR #7 全部 4 validators + step 5 registry refactor，三个 PR worth 的内容混在一个 branch；merge 前需要拆分但工作量大，是 PR 工作流被 iron rule "不准停" 推到极端的结构性问题。
- `tests/graph_agent/core/` 没 `__init__.py`（与同级 `callbacks/` `io/` 等不一致），靠 pytest rootdir + pythonpath 配置兜住；新建 `validators/` 子目录时遗留这个不一致。
- `parse_skill_file` 返回 `{frontmatter, human_body}` 而不是 flat dict；plan v1 直接传整个 dict 给 `TypeAdapter.validate_python` 撞坑；API 形状对调用者不直观。
- pytest 的 `pythonpath = ["src/core"]` 配置让 import 路径变成 `from graph_agent.core.*` 而非看起来更"对"的 `from core.graph_agent.core.*`；plan 撞了一次坑。

### 5. 决策转折点

- [11:02] Gemini 多次卡死后，Claude 决定全部 review 走 Codex（roles table 里就是 reviewer），Gemini 当 best-effort 二审不 gate；写入 memory `project_gemini_unreliable_2026-04-25.md`。
- [11:09] Codex must-fix #2（path escape `..`）被 Claude 主动驳回——`story-deconstruction` 用的就是 `../global-synthesis/SKILL.md` 合法跨目录，validator 的契约是"trusted file tree"，staging 边界归 upload layer。这是 Claude 第一次反驳 reviewer 而不是无脑修。
- [11:39] 关于"step 2 是否抽 helper 给 step 1+2 共用"——Claude 自评估三种姿势 A/B/C 后，**先选 A**（同步重构 step 1）；Codex plan-review 改判为"三振才重构"原则，Claude 接受改用最小 scope（不抽 helper），把决定推迟到 step 3+4。
- [11:53] Codex code-review NEEDS-FIX：subgraph_cycle chain message 用绝对路径信息密度太低；Claude 立刻修成 `Path.relative_to(anchor)` + 加锁定测试，未抗辩。
- [12:16] 4 个具体 validator 全部 ship 后，第 5 项 rules.yaml 是产品决策，Claude 选 punt（在 docstring 标 DEFERRED + 写出 A/B/C 三选项），按 iron rule 默认走 C 让用户判断。
- [17:38] PR #7 step 5 persona registry 计划被 Codex 三次返空 reply 后，Claude 决定按 "max 3 rounds 后 proceed" 自审 + TDD 411 pytest 走完整个 5 commit 重构，未等 review 就 push；事后写 memory 标 review 链路降级。
- [17:45] 任务 3+4 合并 commit 决策：Claude 在执行 Task 3 中途发现"删 `_resolve_persona` 会破坏 validator 的 cross-module-private import"，立即把任务序重排成"先迁移 validator → 再删 loader 私有"，作为单 atomic commit。
- [01:32] 用户"kill，快"打断 Claude 的征询性犹豫，Claude 当场放弃 3 选项分析直接执行 kill。
- [01:51] Gemini API key 失效被 capture-pane 发现后，Claude 在 memory 里隐含承认之前"Gemini 严重不响应"的归因可能错位。

---

**核心主题**：CCB stale-session 持久化 + provider auth 失效 + ccbd 状态机抖动叠加成"Gemini/Codex 严重不响应"的伪相，逼 Claude 多次重启 + 走"max 3 rounds 后单审 + TDD 自盲" 把 PR #7 4 个 validator + persona registry refactor 全部 411 测试 pytest-green ship 进 PR #5（累计 41 commits），用 Codex 3-yes/no 窄焦点 review pattern + iron rule "不准问要不要继续" 顶住协作工具的不稳定。
