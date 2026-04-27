# home-sevenx 2026-04-26 session findings

### 1. CCB / Claude / Gemini / Codex 的 bug 或失败行为

- **[17:42] CCB cleanup-orphans 漏扫 project_dir** — `claude-ccb-orchestrator cleanup-orphans` 只对账 systemd unit + tracking.json，完全不扫 `~/.local/state/claude-ccb-projects/` 下的孤儿 project_dir（11 个昨天的孤儿目录从昨天积到今天没清）。和 `~/.claude/rules/ccb-orchestration.md` 里 "Janitor 自动兜底...默认路径下孤儿 project_dir" 描述对不上。
- **[17:47] janitor.timer 单调时钟 wedge** — `claude-ccb-janitor.timer` 卡在 `Trigger: n/a`，`NextElapseUSecMonotonic=infinity` + `LastTriggerUSecMonotonic=0`。原因：timer 用 `OnBootSec=5min` + `OnUnitActiveSec=5min` + `Persistent=true`，user-systemd 在 8h 前重启过，磁盘持久化的 LastTriggerUSec 跟 monotonic baseline 对不上。`systemctl --user restart` 修不掉，必须 `daemon-reexec`。Fix：改成 `OnCalendar=*:0/5`。
- **[17:42] 同项目两个 master Claude 共用同一个 ccbd** — pts/1 (10:22 启) + pts/2 (17:41 启) cwd 都在 agent-harness，cwd-walk 都命中同一个 ccbd singleton，a2/a3 上下文会污染（消息会串入同一个 agent CLI 对话历史）。
- **[18:02] CCB ask 投递 + completion detector 误判（核心 bug）** — Claude 发 13KB prompt 给 a2 Gemini，CCB 90s 后标 `completion_item` 但 Gemini TUI pane 仍停在 17:44 的旧 "READY" 探针上，prompt 完全没注入。`anchored_session_stability` detector 把旧 READY 当成新 job 完成，job 假死在 running。
- **[18:18] `ccb ask wait <job_id>` 命令格式 bug** — 文档写 `ccb ask wait <job_id>` 但实际 `ccb ask wait <job_id>` 直接报 `error: ask wait requires <job_id>`，无论 `--timeout` 放在前后都不行（`command ccb ask wait job_xxx --timeout 60` 也报同样错）。
- **[18:19] mv 误把 `~/.ccb` 备份成 ccbd 子目录** — Claude `mv /tmp/ccb-stale-backup /home/sevenx/.ccb` 把备份嵌成 `~/.ccb/ccb-stale-backup-...` 子目录，因为目标目录已存在。Claude 误以为 `~/.ccb` 已删，实际另一个 ccbd 还在用它做 anchor。
- **[18:19] Claude 误删用户活的 ccbd state** — Claude 看到 `agent-harness` 是唯一活的 ccbd 就把 `/home/sevenx/.ccb/` 整体 mv 走，但用户的另一个 ccbd（`/home/sevenx` workspace）仍以 `~/.ccb/` 为 anchor，导致 a3 hook `claude-remote-session-start.sh` not found。
- **[20:42] CCB ask 派 Codex 卡死** — `ccb ask --wait a1` 派 Codex 干 Phase 1 brief，30 分钟里只发心跳，Codex pane 还停在欢迎屏 `Implement {feature}`，brief 根本没注入。CCB 投递 bug 二次复现。
- **[21:37] Codex headless `cargo install` 在 sandbox 改不了 git** — Codex headless mode 跑在 sandbox 里 `.git` mounted read-only，`git commit` 失败：`Read-only file system / inability to create .git/index.lock`。Codex 把 patch 全写完了但提不了 commit，要主控 Claude 在外面手动 commit。
- **[00:14] phase1-B HOME 完全隔离意外断了 OAuth** — Claude 把 `$HOME` 全沙盒化后，三个 agent 都看不到 master 的 OAuth credentials，Gemini 跳 Google 授权 URL，Claude 跳 OAuth 登录界面，Codex 死循环 retry resume 不存在的 session ID。**用户原意是登录共享 + 历史隔离，Claude 错做成全隔离。**
- **[00:17] codex launcher 不验证 session 文件存在就 `--resume <id>`** — phase1-B 切 HOME 后，`.ccb/.codex-aN-session` 里 codex_session_id 还指向老 HOME 的 jsonl 文件，新沙盒里没有 → codex 死循环 retry 22 次。
- **[00:24] sandbox `.claude.json` 残留导致 Claude 走 OAuth** — phase1-D 部署后，`_ensure_trust_file` 在沙盒里写了**空 `{}`** 的 `.claude.json`，下次 materialize 看到真文件已存在就 skip symlink。Claude CLI 拿空 `{}` 当 onboarding state → 走 OAuth。要手动 `rm` 残留才 work。
- **[00:34] a3 Claude Stop hook not found（master settings 用 $HOME 写死）** — master `~/.claude/settings.json` 里 Stop hook 写 `$HOME/.claude/hooks/...`，a3 进程 HOME 被 phase1-B 沙盒重写，`$HOME/.claude/hooks/` 在沙盒里没有 → non-blocking error。修法：master settings 改绝对路径。
- **[01:31] phase1 后再次复现 a2 completion_terminal 误判** — Claude 给 a2 发新 prompt，CCB 在 40s 标 `job_failed`，但 Gemini pane title 显示 `✦ Working…` + 在 ReadFile + Thinking。CCB job status 跟 agent 实际状态脱节。
- **[01:36] agent-harness ccbd Codex stale session resume 失败** — 用户截图反馈 codex 在 agent-harness 里也踩 stale session ID 死循环（019db764-...），phase1-D 当时只手动 rsync 了 /home/sevenx 项目的 codex session，agent-harness 项目的 codex session 没动。
- **[18:30+] CCB 把 17:44 的旧 "READY" 探针 reply 当成 18:02 新 job 完成信号** — `gemini-requests/job_77b64e5c6f59.md` 是 17:44 探针（33B "Reply with the single word READY"），但 18:02 新 job_2447cee7662a.md（13KB）的 reply 提取从来没成功，CCB 误把旧 READY 当成新 job 完成。

