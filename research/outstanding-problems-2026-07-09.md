# ah 全量遗留问题清单(2026-07-09 日终,operator 梳理)

> 用途:供清空上下文后的新会话(或任何 agent)无偏见重估优先级。每条带状态标签与证据锚点。
> 记账原则(用户 2026-07-09 明确):**代码闭环 ≠ 实证闭环**。合入 main(实施+审计+CI 绿)只是第一层;没经过活栈 dogfood/真机论证的,一律记在 §G 验证债里,不许从清单上消失。

## 状态标签
- [待换血生效] 已合 main 但活栈跑旧二进制(8aee446 pre-G2),需二进制替换+重启栈(原计划 7/11)
- [冻结待令] 用户停机令冻结中,工单/框线已备
- [设计轮] 需先出设计,未动工
- [backlog] 已知待办,无工单
- [上游] 非 ah 代码问题
- [用户侧] 等用户动作

## A. 感知层/完成检测(北极星:research/perception-layer-first-principles.md)

1. **假 COMPLETED 仍是现行病** [待换血生效]:今天单日 11 例(agy turn-end 被当任务完成)。G2 检测器、hook_push、log-signal 修复全在 main 未上活栈。换血后必须 dogfood 对照复验(应归零)。
2. **Fix D:hook 起止双信号为主** [冻结待令]:任务开始侧 hook 事件缺失(hook_push 只推 stop);prompt_handler 全目录零 hook 引用;scanner 分类前不咨询 hook 时间线。工单框线在 master 手里(WORKORDER-FIXD 曾落盘)。
3. **hook 基础设施两结构洞** [设计轮]:G1 投递 fire-and-forget(无 outbox/ACK/重放,ahd 不在=事件蒸发);G4 hook 配置蒸发无启动自检/合成触发。是"删 pane 推断"路线的可靠性前提。
4. **显式完成协议 R2** [设计轮,头号]:停下≠完成已成共识,但"agent 通过 hooks 显式报告完成 / master 亲验"的协议本体未设计未实施。用户已裁决方向(见 memory feedback_completion_root_remove_stop_equals_done、feedback_delete_pane_lifecycle_inference)。
5. **master 的缺席推断病** [backlog]:"commit 后静默=完工"启发式今天误催过一次 mid-task 的 a1;同族病根治依赖 R2。
6. **llm_unsafe/llm_low_confidence/llm_error 不再 park** [backlog,观察项]:Fix C 语义变更(a4 审计观察项 C),靠 A/B 看门狗+UNKNOWN_PROMPT_DETECTED 事件兜底,是否够用无实证;master 未正式定夺。

## B. 编排可靠性

7. **log 监听 300s 硬超时 < 真实任务时长** [backlog]:MAX_LOG_MONITOR_WAIT 硬编码 300s,真任务 10m47s 被弃日志信号转兜底;health_check last_marker_ts.or(last_output_ts) 优先级 bug 使真实 pane 活动救不回。两处已入 orchestration-reliability 待办。
8. **dispatch-ACK 竞态** [backlog]:目标 agent 未回 IDLE 时派发→job DISPATCHED 但 prompt 未落 pane→330s 判 STUCK。有 SOP(cancel+kill+up)无真修。
9. **STUCK 死胡同终态 part2** [backlog]:只有 CRASHED recovery-eligible;STUCK 后真完成不被接受(CAS 自愈未做)。2026-07-02 实锤过。
10. **QUEUED 饥饿只告警不自愈** [设计如此]:fail-closed 故意;处置靠人。若告警频发需复核。

## C. 泄漏/资源治理

11. **C2 teardown 逃逸坐实** [冻结待令→设计轮]:四向量已证实(pkill 模式不匹配 target/release/ahd vs run-*/bin/ahd;daemon unit 无 BindsTo/PartOf;Rust Drop/shell trap 不清 ahd 本体且 SIGKILL 全 bypass;enable 持久 symlink+GC 见 state_dir 在就跳过)。外部接入方 4 个 ahd 存活 5 天即此因。取证报告在 a1 job 46788625 回执。
12. **C1 空壳 daemon 累积** [冻结待令,设计轮]:与 C2 同根(spawn 无归属链)。
13. **test-hygiene:--lib 拉真 tmux server** [冻结待令]:单元测试喂真 tmux 给 master 级联风险;另有 cargo flock fd 继承死锁旧案。
14. **沙箱 GC** [backlog]:445 沙箱 48G 打满磁盘旧案,自动 GC 未做,现靠 SOP 手清。
15. **orphan-scope 回收 unwired** [backlog]:reconcile_orphan_scopes 写了没接线;BindsTo 已让正常路径不泄漏;删 vs 留需 kill-ahd 压测实证,别盲删。

## D. 拓扑/运营

16. **活栈旧二进制** [用户侧拍板时机]:今天全部合入(状态契约系列、G2、A/B、Fix C)+ 每角色模型配置(a4=opus4.8+xhigh,master=sonnet5+medium,statusline)全部未生效;换血需重启栈,原计划搭 7/11 codex 解冻车。
17. **codex 冻结至 7/11** [用户侧]:过渡拓扑执笔受限(tasks.md/TDD 的 codex 笔由 claude fallback);7/11 拓扑重议。
18. **共享 OAuth 凭据轮换登出** [backlog]:master+worker symlink 同一 credentials,任一 refresh 其他方登出;per-worker 独立凭据未做。
19. **master 等 operator 拍板监控盲区** [backlog]:PR2a 案 87 分钟 52 分钟在等;pane 阻塞告警机制未建,现靠"全闲静默即亲查"自律——违反"机制不靠自律"原则。
20. **worker context 卫生** [新规刚立]:派单前 ≥2 任务未清先 /clear;agy 侧无 context 可视化(claude 侧 statusline 待换血);a1 已清一次(1041 步存量)。
21. **doctor 拓扑漂移告警** [backlog]。
22. **antigravity 空闲自退** [上游+待换血生效]:agy 周期轮询点自崩;REVIVE_IDLE 已合 main 但活栈未生效,拓扑仍会悄缩。

