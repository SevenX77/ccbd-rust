# 编排底座第一性重构 · 设计草稿

# ⚠️ DRAFT · 待 operator/用户过目,未冻结 ⚠️

> **本文是草稿,不是冻结稿。** 本轨设计收敛后**必须先经 operator 带用户过目拍板**,d1(执笔)与 c2(实施)均**无权自行放行实施**。在 operator gate 通过前,本文不得作为 c2 实施依据。
> **执笔**:d1-claude(设计主笔,唯一执笔席)。**发散/红队**:o1-antigravity(`o1-divergence-memo-2026-07-11.md`)。**输入语料**:`CORPUS-lane-problems-2026-07-11.md`、北极星 `research/perception-layer-first-principles.md` + `research/perception-final-convergence-2026-07-09.md`。
> **最高原则(CEO 定,不可违)**:第一性、不打补丁、模块化低耦合高内聚、不后兼容(可推倒重写)。

---

## 〇、收敛立场:推翻还是重立"同一结构病"判断

**结论:operator 的"同一结构病"判断在症状层正确,但它是症状归纳不是根因;o1 指出"根因非应用层"也对,但 o1 给的两个根因(工作区隔离 + 原子分布式事务)其中"分布式事务"是错的原语。我重立为:只有一个缺失原语,不是四关节也不是两根因。**

### 唯一缺失原语(第一性根因)

> **缺失的是:一个独占权威的 Reconciler,由它单一拥有"物理现实(T0 OS 真相)↔ 逻辑状态(DB)"的映射,由有序、带 epoch 的事件日志驱动收敛;每个物理资源(tmux/cgroup/工作区目录/IPC socket)都绑定到 slot 的逻辑纪元,漂移由 GC 收割。**

operator 归纳的四关节(感知 / 完成协议 / 控制面状态机 / 生命周期)是**同一个缺失原语暴露的四个切面**,不是四个并列的病。全部 A-E 症状可还原为下述三种同源失配,而三者都是"没有单一 Reconciler 拥有收敛"的直接后果:

