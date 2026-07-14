# ah 架构评估收敛终稿(2026-07-09,operator × antigravity 辩论收敛)

> 流程:双方各自独立评估(互为盲测)→ 逐条分歧互审(各自以代码亲验裁决)→ 本文=收敛后双方共同接受的结论。
> 输入:architecture-assessment-operator-2026-07-09.md / architecture-assessment-antigravity-2026-07-09.md / arch-convergence-agy-verdicts-2026-07-09.md。
> 所有条目均经**双方至少一方代码亲验+另一方认账**,无单边未验主张。

## 一、收敛的结构判决(双方一致)

1. **job 没有状态机**:约 11 处裸写 UPDATE(jobs.rs:301/383/494/528/633/680/894/555、recovery.rs:1155),合法性散在 WHERE 子句。[a3 verdict D1 AGREE]
2. **F3=F2 硬耦合,三处内联点**:mark_agent_idle_{matched,hook_event,log_event} 同事务寄生 mark_job_completed(state_machine.rs:736/943/1104)。job 完成是回合结束的副产物——假 COMPLETED 系列的最深结构根源。[D2 AGREE]
3. **感知层 6 推断器无仲裁**:唯一协调是 DB CAS 先到先得+零散让位 if;T3 判据(marker 行尾正则)复用进 6 条完成路径;高层信号超时无声下沉 pane 顶班。[双方独立得出]
4. **双 send 路径,一条无守卫**:orchestrator(mod.rs:241-248,双重 guard)vs rpc agent.send(agent.rs:1142-1149,直发)。dispatch-ACK 竞态的结构入口。[D3 AGREE]
5. **kill/teardown 四处各自编排**:agent.rs:275-300、sessions.rs:134-165、orchestrator mod.rs:562/577、master_watch:1029→system.rs:381;顺序/删沙箱/发事件各不一致——C1/C2 逃逸与误杀活栈的病根。[D4 AGREE]
6. **spawn_realign_agent 住错传输层**(realign.rs:375),orchestrator(mod.rs:19)与 monitor(master_watch.rs:26)反向依赖 rpc,三层成环。[D5 AGREE]
7. **db/ 是伪装成数据层的领域核心**:~70% 领域/编排逻辑;system.rs SQL 12 处 vs systemctl 副作用 37 处;60+ 处主动 pubsub。[双方独立得出]
8. **rpc/handlers/sessions.rs 严重越权**:级联删除全流程+手写事务裸 SQL(:230-292)、spawn master 副作用编排(:511-627)、cutover saga(:961-1168) 全在 handler。[agy 撤回原"职责合理"评级,C1 REVISE]
9. **master_watch 是 13 职责 god-file**(生产 2245 行+内联测试 3300 行),provider 知识(claude transcript 格式、CLAUDE_CONFIG_DIR 语义)泄漏进监控。[双方一致,agy 称"备用编排器"]
10. **身份嗅探病**:daemon 靠 /proc/self/cgroup 推断自身 unit(platform/linux/identity.rs:3),测试子进程可误认活栈身份并在 teardown 掐死活栈 cgroup。修法=显式参数注入,禁环境嗅探。[agy 独有,operator 亲验确认]
11. **STUCK 自愈是非对称半成品**:迟到完成接受门只认 HEALTH_CHECK_STUCK(state_machine.rs:1142/:1162),PANE_DIFF_STUCK 被排除——比问题清单第 9 条"CAS 自愈未做"更精确的记账。[agy 独有,operator 亲验确认;问题清单该条需订正]
12. **tmux 清理漏网**:expected_pid 存在但已死时 kill_*_if_owned 失败无兜底,session 存活泄漏(agent_io/registry.rs:145-160)。[agy 独有,operator 亲验确认]
13. **共享凭据 symlink**(home_layout.rs:658-664)与已知 OAuth 轮换登出事故直接对应,修=per-worker 独立凭据。[agy 独有锚点,与既有事故记录互证]

## 二、辩论中被修正的主张(记录反转,防止旧结论回流)

- ~~"revival 无熔断"~~(agy 原文)与 ~~"熔断存在,驳回"~~(operator 初判)**双双不准**。收敛真相:
  **熔断存在但被清零逻辑击穿**——retry_count≥5 熔断只针对连续 spawn 失败(recovery.rs:660);respawn 一成功立即 clear_recovery_backoff(orchestrator/mod.rs:798),而毒任务(job 本身致崩)会"崩→复活成功→清零→原样重派→再崩",计数永不累积,熔断在最需要它的场景失效。叠加:cancel_requested 被 requeue 原样携带(recovery.rs:415)却在认领 SQL 从不检查(jobs.rs:286/301),**已取消的毒任务同样参与循环**。这是辩论轮新挖出的复合 bug,双方原报告都没有。
- ~~"SQLite 多头写入易死锁"~~(agy 撤回):Arc<Mutex<Connection>> 单连接串行,机制上无死锁;真实风险=锁竞争阻塞+CAS 失败静默吞事件。[C2 REVISE]
- prompt_handler "内聚良好" 评级**维持**(agy 辩护成立:PromptRunOutcome 向上汇报,不产副作用不写库)。
- agy 报告两处 git_diff.patch 错锚点已订正为真实源码位置(master_watch.rs:2057、agent.rs:358)。[C3 REVISE]

## 三、收敛后的行动清单(合并双方 roadmap,按依赖排序)

1. **感知+控制平面统一设计轮**(吃 a3/master 辩论产能):job 独立状态机+单一写权威(§一.1/2)、感知仲裁器(§一.3)、hook outbox/ACK、pane 降权——北极星 R1-R4 扩容为"仲裁器"明确形态。设计轮同时裁决 §一.11 的对称化方案。
2. **机械修第一批**(换血后随首批 PR,均小且不需设计):身份显式注入替代 cgroup 嗅探(§一.10)、毒任务熔断洞+认领时刻 cancel 检查(§二.1)、tmux 清理兜底(§一.12)、300s 超时+时间戳优先级(既有待办)、C2 两个机械向量(pkill 模式+unit BindsTo)。
3. **分层归位三步**(机械重构,可与设计轮并行):迁 spawn_realign_agent 出 rpc(斩两条反向边)→ 统一 WorkerLifecycle/Dispatcher(收敛 4 kill 序列+2 send 路径)→ 拆 master_watch(先移测试再拆 revival 流水线)。
4. **per-worker 独立凭据**(§一.13,既有 backlog 升级为有锚点工单)。
5. **db/ 重命名归位**(长期,做在 3 之后):状态机纯化为 domain、reconcile/recovery 上提 application、SQL 沉 repository。

## 四、流程注记

- 双盲独立评估的收敛率很高(核心结构判决双方独立重合),但**互审轮仍净赚**:5 条单边主张被对方证据修正/撤回,并新挖出 1 个双方都没有的复合 bug(熔断清零洞)。"独立发散→辩论收敛"管线首考通过,可作为 pack 设计管线的实证样例。
- 本轮两次 a3 派单周期(独立评估+辩论)完成检测全程健康:BUSY 真实、产出落盘后才 COMPLETED,未复发 agy turn-end 假完成(样本 2,记入换血对照数据)。
