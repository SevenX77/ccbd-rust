# 调研任务 B(a2 codex)— 借鉴 agent-harness 的编程 SOP(重点:worktree + git 全流程)

派单人:Master PM(operator 交办)。你是 a2(codex)。**这是纯调研任务:只读 + 产出一份 markdown,不改任何代码、不碰 git,尤其不要碰 agent-harness 那个仓的任何东西。**
这是多阶段任务(调研→设计→实施)的**调研环第二半**;和另一路(ah 现状盘点)汇总后交 operator。

## 目标

研究 **`/home/sevenx/coding/agent-harness`** 这个项目**自己的编程场景 SOP**,尤其两块:
1. **git worktree** 怎么用——它靠 worktree 做什么隔离、怎么组织并行工作、和分支/PR 怎么配合。
2. **git 的一整条流程**怎么组织——分支命名、commit 规范、PR 流程、合并门、有没有把固定动作做成**脚本/tool/skill/hook**(而不是靠人脑记)。

提炼出「**值得 ah 学的 N 条 + 每条为什么**」,以及每条在 agent-harness 里**物化在哪(file:line)**(是脚本?skill?rules?hook?CI?)。

## 背景(ah 现状,供你对比)

ah(本项目)现在的编程 SOP 大致是:master PM 通过 `ah ask` 分派 codex/claude worker;**共享单一 git 工作树、串行 commit**(据我所知**没用 worktree**);从 main 开分支、worker 干完不 push、master PM-audit + a2 严审后 push、再由 PM 开 PR/CI/合并;串行 cargo 测试;TDD 红绿。很多流程约定是 **in-head**(没落成脚本/工具)。
你研究 agent-harness 时,重点找**它把哪些我们靠脑子记的东西做成了确定性的工具/脚本/worktree 机制**。

## 要查的地方(agent-harness 内,只读,绝对路径)

- 根 `CLAUDE.md`、`AGENTS.md`、`README`、`CONTRIBUTING`、`docs/`(尤其 SOP / workflow / process 相关)。
- `.claude/`(rules / skills / commands / settings / hooks)、`.ah/`(若有)、`.github/`(workflows / PR 模板)。
- **worktree 相关**:grep `worktree`、`git worktree`、任何创建/清理 worktree 的脚本(`scripts/`、`bin/`、`Makefile`、`justfile`、`package.json` scripts)。
- **git 流程脚本/工具**:grep 出把 branch/commit/PR/merge 固化成命令的东西(自定义 slash command、skill、shell 脚本、gh 封装)。
- commit/PR 规范:`git -C /home/sevenx/coding/agent-harness log --oneline -40`、`gh -R <agent-harness repo> pr list --state merged --limit 20`(若 gh 能访问该仓)。

## 产物

写到 `research/sop-agent-harness-learnings.md`,结构建议:
1. **§A agent-harness SOP 概览**:它的编程流程一句话画出来(分派/隔离/分支/审查/合并),和 ah 的差异点。
2. **§B worktree 机制详解**:它怎么用 worktree、解决了什么问题(并行不撞工作树?每 agent 一棵树?)、物化在哪(脚本/命令 file:line)、生命周期(建→用→清)。
3. **§C git 全流程物化**:分支命名 / commit 规范 / PR 流程 / 合并门,各自是「靠人脑」还是「有工具/脚本/CI 兜」,file:line。
4. **§D 值得 ah 学的 N 条 + 为什么**:每条给 (a) 是什么 (b) 解决 ah 现在哪个 in-head 痛点 (c) 在 agent-harness 里的物化形态可否照搬 (d) 风险/不适配点。

## 纪律

- 纯只读,**不改任何东西、不碰 git、不动 agent-harness 仓**。产物就是那份 markdown。
- 若 `/home/sevenx/coding/agent-harness` 某些路径不可达(沙箱限制),如实标注,别编。
- 带 file:line。区分「已物化成工具/脚本」vs「也只是文档说说」——我们要学的是前者。
- 拿不准/不适配 ah 的,标「待定+原因」,别硬推荐。
- 产物写完回执给我:路径 + §D 那 N 条的**标题列表**(每条一行)+ 其中你认为最该优先学的 top 3。
