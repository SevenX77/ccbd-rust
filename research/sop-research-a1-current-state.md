# 调研任务 A(a1 codex)— ah 现状「编程场景 SOP」物化盘点

派单人:Master PM(operator 交办)。你是 a1(codex)。**这是纯调研任务:只读 + 产出一份 markdown 清单,不改任何代码、不碰 git。**
这是一个多阶段任务(调研→设计→实施)的**调研环第一半**;你的产物我会和另一路(agent-harness 借鉴)汇总后交 operator 过目。

## 目标

盘清:一个编程任务从 **需求 → 调研 → 设计 → 实施 → 审查 → e2e → PR/合并** 的每一环,**现在实际怎么走**、有哪些**约定/纪律**,以及每条约定**现在物化在哪个文件(file:line)**——还是根本没落文件、只活在编排者脑子里(标 `in-head`)。

**这一步的价值就在于区分「已物化」vs「in-head」**:operator 下一步要决定把哪些沉淀成 skill / MCP / tool / 规则文档。所以每条纪律都必须明确标注归属。

## 产物

写到 `research/sop-current-state-inventory.md`,结构建议:
1. **§A 物化载体清单**:逐个文件,列出它承载了哪些 SOP/纪律 + 关键 file:line + 一句话归纳。
2. **§B 分环 SOP 走查**:七个环节(需求/调研/设计/实施/审查/e2e/PR·合并)各一节,写「现在怎么走」+「约定是什么」+「物化在<file:line> 或 in-head」。
3. **§C in-head 清单**:把所有你在文件里**找不到**、但从 git 历史/commit 规范/分支命名/本项目运作方式能推断出确实存在的约定,单独列出(这些是 operator 最关心的「待沉淀」候选)。

## 要盘的物化载体(逐个查,带 file:line)

1. **仓库项目规则**:`/home/sevenx/coding/ccbd-rust/CLAUDE.md`(角色边界:Master PM vs worker a1/a2/a3;各 worker 专职)。
2. **全局层**:`/home/sevenx/.claude/CLAUDE.md`(精简通用层,声明「操作规则按项目注入」)。
3. **ah master 协调内核**:存在于 master 的 sandbox HOME(`/home/sevenx/.cache/ah/sandboxes/<hash>/.claude/CLAUDE.md`)——**你(worker)很可能读不到这个路径**(不同 sandbox HOME)。若读不到,就在清单里标注「该文件存在但 worker 不可达,内容需 master 补」,别猜内容。它承载的是:ah 编排契约(`ah ask` 分派 / `ah pend|watch|logs|ps|attach` 读结果)、cutover/revival ACK、安全边界(不越过 ah 杀 pane/session)。
4. **`.claude/` 目录**(仓库内 + 可达的):找 `rules/`、`settings.json`、`settings.local.json`、hooks、`skills/`、`commands/`。有什么列什么,file:line。
5. **`.ah/` 目录**(若存在):`rules/` 等。
6. **memory**:`/home/sevenx/.cache/ah/sandboxes/2ff8aed8d8f7/.claude/projects/-home-sevenx-coding-ccbd-rust/memory/`(同样可能不可达;若不可达标注)。已知含:`MEMORY.md` 索引 + `master-cannot-run-cargo` / `brief-workers-full-cargo-test`(串行 cargo 纪律!)/ `cargo-dist-build-ooms-vps`。
7. **`.kiro/specs/*`**:逐目录扫一眼,判断它们是**每个 feature 的一次性 spec**(需求/设计/tasks)还是**可复用的 SOP**——大概率是前者,但要确认有没有哪个 spec 其实写了通用流程约定。
8. **git/gh 可推断的约定**:`git log --oneline -50`(commit message 规范:conventional commits?co-author trailer?)、分支命名规律、`gh pr list --state merged --limit 20`(PR 标题/流程)。这些是「约定物化在 git 习惯里」的证据。

## 特别要判定的纪律(逐条给「物化 file:line」或「in-head」)

- **分派纪律**:通过 `ah ask <agent> "<task>"` 分派;worker 只做被指派的单条任务、无分派权、空闲等派单。
- **角色模型**:a1 codex 主力实施 / a2 codex 严审 / a3 gemini(antigravity)设计不写码 / a4 claude 二审+e2e。(注意:实际运行里 a2 也做严审、a4 claude 池紧时可能不派——core 定义在哪?)
- **串行 cargo**:`CARGO_BUILD_JOBS=1`、`--test-threads=1`、跑**完整** cargo test 不过滤子集、`env -u AH_STATE_DIR` 隔离。物化在哪几处?
- **TDD 红绿**:先写失败测试再实现、贴红→绿输出。有没有文件要求?
- **baseline 对照证伪红灯**:红灯别口头「无关」,`git stash`/`git archive HEAD` 对照 main 单跑证明既有 vs 回归。有没有文件?
- **沙箱隔离**:worker 放进自管 sandbox HOME;IS_SANDBOX 等。
- **分支 / commit / PR / 合并约定**:从 main 开分支、别在 main 直接改;只 commit 目标 tracked 文件;co-author trailer;push 后 PM 开 PR + CI + 决定合;串行 commit(共享工作树,两个 worker 不同时 commit)。
- **验证门**:PM-audit → a2 审 → (a4 二审) → push。
- **worktree**:本项目现在**用不用 git worktree**?(据我所知是**共享单一工作树、串行 commit**,没用 worktree——请从证据确认:有没有 worktree 痕迹、CLAUDE.md/rules 里提没提。这条对下一步借鉴 agent-harness 的 worktree 很关键。)

## 纪律

- 纯只读调研,**不改代码、不碰 git、不 push**。产物就是那份 markdown。
- 读不到的路径(master sandbox / memory)**如实标注不可达**,别编内容。
- 带 file:line,区分「物化」vs「in-head」是本任务的核心价值,别含糊。
- 拿不准某条约定是否存在/在哪,标「待定+原因」,别硬下结论。
- 产物写完回执给我:清单路径 + 三段(§A/§B/§C)各自要点 + 你标为 in-head 的纪律条数。
