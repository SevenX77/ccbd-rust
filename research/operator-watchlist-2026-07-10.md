# Operator 观察清单(2026-07-10,换血#1 后 Gen-1 窗口)

## A. Gen-1 dogfood 疗效(账本 research/dogfood-ledger-2026-07-10.md)
1. G1 误闩:每次 claude 派单看有无 PROMPT_PENDING,区分真阳性(输入行真有文本)与误判(banner/ghost)。现况 0/3。
2. G2 假完成:agy job 翻 COMPLETED 时核 reply 是否垃圾过程文本 + pane 是否还在跑。现况 0/2+。
3. G3 假 STUCK:>15 分钟长任务是否被误判。900s 修已在新二进制,但残余 PANE_DIFF_STUCK 写入路径要等模块 A 合入才删——Gen-1 期间仍可能发病,发生即记(不算修复失效,算残余根源实证)。
4. G4 停摆不醒:agy 归 IDLE 零产出、无自动唤醒,需 master 续派的例数。
5. P0-2 新语义实战样本:cancel 干净落 CANCELLED(已有 1 正样本)/毒任务 respawn 熔断是否真拦(尚无样本)/≥300s 稳定确认清零路径。
6. reply 提取净度:claude 单 reply 尾部混 pane 界面杂质(已见 1 例,轻症,攒样本给设计轮)。

## B. 双泳道流水(模块 A/B)
7. 泳道1:a1 实施模块 A(A1' 删残余推断路径 + A2 死码清理)→收口 cargo。
8. 泳道2:a2 首单实施 B1/B2/B3 + tmux 测试卫生追加项(用户已批)是否进单。
9. **沙箱 toolchain gap 必须在收口前解决**(已提醒 master;不解决收口 cargo 必炸)。
10. 双泳道收口 cargo 排队纪律:全机单跑,绝不并行。
11. worker 产物物理验证:每个"完成"都 git log/文件核,不信 job 状态。
12. 共享 git 树约束:两 worktree 并行可以,同一棵树绝不许两 worker 同时动。

## C. 设计线(kiro spec)
13. master 消化 a3 对抗审的 spec 修订——重点盯 RTR 致命击穿的处置(有 6/29 实证事故背书,不许轻放或悄悄绕过)。
14. 定稿后 operator 亲验:必答四题不悬空/对照感知收敛终稿无稀释/a3 审查各条要么采纳要么明确驳回(不许静默忽略)。
15. 亲验过 → 用户过目 → 才排实施。过目前实施不启动。

## D. master 质量边界(sonnet-5 medium)
16. 编排层推理错误(排错依赖顺序/裁决自相矛盾/该 cancel 不 cancel)——出现即为升 effort 的触发条件,记录具体案例。
17. 开工单前是否实证代码现状(模块 A 过时情报教训)——待办:往 .ah/rules/master.md 补"开单前 file:line 抽验"规则。
18. master 阻塞盲区:master 在等拍板/agent 在问没人应——全栈静默超 ~10 分钟即亲自 capture 各 pane。

## B+. worker 上下文卫生(2026-07-10 补,首版清单漏项)
23. claude 系(a4/a5)盯 statusline ctx%:≥50% 或泳道阶段边界必 /clear;审计单必须干净上下文进场(接单前清)。agy 系无 ctx 显示,任务数代理:完成 ≥2 单且 IDLE 即清。只清 IDLE agent。master 执行,我抽查各 pane ctx% 与任务堆积数。

## E. 栈健康/资源
19. 磁盘水位:现 84%,主仓 target 重建 + 沙箱增长会再吃;≥90% 即告警清理(target/ 与沙箱两个填充源都查)。
20. OOM:五 agent 比三 agent 重,收口 cargo 期间盯内存/dmesg。
21. 测试泄漏累积:模块 B 修复落地前,定期巡检 /tmp/tmux-1001/ 野 socket 与 ~/.cache/ah/sandboxes 计数。
22. 换血#2 节拍:模块 A 合入即触发(build→通告收敛→stop→swap→start→双验拓扑→orientation→抽验→账本开 Gen-2)。执行时知会用户。

## F. 机制在位(事件驱动,不靠自律)
- DB monitor:job/agent 状态翻转全量 diff。
- 阶段性 watcher:worktree commit / pane `test result:` 行 / PR merge 状态。
- 例行抽查:pane 直接 capture(status/title 都可能撒谎);claude pane 注入验证认 `[Pasted text #N]` 占位符。
