# home-sevenx 2026-04-22 分析
**输入**: /home/sevenx/coding/ccbd-rust/research/sessions/home-sevenx/markdown/2026-04-22-session.md
**生成**: 2026-04-26T09:25:39Z

---

### 1. CCB / Claude / Gemini / Codex 的 bug 或失败行为
- [03:28] git rebase 撞上 `installed_plugins.json` / `known_marketplaces.json` 已 untrack 但远端 HEAD 还在改的冲突，settings.json 也产生 content conflict，需要手动 git rm + 手编 conflict markers 才能继续。
- [04:01] gh CLI 接受 token login 报 `accepts 0 arg(s), received 1`，因为前一条命令是中文引号 `"…"` 而非 ASCII `"…"`，命令直接失败：`<bash-input>echo "ghp_…" | gh auth login -h github.com —with-token</bash-input>`。
- [07:03] CCB v6 投递 spec review prompt 给 Gemini 卡死：`mailbox_state: delivering`、queue_depth=1，但 gemini pane 输入框完全空，`paste-buffer + send-keys Enter` 静默失败。手动 `tmux send-keys ... Enter`（不是 `C-m`）才成功提交——问题 ① "消息卡输入框" 100% 活复现。
- [07:21] 手动 paste hex `0d` 给 Gemini 想模拟 Enter，被 Gemini CLI 当字面字符串入队列：`Queued (press ↑ to edit): Return`。
- [07:08] scratch project 的 gemini 没配 API key，提交后返回 `API_KEY_INVALID` "API key not valid. Please pass a valid API key."，导致这次想验证 v6 delivery 的 Gemini 审查整轮报废。
- [09:00] CCB v5 备份的 `~/.ccb/ccb.config` 是 v5 JSON 格式，v6 不识别：`error: /home/sevenx/.ccb/ccb.config: invalid compact layout: invalid layout token '{'; expected 'cmd', 'agent', 'agent:provider', or 'agent:provider(worktree)'`。
- [09:00] 在 scratch project 重导凭证 + 重启 ccbd 时连环失败：`ccbd is unavailable: lifecycle_starting` / `lifecycle_failure: config_check_failed:timed out` / Bash 命令直接 `Exit code 144`（SIGPIPE）连续踩坑。
- [12:33] Gemini 审 spec review 任务挂 18+ 分钟无 tool call、无 stream 输出（status=running 但 pane 没动），用户感知"又卡住了？"。最后 9 分钟静默后才出来。Monitor 心跳触发 "WARN: no activity 3min, may be stuck"。
- [13:18] Patch 1.1 之前 `CCB_VERIFY_DELIVERY=1` 看似设了，但 ccbd 进程的 `/proc/<pid>/environ` 里**没有**这个 env——根因：`runtime_env/control_plane.py` 的 `_CONTROL_PLANE_ALLOWLIST` 把所有未列入的 env 一律剥光。Patch 1 的代码存在但 gate 永远读 0，verify 路径静默死亡。
- [14:04] Gemini 任务回复了 `status: completed`，但主控 Claude 不知道——因为 async guardrail 要求 end turn 后不准 poll，结果 reply 躺在 mailbox 里没人 surface 给用户。"Gemini输出了，但是没有返回结果"。
- [14:00] CCB 启动 ccb3 报 `command_status: failed, error: timed out`，但 `ccb ping ccbd` 显示 healthy + 3 agent pane_state=alive——CLI 退出码层面的"失败"和真实健康状态脱节，UX 误导。
- [14:21] Gemini ready timeout 默认 20s，但 Gemini 实际启动要 40s+，每次都到 20s 就 kill 重启——用户直觉指出："Gemini启动要40s每次到20s就kill了不是无限循环吗？永远都启动不出来"。
- [14:21] `CCB_GEMINI_READY_TIMEOUT_S` / `CCB_CLAUDE_READY_TIMEOUT_S` 也没在 allowlist 里（和 Patch 1.1 同样的坑），就算 export 了也进不到 ccbd。
- [14:45] 自定义 ask skill（强制 async-only + provider-name 目标）和 v6 maintainer 设计冲突，导致 reply 被困在 state file 里。
- [14:49] `command ccb ask --wait a2` 在 cwd=`~/coding/claude_code_bridge` 下报 `unknown agent: a2`——因为该目录下 maintainer 自带的 `.ccb/ccb.config` 用的是 `agent1..agent5` 命名，cwd 飘错就用错 registry。
- [15:25] `ccb3` 重启依然报 `timed out`，根因不是 ready timeout 而是 **Patch 1 自己**：`tmux_send.py` 的 fingerprint 取最后 3 行非空内容，但 Gemini CLI 的最后 3 行永远是静态状态栏（`workspace / sandbox / model`），**Enter 是否生效都不变**——verify 永远失败 → `CcbDeliveryError: pane %3 prompt unchanged after Enter + retries`。导致 Patch 1 在 Gemini 上是"薛定谔的验证"，时好时坏。
- [19:58] VPS 资源严重不足：1C/951MB/23G，load 24.55，14 个 PPID=1 的 `codex_dual_bridge.py` 僵尸进程跨 10 天（4/13–4/22）累积，每个 0.2% 内存——codex-dual cleanup bug：claude 退出时 bridge 未被回收。
- [19:58] 磁盘 `/dev/vda2` 已用 97%，剩 853MB，连带拖累 swap/日志/临时文件。
- [21:20] Hysteria2 systemd 单元 `hysteria-server.service` 是 active running 但只在 UDP 监听，普通 `ss -tlnp` 看不到，需要 `ss -ulnp` 才能确认。
- [21:49] Claude session 崩溃（"刚才为啥直接崩了?"）— OOM 推测，但用户重连时进程已死，无 OOM 记录可看（dmesg 需 sudo）。
- [22:54] CCB v6 的 a3 (claude) provider-state 只 symlink 了 `.credentials.json` 和 `hooks/`，**没有** symlink `settings.json`/`stats-cache.json`/`settings.local.json`/`sessions/`——新项目第一次启动时 Claude CLI 会判定"未登录"+ hook error，几秒后冷启动竞态过去就静默登录上但红字错误留在 UI 历史。
- [23:00] 主控 Claude 自己之前推断 "Claude CLI 拒绝 symlink credentials" 错了——基于单一样本过度推论；用户让查全局后被打脸（所有 a3 都是 symlink，token sha256 一致）。
- [23:08] `~/.claude/settings.json` inline statusLine 用了**根本不存在的字段** `.context_window.current_usage.total_input_tokens`，被 `// 0` 兜底成永远 0%，导致状态栏 ctx 始终显示 0%——和同目录下 `statusline-command.sh` 用的正确字段 `.context_window.used_percentage` 不一致。
- [23:08] 服务器升级 4GB 后又快爆：6 个 claude（共 ~2.4GB）+ 6 个 gemini launcher/worker（~1.3GB）+ 3 个 codex（~0.3GB），used=2.8Gi/free=103Mi。3 个 CCB namespace 同时挂着（`/home/sevenx` + `video_analysis` + `agent-harness`），2vCPU/4GB 不够。
- [22:47] CCB 关闭流程不可硬杀：直接 `pkill ccbd` / `kill -9` 会让 ccbd 维护的 provider session 状态丢失，下次 mount 时 `health: restored` 失败甚至 session 丢。