## E. 产品线

23. **Windows 原生** [用户最高优先级,未动]:research/M0/M1 spec 在 .kiro/specs/ah-windows-native/;tmux→ConPTY 3-4 周硬核心;本机只能交叉编译,真测靠 windows CI;antigravity 管线复测推到其后。今天全天被硬化线占用,零进展。
24. **Host-environment parity 6 issues(ah#7-12)** [冻结待令,设计先行]。
25. **ah#6 per-agent settings for codex/agy** [upstream issue 开放]:codex/agy 的 CLI 配置只能走宿主全局,无沙箱级注入。
26. **Studio Req1 / v1.3.0-rc** [用户侧]:等 Win11/WSL2 真机 runbook 签 PASS 才提升正式版+同步公开仓。
27. **发版同步公开仓** [纪律项]:下次 dev tag 必同步 SevenX77/ah(v1.1/v1.2 只发 dev 的坑)。

## F. 流程/质量复盘项

28. **Fix C 三轮 CI 往返复盘** [教训]:第一轮返工 brief 没逼"全量枚举出口"导致打地鼠两轮;凡"N 个站点同病"类修复,brief 必须首轮就要求枚举表。已在第三轮兑现(commit e5178f1 出口契约表可作模板)。
29. **周期预算** [纪律]:单 PR 全量串行最多一遍、严审只定向跑;今天 Fix C 线含 3 轮 CI,时长可接受但方法论教训见 28。
30. **假 COMPLETED 控制组** [数据]:当日 11 例已记录,换血后对照。

## G. 已闭环但未实证(验证债——代码在 main,活栈论证=0)

31. **Fix C(#126)全链路** [待换血后 dogfood]:回归测试钉住的是 fixture 模拟的幽灵/横幅,真 claude CLI 场景零实证。必验:①真 banner/ghost 出现时不 park+发 UNKNOWN_PROMPT_DETECTED;②真 trust/update 对话框仍正确 park(白名单没误伤);③llm_unsafe/llm_low_confidence 不再 park 后,看门狗+事件监看是否真能兜住"LLM 有信号但不敢动"的场景(观察项 C,纯推断)。
32. **Fix C 第 2/3 轮 commit 无独立审计** [审计缺口]:a4 只审了第 1 轮(1482c8e);2105424(测试契约改写+matched_case_id)和 e5178f1(17 出口契约表)靠 operator 抽验+CI 绿合入,契约表本身是 a1 自报。按周期预算纪律允许,但需下轮定向审或 dogfood 时连带验证。
33. **A/B 看门狗(#125)** [待换血后 dogfood]:静态审计+单测绿,从未见过真实触发。必验:人工制造 QUEUED 饥饿→告警恰一条+job 不转 STUCK;真幽灵 park→5 tick 后升级事件+重跑解锁。
34. **G2 完成检测器全家(含 #123 parser)** [待换血后对照复验]:"换血后假 COMPLETED 应归零"是**未论证假设**——今天 11 例控制组数据在手,新二进制上必须跑对照;antigravity 的 log 完成信号(transcript PLANNER_RESPONSE/DONE)补齐状态需核实,agy 侧可能仍靠兜底。
35. **每角色模型/statusline/effort 配置(1f47f37)** [待换血后验证]:只过了 config validate,从未物化进真沙箱。必验:master 真跑 sonnet5+medium、a4 真跑 opus4.8+xhigh、statusline 真渲染 context%、effortLevel 重启后持久。
36. **REVIVE_IDLE** [待换血后复验]:合入时自测过一次(2026-06-28),现活栈未生效;agy 空闲自退→复活→拓扑不缩的完整链路在新二进制下零复验。
37. **pack v0.5.0 设计管线(辩论侧)** [待首个设计课题开考]:执笔侧当日已在用(brief 全 claude 主笔),但 6 步管线的辩论收敛半边从未跑过完整课题;首考=显式完成协议或 C1 设计轮。
38. **context 卫生新规** [待执行验证]:纪律刚发 master,尚无一次"派单前自动 /clear"的实际执行记录;master 是否真执行、/clear 后派单链路是否顺滑,未验。

## 优先级建议(operator 视角,供重估挑战)
1) 换血+重启栈(解锁 16 的全部存量修复生效,是最大杠杆)→ 2) **立即批量清 §G 验证债**(31/33/34/35/36 全挂在换血后,一轮 dogfood 矩阵能收掉大半;34 的对照数据只有换血日最新鲜)→ 3) 显式完成协议设计轮(R2,含 G1/G4;顺带首考 37)→ 4) Fix D → 5) Windows 原生(用户最高优先级,与 3/4 可并行:设计轮吃 a3/master,Windows 吃 codex 解冻后的实施力)→ 6) C1/C2 泄漏设计 → 其余按 backlog。