### 2. 用户纠正 / 抱怨 / 吐槽 Claude 的内容（带原话）

- **[18:46] 用户对 Gemini 评估的 Python 推荐反驳** — "现在是AI vibecoding，原来几周的代码量现在几个小时就完成了。更没有心智负担一说，反而对于AI而言严格的编译对结果更友好"；"你得从现实情况去重新评估ccb这个项目，他和AI几乎没有关系，他说白了就是个cli多开的管理器和操作系统，根本没有调用llm"
- **[19:00] 用户表态对 ccb 现状不满** — "我现在非常想重构整个项目，这个项目本身非常不成熟，是用传统的开发思维，小步快跑，快速迭代去推进的，所以在我看来就是一直在打补丁，根本没有从中局考虑去顶层设计"
- **[23:39] 用户纠正 phase1-B 完全隔离方向错** — "我不是要登录信息完全隔离啊，现在我的codex，Gemini，Claude都是账户订阅登录的，我希望项目所有打开的ccb agent都能软连接到服务器根目录的登录信息，否则像Gemini现在ccb打开就要验证登录"
- **[00:25] 用户指出 phase1-D 后 Claude 仍坏** — "codex似乎是OK了，Claude还是不行，没改之前是可以的，查一下改了什么"（指出回归：phase1 之前可用，phase1 之后反而坏了）
- **[00:32] 用户提出主控应该自己看 pane** — "我其实在终端不是很能看得清，你自己通过后台看一下每个agent的情况吧"
- **[01:04] 用户尖锐质疑 Claude 工作方式（核心反思点）** — "1. 你是怎么用Gemini的？我已经启动了的ccb没有用吗？2.你和Gemini的沟通方式是怎么样的？为什么总感觉你要把上下文喂给他而不是让他自己去看呢？他也是个agent，有读写工具的啊"
- **[01:05] 用户进一步强调** — "而且他自己读肯定比你断章取义要好，信息完整度要高"
- **[01:36] 用户对 phase1 仍有问题不满** — "为什么另一个项目还是会遇到这个问题"
- **[01:37] 用户要求修 phase1 而非继续推进** — "phase1还是有问题，先把phase1修好"