### 2. 用户多次纠正 / 抱怨 / 吐槽 Claude 的内容
- [10:00] 用户大爆发表达 Claude 表述混乱："现在有点混乱，我们是在决策这次怎么改还是以后怎么做？能不能表述清晰一点，不要夹一堆术语，看不懂。我的最终目标是 Claude主控，codex编码，Gemini思考，凡事Claude要用户做判断的先交给Gemini，两人battle出结果。…claude.md违背我医院的必须改"。
- [10:11] 用户纠正 Claude 让规则妥协："1. Route肯定要。2、文档应该只保留一个。3、ccb原生的规则和我有冲突，应该按我的来。4、防止下次升级覆盖掉我的配置"——明确表达 ccb installer 不能再覆盖自己的定制。
- [13:38] 用户抱怨 Claude 习惯性问"是不是要下一步啦"："原先Claude每次都要停下来问我，是不是要下一步啦？这种stupid question。我需要主控帮我思考和回答这种问题，像一个项目经理帮我推进项目进度直到项目完成"。
- [14:21] 用户对 async guardrail 表达不满："为什么不是主动回复结果+主动取结果的双保险呢？如果Gemini任务回来了，不应该直接在这个对话中显示吗？为什么一点反应都没有"——指出 v5→v6 倒退（v5 主动回复，v6 啥反应没有）。
- [14:30] 用户继续质疑设计意图："之前v5不是主动回复吗？v6不主动回复是觉得哪里浪费资源了呢？问题是也没有解决好这个问题呀？库里面有没有写设计意图或者应该怎么执行的，是不是我们的配置又问题所以没有按照maintainer的设计意图执行？"。
- [13:15] 用户表达架构混乱困惑："我有些混乱,现在的流程应该是怎么样的?ccb中的Claude承担什么样的角色?ccb中的3个agent互不通信,全部集中到这里? 我非常confuse"。
- [14:37] 用户质问 Claude 改了 installer 自带的文件："==，为什么skill.md会错，这不是安装时候自带的吗？"——指出 Claude 之前提交的自定义 ask skill 覆盖了 v6 installer 的官方版本。
- [11:08] 用户两次纠正 Claude 描述"完整启动命令"含糊："不是这个意思, 我的意思是不实用任何配置的情况下,完整的标准启动命令应该是什么?还是ccb codex gemini claude 吗?"。
- [11:13] 用户嫌 Claude 给的方案麻烦："那么麻烦吗... dot files仓库必须把这些命令配置也放到checklist中,否则每次配置一次也很麻烦"。
- [11:16] 用户提醒 Claude shell 不一致："这里有一个问题,mac和linux用的shell不一样,mac用的zsh,所以在init的时候需要判断和确认,llm根据标准/指南去做动态配置"——Claude 没主动考虑跨 shell 兼容。
- [13:38] 用户截两次 paste 同一段需求（[Pasted text #1 +19 lines]），是因为 Claude 第一次回 "No response requested." 装聋作哑，用户被迫重发。
- [15:13] 用户多次要求 Claude "梳理一下当下的情况"——暗示 Claude 描述太分散难跟。
- [21:42] 用户要求计算单 ccb 内存："启动一次ccb3会吃掉多少内存? 我现在开了2个Claude code还剩多少内存"——Claude 之前没主动给量化数据。
- [22:31] 用户挑战 Claude 给的诊断："为什么会有两个Gemini" —— 后被告知是父子进程不是双实例。
- [22:54] 用户挑战 Claude 给的"hook error 是 bootstrap 时序"假设："为什么第一次启动的项目还是没有Claude登录信息，不是symlink吗?"——逼 Claude 重新查全局后承认前一次推论是错的（Claude：[23:00] "我之前的假设是错的"、"基于单一样本的过度推论"、"被数据打回了我的假设"）。
- [23:00] 用户纠正 Claude 不要急着改东西："我不要重新登录啊,从其他项目把登录信息复制过去,服务器上登录很麻烦的"。
- [23:05] 用户怀疑 Claude 偷改了文件："==现在那个项目已经登录了,你确认一下,是因为你改过了?还是他原本就已经在登录状态了"——用户主动 Request interrupted 阻止 Claude 写动作。
- [22:43] 用户被迫两次连续输入 "Continue from where you left off."、"继续?" —— Claude 在 session 切换后没自动衔接。

### 3. 用户表达过强烈意图
- [05:03] "我希望对正在使用的ccb进行优化，主要几个问题：1、发送内容会卡在对话框没有发送出去；2、根本没有发送过去；3、完成的内容没有自动发送回来，主控调度也没有主动轮询；4、不会主动清理上下文。根源问题：现在的ccb只是把命令发出去，没有一个系统性的确认结果步骤" —— 整轮工作的初始诉求，4 个问题点 + 根因。
- [06:51] "直接fork开发维护自己的库可以吗？包括一整套的superpower安装，Claude.md的配置，原先的库里都是没有的需要手动" —— 强烈表达"我要自己 fork 维护"。
- [07:03] "我觉得可以试策略1，但是不要停止我们的开发，因为我要快点解决这个问题才不会影响我其他项目的开发和使用" —— 双轨并行，不能阻塞日常项目。
- [08:36] "确认，开始干不要停" —— 明确禁止 Claude 中途停下问。
- [10:00] "Claude主控，codex编码，Gemini思考，凡事Claude要用户做判断的先交给Gemini，两人battle出结果。至于ccb的maintainer为什么要让codex审查计划，我觉得也可以啊，让codex一起审一下。至于你中间如何实现，我不关心ok？" —— 角色铁律 + 不关心实现细节。
- [10:11] "1. Route肯定要。2、文档应该只保留一个。3、ccb原生的规则和我有冲突，应该按我的来。4、防止下次升级覆盖掉我的配置" —— 4 条配置纪律。
- [10:19] "清理codex， 先不用codex，你自己把该改的改好，让我能重新启动ccb后你可以自己操作" —— 明示 Claude 自己干，别问用户。
- [13:38] "我需要主控帮我思考和回答这种问题，像一个项目经理帮我推进项目进度直到项目完成" —— 想要项目经理式自动推进，不是问问题机器。
- [13:53] "1.CCB_VERIFY_DELIVERY是我们自己加的?现在就改;2.先把我的需求落盘到文档,后续肯定要clear 上下文,不能把现在的需求丢失" —— 明确要求落盘防 /clear 丢失。
- [14:21] "为什么不是主动回复结果+主动取结果的双保险呢？" —— 想要双保险机制。
- [14:45] "我选A，与agent对话时会自动识别翻译成对应的命令吧，比如我在对话中让你用Gemini分析，我不会去用/ask gemini，如果要用command我也知道要用/ask a2" —— 明确自然语言 → 命令翻译是主控 Claude 的职责。
- [15:07] "1. coding/claude_code_bridge里面有错误的agent名应该要处理掉吧，保持和官方一直；2. …; 3. 得确保之后的改动不会影响官方的内容，除非是明确确认过的" —— 要 fork 安全纪律。
- [12:45] "我希望能在这个窗口给到在后台跑的进程提示,我想要注意力聚焦,并且我会remotecontrol这个Claude,开别的窗口我看不到" —— 要注意力聚焦的当前窗口提示。
- [20:11] "除了当前的claude会话窗口,清理掉所有的claude,gemini,codex,ccb" —— 清场。
- [22:35] "把tele gram的插件删掉吧，现在也不用tg了，然后终端命令cctg也删掉吧" —— 删 tg 整套。
- [23:00] "我不要重新登录啊,从其他项目把登录信息复制过去,服务器上登录很麻烦的" —— 拒绝走 OAuth 重新登录。

### 4. 对话中暴露的设计缺陷
- **CCB 是 fire-and-hope 单向通道**：投递、执行、完成三个层面都没有 ACK 闭环；mailbox_state=delivering 但 pane 实际没收到，只能靠用户感觉"卡住了？"才发现。
- **CCB 的 env allowlist 静默剥光未列入的 CCB_\* 变量**：Patch 1 的 `CCB_VERIFY_DELIVERY`、ready timeout 的 `CCB_GEMINI_READY_TIMEOUT_S`/`CCB_CLAUDE_READY_TIMEOUT_S` 都不在 allowlist，加 export 后仍然打不到 ccbd 子进程；连续两次踩同一个坑。
- **CCB v6 完成态没有"主动通知"通道**：完成只写 mailbox/executions 状态文件，不 push 给主控 Claude；主控因 Async Guardrail 又不准 poll → 双方都在等对方，用户成消息中转站。
- **Patch 1 fingerprint 策略基于"最后 3 行非空文本变化"**，对 TUI（Gemini CLI）静态状态栏完全无效——任何"取尾部行 hash"的 verify 都会有这种 false positive。
- **CLI 退出码与真实健康状态脱节**：`ccb3` 报 `command_status: failed, error: timed out`，同时 `ccb ping ccbd` 是 `healthy` + 3 agent alive；用户看到 failed 就以为坏了。
- **provider ready 检测是单次时间驱动而非进度驱动**：20s 死线一到就 kill 重启，没法区分"还在登录刷新中"和"真死了"，对 40s 才起的 Gemini 等于无限循环。
- **CCB v6 a3 (claude) provider-state 的 symlink 集合是部分镜像**：只 link `.credentials.json` 和 `hooks/`，没 link `settings.json`/`stats-cache.json`/`sessions/`，导致冷启动时 Claude CLI 的多文件登录态判定有时序竞态，UI 红字"未登录"+ hook error 误报。
- **CCB v6 在 source 仓自带 `.ccb/ccb.config` 用 `agent1..agent5` 命名**（maintainer 开发用的），和用户日常 `a1/a2/a3` 不一致；只要 cwd 飘到这个仓就 `unknown agent: a2`——cwd 决定 registry 但没有显眼提示。
- **同项目下并发多个 Claude Code 没有 caller 隔离**：会连同一个 ccbd singleton，消息靠 `serial-per-agent` 串行不冲突，但 agent context（如 a2 的 Gemini）共享，会**互相污染对话历史**；CCB 没有"per-caller agent session"机制。
- **CCB installer 重写 CLAUDE.md 会擦掉用户的 analyst/executor/Domain Analysis 段落**——v6 升级直接覆盖。Route 模式存在但默认 inline，用户必须显式 `CCB_CLAUDE_MD_MODE=route` 才能避免。
- **CCB installer skills 写到 ~/.claude/skills/ 时递归 cp 会留下嵌套重复目录**（references/references、agents/agents、templates/templates）—— 重装一次就多一层。
- **CCB v5→v6 配置格式不向后兼容**（旧 JSON `~/.ccb/ccb.config` 让 v6 直接报 `invalid layout token '{'`），升级后没自动迁移也没显眼报错。
- **codex-dual cleanup bug**：每个 claude session spawn 一个 `codex_dual_bridge.py`，claude 退出时 bridge 不被回收 → PPID=1 孤儿 + 内存泄漏。10 天 14 个累积。
- **claude-hud / status line 设计**：claude-hud 用 bun（spawn 成本 ~200ms + 40-50MB RSS），但所有数据 stdin JSON 已经原生提供，一行 jq 就够；choosing 重 statusline 等于把 1C1G 的 VPS 拖死。
- **inline statusLine jq 表达式和文件型 `statusline-command.sh` 字段名不一致**（前者错用 `.context_window.current_usage.total_input_tokens`，后者用对的 `.context_window.used_percentage`），同一个 dotfiles repo 里两份字段定义飘忽。
- **Claude 主控写 spec 时单一样本就下结论的倾向**：[23:00] 自己承认"基于单一样本的过度推论"——修复方向应是规则强制"做出判断前先核对全局/对照样本"。

### 5. 决策转折点
- [05:03] 用户开新 session "/clear" 后启动整轮重大工作："我希望对正在使用的ccb进行优化，主要几个问题：1、发送内容会卡…" —— 启动 4 个 CCB reliability 痛点的全面优化，进入 brainstorming skill。
- [06:30] "先快速按b执行" —— 4 选 1 选 B（Execution State tracking），抛弃 A（仅 ACK）和 C（完整会话管控）。
- [06:51] "直接fork开发维护自己的库可以吗？" —— 转折：从"包 wrapper"转向"fork CCB + 建 dotfiles repo 双仓"。
- [07:03] "我觉得可以试策略1，但是不要停止我们的开发" —— 决策双轨并行（先发 upstream issue + 同时本地干）。
- [07:06] "你能直接操纵git发吗？ 纳入" —— 同意 Claude 直接用 gh CLI 发 issue，并把"上下文清理"问题 ④ 纳入本轮。
- [09:39] "b" —— Codex plan review 跳过；spec 直接进入实施（Patch 1 + Patch 2）。
- [10:00] "现在有点混乱…claude.md违背我医院的必须改" —— 重大转折：要求重写 CLAUDE.md 角色规则（Codex 编码 / Gemini 思考 / 不交叉），抛弃之前 installer 写的 executor=claude。
- [10:11] "1. Route肯定要。2、文档应该只保留一个。3、ccb原生的规则和我有冲突，应该按我的来。4、防止下次升级覆盖掉我的配置" —— 决定改用 `CCB_CLAUDE_MD_MODE=route` + 把规则迁到 `ccb-collaboration.md`（installer 不碰）。
- [10:19] "清理codex， 先不用codex，你自己把该改的改好" —— 决定不走 Codex 执行，主控 Claude 自己改 patch 代码（违反"不写代码"铁律但用户授权）。
- [13:53] "1.CCB_VERIFY_DELIVERY是我们自己加的?现在就改;2.先把我的需求落盘到文档,后续肯定要clear 上下文,不能把现在的需求丢失" —— 转折：写 `WORKFLOW-VISION.md` 防 /clear 丢需求。
- [14:21] "先做A" —— 选 A（只调大 timeout），暂不做 B（重试机制）。
- [14:37] "==，为什么skill.md会错，这不是安装时候自带的吗？" —— 触发 ask skill 回滚到 v6 官方版本（rm 自定义、让 installer 重装）。
- [14:45] "我选A" —— 让自然语言→命令翻译留在主控 Claude，不在 skill 里做。
- [15:34] "需要" —— 决定关闭 Patch 1（fingerprint false positive 已确认），日常依赖 `ccb ask --wait`。
- [20:11] "除了当前的claude会话窗口,清理掉所有的claude,gemini,codex,ccb" —— 系统级清场。
- [21:56] "我把服务器升级可以吗？" —— 从 1C/1G/25G 升到 2C/4G/80G（不可逆 + 需停机）。
- [22:35] "把tele gram的插件删掉吧" —— 决定彻底删除 telegram plugin + cctg alias，回收 466MB RAM。
- [23:25] "修statusline 改A" —— 选最小改动（修字段名）而非切到 sh 脚本。
- [23:00] "我不要重新登录啊" —— 拒绝 Claude 提议的 `/login` 方案，逼 Claude 重新查根因（结果是 Claude 自己之前推论错了，credentials 其实早就 symlink 共享了）。

**核心主题**：用户从"4 个 CCB 痛点"出发，把 CCB v6 当 fork 实战场——发 upstream issue #178、改 ask skill、加 Patch 1 verify + Patch 1.1 allowlist、改 CLAUDE.md route 模式、写 WORKFLOW-VISION 防丢需求；同一天反复发现"Claude 主控自己也是问题"（停下来问蠢问题、不主动取 reply、单样本就下结论），并因 1C/1G VPS 资源极度紧张而被迫升配 + 整轮清场（杀 14 个 codex_dual_bridge 僵尸、删 tg 插件、修 statusline）。
