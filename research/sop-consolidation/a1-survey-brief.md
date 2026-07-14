你是 worker(codex,调研角色)。**只做这一件事:调研 + 出一份结构化清单。这一轮不写任何实现代码、不碰 git。** 说人话。

## 背景
我们要把"ah 里现在实际在跑的这套**编程场景 SOP**"整理沉淀下来:哪些该做成 **skill**、哪些该做成 **MCP**、哪些该做成 **tool**(比如一些固定的 git 流程),哪些留作 **规则文档**。
**底线:整理后行为必须和现在一模一样,不许有任何回归/改变现状。** 这一步只调研 + 提分类建议,不动手改。

## Part A:盘清"现在实际怎么跑"的 SOP
把一个编程任务从头到尾的实际流程盘出来:需求 → 调研 → 设计 → 实施 → 审查 → e2e → PR/合并。逐环节答:
1. 每一环现在**实际怎么做的**、有哪些约定/纪律(例:派单纪律、串行 cargo、沙箱隔离、分支/commit/PR/合并约定、验证门、有没有用 worktree)。
2. 这些约定**现在物化在哪**:`/home/sevenx/coding/ccbd-rust/CLAUDE.md`、`.claude/rules/`、`.ah/rules/`、memory 索引、还是只在主控脑子里/口头?
3. 哪些是**反复手动重复**的固定动作(尤其 git:分支、commit 格式、PR、合并、发版),最适合固化。
把你实际读到的文件路径 file:line 标出来,别脑补。

## Part B:参考 agent-harness 项目
读 `/home/sevenx/coding/agent-harness`,重点看它的 SOP 里**值得我们学的**:
- **worktree** 怎么用(隔离并行改动?)
- **git 的一整条流程**怎么组织的
- 它把哪些东西做成了 skill/tool/脚本,我们能借鉴什么
给出"值得抄的 N 条 + 各自为什么"。

## Part C:第一版分类建议
把 Part A 盘出的每一个反复出现的 SOP 片段,给一个归属建议:**skill / MCP / tool / 规则文档**,each 配一句为什么。
判据参考:固定确定性动作(git 流程等)→ tool/脚本;需要模型按流程走的判断类 → skill;要跨会话查询/检索的 → MCP/规则库。
**再强调底线:沉淀方式不能改变现有行为。** 拿不准的标"待定+原因",别硬分。

## 产出
写 `research/sop-consolidation/survey.md`。结构清晰:Part A 现状盘点(带 file:line)/ Part B agent-harness 可借鉴 / Part C 分类建议表。
完成回一句话:最值得先固化的 top 1 条 + 已落盘。

## 约束
只读 + 写这一个文件;不写实现、不碰 git、不改配置。可读整个仓库和 agent-harness。这一步是调研,不是实施。