### 3. 用户强意图（带原话）

- **[17:42]** "扫描整个服务器:有没有僵死的进程. ccb进程是否与项目绑定, Gemini是否可用"
- **[17:47]** "清, 检查服务器健康状态"
- **[17:52]** "先清干净"
- **[17:56]** "让Gemini全局审核现在的ccb系统, 目前的实现和最初的design对齐情况,分析现在的整体情况和问题,把刚才的问题也考虑进去. 另外再思考一个问题,我们修了那么多内存泄漏问题, 如果用rust重新写ccb功能是否会更好"
- **[17:59]** "不要断章取义我的问题,也不要断章取义Gemini的回答,把他的回答原封不动的贴给我. 如果Gemini的回答不够深入,让他一次旧单个问题深度分析"
- **[18:59]** 三个核心诉求：
  - "1、先把现在的ccb修复到靠谱可用，哪怕回归我人为介入也可以，但是至少在我后面的开发过程中可用，项目之间的ccb不要串台，各个agent登录信息管理稳定，用最快速的方式实现"
  - "2、如果要重构的话需要重新设计，包括我的workspace，现在我一直是在存放所有projects的根目录下开发，因为我要做的其实是我的整个开发辅助系统，他其中的一部分就是主控Claude的全局配置.Claude"
  - "3.我的终极目标是做一个spec coding的辅助开发系统，开发引擎是由一个主控agent通过ccb操控多个辅助agent，自动完成项目开发。每开一个项目需要自动配齐agent所需的各种插件、skill、hook、rules等，不需要我再去关心这些配置的问题。并且需要一个强力的spec_driven系统，开发过程需要一个流程化的pipeline，而不是依赖agent本身去"尽可能"的逼近规范流程，agent是去解决具体问题，拿到最后一公里的"
- **[20:23]** 深入讨论意图：
  - "1、phaseI 止血方案"
  - "2.rust重构workspace放在根目录开发没问题吗？还是进到rust新项目路径下开发？这需要考虑的是，运行环境可能不一致，AI是否能够进行自主测试？"
- **[20:39]** "尽快把phase1完成"
- **[00:01]** "做，把所有ccb和Claude都清理，我项目已经都停下来了，顺便看下服务器还有那些孤儿进程需要清理掉"
- **[00:39]** "OK，把之前和Gemini聊的rust重构方案，写成一个文档，然后建一个新的仓库，我们要开始新项目"
- **[00:56]** "有现成的当然最好了，把他们拉下来你和Gemini一起看，仔细辨别功能与我需求的差别"
- **[01:30]** "重做一次评估分析"

### 4. 对话中暴露的设计缺陷

