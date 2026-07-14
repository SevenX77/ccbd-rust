# 阶段派单 Brief — 感知/完成协议设计轮→spec→实施(2026-07-11,operator→master)

用户已拍板:新阶段主攻**域 1 感知/完成协议**。这是架构级收敛,不是补丁排期。你(master)按本 brief 规划并派发泳道,计划先回报 operator 过目再开工实施;设计轮可以立即启动。

## 一、阶段目标

把「停下≠完成」从裁决落成协议:agent 状态与任务完成只由显式协议信号驱动,unknown 永不造状态,高可靠信号缺席时响亮降级(watchdog 语义)而非静默回退 pane 推断。北极星验收(perception-layer-first-principles.md §五)四条全部有自动化测试钉死。

## 二、输入(钉死,不重开已收敛的问题)

- `research/perception-layer-first-principles.md` — 北极星(T0-T3 分级/R1-R5 依赖序/验收定义)
- `research/perception-final-convergence-2026-07-09.md` — 发散×deep-research 裁决终稿(骨架已钉死:单写仲裁+分级定责不投票+三态缺席=Unknown+watchdog 缺席即判决+sd_notify 先例;两条 refuted 红线不得引用)
- `.kiro/specs/ah-perception-arbiter/` — 仲裁器 design 已答四题,Phase 1(写闸+事件通道,#136/#138)已合;Phase 2-4 待推
- `.kiro/specs/ah-job-events/design.md` — job_transitions 载体已定
- handoff `research/OPERATOR-HANDOFF-2026-07-11.md` §域1 — 现行病对照(agy 假完成/假 BUSY、幽灵文本第 3 路径=ah#17、reply 载荷错位=观察日志 #33)

## 三、本阶段缺口(设计轮要收的)

按 R1→R2 依赖序:

1. **G1 hook 投递可靠化**(R1,地基):outbox 先 journal 后投递、ACK、ahd 回来先读错误本重放;at-least-once+事件 id 幂等。归属竞态机制沿用 arbiter design Phase 4 的答案,不重设计,只补投递事务性。
2. **R2 显式完成协议本体**(头号):
   - 派单带 job id,完成=worker 主动声明(`ah job done <id>` 或等价 tool);
   - claude Stop hook 强制层(未声明不许结束,block+reason);
   - 检测降级为看门狗("停了却没声明"=告警,不再推断完成);
   - **reply 载荷归属显式化**(观察日志 #33:COMPLETED 但 reply_text 是 brief 残片——reply 也走协议上报,不刮屏);
   - **per-provider 缺口矩阵**:agy 无 Stop-hook 等价物(G1/G4 补法?产物轨 git HEAD 锚定如何进协议?)、codex task_complete 语义边界。这是设计轮必答新题。
3. **G4 控制路径自检**:hook 配置 diff+合成触发+接线完整性断言,启动三档检查,深检进 ah doctor。
4. **物理证据闸门**(收敛稿 2.4②,job 级):mutating job 无 git diff 不放行;带两护栏(派发时静态标注 is_mutating;连续拦截 2 次 nudge 后第三次放行+上抛)。
5. R3(pane 生命周期推断整体拆除)**依赖 R1/R2 稳定后才能拆**——本轮只设计拆除计划与替代信号覆盖表,不实施拆除。

## 四、管线与纪律(铁律)

- **执笔权**:spec/design/tasks 执笔归严谨 agent——当前拓扑无 codex,由 claude 闸门(g1/g2)执笔;agy(o1)只坐辩论席,发散纪律=只给问题不给结论+显式反讨好。
- **发散→辩论收敛**:对"per-provider 缺口矩阵"这类未收敛新题,双盲发散+互审两步不可省;已收敛项(骨架)不重开。
- 泳道层级:agy 只向本泳道闸门汇报,闸门终裁,你零裁决纯中继;向 operator 汇报用约定落盘文件。
- 实施期 TDD 红绿;全机 cargo 串行(CARGO_BUILD_JOBS=1);大模块批量编译测试;每 worker 派单前上下文卫生(≥2 任务未清先 /clear,只清 IDLE)。
- 产出落盘出口:新 spec 建 `.kiro/specs/ah-completion-protocol/`(design.md/requirements.md/tasks.md);辩论材料同目录。
- 代码闭环≠实证闭环:tasks 里每项标注实证计划挂哪个 dogfood 节点;Gen-4 疗效账本开窗项顺带收数据。

## 五、汇报节点

1. 你的阶段计划(泳道分工+顺序)→ 落盘 + 回报 operator 过目(不阻塞设计轮启动)。
2. 设计轮产出(ah-completion-protocol design 初稿+辩论收敛记录)→ operator 亲验后才开实施。
3. 实施按 PR 粒度走台账更新纪律。
