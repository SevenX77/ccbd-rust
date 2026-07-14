# 轨2 调研语料:编排底座结构病全集(2026-07-11 汇编)

> 用途:o1(发散/红队)+ d1(执笔收敛)做**编排底座第一性重设计**的输入证据集。
> **为什么 operator 代为汇编**:agent 沙箱无 gh 认证(设计如此),公开仓 issue 你们自己拉不到,由 operator 桥进本文件。
> **最高原则(CEO 定,不可违)**:第一性原理、**不打补丁**(同族病≥2 次=结构病必须升维)、模块化低耦合高内聚、**不要后兼容**(可推倒重写,不背历史包袱)。
> **产出定位**:这不是"逐条修 issue"的排期表——是**把下列全部症状当同一个编排底座结构病的切面**,反推理想底座,再出差距表与重构设计。别一条症状打一个补丁。

---

## A. 公开仓 SevenX77/ah 的编排/感知类 open issue(用户点名:必须纳入)

| # | 症状 | 归属结构域 |
|---|---|---|
| **ah#16** | `ah up` realign 非原子:agent 丢失(DB 说 IDLE 但 tmux session 没了)+ 另一 agent 出现重复 tmux session | 控制面/状态机一致性 |
| **ah#17** | dispatcher 对 composer 幽灵文本/survey 浮层无免疫,job 永久 DISPATCHED↔QUEUED 弹跳,单个 C-u 就能解 | 感知/派单就绪判定 |
| **ah#19** | 生命周期看门狗缺口:BUSY agent 死 turn(idle pane、零输出)从不被标记,job 挂 DISPATCHED 60-90min | 感知/完成协议 |
| **ah#20** | ahd 无持久错误日志:ahd.log 恒 0 字节,失败只活在可变(且会被 prune)的 DB 行里 | 可观测性 |
| **ah#21** | respawn 后 agent 被标 UNKNOWN/INIT_PROBE_TIMEOUT 且 job 判 FAILED,而 agent 实际正在执行送达的接力棒 | 感知/恢复 |
| **ah#22** | master 唤醒/自续文本打进 composer 但从不提交,master 静默沉睡到人按 Enter | 感知/master 自驱 |
| **ah#23** | ahd.sqlite 27h 涨到 2GB 而活数据 <2MB(无 vacuum,prune 的行留死页) | 存储卫生 |
| **ah#24** | `ah stop` 留下 per-stack ahd unit `enabled`+在盘,孤儿 unit 累积,下次登录自启死栈 | 生命周期/teardown |

> 其余公开 issue(ah#3/4/5 docs、ah#6 per-agent settings、ah#7-12 host-parity、ah#14/15 CLI 一致性、ah#18 凭据)属**轨1 产品/功能面**,不在本轨重构范围;ah#18=轨1 首任务(模块 D)。

## B. dev 仓 ccbd-rust open issue

| # | 症状 |
|---|---|
| **dev#139** | issue #13 storm 加固遗留:SIGKILL orphan-reap + init-probe 节流 |
| **dev#140** | flaky:grand_tour_realign_extra_matrix(BUSY-fixture 生命周期时序) |

## C. 内部 dogfood 暴露的结构病(operator 观察日志,当日四要素)

`logs/operator-observation-log.md` 全量,重点近期:
- **#47** 规则 spawn 物化不热更 → 改规则未换血,两 codex 持旧规则在主树混写碰撞。
- **#48** 止血 stash 无差别扫走活栈配置(次生事故)。
- **#49** cancel→respawn 链三连:pane 命名错位(respawn 落错名 session)+ recovery 重投×cancel 竞态 + codex 无视 worktree 钉死 commit 本地 main。
- **#50** 冻结 brief 引用的权威文档 untracked → worktree 不可见,两臂按旧设计开工。
- **#51** 泳道死锁三重奏:agy Stop 钩子静默不触发 × log 监听 300s 硬超时 × ahd 硬编码错文案催单逼 agy 未批先实施(nudge-livelock)。
- **#52** 僵尸 job 被 cancel → ahd 排水积压归档队列把数小时前过时 brief 真派给 agent + cancel×dispatch 竞态第二例。

## D. incident / spec 病例单(已立案的结构病深描 + 修向)

`.kiro/specs/ah-orchestration-reliability/` 目录全部,尤其:
- `lane-completion-channel-triple-failure-2026-07-11.md`(三重奏根因+回归契约)
- `recovery-reinsert-vs-cancel-race-2026-07-11.md`(含 obs#52 队列排水变体 B)
- respawn 命名错位 / agent-workspace-assignment / dispatch-ack-race 等。

## E. A/B 实验暴露的泳道败因(活体证据)

`research/ab-experiment-gateway/REVIEW-gateway-ab-verdict.md`:泳道臂(codex 闸门+agy 实施)在同一任务上 14 轮返工、CI 从未绿、DNF,而 codex 单干臂自主达绿——**泳道多 agent 交接的完成通道/自诊断能力在真实压力下崩塌**的一手数据(附 obs#51/#52 的 infra 故障牵连)。

## F. 北极星(必读,设计必答四题)

- `research/perception-layer-first-principles.md`(感知层第一性基准)
- `research/perception-final-convergence-2026-07-09.md`(收敛终稿;设计轮**必答四题**:①单写入口硬约束形态;②各信号类 Unknown 预算;③cgroup 委托布局 PoC;④hook 归属竞态)

---

## 汇编者note(给 o1/d1 的方向)

上面 A-E 的症状,operator 的判断是它们不是 N 个独立 bug,而是**一个编排底座在"感知 / 完成协议 / 控制面状态机 / 生命周期"四个关节上反复漏的同一批结构缺陷**——单写仲裁缺失、unknown 造状态、高可靠信号缺席时静默回退 pane 推断、cancel/dispatch/respawn 路径非原子竞态、完成信号无显式协议。**你们的任务不是确认这个判断,是推翻或重立它**:从第一性反推一个理想编排底座该长什么样(它如何表示 agent 状态、如何判完成、如何原子地 dispatch/cancel/respawn、信号缺席时如何响亮降级),再对照现状出差距表,最后出**不后兼容的模块化重构设计**。o1 先发散/红队把问题域和候选架构铺开,d1 执笔收敛。
