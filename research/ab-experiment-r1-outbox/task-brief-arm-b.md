# 任务 Brief(冻结版)— 实现 ah Completion Protocol 的 Group R1 + JC-1

> 本 brief 为 A/B 对照实验冻结文本,除【工作区】一节外两臂字节级相同。收到后不会有任何追加指示或答疑;所有决策依据以下冻结输入自行做出并在产物中留痕。

## 【工作区】(按臂替换)
- 工作目录:/home/sevenx/coding/ccbd-rust-wt-ab-b
- 分支:ab/r1-outbox-solo(已建好,基于 main 97104cd)

## 任务

在上述工作目录内,完整实现 `ah-completion-protocol` spec 的 **Group R1(R1-T1、R1-T2、R1-T3、R1-T4)与 JC-1(传输侧)**。scope 严格限定于此:**不要**实现 R2/G4/证据闸门/R3 的任何部分;JC-1 中 F3(`job_done`)消费方尚不存在,去重闸门须建在 kind 分叉之前并对两类 kind 的去重语义都有测试覆盖,F3 的实际应用点可挂明确标注的受测 stub。

## 冻结输入(唯一依据,只读)

- 设计:`/home/sevenx/coding/ccbd-rust/research/ab-experiment-r1-outbox/frozen-spec/design.md` 的 Part R1(R1-Q1~Q4)及 §0-§4 总则
- 需求:同目录 `requirements.md` 的 CP-R1.1~CP-R1.4 与 CP-R1.2(JC-1)
- 任务:同目录 `tasks.md` 的 Group R1 与 JC-1 条目(含每条的验收与 open item 说明)
- 参考:同目录 `convergence-provider-matrix.md`(如需 provider 背景)

设计与现实代码冲突时:以设计的意图为准,在产物报告里记录冲突点与你的处置;不得擅自扩大 scope 去"顺手修"设计未涵盖的问题。

## 工程纪律(硬约束)

- TDD:先写红测试再实现;红绿轨迹要能从 commit 历史看出。
- 资源:`CARGO_BUILD_JOBS=1`;测试串行(`--test-threads=1`);不得并行多路 cargo。
- 全部工作(代码、测试、commit)都在上述工作目录与分支内完成;不得改动主仓或其它 worktree。
- commit 粒度自定,信息规范;最终分支上必须包含全部实现与测试。

## 完成定义(全部满足才算完成)

1. R1-T1~T4 与 JC-1 传输侧全部实现,逐条对应 tasks.md 验收点。
2. 关键不变量有自动化测试钉死,至少包括:
   - "exit 0 ⇔ durable outbox record exists"(journal 提交失败路径非零退出);
   - 重复投递不重复生效(`outbox_consumed` 去重,replay 后无 double-apply);
   - 冷扫重放:daemon 强杀重启后 outbox 记录重放无洞、错误本隔离 un-applyable 记录(DF-A1 的自动化近似)。
3. 全量 `cargo test`(workspace,串行)绿。
4. 在工作目录根写 `COMPLETION-REPORT.md`:实现摘要、逐条验收对照表、测试运行输出摘录、设计冲突点及处置、自评遗留项。
5. 最终 commit 后声明完成。

## 时限

从收到本 brief 起 8 小时硬顶;超时以当时分支状态为准结算。
