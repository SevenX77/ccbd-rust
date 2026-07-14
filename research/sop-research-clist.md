# 编程场景 SOP 沉淀 — 调研环汇总清单(给 operator 过目)

汇总自两路调研:`sop-current-state-inventory.md`(a1,ah 现状物化)+ `sop-agent-harness-learnings.md`(a2,agent-harness 借鉴)+ master 编排者脑内流程(本会话 T1/T2/T4 实证,workers 看不到)。

---

## 0. 一句话现状

ah 的编程 SOP **确实存在且稳定在跑**,但**物化得很碎**:一部分在 sandbox kernel(贡献者在仓库里看不到)、一部分在 memory、一部分散在 feature brief / PR report、**相当一部分只在编排者脑子里(a1 点了 15 条 in-head)**。仓库唯一面向人的 `CLAUDE.md` 反而**过时**(见下)。

**核心张力(和你的零回归红线直接相关)**:这次任务是「**整理并沉淀现在实际在跑的 SOP**」,不是「引入更好的 SOP」。所以要把两类东西严格分开——
- **A 类=纯沉淀**:把已经在跑的行为写下来 / 归位,行为一模一样(零回归)。← 本任务主体。
- **B 类=行为变更**:agent-harness 那套 worktree/脚本/auto-merge 大多是 ah **现在没在做**的新能力。**采纳它们=改变现状**,越过你的零回归红线。← 应作为**单独议题**,不混进这次沉淀。

---

## 1. 三个必须先拍的发现

### 发现①:仓库 `CLAUDE.md` 已过时(materialized-but-stale)
- `CLAUDE.md:17` 仍写 `ccb ask`,实际早已是 `ah ask`(真契约在 sandbox master kernel:`.../2ff8aed8d8f7/.claude/CLAUDE.md:16-20`)。
- `CLAUDE.md:22-25` 仍是**旧三角色** a1codex/a2**gemini**/a3**claude**;实际在跑的是**四角色**:a1 codex 主力实施、a2 codex 严审、a3 gemini/antigravity 设计不写码、a4 claude 二审+e2e。
- **这条本身就是零回归沉淀的正当对象**(把文档对齐到实际行为),但它会改 `CLAUDE.md` 文本 → **要你点头**。

### 发现②:真正的编排契约不在仓库里,在 sandbox kernel
- `ah ask` 分派、`ah pend/watch/logs/ps/attach` 读证据、cutover/revival ACK、安全边界(不越过 ah 杀 pane/session)——全物化在 master/worker kernel(`.../sandboxes/<hash>/.claude/CLAUDE.md`),**仓库贡献者/新人看不到**。
- 沉淀方向:是否把「面向人可读」的一份 SOP 放进仓库(如 `AGENTS.md`/`docs/`),让 kernel 保持机器契约、仓库有人读版本。

### 发现③:15 条 in-head 纪律(a1 §C 全量,我按可沉淀性归类)
| # | in-head 纪律 | 现在零散在哪 | 归类 |
|---|---|---|---|
| 1 | 四角色模型(当前口径) | 无中央;CLAUDE.md 是旧版 | A 纯沉淀(改 stale) |
| 2 | 分派命令 `ah ask`(非 ccb) | master kernel 有,仓库 stale | A |
| 3 | 串行全量 cargo(`env -u AH_STATE_DIR CARGO_BUILD_JOBS=1 --test-threads=1`,不过滤子集) | memory `brief-workers-full-cargo-test` | A(memory→规则/skill) |
| 4 | TDD 红绿:先贴红灯输出再转绿 | brief/PR report 样本 | A |
| 5 | baseline 证伪红灯(stash/archive 对照 main,别口头"无关") | 仅 t4/t2 brief | A(这次实证抓到价值) |
| 6 | worker brief 模板(只改X文件/别碰未跟踪/别push/回报 diff+test) | 每个 brief 手写 | A(最适合做 tool/模板) |
| 7 | 分支策略(从 main 开、别在 main 改、命名、同任务不再切分支) | git 习惯+brief | A |
| 8 | 提交策略(只 add 目标 tracked、不 `add -A`、Co-Authored-By trailer) | task_plan + git 历史 | A(最适合做 tool/脚本) |
| 9 | PR/CI/合并权责(worker 不 push;PM-proxy 开 PR/看 CI/合) | CLAUDE:18 + memory 部分 | A |
| 10 | 验证门顺序(PM-audit → a2 严审 → a4 二审/e2e → push) | PR report 记录,无门文档 | A |
| 11 | real-provider flake 处理(单跑确认+说明耦合/baseline) | memory+brief | A |
| 12 | worktree 策略(现在到底用不用) | 见发现④ | **待定** |
| 13 | 沙箱隔离细则(sandbox HOME/IS_SANDBOX/OAuth-only/禁改 host 路径) | kernel+memory+handoff 散落 | A(汇一份) |
| 14 | 三方职责(master 不跑 cargo / worker 跑 / PM-proxy gh) | memory 清晰,不在仓库规则 | A |
| 15 | 调研纪律(读不到标不可达、拿不准标待定+原因) | 本任务 brief | A |

