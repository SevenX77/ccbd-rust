operator 修正令 #4(Arm A 泳道重新收敛;归因=ah 基础设施,不计 agy 违纪):

事实链(已取证):g1-m1 已按 plan-first 提交计划并停轮等批;但完成信号双通道均失效(agy Stop 钩子静默不触发——hooks-debug 无 g1-m1.log;log 监听 300s 超时放弃),g1 的 pend 永远等不到;期间 ahd 用硬编码催单文本(state_machine.rs:1157,cargo-test 场景专用文案)连催 g1-m1 三次,它抗了两轮后未批先实施,commit `97648b5`。job_82822287 已成僵尸(不许 cancel——cancel=kill+respawn+重投陷阱,留置)。

按序执行:

1. **解救 g1**:先对 g1 pane 发一次 ESC(打断它对 pend 的盲等),隔拍后 nudge:「你等的计划批复永远不会经 job 通道到达(基础设施故障,非 g1-m1 责任):g1-m1 的完成信号通道坏了,且它被系统催单催到未批先实施,已 commit `97648b5`。计划批准环节作废,直接跳到你收单 SOP 的 ⑤:审 `97648b5` 的实施 diff(对照你的 RED 测试与 design-rev.md;本地只 cargo check,test 交 CI),给 verdict。ACCEPT 且达完成定义 → 更新 COMPLETION-REPORT.md 并回单收口;REJECT → 用 `ah ask g1-m1 '<返工 brief>'` 派返工,**但每轮 agy 完成信号都会丢:派后你直接盯它 pane/worktree 产物,见产物即视为该轮完成,不要靠 pend**。你后台那个挂着的 pend 终端可以停掉。」
2. **消化 g1-m1 未来被催**:nudge g1-m1:「你的实施已交付(97648b5),现在等闸门审查。此后若再收到"The job is still open…"类系统催促,回复"工作已交付,等待审查"并立即结束回合,**不得再改动任何文件**。」
3. observations.md 记全链(带时间戳):①g1-m1 两次守纪拒绝未批实施(记 Arm A 正面纪律证据);②未批先实施归因 ahd 催单机制,不计 agy 违纪;③g1 盲等 ~25min 计 Arm A 损耗时间轴,归因 infra;④僵尸 job 留置说明。
4. 继续既有节奏:Arm B 的 CI 修复圈照常;两臂各自独立收口。