- **[18:00] CCB cleanup-orphans 实现与文档不一致** — "Janitor 自动兜底...默认路径下孤儿 project_dir 自动清"是文档承诺，但 `cleanup-orphans` 实现里只做 systemd unit + tracking.json 对账，根本不扫 project_dir。这是文档与实现的契约漂移。
- **[18:00] CCB singleton 隔离粒度只到进程级** — Gemini 在 (1.1) 评估指出："守护进程**仅仅做到了进程级别的排他，却没有实现会话（Session）与调用方（Caller）的逻辑多路复用与隔离锁定**"。两个 master 同 cwd → 同一个 ccbd → 上下文污染。
- **[18:00] systemd OnBootSec/OnUnitActiveSec 在 user-systemd 重启后 wedge** — monotonic clock 与磁盘持久化的 LastTriggerUSec 不一致时无法计算下次触发，restart timer 修不掉，必须 daemon-reexec。这是 timer 选型问题，不是配置问题。
- **[18:00] CCB 完成检测把 stale 文件残留当成新 job 完成信号** — `gemini-requests/job_*.md` 文件、tmux pane buffer 里的旧 "READY" 字符等多个状态源，但 detector 没有把"哪个属于本 job"绑定起来。
- **[18:00] CCB Python 实现充斥 4 层嵌套 compatibility facade** — Gemini (1.2) 直接定性 "Shotgun Surgery"：核心状态机 happy-path 设计，无法消化系统级异常（进程残留 / IO 阻塞 / 异常退出），开发者只能在外层堆 if/else 和拦截器掩盖问题。
- **[18:00] CCB 用文件系统状态代替数据库状态导致脑裂** — Gemini (1.3) 判定 5 个具体问题（双 master 共享 / 孤儿 session / 11 个孤儿 project_dir / cleanup 漏扫 / janitor wedge）根因同一类："系统缺乏一个强一致的、中心化的 Source of Truth (SoT)"，状态变更和记录状态不是一个原子事务。
- **[18:00] CCB 用外部 systemd timer 解决内部状态不一致** — Gemini (1.3.4) 指出：现代分布式系统标准做法是内部 Reconciliation Loop（Kubernetes 风格），不是依赖外部时钟触发器。
- **[20:23] 用户提示 Phase 2 Rust workspace dev/prod 路径问题** — AI vibecoding 时，dev path（cargo target/）和 prod path（XDG ~/.local/state/）需要明确切换机制，否则 AI 自己跑测试时会污染 prod 状态。
- **[20:23] AI 自主测试有边界** — 主控 Claude 不能边改 ccbd 代码边测自己依赖的 ccbd 实例。需要 mock_agent + cargo test + 用户介入 sanity check 三层。
- **[00:25] phase1-D 部署后 .claude.json 真文件残留导致 symlink 永远不生效** — `_ensure_trust_file` 早期跑写了空 `{}`，后续 materialize 看到真文件就 skip。这是状态机设计的 ordering 问题——materialize 应该先于 ensure。
- **[00:34] master Claude `$HOME/.claude/hooks/...` 在 sandbox HOME 重写后失效** — 用户全局 settings 用 `$HOME` 表达式假设 agent 进程 HOME 等于 master HOME，但 phase1-B 沙盒打破了这个假设。这是"宿主进程环境继承"假设被推翻。
- **[01:01] Gemini 7 项目对比给出过快结论** — Gemini (4.4) "1-2 周"和之前自己说的"Phase 2 = 2-3 天"差距大，估算来源不一致。没有给到中间值。
- **[01:04] Claude 滥用 headless `gemini -p` 而不用 a2 agent** — Claude 因为一次 CCB 投递失败就退到 headless 模式，把 a2 当摆设；同时把候选项目 README 摘要后才喂给 Gemini，等于让领域专家在 Claude 偏见过滤过的二手材料里挑选。
- **[01:31] CCB 投递修了一部分但 completion_terminal 误判仍未修** — 18:02 那次 prompt 完全没注入，01:31 这次 prompt 注入成功了（部分修复），但 completion detector 仍在 40s 误判 job_failed，Gemini 实际还在跑。

### 5. 决策转折点