### 发现④:worktree 现状被我上一轮说错了,需澄清
- 我之前判「共享单一工作树、没用 worktree」——**不准确**。`git worktree list` 实际有 4 棵兄弟树(`ccbd-rust-completion`/`-skills`/`-wsl-onboarding` + 一棵 release worktree)。
- **但**:本会话 T1/T2/T4 我是**在主树 main 上开分支、串行 commit** 跑的,**没用**那些 worktree。所以真相是:**worktree 在仓库里可用、别的 feature 线在用,但 worker 派单流程当前跑的是"主树串行 commit"**。
- 这正是 agent-harness 那套「一任务一 worktree」能补的位——但**采纳=行为变更(B 类)**,不是沉淀。

---

## 2. master 编排者脑内流程(本会话实证,workers 看不到,补全 in-head)

本会话 T1/T2/T4 实际跑的完整闭环(这就是「现在在跑的 SOP」的活样本):
1. **调研**:master 用 bash grep / 少量 Explore 先钉 file:line(池紧时不开 claude 子代理),写进 `research/t*-brief.md`。
2. **派单**:`ah ask a1 "...读 brief 文件..."`;brief 带死:分支名、落点、TDD、串行 cargo、只改X、别 push、回报格式。
3. **盯**:后台 `ah pend <job>` 阻塞等完成(不轮询);完成读 output file。
4. **PM-audit**:master 亲自 `git diff` 审 worker 改动(master 不能跑 cargo,靠 diff+worker 测试输出)。
5. **审查**:派 a2(codex)严审;关键改动 + 池够时 a4(claude)二审;**逼 baseline 证伪红灯**。
6. **收敛**:审出问题 → **一次性**并成一轮修给 a1(避免多来回)→ 复验。
7. **收口**:`git add <目标文件>`(不 `add -A`)+ Co-Authored-By trailer → commit → push 分支 → 回 operator 开 PR/CI/合并。
8. **池管理**:claude 池紧 → 审查优先 codex(a1/a2),a4 留关键处。

**这 8 步几乎全是 in-head**——没有任何一个仓库文件把它写全。这是本次沉淀最大的价值点。

---

## 3. agent-harness 借鉴(a2 §D 九条)—— 按零回归红线重新标类

| # | 借鉴 | 对 ah 是 | 说明 |
|---|---|---|---|
| 6 | 跨工具单一 SSOT(AGENTS.md 一份、CLAUDE/CONTRIBUTING 只指过去) | **A 可沉淀** | ah 就该把散落的 SOP 收进一个人读入口,零行为变更 |
| 8 | 声明「哪些共享文件不能并行改 / owns_files 不相交」 | **A 可沉淀** | 派单层前置冲突,不改现有行为 |
| 1 | 一任务一 worktree,根树只保 main | **B 行为变更** | ah 现在主树串行 commit;采纳=改现状 |
| 2 | 开工脚本从 origin/main 切树 + 预热依赖 | **B** | 新增 `ah task start` 级动作 |
| 3 | `wt-ship.sh` push+PR+auto-merge 收口 | **B(且危险)** | ah 要保 PM/a2 审查门,**不可**直接学 0-approval auto-merge |
| 4 | 清理脚本默认只清自己命名的树 | **B** | 仅当采纳 worktree 才有意义 |
| 5 | per-worktree 预览/测试绑自己的代码 | **B** | 同上;对 ah 映射到 cargo/ahd socket/tmux/sandbox |
| 7 | CI required checks 做机器合并门 | **B** | ah 现在 PM 人肉守门;上 CI 门=改流程 |
| 9 | 合并后主树刷新 + 运行态重建 SOP 化 | **B** | 涉及 ahd 长驻重启,需设计 |

**top 该学(a2 判)**:一任务一 worktree / 清理只清自己 / 预览绑自己代码——**三条都是 B 类行为变更**。

---

## 4. 需要你在这个 gate 拍的决策

1. **范围**:这次只做 **A 类纯沉淀(零回归)**,把 B 类(worktree/脚本/auto-merge/CI 门)另立一个「SOP 增强」议题以后单独评?
   —— 我的建议:**是**。零回归红线下,B 类不该混进来。但其中 **SSOT(#6)和文件所有权声明(#8)是 A 类**,可纳入本次。
2. **stale 文档**:同意本次把仓库 `CLAUDE.md` 的 `ccb ask`→`ah ask`、旧三角色→当前四角色**对齐到实际行为**?(这是沉淀,不是改行为,但会改文本。)
3. **人读 SSOT 落点**:同意新建/复用一份仓库内人读 SOP(如 `AGENTS.md` 或 `docs/sop/`),把编排契约(现只在 kernel)+ §2 那 8 步 + 15 条 in-head 收进去?kernel 保持机器契约不动。
4. **worktree(发现④)**:确认本次**不**改派单流程为 worktree(维持主树串行 commit),worktree 采纳归 B 类以后议?

---

## 5. 下一步(设计环,待你放行)

拿到你对 §4 的答复后,设计环产出「**每条 A 类 SOP 的归属建议**」:
- **tool/脚本**(确定性动作,优先):`git add 目标文件 + Co-Authored-By commit`、worker brief 模板生成、`ah pend` 收结果封装等。
- **skill**:可复用的多步流程(如「派单→盯→PM-audit→审→收口」一条龙 checklist)。
- **MCP**:是否有需要跨会话/跨工具的状态或外部集成(初判:本轮几乎用不到,多为本地 git/ah 动作)。
- **规则文档(SSOT)**:角色模型、纪律、验证门、沙箱/cargo 约定等声明性内容。
- 每条附「为什么这个归属」+「怎么保证零回归」;拿不准的标「待定+原因」。
