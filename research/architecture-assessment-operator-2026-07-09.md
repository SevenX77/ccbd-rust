# ah 全项目架构第一性原理重估(operator 席,2026-07-09)

> 方法:先从事故账本反推需求域,再以四路代码取证(master_watch / db 层 / 感知链路 / 编排与RPC)对照评估。所有结论带 file:line 锚点。与 antigravity 独立评估(research/architecture-assessment-antigravity-2026-07-09.md)互为对照组,本文不曾输入给对方。

## 一、从问题出发:15 类已实证事故 → 5 个需求域

| 需求域 | 对应事故(全部有账) | 系统必须满足的第一性需求 |
|---|---|---|
| N1 状态真相 | 假 COMPLETED 三连(codex bullet/agy turn-end/claude 跨 tick)、幽灵文本误锁、假 STUCK、300s 超时降级 | 确定性知道 F1-F5;信号分级有仲裁;一个状态一个写权威 |
| N2 生命周期归属 | 探针不重装、级联杀击穿复活、revival 僵尸、gen-2 孤儿壳、teardown 逃逸(C1/C2)、realign 丢 agent、误杀活栈 | 每个 spawn 出来的资源有唯一 owner 与确定性回收路径(监督树) |
| N3 事务性控制 | dispatch-ACK 竞态、cancel-race、realign 非原子 | 控制动作原子、幂等、有回执 |
| N4 隔离完整性 | 相对路径配置泄漏、共享 OAuth 轮换登出、沙箱 48G 泄漏 | 身份/凭据/配置按 agent 物化且启动即验 |
| N5 控制路径自检 | hook 配置蒸发、reconcile 写了没接线、探针没装 | "零件好的忘了装"能被系统自己发现 |

关键观察:**每一类反复发作的事故都能定位到一条具体的结构违规**(下文)。按用户既定原则"同族 bug 两次=结构病",这份账本本身就是架构判决书。

## 二、现状取证:四个结构病灶

### 病灶 1:`db/` 是伪装成数据层的领域核心
- 17.8k 行中约 70% 是领域/编排逻辑,非持久化。最大文件 system.rs(3265 行)里 SQL 调用 12 处、systemctl/进程副作用 37 处——它在管 systemd scope 与级联杀,不在管数据。
- 反向依赖 provider/prompt_handler/monitor/orchestrator/platform;每次写状态在 60+ 处主动 `notify_runtime_changed`(orchestrator::pubsub)——数据层在发布领域事件驱动上层。
- agent 状态机(state_machine.rs)是"接近权威",但被 4 处旁路稀释:agents.rs:65 重复的通用 transit、agents_lifecycle.rs:83/:179、state_machine_assert.rs:57、system.rs:1075(reconcile 直写 CRASHED)。
- **job 状态完全没有状态机**:11 处散落的裸 UPDATE(jobs.rs:301/383/494/528/633/680/894/555、recovery.rs:1155),合法性语义散在各函数的 WHERE 子句里。

### 病灶 2:感知层是"6 个推断器抢跑",没有仲裁器
- BUSY→IDLE(完成)有 **6 条**独立路径:FIFO marker(agent_io/reader.rs:77,186)、transcript(completion/monitor.rs:37)、hook(rpc/handlers/agent.rs:880)、pane_diff UI recapture(pane_diff/mod.rs:360)、health_check pane recapture(provider/health_check.rs:115)、dispatch-ACK 扫描(rpc/handlers/ack.rs:196,225,284)。→STUCK 有 3 条、三套阈值,互不知情。
- 唯一"协调"是 DB 乐观锁 CAS——**先到先得的数据竞争,不是优先级仲裁**;外加两三个手写让位 if(pane_diff/mod.rs:162-166、agent.rs:900)。北极星要求的等级制连投票制都不是,是抢跑。
- T3 判据(marker/matcher.rs:60 行尾正则)被复用进 6 条完成路径——pane 级启发式污染了本应是 T1/T2 的路径;log monitor 超时后**无声下沉**给 pane 顶班(completion/monitor.rs MAX_LOG_MONITOR_WAIT→交棒 UI recapture),违反"响亮降级"。
- **F3=F2 在 SQL 事务里硬耦合**:`mark_agent_idle_*` 同事务内既写 agent IDLE 又 `mark_job_completed`(db/state_machine.rs:938-946)。假 COMPLETED 三连案的结构根源就在这一行事务——job 完成是回合结束的副产物,不是独立信号。

### 病灶 3:控制平面多头,领域逻辑住在传输层
- 向 pane 写入有**两条**几乎逐行同构的路径:orchestrator run_once(mod.rs:183-355,带双重 dispatch guard)与 rpc handle_agent_send(agent.rs:1076-1197,**无 guard 直发**)。dispatch-ACK 竞态类事故在后一条路上没有防线。
- "杀 agent 并清理"有**四处**各自编排:agent.rs:760、sessions.rs:96、orchestrator mod.rs:562-579、master_watch ~1029。各自决定顺序/删不删沙箱/发不发事件。N2 域的僵尸/孤儿/误杀事故全部产自这四条不一致的序列。
- `spawn_realign_agent`(完整的 agent 供给领域逻辑)物理上住在 rpc/handlers/realign.rs:375,导致 orchestrator(mod.rs:19)和 monitor(master_watch.rs:26)**反向依赖传输层**;三层耦合成环。
- sessions.rs 的 handler 直接内联级联删除全流程含手写事务裸 SQL(sessions.rs:230-292)、master cutover 多阶段 saga(sessions.rs:961-1168)。按"handler 薄、service 厚"标准约 3/10。