- **[18:00] Gemini 第一轮：保留 Python，引入 SQLite SoT** — Gemini 给出 35/100 健康度评分 + 推荐 (d) 保持 Python 修架构层 source-of-truth 问题，理由是 AI 研究者母语是 Python、12000+ 行文档作废成本太高、跨语言 IPC 是最差选项。
- **[19:00] 用户两个反驳推翻 Gemini 第一轮** — (i) AI vibecoding 时代心智负担在 AI 不在人，编译器严格反而对 AI 更友好；(ii) CCB 不是 AI 应用，是系统级进程管理器/IPC 总线/微型操作系统，Python 是错的语言。
- **[19:00] Gemini 第二轮：推翻自己，强烈推荐 Rust 重写** — "我用传统的人肉工程学评估 AI 时代开发范式，完全错判"+"错判项目领域模型"。给出 3 阶段时间线（3h 止血 / 2-3 天 Rust ccbd / 4-5 天 spec pipeline）+ Big Bang 重写而不是 Strangler。
- **[20:23] 用户决定 Phase 1 先做止血** — 不再讨论后续，专注 Phase 1。
- **[20:42] Codex 通过 CCB 派活失败 → 切 headless** — `ccb ask --wait a1` 30min 心跳但 brief 没注入，Codex pane 卡欢迎屏。Claude 切 `codex exec --full-auto --cd <repo>` headless 模式直接跑，绕开 CCB 投递。
- **[21:37] Codex 完成 phase1-A/B 但 sandbox 改不了 git，主控代理 commit** — Codex headless 输出 patch，但 .git read-only。Claude 在主控环境 review + git add + git commit + git log，落地 `bb480ab` 和 `a2cbcdd` 两个 commit。
- **[23:39] 用户纠正方向：登录共享 + 历史隔离，不是全隔离** — 反向修正方向。phase1-C 添加 `PROVIDER_AUTH_WHITELIST` 包括 .claude/.credentials.json + .codex/auth.json + .gemini/oauth_creds.json 等 6 个文件 symlink，故意不 symlink config.toml 等 launcher 会写的文件。
- **[00:14] phase1-D 加 .claude.json 修复 Claude OAuth 弹窗** — 发现 Claude Code v2.x 读顶层 `~/.claude.json` 38KB onboarding 文件来判断已 onboard，phase1-C 漏了这个。
- **[00:25] 用户挑战："改之前可以"** — 用户指出回归，Claude 查到 `_ensure_trust_file` 写空 `{}` 残留覆盖 symlink 路径，删残留 `.claude.json` 后修复。
- **[00:39] 用户决定开 ccbd-rust 新项目** — Phase 1 暂告段落，开始 Phase 2 准备工作。Claude 装 rustup，建 `/home/sevenx/coding/ccbd-rust/`，cargo init，写 DESIGN.md + README.md，初次 commit `40467ff`。
- **[00:56] WebSearch 发现 7 个高度重合的开源项目** — Batty / Tamux / ao-rs / Overstory / ccswarm / CAO / metaswarm / agent-orchestrator。从"自己写"转向"先评估 build vs fork"。
- **[01:01] Gemini 第三轮：拒绝 fork，自研 ccbd-rust + 抄 Metaswarm prompt 资产** — 7 个候选都不能直接用，但可以函数级别抄 Batty 的 stream-json 解析 + stall detection、Metaswarm 的 skills/rubrics、CAO 的 Handoff/Assign/SendMessage 模式。
- **[01:04] 用户根本性纠正 Claude 与 Gemini 协作方式** — 指出 Claude 用 headless 而非 a2、用摘要而非指针。Claude 承认错误，准备重做评估时让 Gemini 自己读源码。
- **[01:30] 用户决定重做评估** — 通过 CCB a2 而非 headless，prompt < 1KB 只给路径不给摘要。验证 phase1 后 CCB 投递 bug 是否真修好（部分修好：注入 OK，但 completion 仍误判）。
- **[01:37] 用户暂停 Rust 重写，要求继续修 Phase 1** — agent-harness 项目 codex session resume 仍坏 + Gemini auth 在另一项目失效，Phase 1 还没真正稳定。

---

**核心主题**：Claude 主导 CCB Phase 1 止血（owner-PID lockfile + agent HOME 沙盒 + auth symlink 白名单 + .claude.json 修复，4 个 commit），但每一轮修复都触发新的回归（HOME 全隔离断 OAuth → 改白名单 symlink → .claude.json 残留覆盖 → 还要手动迁 codex stale session）；同时 Gemini 完成 CCB 全局架构审计（35/100，Shotgun Surgery，缺 SoT），用户基于 AI vibecoding + 项目领域不是 AI 而是系统编程的反驳推翻 Gemini 第一轮"保留 Python"结论，确立 Rust 重写终极路径；用户最后犀利指出 Claude 滥用 headless gemini + 摘要喂 prompt 的协作反模式，要求改用 CCB a2 + 给指针让 Gemini 自己读，揭示 Claude-Gemini 协作纪律的根本缺陷。