1. **多写者各自用启发式修补物理↔逻辑漂移**(ah#16/#17/#19/#21、obs#49/#52)——因为没有单写仲裁,任何 Monitor 都能 CAS 落状态。实证:`src/db/state_machine.rs` 有约 10 个 `pub(crate) fn mark_agent_*_sync`(`:188/:210/:299/:524/:545/:560/:1164/:1297/:1383`),全 crate 可调,无单一拥有者。
2. **物理世界变更与逻辑提交被拆成非原子多步**(ah#16 delete-then-spawn:`src/rpc/handlers/realign.rs` 引入 `delete_agent`(`:6`)后再物理 spawn;obs#49/#52 cancel×respawn 竞态)——因为编排层把"物理"当成能和"DB 提交"一起 2PC 的资源,而它们**根本不是事务性资源**。
3. **完成/存活用不可重读的低可靠信号顶班**(ah#19/#22、obs#51)——因为没有以 T0/T1 为权威、缺席即响亮降级的收敛规则,系统静默滑落到 pane 推断。

### 对 o1 两个根因的裁决

- **采纳(substance)**:o1"缺乏工作区物理隔离与硬环境绑定"——**接受**,实证 `src/rpc/handlers/agent.rs:135` `agent_cwd = session.absolute_path`(一个 session 内所有 agent 共用一个 cwd,无 per-slot workdir),obs#47/#49③ 是活体证据。但见 §四:我**重立其架构归属**(不是第五关节,是同一原语作用于文件系统资源)。
- **带机制驳回(o1 根因二的原语选错)**:o1 称根因是"缺乏原子**分布式事务**(Distributed Transaction)边界"。**驳回"分布式事务"这个原语**:tmux spawn 是对外部 server 的副作用 RPC,进程 spawn 是内核 syscall,二者**没有 prepare/rollback**,你**无法**把"tmux session 已建"和"DB row 已插"放进一个可回滚的 2PC 事务。追求"分布式事务"是南辕北辙。正确原语是 **reconcile-to-converge(最终一致 + 单一拥有者 + 物理侧幂等可重驱动 + 孤儿 GC)**——即 o1 自己的候选架构一(Reconciler)才是对的答案,o1 却给根因贴错了标签。这一步是执笔席对 o1 的实质纠偏,不是转述。北极星本身已经背书这条路(K8s reconcile、电平触发重导出、单写纪律),所以重构=**把北极星描述的底座,作为一个 Reconciler 真正建起来,再把每个症状修复归位其中**。

---

## 一、理想底座:Reconciler 基座的五条第一性属性

从"hypervisor 必须确定性知道 F1-F5"(北极星)反推,理想编排底座 = 一个 Reconciler 基座,具备且仅需五条属性:

- **P1 单写仲裁**:`agents`/`jobs` 状态列的写连接**只**由一个 `StateReconciler` 拥有(私有、不 `Clone`、不共享)。所有 Monitor 降级为只读事件生产者,经 MPSC 通道投递 `PerceptionEvent`,**永不 CAS**。单写是编译期 + 审计可查的硬约束,不是约定(北极星 1.1 refuted 红线:单写靠代码纪律,非平台赠品)。
- **P2 有序 epoch 事件日志为唯一驱动**:所有感知观测先落 `perception_events`(全局递增 seq + 逻辑 epoch),Reconciler 串行消费、单次 CAS 落状态。天然获得事件回放审计(北极星 1.1 / ah#20 持久错误本 / R5 telemetry 同脊柱)。
- **P3 电平触发 + 缺席=Unknown + 异常真极性**:状态从**当前可重读观测**(cgroup populated、transcript 游标、DB 行)重导出,漏事件可容忍;信号缺席解读为 **Unknown**(三态,非 False 非忽略);"卡死"是被**显式设置的异常真信号 STALLED**(北极星 1.3 kstatus),不是超时静默推断。pane 文本不可重读→**原理上排除**在容错模型外(北极星 1.5),永不参与生命周期。
- **P4 物理资源皆随 epoch 绑定、漂移即 GC**:tmux session、cgroup、工作区目录、IPC socket、job-cookie 全部随 slot 的逻辑 epoch 命名/绑定;旧 epoch 的迟到信号进控制面即被丢弃(不触发任何自动重启);任何**物理实体无对应 active DB 行 = 孤儿**,由 Reconciler 收敛 pass 单向收割。**GC 不是 bolt-on 看门狗,是 Reconciler 的内在属性**——它持续比对"物理清单 vs 期望态"并拉平。
- **P5 意图声明式、物理侧幂等收敛**:dispatch/cancel/respawn 不是命令式多步序列,而是**声明期望态**(desired-state),由 Reconciler 收敛(缺失则幂等 spawn、孤儿则 reap、cancel 则置终态+reap 进程组)。**永不 delete-before-spawn**;完成(F3)只认 T1 显式协议 + T0 cgroup populated 兜底,永不启发式。

**验收锚(北极星第五节,理想底座必须钉死)**:kill -9 ahd 再拉起事件流无洞(outbox 重放可证);挂后台任务 end_turn 不判完、看门狗响亮告警;幽灵文本零生命周期影响;删 hook 配置→启动自检报警+自愈+响亮降级日志。

---

## 二、o1 三候选架构裁决:不是三选一,是同一系统的三层

o1 把 E1(事件溯源+单 Reconciler)、E2(epoch 租约)、E3(sd_notify 双向握手完成协议)列为三个候选并配"按爆炸半径挑一个"的表。**我驳回"三选一"这个框架**:它们不是替代品,是理想底座的三个层,必须组合。选"只做 E3(最便宜的 local)"会把控制面竞态(ah#16、obs#49/#52)全留下;选"只做 E1 不做 E2"会留下 epoch 漂移(ah#21)。

| o1 候选 | 在理想底座中的角色 | 裁决 | 红队失效模式的机制化解(采纳并修正) |
|---|---|---|---|
| **E1** 事件溯源 + 单 Reconciler 串行写 | **控制面脊柱**(= P1+P2) | **采纳为脊柱** | ①Reconcile 延迟/SQLite 写锁瓶颈 → Reconciler **事件驱动而非轮询**(事件到达即 reconcile),dispatch 就绪读**物化视图**不读原始日志,WAL 模式;②事件表膨胀(加剧 ah#23 2GB)→ **快照 + 日志截断 + vacuum 纳入脊柱内在职责**(P2),不是外挂。 |
| **E2** epoch 绑定 | **物理资源身份机制**(= P4) | **采纳绑定,带机制驳回"可续租约"形态** | 租约活锁(新 spawn 未续租→判超时→递增 epoch 重启→旧进程恢复发现 epoch 过期→无限 spawn-expire)。**根除机制**:epoch **不是可自动过期/自动重启的租约**,而是 spawn 时打的**单调版本戳,只由显式控制动作(respawn/cancel)递增,永不由超时递增**;超时只产**告警(响亮降级)**,不产 epoch bump。这与北极星 observedGeneration(版本非租约)+ watchdog(缺席即判决非静默重试)一致,且复用**现存 `state_version` 列**(实证:`ack.rs:346-484`、`master_revival.rs:356` 已用 `AND state_version=?` 做 CAS),**不另造 `logical_epoch`**。 |
| **E3** sd_notify 显式完成握手 | **F3 主信号**(= P5 的完成分支) | **采纳为完成主信号** | 协作死锁(agent 完成但 IPC 写失败→无限挂等 ACK)。**化解**:完成走 **outbox 先 journal 后投递**(北极星 R1),IPC 失败≠事件丢失;agent **不阻塞等 ACK**——声明 done+落 journal 即可,Reconciler 从**耐久 journal**捡起(即便活 socket 已死)。配 no-op job **最大逼单上限**(2 次 nudge 后放行+上抛,北极星 2.4②)防只读任务误标死循环。 |

**整合结论**:理想底座 = E1 脊柱 + E2 身份 + E3 完成,三者是 P1-P5 的落地,**一体设计,非菜单**。

---

## 三、第五关节假说裁决:采纳实质,重立归属(不是第五关节,是同一原语的文件系统面)

- **采纳实质**:工作区物理隔离 + 硬环境绑定**必须**作为地基级变更落地,实证 `agent.rs:135`(session 级共享 cwd,无 per-slot workdir)、obs#47(未换血 codex 主树混写)、obs#49③(codex 钉死本地 main)。**不允许**继续靠 brief 里的 `cd` 叮嘱(LLM 是概率模型,可被指令妥协的隔离必然在规模下失守——o1 此论成立)。
- **带机制重立归属(驳回"第五关节"这个架构定性)**:o1 把它列为"与状态机同等的第五关节"。**驳回并入 P4**:工作区目录只是**又一类物理资源**,和 tmux session / cgroup / UDS 同构,必须随 slot epoch 绑定、孤儿 GC。把它当"新关节"会让关节清单继续膨胀(正是打补丁思维);把它归到"物理资源皆随 epoch 绑定(P4)"这**同一个原语**,才是第一性收敛。故它是 **GF3(见 §六),P4 作用于文件系统的实例**,不是独立第五关节。

---

## 四、北极星四题逐题立场(明确,不留开放式空话)

### Q1 · 单写入口硬约束形态 —— 立场:`StateReconciler` 独占写连接 + Rust 模块私有性编译期强制 + 审计测试

- **形态**:`agents`/`jobs` 状态列的**唯一可写 DB 连接**由 `struct StateReconciler` 私有字段持有(不 `Clone`、不 `Arc<Mutex>` 外借)。现存 `state_machine.rs` 的全部 `pub(crate) fn mark_agent_*_sync`(`:188/:210/:299/:524/:545/:560/:1164/:1297/:1383`)**降为 `StateReconciler` 的私有方法或直接删除,替换为事件 handler**;移除其 `pub(crate)` 可见性——**编译期使外部模块根本无法调用**。
- **生产者侧**:Monitor(`fifo_reader`/`health_check`/hook 接收/pane scanner)只拿 `Sender<PerceptionEvent>`(MPSC),只 INSERT 观测,永不 CAS。
- **强制手段**:模块私有性(编译期)+ **一条审计测试**断言"无任何 `pub` 写函数逃出 reconciler 模块"(北极星 1.1 要求的"编译期/审计可查硬约束,非约定")。
- **读写不对称**:单**写**者,非单读者——dispatch/UI 从状态的物化只读视图并发读,不经 Reconciler。
- **采纳 o1** Q1 的 `SessionWriter`/`PerceptionEventChannel` 方向;命名统一为 `StateReconciler`/`PerceptionEvent`(与 P1/P2 一致)。

### Q2 · 各信号类 Unknown 预算与降级动作 —— 立场:采纳预算骨架,**驳回 o1 偷渡回来的 T2 主动探测**,改为被动 T0 重读

| 级别 | 来源 | Unknown 预算 | 预算超时后的权威动作(响亮降级) |
|---|---|---|---|
| **T0** OS | pidfd 退出 / cgroup populated | **0s**(立即) | pidfd 退出 且 populated=0 且无 T1 done → `CRASHED`,挂起 job,释放物理资源,**不自动恢复**(自动恢复正是 obs#49 respawn 竞态之源;恢复是显式控制动作)。 |
| **T1** Hook/完成协议 | outbox IPC / `ah job done` | **~10s**(进程已退但未见显式声明的宽限) | 进程退出却无显式 done → `UNKNOWN`(真三态,**非 FAILED**——可能崩在半路)+ 响亮告警"停了未声明"(北极星 G2/五验收),禁止派单。**投递延迟(hook 已发但 ahd 曾下线)不设预算**——由 R1 outbox at-least-once + 重放覆盖,不是 Unknown。 |
| **T2** Log | FIFO transcript 游标 | **180s**(无输出) | 繁忙但 3 分钟零输出 → **被动重读 T0**(cgroup populated,可重读、零注入):populated=0 且无 done→按 T0 判 `CRASHED`;populated=1→置**异常真 `STALLED`** + 响亮告警 + 上抛 master/人,**不自动失败、不主动探测**。 |
| **T3** UI | pane diff | **0s** | 永不用作生命周期翻转;仅驱动"已知交互对话框待响应(F4)"告警。 |

- **带机制驳回 o1 的 T2 主动心跳探测**:o1 Q2 表在 T2 写"触发主动心跳探测/30s 探测无回显判 STUCK"——这**违反已冻结的北极星铁律**(1.4 / 2.4③ 明确否决向干活 agent 注入 `echo $?`/DSR:违运行铁律 + fail-dangerous + Esc=打断推理 + 无生产先例)。**替换为被动重读 T0**(cgroup populated 可重读,非注入),既回答"还活着吗"又不碰 agent。
- **带机制修正 o1 的 T1 "30s 强制 SIGABRT 杀栈"**:o1 把"已退出"与"仍存活但静默"混为一谈。已退出的进程无栈可杀(moot);仍存活但静默属 T2 域。故删去 SIGABRT 分支,按上表分治。
- **采纳** o1 的 T0=0s→CRASHED-不自动恢复、T3=0s-永不造状态(与北极星 1.5 一致)。

### Q3 · cgroup 委托布局 PoC —— 立场:采纳拓扑与 PoC,定为 LF1 先行 spike,给降级兜底

- **拓扑(采纳 o1 + 北极星 2.2)**:`Delegate=yes` transient scope;**agent CLI 进程留父 scope**,其 spawn 的 shell/编译/测试子进程放**委托子 cgroup `payload`**;监控 `payload/cgroup.events` 的 `populated`。这精确解决北极星 2.2 点名的坑("agent CLI 自己在 scope 里→populated 恒 1")。
- **信号定责**:`payload` populated 1→0(去抖 300-500ms,北极星 2.1②)= "无非 agent 工作在跑" = **完成候选(非完成证明)**;完成**证明**归 T1 显式协议。整 scope populated=0 = **F1 进程死**的 OS 权威。即 cgroup 是 **F1 权威 + F3 兜底/佐证**,不是 F3 主信号。
- **PoC(采纳 o1 五步,定为 LF1 独立 spike,必须先落地再让 Reconciler 依赖)**:①`Delegate=yes` transient scope 起 python 模拟 agent CLI;②python 建子 cgroup `payload` 并把 spawn 的 shell PID 写入 `payload/cgroup.procs`;③python 自身留父 scope;④监 `payload/cgroup.events` populated;⑤验证 shell 退出后即使 python 仍活,populated 准确翻 0。
- **✅ 已在真机验证 PASS(2026-07-12,证据 `wsl2-cgroup-poc-result-2026-07-12.md`)**:用户 WSL2 真机(WSL Ubuntu-24.04 / kernel `6.18.33.2-microsoft-standard-WSL2` / PID1=`systemd` `running` / `cgroup2fs` 纯 v2 unified / `XDG_RUNTIME_DIR=/run/user/0` / `systemctl --user` `running`)执行 `systemd-run --user --scope -p Delegate=yes --collect python3 <poc>`,`summary.success=true`,`observed_populated_sequence = ["1","1","0","0"]`——**与 c2 之前纯 Linux 沙箱基线逐字一致**。**故 WSL2 `--user` 形态下 `Delegate=yes` + 委托 cgroup `populated` 信号的可用性已实证,该环境闭环**;北极星开放问题#2(此前 ~85% 置信里剩的那 15%)**已划掉**,GF4 的 F3 cgroup 兜底传感在 WSL2 环境成立,不必回退到自制降级路径。生产接线仍依赖 GF1 脊柱消费 populated 事件(部署序不变)。
- **降级兜底(响亮,不静默;现降为防御性冗余)**:目标形态 WSL2 `--user` 既已实证可用,降级路径**不再是主目标所需**,仅对**未预见的非委托宿主**(如未来某宿主 `Delegate=yes` 被策略禁用)保留:退 `cgroup.procs` 枚举 + 白名单(自制成分回升、置信降,北极星 2.2),**并打响亮日志标注降级**。当前实证:代码库用 `systemd-run --user --scope`(`platform/linux/scope.rs:125/:227`、`tmux/scope.rs`),**无 `Delegate=`**,故此为纯增量绿地。

### Q4 · Hook 归属竞态 —— 立场:job-cookie + epoch 双因子,入口(ingress)校验

- **机制(采纳 o1 job-cookie,并与 E2 epoch 合一)**:dispatch 时生成强随机 `job_cookie`(128-bit)写入 job 行 + 以 `AH_JOB_COOKIE` 注入沙箱 env;hook/完成工具上报**必须**在 IPC payload 携带 cookie;ahd 在**事件入口**校验 cookie == 该 slot 下 `DISPATCHED` job 的 cookie,**且** epoch 匹配;不符/缺失/旧 epoch → 丢弃 + 安全告警。**实证绿地**:代码库无 `job_cookie`/`AH_JOB_COOKIE`。
- **cookie 粒度 = per-dispatch(job×epoch)**:respawn(新 epoch)得新 cookie;旧化身的迟到 hook 携旧 cookie → 不匹配 → 丢弃。**一个机制同时解 ah#21(respawn epoch 漂移)+ obs#49/#52(跨泳道/陈旧归属)**。
- **为何不靠 SO_PEERCRED PID**:北极星 2.3 明示 hook 进程发完即退(sd_notify 文档点名高危形态),PID 回收复用竞态使 PID 归属危险;cookie+epoch **不依赖发送者存活**,正是北极星要求的"job-cookie/文件落盘"归属。
- **校验位置**:在事件**进 `perception_events` 脊柱之前**(ingress),坏事件永不污染日志。

---

## 五、对 o1"不打补丁红队"的回应(三条既有 spec 批判:全部采纳,且归并入脊柱不做三个补丁)

| o1 批判对象 | o1 的彻底方案 | d1 裁决 |
|---|---|---|
| `stuck-false-positive`(`health_check.rs` `.or()`→`.max()` 时间差推导) | 引入 `lease_expires_at` + 心跳,状态机内部租约到期自动 STUCK | **采纳"消除外部时间差推导"批判,带机制驳回"可续租约"方案**。租约会重蹈 E2 活锁,且"超时自动转 STUCK"是**静默推断**(北极星要响亮)。改为:T2 预算超时→被动重读 T0→`STALLED` **响亮告警**(§四 Q2)。`health_check.rs` 的推导代码路径**删除**而非 `.max()` 打补丁。 |
| `realign-atomicity`(spawn 成功后 SQL swap 替换老 row) | 引入物理资源 GC 看门狗 + active-reconcile(防物理孤儿泄漏) | **完全采纳——这正是本设计中心论点(P4/P5)**。realign 不再是"delete-then-spawn 命令序列",而是"声明期望态,Reconciler 收敛(缺则幂等 spawn、孤儿则 reap)"。GC **不是外挂看门狗,是 Reconciler 内在收敛 pass**。**由构造消除 ah#16**(Reconciler 在物理替换确认前绝不删 row,且无论如何 reap 孤儿物理)。 |
| `recovery-reinsert-vs-cancel-race`(reinsert 前查 `cancel_requested`) | cancel 原子化:同事务置 `CANCELLED` 终态 + 同步 SIGKILL 进程组,控制指令与恢复循环解耦 | **完全采纳**。cancel = 声明期望态 `job.desired=CANCELLED`,Reconciler 在单写上下文中收敛(置终态 + reap 进程组),recovery 循环**永不独立决定 respawn**——它见 desired=CANCELLED 即 reap,绝不复活。`cancel_requested` 字段已存(`recovery.rs:162`,schema `:66`),但修法不是"在 reinsert 查 flag",是"cancel 归 Reconciler 收敛、恢复不再自作主张"。 |

**闭环意义**:三条彻底方案**全部归并进同一个 Reconciler(P1-P5),不是三个独立补丁**——这本身反过来证明 operator"同一结构病"在症状层判断正确,而"单一 Reconciler 原语"是其根因。

---

## 六、现状 vs 理想 差距表(grounded,file:line 实证)

| 差距 | 现状(实证) | 理想底座要求 | 定性 |
|---|---|---|---|
| **G-写** 多写者 CAS | `state_machine.rs` 约 10 个 `pub(crate) mark_agent_*_sync`(`:188…:1383`)全 crate 可调 | P1 单写 `StateReconciler` 独占写连接 | 地基级 GF1 |
| **G-脊柱** 无事件日志 | 无 `perception_events`,无 Reconciler(grep 零命中) | P2 有序 epoch 事件日志唯一驱动 | 地基级 GF1 |
| **G-epoch** 版本部分存在 | `state_version` 已有但仅 ack/master 局部 CAS(`ack.rs:346-484`) | P4 epoch 全物理资源绑定 + 旧 epoch 信号丢弃 | 地基级 GF2(**复用**现列,非新造) |
| **G-原子** 物理逻辑非原子 | `realign.rs` delete_agent(`:6`)后 spawn;destructive respawn 靠 stagger(`:224/:265`) | P5 声明式期望态 + 幂等收敛 + 孤儿 GC | 地基级 GF1(realign 归入) |
| **G-工作区** session 级共享 cwd | `agent.rs:135` `agent_cwd=session.absolute_path`,无 per-slot workdir | P4 工作区随 slot epoch 绑定隔离 | 地基级 GF3 |
| **G-完成** 无显式协议 | 从 end_turn 推断;pane 兜底判完(北极星 G2/G3) | P5 T1 显式 `ah job done` + Stop 强制 + populated 兜底 | 地基级 GF4 |
| **G-cancel** 非原子竞态 | `cancel_requested` 存(`recovery.rs:162`)但 reinsert 不强制;cancel×dispatch 竞态(obs#49/#52) | P5 cancel 归 Reconciler 收敛终态+reap | 地基级 GF1(cancel 归入) |
| **G-cgroup** 无委托 | `--user --scope` 无 `Delegate=`(`platform/linux/scope.rs:125`) | P3/P4 委托子 cgroup populated 传感 | 局部级 LF1(PoC 先行) |
| **G-归属** 无 cookie | 无 `job_cookie`/`AH_JOB_COOKIE`(grep 零命中);SO_PEERCRED PID 归属危险 | P4 job-cookie+epoch 入口校验 | 局部级 LF2 |
| **G-存储** 事件日志膨胀风险 | ah#23(27h→2GB 死页,无 vacuum) | P2 快照+截断+vacuum 纳入脊柱内在 | 随 GF1(脊柱内在) |
| **G-可观测** 无持久错误本 | ah#20(ahd.log 恒 0 字节) | P2 事件日志=持久观测底座 | 随 GF1(脊柱内在) |
| **G-master自驱** PTY 投键 | ah#22 master 靠"投键期待 shell 回车"伪交互 | P5 master 自驱走显式控制 RPC 原语 | 地基级 GF4(控制面 API 化) |

---

## 七、爆炸半径分级(**直接决定 operator 带用户拍板难度,必读**)

> **给 operator 的一句话**:本重构**以地基级为主,是底座重写而非补丁集**——这正是"不打补丁"的代价,也是价值。用户需拍板的是"是否接受一次大范围底座重写(经可回退灰度分阶段交付)",而非"改几个 bug"。四项地基级变更**在架构上共享 Reconciler 脊柱(概念高内聚),但在部署上可解耦**——通过影子模式 + 配置门控可分四阶段渐进灰度、随时回退,**不需 big-bang 一次性切换**(完整迁移路线见 §九)。局部级三项在脊柱落地后独立挂载。**总范围不因分阶段而缩小,但落地风险与不可逆性显著降低——这直接降低 operator 带用户拍板的难度。**

### 地基级(Ground-level)—— 动一发牵全身,需推倒重写,operator 决策重
- **GF1 · 单 Reconciler + 事件日志脊柱(P1+P2+P5)** — 最高半径。重写 `state_machine.rs` 全部写入口→事件 handler;新增 `perception_events` 表 + `StateReconciler`;realign / cancel / dispatch 全部改声明式期望态收敛;GC 收敛 pass 内在;事件日志快照+截断+vacuum(吞并 ah#23/#20)。**这是其余一切的地基,不先落它,别的都悬空。**
- **GF2 · epoch 全物理资源绑定(P4)** — 中高半径。**复用**现存 `state_version` 而非新造,但要把 epoch 透传进 IPC payload / tmux session 名 / 文件路径 / job-cookie;旧 epoch 信号入口丢弃。
- **GF3 · slot 工作区物理隔离(P4 文件系统面)** — 高半径。改 `ah.toml` schema 增 per-slot workdir;`agent.rs` spawn/respawn 强制 workdir 绑定(替换 `:135` 的 session 级 cwd)。**破坏"所有 agent 在 main 主树直接开跑"的历史行为**——这是杜绝主树脏写的必需破坏(不后兼容)。
- **GF4 · 显式完成协议 + pane 生命周期摘除 + 控制面 API 化(P5/P3)** — 高半径。`ah job done` 工具 + Stop hook 强制 + 检测转看门狗;master 自驱改显式 RPC;**pane 生命周期推断整体拆除**(北极星 R3:必须在 GF1/GF4 完成协议稳定后拆,拆早了无替代信号)。

### 局部级(Local-level)—— 模块内隔离可独立替换,operator 决策轻
- **LF1 · cgroup 委托子 scope populated 传感** — 低风险高收益,PoC 可独立单测验证(§四 Q3)。**建议最先做 PoC**(去风险北极星剩余 15%),但其**生产接线依赖 GF1 脊柱**消费 populated 事件。
- **LF2 · job-cookie + epoch hook 归属校验** — 中低,仅动 spawn env 注入 + 事件入口校验,不碰 DB 核心转换。依赖 GF2 的 epoch。
- **LF3 · 物理资源 GC reaping** — **诚实标注耦合分歧**:o1 把它列为 local 外挂看门狗;但本设计中它是 **GF1 Reconciler 的内在收敛 pass(地基级),不是可独立的 local 模块**。作为 bolt-on 它 local,作为正确形态它 ground。这个差异 operator 需知悉:选"正确形态"= 它随 GF1 一起上,不单独立项。

### 依赖序(实施若获放行时的方向,非排期)
```
LF1(cgroup PoC,可即刻并行去风险)
        │
GF1(脊柱:单写+事件日志+声明式收敛+GC+存储卫生)  ← 一切地基
   ├── GF2(epoch 绑定,复用 state_version)
   │       └── LF2(cookie+epoch 归属)
   ├── GF3(工作区隔离)
   └── GF4(显式完成协议 → 稳定后 → pane 生命周期摘除)
                                          └── LF1 生产接线(populated 入脊柱)
```

---

## 八、模块化重构分解(低耦合高内聚的模块边界,方向性)

理想底座按职责切成高内聚模块,模块间只经**事件/期望态**耦合(不共享写状态):

- **`reconciler`**(新):唯一写者。独占 `agents`/`jobs` 写连接;消费 `perception_events`;跑收敛 pass(spawn 缺失/reap 孤儿/落 CAS);GC。对外只暴露"提交期望态"与"投递事件"两个入口。
- **`perception`**(重构现 Monitor 群):只读事件生产者集合(`fifo_reader`/`health_check`/hook 接收/pane scanner)。每个 Monitor 产 `PerceptionEvent`(带 seq+epoch+cookie),经 MPSC 投递。**删除**其对 `mark_agent_*` 的一切调用。
- **`epoch`/资源身份**(横切,寄居 reconciler + spawn):epoch 戳、物理资源命名绑定、旧 epoch 丢弃规则。
- **`completion`**(重构):`ah job done` 协议 + Stop 强制 + no-op 逼单上限;cgroup populated 兜底传感(LF1)。
- **`isolation`**(新):slot workdir 绑定 + 沙箱环境硬指派(GF3);与轨1 网关沙箱 bind 复用同一 `SandboxOverrides` 装配路径(低耦合复用,不重造)。
- **`gc/reaper`**:作为 reconciler 内在 pass(非独立进程),比对物理清单 vs 期望态单向收割。

**低耦合断言**:除 `reconciler` 外无模块持有状态写连接;模块间无共享可变状态,只有事件与期望态。这使各模块可独立测试(mock 事件流 / mock 期望态)。

---

## 九、分阶段灰度迁移路线(切换策略 · 应答 o1 分阶段反驳)

> **裁决:采纳 o1 的分阶段灰度反驳(`o1-rebuttal-phasing-2026-07-12.md`),带三条机制加固。** 我原稿 §七/§十 断言"四地基级高耦合、难拆开、切换策略未展开"——这一条**更正**:我把"架构高内聚"错当成了"部署强耦合"。二者不同。四项 GF 共享同一 Reconciler 原语(概念上一体),但**部署上**可经**影子模式 + 配置门控**分阶段解耦、随时回退。o1 的路径机制合理、回退真实、未偷渡任何已裁决机制点(影子只读→单写不变量全程守住;epoch 复用 `state_version`→无 schema 迁移;GC 随写接管作脊柱内在 pass)。故采纳。

### 四阶段路线(每阶段:上线内容 / 部署耦合 / 回退开关 / 不可逆点)

| 阶段 | 上线内容 | 部署耦合 | 回退开关 → 效果 | 不可逆点(point of no return) |
|---|---|---|---|---|
| **P1 · GF3 工作区隔离先行** | per-slot workdir 绑定替换 `agent.rs:135` 的 session 级 cwd,先阻断主树脏写(obs#47/#49③) | **与 GF1/2/4 部署完全解耦**;读现存 `state_version` 命名 workdir,不碰 DB 写权限 | `[isolation].per_slot_workdir=false` → `agent_cwd` 回 session 根,零物理痕迹 | 无(纯配置,永远可回) |
| **P2 · GF1 影子模式(只读仿真)** | 建 `StateReconciler`+`perception_events`,**只读**双轨:legacy `mark_agent_*_sync` 照常生产写,reconciler 读快照算"预期转移"、比对 legacy 实写、不一致抛 `MismatchedStateTransition` 告警 | 依赖无(读 legacy 状态);**零物理写入、零副作用**(reconciler 拿只读连接,物理执行器 mock,建议仅写旁路审计表/日志) | `[reconciler].shadow=false` → 关旁路线程,回纯 legacy,零运行时消耗 | 无 |
| **P3 · GF1 写接管 + GF2 epoch 绑定** | 影子 0% 不一致后开写接管:reconciler 独占写 + 声明式收敛 + 物理 GC(内在 pass)同步上线;GF2 把 `state_version` 透传进 IPC payload / tmux session 名 / 路径 | GF2 随此上;**复用 `state_version`,无不兼容 schema 变更** | `[reconciler].write_enabled=false` → `mark_agent_*_sync` 重获写,reconciler 退回影子只读 | legacy 写入口**删除**时 |
| **P4 · GF4 显式完成协议 + pane 摘除** | 上线 `ah job done` + Stop 强制 + 看门狗;master 自驱 RPC 化;稳定后置灰并删除 pane 生命周期推断 | 依赖 GF1 稳定(北极星 R3 序) | `[completion].enforce_explicit_done=false` → 重启 pane 扫描兜底(**已知带病,仅过渡网**) | pane 扫描代码**删除**时 |

- **切入顺序**:P1 与 P2 相互独立、**可并行**;**建议 P1 最先单飞**(主树脏写是当前高频活痛,GF3 爆炸半径最低、即刻止血)。LF1 cgroup PoC 可即刻并行去风险(§四 Q3);LF2 依赖 P3 的 epoch。

### 三条机制加固(o1 留白处,我补,守已裁决不变量)

- **加固 A · 切换瞬间排空屏障(守 P1 单写不变量)**:P2→P3 的写接管**不是同时双写**。单次 flag flip 内含 drain barrier:**先**令 legacy 写入口停收新转移(quiesce)、在途写 drain 完,**再**激活 reconciler 写。任一时刻只有一个活跃写者——影子期是 legacy 单写,切换后是 reconciler 单写,**中间无双写窗口**。o1 未言明此排序,补之。
- **加固 B · 门控窗口 schema 仅增不改(保回退可行)**:迁移共存期所有 schema 变更必须 **additive-only**(新表 `perception_events`/新列 desired-state,legacy 一律忽略;**不 drop、不 rename、不语义重用现列**)。这样任一阶段回退都**不会撞上 legacy 读不懂的 schema**。`state_version` 复用天然满足;新增物皆 legacy 无视。o1 称"不涉及 schema rollback 难题"只对 `state_version` 成立,对新增物需此约束兜底。
- **加固 C · 脚手架短暂性(守"不后兼容"终态)**:所有回退开关是**过渡脚手架,不是永久配置面**。每个不可逆点都有**承诺的 legacy 删除**;尤其 P4 的 pane-scan 回退是**已知带病兜底**(三次幽灵事故的病根),只可作过渡安全网,**必须在确认后删除、不得长驻**——否则等于把病永久走私回来,违背"不后兼容"。**CEO"不后兼容"守在终态(终态是干净重写);迁移期 legacy 并存是临时脚手架,不是历史包袱**——二者不矛盾。

### 采纳分阶段的额外收益(去风险原稿最弱区)

**影子模式(P2)用真实生产流量无风险实测 reconcile 延迟与 SQLite 单写吞吐**——正好把原稿 §十一 最弱区#2("单写吞吐未压测,数值边界待实测")从"上线才知道"提前到"影子期可测、可调、可决定是否切"。这是分阶段相对 big-bang 的实质增益,不只是"更安全"。

### 一处需在 GF3 自身设计钉死、不在迁移路线冻结的深度点(诚实标注)

o1 示例 workdir = `session.absolute_path.join(".workspace/slot_...")` 仅为**图示**。**GF3 的隔离深度(git worktree 级 vs 同仓子目录级)是 GF3 内容设计点,不在本迁移路线冻结**:obs#49③(codex 钉死本地 main)提示,若隔离只是同仓子目录、`.git` 仍共享,`git` 操作仍作用同一工作树——真隔离可能需 worktree 级。此深度留 GF3 自身设计/冻结时定,迁移路线只锁"P1 先行、配置门控、可回退"这一层,不预判其内容强弱。

---

## 十、未来 tasks.md 大纲级方向(**非可执行 tasks,非验收测试代码——待冻结+放行后才写**)

> 铁律6:此处只给方向性描述,不落可执行任务/测试。冻结并经 operator 放行后,由 c2 据冻结稿细化。

- **方向 PoC**:cgroup `Delegate=yes` 父/子 scope populated 翻转验证(LF1),含 WSL2 `--user` 形态可用性 + 降级路径确认。
- **方向 GF1**:`perception_events` schema + `StateReconciler` 骨架 + 单写编译期约束 + 审计测试;realign/cancel/dispatch 改声明式期望态;GC 收敛 pass;事件日志快照/截断/vacuum。
- **方向 GF2**:epoch(复用 `state_version`)透传 IPC/tmux/路径;旧 epoch 入口丢弃。
- **方向 GF3**:`ah.toml` per-slot workdir schema;spawn 强制 workdir 绑定;主树脏写回归防线。
- **方向 GF4**:`ah job done` + Stop 强制 + 看门狗;master 自驱 RPC 化;pane 生命周期摘除(序在完成协议稳定后)。
- **方向 LF2**:job-cookie 生成/注入/入口校验。
- **方向验收锚**(北极星第五节,冻结后转测试):kill-9-ahd 事件流无洞;后台任务 end_turn 不误判完;幽灵文本零生命周期影响;删 hook 配置自检+自愈+响亮降级。

---

## 十一、执笔收敛自述(采纳/驳回台账 · 最弱区 · 留给 operator/用户的决策点)

### 对 o1 的处理台账
| o1 项 | 处理 |
|---|---|
| "同一结构病=应用层症状归纳,根因非应用层" | **采纳方向**;但**重立**为"一个缺失原语(单 Reconciler 拥有收敛)",非 o1 的两根因 |
| 根因二"原子分布式事务" | **带机制驳回**:物理资源非事务性,2PC 不可能;正确原语=reconcile-to-converge |
| 第五关节"环境物理隔离" | **采纳实质**(必须做)**,重立归属**(并入 P4,非独立关节)= GF3 |
| E1 事件溯源+Reconciler | **采纳为脊柱**(GF1),红队延迟/膨胀机制化解 |
| E2 epoch 租约 | **采纳绑定,驳回可续租约形态**(改单调版本戳,复用 `state_version`),根除活锁 |
| E3 sd_notify 完成握手 | **采纳为 F3 主信号**(GF4),outbox journal 化解协作死锁 |
| Q1 单写(SessionWriter/module 私有) | **采纳**,统一为 `StateReconciler` + 审计测试 |
| Q2 Unknown 预算表 | **采纳骨架,驳回 T2 主动探测**(违北极星铁律),改被动 T0 重读;删 T1 SIGABRT 混淆 |
| Q3 cgroup 委托 PoC | **采纳**拓扑+PoC,定 LF1 先行,给降级兜底 |
| Q4 job-cookie 归属 | **采纳**,与 epoch 合一,入口校验 |
| 三条"不打补丁"批判 | **全采纳**,且归并进 Reconciler(非三补丁);其中"可续租约"部分带机制驳回 |
| L1/L2/L3 爆炸半径分级 | **采纳 L1/L2**;**L3 更正**:GC 是脊柱内在(ground)非 local 外挂 |
| **分阶段灰度反驳(窄口径,`o1-rebuttal-phasing-2026-07-12.md`)** | **采纳**(带三条机制加固:排空屏障/schema 仅增/脚手架短暂性,§九);**更正原稿**"四地基级高耦合难拆"为"架构共享脊柱但部署可解耦",big-bang 论断作废 |

### 最弱区域(诚实标注)
1. ~~cgroup 委托在 WSL2 `--user` 形态的可用性未经实测(北极星剩 15% 全在此)~~ **✅ 已解决/闭环(2026-07-12 真机验证 PASS,证据 `wsl2-cgroup-poc-result-2026-07-12.md`)**:WSL2 真机 `populated` 序列 `1,1,0,0` 与 Linux 基线逐字一致——此前标注的"唯一残留感知层不确定性(那 15%)"已划掉,GF4 的 cgroup 兜底传感在 WSL2 成立,无需降级。详见 §四 Q3。
2. **GF1 脊柱的 SQLite 单写吞吐**在高并发多 agent 事件下的实际 reconcile 延迟未经压测——机制已给(事件驱动+物化视图读+WAL),但数值边界待实测。
3. ~~实施排期/切换策略本稿未展开~~ **已在 §九 展开**(应答 o1 分阶段反驳):四阶段可回退灰度路线 + 配置门控 + 三条机制加固。剩余未展开的是**各阶段内部的细粒度任务拆解与压测数值边界**,那属冻结+放行后由 c2 细化(其中吞吐边界已可由 P2 影子模式提前实测,见 §九)。

### 留给 operator/用户的决策点(是决策不是开放设计问题)
- **D1(最重)**:是否接受"编排底座重写、以四项地基级变更为主"的**范围**?注意:范围虽大,但经 §九 四阶段**可回退灰度**交付(非 big-bang),每阶段有一键回退开关、影子期可先验证再切——**决策难度已从"接受一次高风险大切换"降为"接受一条可随时回退的渐进路线"**。这是 gate 的核心拍板项。
- **D2**:GF3 会**破坏"所有 agent 在 main 主树开跑"的历史工作方式**(强制 per-slot workdir)——用户是否接受这一行为改变?
- **D3**:LF1 cgroup PoC 是否先行独立启动去风险(建议是,风险最低、解锁 GF4 兜底)?

---

*执笔:d1-claude(设计主笔,唯一执笔席)。**本文为 DRAFT,未冻结,须经 operator 带用户过目拍板方可进入 c2 实施。** 仅产 markdown,未碰代码/构建/git。所有现状断言带 file:line 实证(main @ 当前 repo);北极星四题、o1 候选与三批判、第五关节假说均给明确裁决,无开放式搁置。*