### 病灶 4:master_watch 是 13 职责 god-file
- 5566 行(生产 2245+内联测试 3300),横跨:探活、arm/route、巡检、死亡分类、340 行单函数的 revival 流水线、cascade kill、readiness probe/ACK、env 构造(硬编码 HOME/CLAUDE_CONFIG_DIR 语义,:773-803)、tmux spawn、transcript 解析(耦合 claude 日志格式,:981/:1568)、recovery window 状态机、worker 重供给、marker 文件 IO 与「继续」注入。
- provider 知识(claude transcript 格式、字符串匹配判 provider :989-997)泄漏进监控文件——每接一个新 provider(或 Windows/ConPTY)都要改这里。

## 三、判决

**低耦合高内聚:不符合。** 模块按技术切面(db/rpc/monitor)命名,实际职责按"触发时机/入口"切分而非按领域语义切分;同一件事(判完成、杀清理、供给 agent)散落 4-6 处,同一个文件(system.rs、master_watch.rs)又聚了 3-13 件不相干的事。依赖方向:持久层依赖上层、监控层依赖传输层、三层成环。

**行业最佳实践对照:**
| 业界基准 | ah 现状 | 差距定性 |
|---|---|---|
| 单一状态权威(K8s apiserver 模式:所有写经一个门,controllers 只 reconcile) | agent 状态 4 旁路、job 状态 11 裸写、6 推断器 CAS 抢跑 | 结构缺失 |
| 监督树(Erlang/OTP:owner 唯一、死亡按策略传播、teardown 确定) | 4 条 kill 序列各写各的;C1/C2/孤儿壳/僵尸全是其后果 | 结构缺失 |
| 事务事件脊柱(outbox+ACK+重放) | hook fire-and-forget;ahd→消费者半建(job_transitions+游标) | 半建 |
| 端口-适配器(provider 知识收敛在适配器内) | claude transcript 格式散进 master_watch;marker 正则跨 3 信号源复用 | 渗漏 |
| 分层(domain/application/infrastructure) | db/=三层揉一起;rpc handler=业务流程;唯 SystemctlRunner trait、platform/ cfg 分割、CAS 守卫是正确方向的存量 | 错位 |

**做对了的(不抹杀):** T0 层事故硬化后基本达标(pidfd+启动重装+巡检+scope 连坐);状态转换全部带乐观并发守卫;级联杀的 DB 原语确实沉到了一层;platform/ 的 Windows seam(M0)切得干净;测试量大(master_watch 60% 是测试)。病不在没有好零件,在零件的归属和装配。

## 四、重构方向(按依赖与风险排序,与既定设计轮合流)

1. **job 独立状态机+单一写权威**(直接根治 F3=F2):新建 job state machine,拆掉 mark_agent_idle_* 事务里的 job 完成寄生;这是显式完成协议(R2)的地基,应并入感知层设计轮而非单做。
2. **感知仲裁器**:6 推断器降级为信号源,统一发事件给一个仲裁器按 T0>T1>T2>T3 定级裁决;高层缺席必须响亮告警而非无声下沉。R1(outbox)/R2/R3 即此方向,设计轮范围应明确写"仲裁器"这个词。
3. **把 `spawn_realign_agent` 迁出 rpc/handlers** 成 domain service:一步斩断 orchestrator→rpc、monitor→rpc 两条反向边,机械重构,风险低收益大,可先行。
4. **统一 WorkerLifecycle/Dispatcher 两个 service**:kill 四序列收敛为一条、send 两路径收敛为一条(带 guard 的那条);N2/N3 域事故的结构性根治。
5. **拆 master_watch**:第一刀移测试(-3300 行);第二刀 revival 流水线独立模块;env/spawn 构造下沉 sandbox/provider 域;transcript ACK 下沉 provider 适配器。
6. **db/ 重命名归位**(长期):状态机纯化(无 Connection 无 notify)为 domain;reconcile/recovery 上提 application;SQL 沉 repository。做在 4/5 之后,避免大爆炸。
7. **控制路径自检(R4/G4)**:启动 hook 配置 diff+合成触发+接线完整性断言,根治"零件好的忘了装"病族。
8. **Windows 关联提示**:M2 的 ConPTY 会新增一路信号源——若仲裁器先立,它是"第 7 个信号源接入仲裁器";若不立,它是第 7 个各自为政的推断器。感知层设计轮先于 M2 收口,恰好顺拍。

## 五、一句话总结

ah 的事故史不是运气差,是架构账:**没有单一状态权威、没有监督树、没有信号仲裁、领域逻辑住错楼层**——四条结构违规精确对应四族反复发作的事故。修法不是大重写,而是把已排期的感知层设计轮(R1-R4)扩为"感知+控制平面"设计轮,再用 3 步机械重构(迁 spawn_realign_agent、并 kill/send 序列、拆 master_watch)在实施层同步收敛。
