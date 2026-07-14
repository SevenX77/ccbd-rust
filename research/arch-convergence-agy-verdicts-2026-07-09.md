# ccbd-rust 架构评估分歧收敛裁决书 (2026-07-09)

> **评估者**: Worker a3 · Antigravity 设计者与架构审
> **原则**: 忠实度对抗审，逐条对齐，基于代码 file:line 铁证进行客观裁决。

---

## 1. 对方独有、上轮未提主张的裁决

### D1: Job 状态裸写 UPDATE 与状态机缺失
- **裁决**: **AGREE** (对方对，我认)
- **证据**:
  - [src/db/jobs.rs:301](file:///home/sevenx/coding/ccbd-rust/src/db/jobs.rs#L301) & [src/db/jobs.rs:383](file:///home/sevenx/coding/ccbd-rust/src/db/jobs.rs#L383) (裸写 DISPATCHED)
  - [src/db/jobs.rs:494](file:///home/sevenx/coding/ccbd-rust/src/db/jobs.rs#L494) (裸写 COMPLETED)
  - [src/db/jobs.rs:528](file:///home/sevenx/coding/ccbd-rust/src/db/jobs.rs#L528) & [src/db/jobs.rs:633](file:///home/sevenx/coding/ccbd-rust/src/db/jobs.rs#L633) (裸写 CANCELLED)
  - [src/db/jobs.rs:680](file:///home/sevenx/coding/ccbd-rust/src/db/jobs.rs#L680) & [src/db/jobs.rs:894](file:///home/sevenx/coding/ccbd-rust/src/db/jobs.rs#L894) (裸写 FAILED)
  - [src/db/jobs.rs:555](file:///home/sevenx/coding/ccbd-rust/src/db/jobs.rs#L555) (裸写 cancel_requested)
  - [src/db/recovery.rs:1155](file:///home/sevenx/coding/ccbd-rust/src/db/recovery.rs#L1155) (在恢复逻辑中直写 FAILED)
- **说理**: 
  系统中确实没有任何显式的 Job 状态机结构体或转移控制逻辑，所有的状态修改全部依靠在数据访问层散落拼凑的裸 SQL 语句。转移的“合法性”被隐式隔离在各自 UPDATE 的 WHERE 条件中。这不仅导致了逻辑分散与冗余，也使得新增状态（例如挂起或重试状态）时难以实施一致性断言。

### D2: Job 完成（F3=F2）在 SQL 事务中与 Agent IDLE 硬耦合
- **裁决**: **AGREE** (对方对，我认)
- **证据**:
  - [src/db/state_machine.rs:736](file:///home/sevenx/coding/ccbd-rust/src/db/state_machine.rs#L736) (在 `mark_agent_idle_matched_outcome_sync` 事务中内联)
  - [src/db/state_machine.rs:943](file:///home/sevenx/coding/ccbd-rust/src/db/state_machine.rs#L943) (在 `mark_agent_idle_hook_event_outcome_sync` 事务中内联)
  - [src/db/state_machine.rs:1104](file:///home/sevenx/coding/ccbd-rust/src/db/state_machine.rs#L1104) (在 `mark_agent_idle_log_event_outcome_sync` 事务中内联)
- **说理**:
  这触及了“假 COMPLETED”系列事故的更深层设计根源。目前，Job 成功完成在数据库底层被处理成“Agent 变成 IDLE”的寄生副产物。在同一个原子事务中，只要 Agent 被判定为 IDLE，Job 就会被强行标记为 COMPLETED。这导致即使在上层感知到仍有后台子进程正在运行，也无法阻止数据库内 Job 状态的提前关闭。Job 应该具有自身独立的生命周期和状态转换通道。

### D3: 向 Pane 发送 Prompt 的双重同构路径与安全闸门绕过
- **裁决**: **AGREE** (对方对，我认)
- **证据**:
  - 路径一 (Orchestrator): [src/orchestrator/mod.rs:241-248](file:///home/sevenx/coding/ccbd-rust/src/orchestrator/mod.rs#L241-L248) (拥有双重 `run_dispatch_guard` 和 `wait_for_pre_send_dispatchable` 守卫)
  - 路径二 (RPC Handle): [src/rpc/handlers/agent.rs:1142-1149](file:///home/sevenx/coding/ccbd-rust/src/rpc/handlers/agent.rs#L1142-L1149) (无守卫直发)
- **说理**:
  这两条发送路径在终端底端的操作（捕获 baseline、调用 `send_text_to_pane_with_options`、启动 marker 扫描和 capture seed）几乎完全同构。然而，通过 RPC 方法 `agent.send` 触发的发送完全绕过了编排器中为了防御“物理-逻辑竞争”而设计的双向防线。这种“多头发送”是导致 dispatch-ACK 竞态的关键漏洞，必须将发送管道归口收拢。

### D4: Teardown 清理与杀 Agent 序列的四处各自编排
- **裁决**: **AGREE** (对方对，我认)
- **证据**:
  - 编排一: [src/rpc/handlers/agent.rs:275-300](file:///home/sevenx/coding/ccbd-rust/src/rpc/handlers/agent.rs#L275-L300) (在 spawn 失败时执行 `cleanup_spawn_resources`)
  - 编排二: [src/rpc/handlers/sessions.rs:134-165](file:///home/sevenx/coding/ccbd-rust/src/rpc/handlers/sessions.rs#L134-L165) (在 session.kill 时执行 DB KILLED 级联并选择性 kill Pane/Session)
  - 编排三: [src/orchestrator/mod.rs:562](file:///home/sevenx/coding/ccbd-rust/src/orchestrator/mod.rs#L562) & [src/orchestrator/mod.rs:577](file:///home/sevenx/coding/ccbd-rust/src/orchestrator/mod.rs#L577) (在 reaper 路径删除 DB Row 并调用 I/O cleanup)
  - 编排四: [src/monitor/master_watch.rs:1029](file:///home/sevenx/coding/ccbd-rust/src/monitor/master_watch.rs#L1029) -> [src/db/system.rs:381](file:///home/sevenx/coding/ccbd-rust/src/db/system.rs#L381) (在 master 死亡级联时调用 `clean_worker_runtime_resources_sync` 停 scope / 杀 pidfd)
- **说理**:
  物理资源的注销和回收散落在这四处。它们各自决定的物理与数据销毁顺序各不相同：有的直接 `delete_agent`（导致 DB 的 foreign-key ON DELETE CASCADE 提前级联切断），有的仅更新状态为 `KILLED`，有的使用 `cleanup_agent_runtime_resources` 强杀 tmux，有的调用 systemd 停止 scope。这种混乱导致了 C1/C2 Teardown 逃逸与误杀活栈。必须将这四条序列统一重构为单一的生命周期服务。

### D5: `spawn_realign_agent` 被反向依赖导致的三层循环成环
- **裁决**: **AGREE** (对方对，我认)
- **证据**:
  - 定义锚点: [src/rpc/handlers/realign.rs:375](file:///home/sevenx/coding/ccbd-rust/src/rpc/handlers/realign.rs#L375)
  - 导入锚点: [src/orchestrator/mod.rs:19](file:///home/sevenx/coding/ccbd-rust/src/orchestrator/mod.rs#L19) / [src/monitor/master_watch.rs:26](file:///home/sevenx/coding/ccbd-rust/src/monitor/master_watch.rs#L26)
- **说理**:
  `spawn_realign_agent` 协调了 Agent 容器/物理沙箱的重新供给与状态对齐，这属于核心的 Application/Domain 领域逻辑。但它在物理文件层被放在了传输层 rpc handlers 中，导致内核层（Orchestrator）与监控层（Monitor）必须逆向引用传输层代码，打破了“单向分层依赖”的原则。将其剥离并迁入 domain/application 势在必行。

---

## 2. antigravity 上轮主张被质疑的辩护或撤回

### C1: rpc "职责合理" 与 prompt_handler "内聚良好" 评级的修正
- **裁决**: **REVISE** (我方认错，撤回 rpc 评级，保留 prompt_handler 评级)
- **证据**:
  - [src/rpc/handlers/sessions.rs:230-292](file:///home/sevenx/coding/ccbd-rust/src/rpc/handlers/sessions.rs#L230-L292) (内联了完整的 DB 状态级联清理事务与裸 SQL 写入)
  - [src/rpc/handlers/sessions.rs:511-627](file:///home/sevenx/coding/ccbd-rust/src/rpc/handlers/sessions.rs#L511-L527) (内嵌了完整的 master 物理与逻辑 spawn 控制流)
  - [src/rpc/handlers/sessions.rs:961-1168](file:///home/sevenx/coding/ccbd-rust/src/rpc/handlers/sessions.rs#L961-L1168) (内嵌了复杂的 master cutover 阶段性状态机 SAGA 逻辑)
- **说理**:
  我承认在上轮评估中对 `src/rpc/handlers/` 特别是 `sessions.rs` 的走读不够彻底，给出了不当的“合理”评级。该模块在多个 Handler 中直接承载了极重的核心供给和事务流转逻辑，严重违背了“传输层仅做参数解析，业务逻辑下沉服务”的架构共识。为此，我撤回对 rpc 的原评级，并同意将其判定为“严重职责越界”，支持将这些 SAGA 与事务清理重构剥离。对于 `prompt_handler`，其通过 `PromptRunOutcome` 将动作决策安全上报，未在内部产生副作用或写库，仍维持“内聚良好”评级。

### C2: 数据库锁死锁 claims 的撤回与修正
- **裁决**: **REVISE** (撤回 SQLite 死锁论断，修正为 Mutex 线程阻塞 contention 与 CAS 吞事件分析)
- **证据**:
  - [src/db/mod.rs:26-27](file:///home/sevenx/coding/ccbd-rust/src/db/mod.rs#L26-L27) (定义数据源为 `Arc<Mutex<Connection>>`)
  - [src/db/mod.rs:32-36](file:///home/sevenx/coding/ccbd-rust/src/db/mod.rs#L32-L36) (锁定 mutex 并返回 `MutexGuard`)
- **说理**:
  我撤回关于“多个写入线程在 SQLite 层面造成死锁”的论断。对方指出，由于共享连接在 Rust 层面是通过 `std::sync::Mutex` 进行序列化同步的，任何时间只可能有一个线程持有 MutexGuard 执行 SQL，这在机制上就阻断了 SQLite 并发锁冲突导致的死锁。但这种单连接同步架构引入了严重的线程阻塞风险（contention）；且目前多头写入依靠的 CAS 检测机制（`state_version` 比较）在发生并发写冲突时，表现为“静默吞掉更新/事件”（CAS 失败回滚事务），而非抛出死锁。这一修正更符合物理运行的事实。

### C3: 错挂到 git_diff.patch 的文件锚点修正
- **裁决**: **REVISE** (修正锚点)
- **证据**:
  - 原错挂锚点修正为实际源码:
    - [src/monitor/master_watch.rs:2057](file:///home/sevenx/coding/ccbd-rust/src/monitor/master_watch.rs#L2057) (替代原 git_diff.patch 引用，指向 `spawn_realign_agent` 的调用位置)
    - [src/rpc/handlers/agent.rs:358](file:///home/sevenx/coding/ccbd-rust/src/rpc/handlers/agent.rs#L358) (替代原 git_diff.patch 引用，指向 `AgentSpawnDbAction::ReplaceKilledAndRequeue` 业务处理位置)
- **说理**:
  原评估报告中为了追踪最新修复的原子性设计，直接使用了 patch 补丁中的行号，这不符合“以当前 main 活栈代码为依据”的评估规范。现已查实并修正为实际 Rust 源文件行号。

---

## 3. 收敛后共同结论 (Convergent Conclusions)

1. **建立 Job 状态机与写权威**: 剥离与 Agent IDLE 状态的物理寄生耦合，消除散落的裸 SQL，实现 Job 独立生命周期。
2. **构建声明式感知仲裁器**: 降级 6 个推断器为只读信号源，收拢至 Orchestrator 主循环执行优先级判定。
3. **收拢物理生命周期服务**: 将四路分散的 teardown/kill 逻辑统一归口为 `WorkerLifecycle` 供给与回收模块。
4. **清理 rpc/ 传输层越权行为**: 剥离内联在 session handlers 中的 SAGA 与裸事务，将 `spawn_realign_agent` 下沉至领域服务层。
5. **实行参数化身份与独立凭据**: 彻底废除基于环境的 cgroup 嗅探，对 sandbox 提供 per-worker credentials 拷贝隔离。
6. **Orchestrator 自愈引入熔断与 Cancel 校验**: 引入 Crash-loop 计数限制防自燃循环，并在 Recovery 动作前强检 `cancel_requested`。
