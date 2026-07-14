# o1 发散与红队设计备忘：编排底座第一性重构 (2026-07-11)

本备忘录由设计辩论席（o1-antigravity）针对轨2“编排底座第一性重构”课题产出。本着第一性原理与对抗性红队视角，对当前设计课题的 situational judgment、架构候选方案、北极星四题以及潜在的补丁修补倾向进行深度批判与发散，旨在为 d1 执笔收敛设计提供最强反方论据。

---

## 明确立场与声明

1. **对“同一结构病”判断的立场**：
   - **支持部分**：完全同意“感知、完成协议、控制面状态机、生命周期”四大关节存在同源性设计缺陷，特别是并发写入冲突与启发式信号越权导致的系统性失控。
   - **修正与推翻部分**：我认为 operator 汇编的“同一结构病”判断仍属于**应用层的症状归纳**，未能直达最底层的原语缺失。其根源在于：
     1. **缺乏物理与逻辑工作区的强隔离与硬性环境绑定**（非感知/协议问题，而是基础设施配置缺陷，如 [agent-workspace-assignment-2026-07-11.md](file:///home/sevenx/coding/ccbd-rust/.kiro/specs/ah-orchestration-reliability/agent-workspace-assignment-2026-07-11.md) 所指出的环境缺失）。
     2. **缺乏原子分布式事务（Distributed Transaction）的边界隔离**。在跨 SQLite 数据库和外部 tmux 伪终端/物理进程时，编排层把“物理世界的变更”与“逻辑世界（DB）的提交”拆成了非原子的分步操作，导致了级联竞态。

2. **本案闸门位置认知**：
   - 本备忘录仅作为发散与红队输入。本单收敛后，将由 d1 执笔冻结设计方案，在 operator 带用户过目拍板前，**绝对不放行 c2 开始实施**。

---

## 一、 “同一结构病”判断的批判与根因再分类

对于 A-E 类症状，我们可以将其按照“最底层原语缺失”进行重新归类和批判：

### 1. 结构病症状归类与真实代码索引

| 症状 ID | 归类（四大关节） | 现象深描 | 最底层根因与代码/失效场景定位 |
| :--- | :--- | :--- | :--- |
| **ah#16** | 控制面/状态机 | `ah up` 导致 agent 重复 tmux session 或丢失。 | **非原子分布式事务**：[realign.rs:266](file:///home/sevenx/coding/ccbd-rust/src/rpc/handlers/realign.rs#L266) 与 [realign.rs:332](file:///home/sevenx/coding/ccbd-rust/src/rpc/handlers/realign.rs#L332) 先执行毁灭性的 `delete_agent` 提交，然后才尝试物理 spawn。一旦 spawn 阻塞或超时，状态机中途崩塌，导致 agent row 彻底丢失。 |
| **ah#17** | 感知/派单判定 | 幽灵文本引起 `PROMPT_PENDING` 弹跳与卡派单。 | **启发式信号越权**：[state_machine.rs:258](file:///home/sevenx/coding/ccbd-rust/src/db/state_machine.rs#L258) 允许从 `IDLE/SPAWNING` 任意被 PTY/Pane Diff 监视器推向 `PROMPT_PENDING`。由于缺乏单写仲裁，转瞬即逝的 UI 噪点被固化为了逻辑状态。 |
| **ah#19** | 感知/完成协议 | 挂在 `DISPATCHED` 状态 60-90min 从不被判定。 | **单写控制面死锁**：[monitor.rs:10](file:///home/sevenx/coding/ccbd-rust/src/completion/monitor.rs#L10) 强制 300s 后退避到 Pane 扫描，但 Pane 扫描因 [gating.rs:166](file:///home/sevenx/coding/ccbd-rust/src/prompt_handler/gating.rs#L166) 拦截而无法安全收敛，导致状态机彻底失去唤醒源。 |
| **ah#21** | 感知/恢复 | respawn 后 agent 被判超时，实际在执行接力棒。 | **状态纪元漂移（Epoch Drift）**：进程被物理杀死并 respawn 获得了新 PID，但旧的 pending ACK 扫描器或定时器仍在使用旧状态版本在后台竞争，强行写入 `INIT_PROBE_TIMEOUT` 判定（[health_check.rs:133](file:///home/sevenx/coding/ccbd-rust/src/provider/health_check.rs#L133)）。 |
| **ah#22** | 感知/master自驱 | master 唤醒文本打进 composer 但从不提交。 | **人机协作边界模糊**：Master 自驱没有走显式的 RPC 交互协议，而是依赖“向 PTY 投键并期待 shell 自动回车”的伪交互。这在本质上不属于感知层缺陷，而是**控制链路缺乏 API 化原语**。 |
| **obs#49** | 生命周期/控制面 | cancel→respawn 重投，取消的任务被完整执行。 | **缺乏事务取消级联机制**：[recovery.rs:393](file:///home/sevenx/coding/ccbd-rust/src/db/recovery.rs#L393) 处的 reinsert 逻辑在重投 prompt 时，只盲目读取 `CapturedInterruptedJob`，却完全不检查 `cancel_requested` 标志位，导致已撤销的 job 在新 sandboxed 席位上复活。 |
| **obs#51** | 完成协议 | Stop 钩子不火 × 超时 × 催单器逼单造成的误实施。 | **控制链注入侧信道攻击**：催单器在 [state_machine.rs:1157](file:///home/sevenx/coding/ccbd-rust/src/db/state_machine.rs#L1157) 硬编码单一场景文案，无视 agent 正处于 `PLAN_FIRST` 的等待批准阶段，通过“系统强制文本”强行击穿了 agent 的安全纪律。 |
| **obs#52** | 控制面/状态机 | cancel 占席僵尸导致队列排水，派发古董 brief。 | **历史状态残留与非原子派单**：cancel 的状态副作用越界，且 [jobs.rs:340](file:///home/sevenx/coding/ccbd-rust/src/db/jobs.rs#L340) 在捞取 QUEUED 任务时没有对 age 或 session 关联性做时效屏障。 |

### 2. 第五大关节假说：环境物理隔离边界（Environment Isolation Boundary）的缺席

除了上述四关节，我们提出**第五个必须重构的结构域**：**环境物理隔离边界**。
- **论据**：在 `obs#47`（未换血的 codex 在主树混写）和 `obs#49③`（codex 钉死 commit 本地 main）中，表面上是“规则未送达”或“agent 违反了叮嘱”。但从第一性原理来看，**LLM 是概率模型，任何可以通过指令妥协的隔离区，最终在规模压力下必然失守**。
- **代码物证**：`src/rpc/handlers/agent.rs` 在拉起 tmux session 时，默认使用 `agent_cwd`。如果 config 层的 `agents.toml` 没有硬性的 `workdir` 强制指派（目前 `src/config/` 下没有任何 slots-specific directory 逻辑），所有 agent 都共享同一个物理工作区。
- **结论**：本版重构**必须**将“工作区物理隔离与 slot 环境指派”作为与状态机同等重要的地基级变更进行强制落地，不允许继续依靠 brief 里的 `cd` 叮嘱指令。

---

## 二、 候选架构铺开与红队对抗

### 候选架构一：事件溯源 (Event Sourcing) 与单一 Reconciler 串行写

- **核心设计**：
  - 剥夺所有异步 Monitor（如 `fifo_reader`、`health_check` 等）直接调用 `mark_agent_xxx` 写入数据库 `agents` 表的权利。
  - 它们必须将高度结构化的 `PerceptionEvent`（带全局递增 Seq ID 与 Logical Epoch）插入 `perception_events` 事件日志表。
  - 引入单一的 Reconciler 循环（串行执行器，类似 K8s Reconcile Loop），独占 `agents` / `jobs` 表的写锁，顺序消费事件日志，输出 CAS 状态更新。
- **红队批判（失效模式与新隐患）**：
  - **Reconcile 延迟与吞吐瓶颈**：串行 Reconciler 如果遭遇 SQLite 写锁竞争（例如在多 agent 密集写事件时），会导致调度 tick 出现明显的延迟。如果 500ms 内事件未能及时 reconcile，调度器可能判定 agent 仍为旧状态，造成延迟派单。
  - **事件爆炸与清理负担**：随着系统长周期运行，`perception_events` 表将迅速膨胀，必须引入类似 `ahd.sqlite` vacuum 及事件截断（Snapshot Pruning）逻辑，否则会加剧 `ah#23`（2GB 空间留死页）的存储卫生危机。
  - **爆炸半径**：**极高（地基级）**。需重写 `src/db/state_machine.rs` 下全部的 CAS 变轨入口，将所有 direct-write 逻辑转为 event-insert。

### 候选架构二：基于 Epoch 租约的悲观分布式锁（Epoch-Leased Pessimistic Lock）

- **核心设计**：
  - 每一个 agent slot 被分配一个全局递增的逻辑纪元（Logical Epoch / state_version）。
  - 所有的物理资源（tmux session 名、cgroup ID、Unix Domain Socket、I/O FIFO）的生命周期都与当前的 Epoch 强绑定。
  - 任何来自于旧 Epoch 的信号（例如旧进程迟到的 exit 信号、旧 hook 的迟到上报）在进入控制面时，因 Epoch 不匹配直接被丢弃。
- **红队批判（失效模式与新隐患）**：
  - **租约活锁（Livelock）**：如果因为网络或 I/O 超时，新 spawn 的 agent 未能在规定时间内续租（Renew Lease），Reconciler 会判定其超时并递增 Epoch 重启。而实际上旧进程可能只是暂时被 CPU 调度卡住，恢复后它发现自己的 Epoch 已过期被判定为非法，而新 Epoch 的 spawn 又在不断重复，导致 agent 陷入“无限 spawn-expire 循环”。
  - **爆炸半径**：**高（地基级）**。不仅改动 DB 状态，还要在所有 IPC、tmux 命令行生成及文件路径管理（如 `/tmp/ahd/agent_{id}_{epoch}/`）中穿透携带 Epoch 参数。

### 候选架构三：基于 sd_notify 协议的任务显式“双向握手”完成协议

- **核心设计**：
  - 废除任何基于“停轮 == 完成”或“pane-diff 出现 UI 提示符 == 完成”的启发式推断。
  - 状态机只承认由 worker 自报的 T1 显式完成信号（如调用专用工具 `ah job done <id>` 或 `sd_notify` 写入 IPC outbox）。
  - 在完成信号到达后，Orchestrator 会挂起状态，强制触发“物理证据拦截器”（EVIDENCE_CHECK），验证 git 工作区或产物。通过后才由 Orchestrator 发送 `ACK` 允许 agent 释放 slot。
- **红队批判（失效模式与新隐患）**：
  - **协作死锁（Co-lock）**：如果 agent 已经完成了任务，但由于某种物理故障（如 socket 缓冲区满、磁盘满导致 IPC outbox 无法写入），完成工具执行失败，agent 将无限挂起等待系统 ACK，而系统又在等待完成工具的调用，造成逻辑死锁。
  - **非互作任务（No-op/Query Job）的阻断**：若静态标注的 `is_mutating` 出错，只读任务在证据核验时因为没有 `git diff` 被拦截器无限制 nudge（[perception-final-convergence-2026-07-09.md:71](file:///home/sevenx/coding/ccbd-rust/research/perception-final-convergence-2026-07-09.md#L71)），需要有坚固的“最大逼单上限（2次 nudge 后强行放行）”兜底。
  - **爆炸半径**：**中等（局部模块级）**。主要改动集中在 agent provider harness 侧的完成逻辑封装与 orchestrator job 状态机的 done 接口。

---

## 三、 感知层北极星四题红队立场

针对 `perception-final-convergence-2026-07-09.md` 提出的四道设计必答题，给出如下对抗性设计立场：

### 1. 单写入口硬约束形态
- **现状批判**：目前的 `state_machine.rs` 充斥着对外直接暴露的写函数，Monitor 可以从任意线程抢占 CAS。
- **硬约束形态**：**利用 Rust 的私有性与 Module 边界，配合 `SessionWriter` 独占所有权**。
  - 彻底将 `src/db/state_machine.rs` 中除了 `reconcile_state_log` 之外的写入函数设为 `pub(self)` 或私有。
  - 只有唯一持有的 `struct StateReconciler` 可以访问变更 DB 状态的底层 SQL 链接。
  - 其他模块只被允许通过一个多生产者通道发送只读的感知事件：`struct PerceptionEventChannel`。

### 2. 各信号类 Unknown 预算与降级动作

当高可靠信号在预算内缺席时，**严禁无声降级**为低可靠性信号（如 pane 扫描提示符判定完成），必须触发对应的“响亮动作”：

| 信号级别 | 信号来源 | Unknown 预算上限 | 预算超时后的权威判定/降级动作 |
| :--- | :--- | :--- | :--- |
| **T0 (OS)** | pidfd / cgroup populated | **0 秒** (立即) | pidfd 退出而 cgroup populated 不为 0 且无 T1 心跳时，判定进程已死或逃逸。**动作：立即将 agent 标记为 `CRASHED`，挂起对应 job，释放 physical resources，不作自动恢复。** |
| **T1 (Hook)** | Outbox IPC / sd_notify | **10 秒** | 进程退出但 10 秒内未收到显式完成 Hook（可能由于调度延迟）。**动作：转为 `UNKNOWN`，向 PM 发送 Alert 事件，禁止派单，启动 watchdog 并在 30 秒后强制 SIGABRT 杀栈。** |
| **T2 (Log)** | FIFO Reader output | **180 秒** (无输出) | 任务繁忙但 3 分钟无任何 stdout 变动。**动作：触发主动心跳探测。若 30 秒探测无回显，判定为 `STUCK`，对 job 执行 `HEALTH_CHECK_STUCK` 强制失败。** |
| **T3 (UI)** | Pane diff scrape | **0 秒** | 永远不允许根据 UI Scrape 推导生命周期状态。UI 扫描出的幽灵文本只能用于辅助生成“交互对话框待响应（F4）”的警报，**预算为零，即绝不用作状态翻转依据**。 |

### 3. cgroup 委托布局的 PoC 方向与最小实验

- **PoC 设计思路**：
  - 目前 ahd 借助 systemd transient scope 把所有进程打包进同一个 scope 单元。
  - **委托布局（Delegation）**：利用 `Delegate=yes` 配置项，允许在 Scope 下由 Rust 进程进一步划分层级。
  - **布局拓扑**：
    ```
    ah-agent-session-xxx.scope (Parent: 包含 agent CLI 自己的进程)
      └─ payload.slice (Child: 包含 agent CLI spawn 出来的 bash/编译/测试子进程)
    ```
  - **PoC 实验验证步骤**：
    1. 生成一个启用 `Delegate=yes` 的 transient scope，拉起一个 python 管理进程模拟 agent CLI。
    2. python 管理进程通过系统调用创建一个子 cgroup `payload`，并将其 spawn 的 shell PID 写入 `payload/cgroup.procs`。
    3. python 进程本身保存在父 scope，不移动。
    4. 监控 `payload/cgroup.events` 的 `populated` 字段。
    5. 验证：当 shell 进程退出后，即使 python 管理进程依旧活着且持续运行，`payload/cgroup.events` 依然能准确监测到 `populated=0` 状态的翻转，从而物理剥离了“常驻管理进程”对“任务真完成”的干扰。

### 4. Hook 归属竞态与身份校验机制

- **安全边界风险**：
  - 如果多个 agent 同时向 ahd Unix Socket 汇报 `notify --event stop`，或者存在恶意/失控的 agent 进程冒充其他 slot 上报状态，会导致状态踩踏。
  - 由于 Unix Socket 的 `SO_PEERCRED` 返回的 PID 在高频 respawn 下可能存在 PID 回收与复用竞态，依靠宿主 PID 判定归属极度危险（sd_notify 点名的归属漏洞）。
- **解法方向**：
  - **基于一次性 Job-Cookie 的静态校验**：
    1. 在 `Orchestrator` 向 agent 派发 Job 时，生成一个强随机的 `job_cookie`（128位 UUID），并写入该 job 在 DB 中的行，同时以 `AH_JOB_COOKIE` 环境变量注入该 Sandboxed 席位的环境。
    2. 当 Hook/完成工具触发上报时，其**必须**在 IPC Payload 中携带这个 `AH_JOB_COOKIE`。
    3. ahd 接收到 IPC 后，将 Payload 中的 Cookie 与数据库中该 agent slot 下正处于 `DISPATCHED` 的 job 的 Cookie 进行强校验。
    4. 任何 Cookie 不匹配、缺失或属于旧 Epoch 的上报直接丢弃，并记录安全告警事件（防止跨泳道冒充与时序漂移污染）。

---

## 四、 “不打补丁红队”：对既有 spec/修复思路的二次批判

对 `.kiro/specs/ah-orchestration-reliability/` 中提出的修复思路进行对抗性检查，指出其依然带有的“打补丁”倾向：

### 1. 对 `stuck-false-positive-log-monitor-handoff.md` 的批判
- **补丁表现**：修改 `src/provider/health_check.rs:46` 中的 `.or()` 运算为 `.max()` 确实解决了当前 output 时间被 shadow 的 bug。
- **结构病根源**：为什么“是否卡死”需要靠在外部去捞 `last_marker_ts` 和 `last_output_ts` 进行复杂的 `max` 算术推导？这种靠外部被动猜测时间差的设计本身就是不稳固的。
- **彻底消除方案**：应该在 `agents` 表中引入显式的“活性租约有效期（lease_expires_at）”。Agent 必须定期向控制面发送心跳（Lease Keepalive）。如果到期未收到心跳，则自动过期。外部计算应该转变为“状态机内部租约到期自动置 STUCK”，消灭在 `health_check.rs` 中写大段推导公式的代码。

### 2. 对 `realign-atomicity.md` 的批判
- **补丁表现**：提出在物理 spawn 完毕前，老 agent row 维持原状；spawn 成功后利用 SQL 事务进行 Swap 替换。
- **结构病根源**：这防住了“DB 里的 agent row 消失”的症状，但它忽略了**“物理资源孤儿化（Physical Orphan Leaking）”**。如果物理 spawn 确实拉起了 tmux session %7，但在最后写入 DB 事务时由于机器断电或 SQLite 磁盘爆满导致失败，虽然老 row 保住了，但那个已经跑起来的 tmux 进程和沙箱目录就成了孤儿。下次 `ah up` 依然会遭遇“有孤儿 session 重名并存”的灵异现象。
- **彻底消除方案**：必须引入 **物理资源 GC (Garbage Collection) 看门狗**。每次 `realign` 扫描物理 tmux/cgroup 时，如果发现任何物理实体没有在 DB 中登记对应的 `active` row，必须强行 reaping。不能指望“spawn 失败后物理清理函数能 100% 成功执行”（fail-dangerous），物理与逻辑的对齐必须由“定时对比拉平（Active-Reconcile）”的闭环机制来保证。

### 3. 对 `recovery-reinsert-vs-cancel-race-2026-07-11.md` 的批判
- **补丁表现**：检查 `cancel_requested` 并使其直接收敛至 `CANCELLED`。
- **结构病根源**：依然把“Cancel”和“Respawn”看作是异步的两个分支在碰撞。
- **彻底消除方案**：强制将 `cancel` 操作原子化。当用户调用 cancel 时，应在同一个 DB 事务中将当前 job 置为 `CANCELLED` 终态，并**同步**向进程组发送 SIGKILL。如果进程组已经死了，就直接清理，绝不把“清理”动作委托给后续的 recovery loop 让他去决定是否 respawn。把“控制指令”与“恢复循环”彻底解耦。

---

## 五、 重构爆炸半径分级目录

为提供清晰的决策参考，将本版重构中讨论的各方案按**爆炸半径**进行显式分级：

### 1. 地基级（Ground-level）变更 —— 动一发而牵全身
> 要求几乎所有实施位置配合迁移，包括数据库 schema 变更、主流程重构。

- **E1: 单写 Arbitrated FSM 改造**
  - **范围**：涉及所有 Monitor 的写入口，需废除所有的 `mark_agent_xxx` 写入函数，改写 `src/db/state_machine.rs` 以支持单一事件流 Reconciler。
  - **评估**：极高，但对彻底解决竞态是不可免的。
- **E2: slot 工作区物理隔离绑定**
  - **范围**：修改 `ah.toml` schema，在 `src/rpc/handlers/agent.rs` 的 spawn/respawn 路径中强制应用 `workdir` 作为 tmux 的 `-c` 启动参数。
  - **评估**：高。将破坏原有的“所有 agent 在 main 主树直接开跑”的历史行为，彻底杜绝主树脏写。
- **E3: 状态 Epoch 纪元标识透传**
  - **范围**：修改 `agents` 数据库表，追加 `logical_epoch`，并在所有的 Hook payload、Unix IPC 数据结构中强制要求携带该 Epoch。
  - **评估**：中高。需要重构 IPC 通道协议。

### 2. 局部级（Local-level）变更 —— 可独立模块化替换
> 可以在单一子系统中完成隔离开发，爆炸半径仅限于模块内部。

- **L1: cgroup 委托子 scope 划分**
  - **范围**：仅限 `src/systemd_unit.rs` 的单元文件模板生成，以及 ahd 检测完成时读取 cgroup.events 的路径。
  - **评估**：低风险，高收益。可通过独立单元测试验证。
- **L2: 基于 AH_JOB_COOKIE 的 Hook 鉴权**
  - **范围**：改动仅限 spawn 时环境变量注入以及 IPC 接收端的数据库验证逻辑。
  - **评估**：中低。不影响 DB 核心转换逻辑。
- **L3: 物理资源 Reaping 垃圾回收看门狗**
  - **范围**：在 `realign.rs` 结尾或定时 tick 中，新增一个独立协程，专门比对 DB 列表与 tmux sessions 列表，执行单向物理清理。
  - **评估**：中等。是一个纯增量、自收敛的容错机制。
