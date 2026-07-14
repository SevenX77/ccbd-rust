reply: 针对 ahd 在 OOM 重启后区分“有意 Kill”与“意外 Crash”并续上断点（Resume）的设计，我从第一性原理出发，提出以下机制架构。

### 1. 第一性原理推演：如何区分有意与意外？

在无状态的守护进程和无外置 supervisor 的环境下，**不能依赖守护进程自身的退出标记（Shutdown Reason）**。OOM-killer 随时会发 `SIGKILL(9)`，根本没有执行 Shutdown Hook 或存库的机会。

因此，**真理（Single Source of Truth）只能存在于被操作对象的细粒度生命周期状态机中**：
*   **有意 Kill**：是显式的 RPC 请求（`session.kill`）或级联动作（如 anchor 销毁、master 自然退出）。这种动作有完整的时间去把 Agent 的 `state` 刷成 `KILLED`。
*   **意外 Crash / OOM**：是物理层面的猝死。进程消失了，但它在 DB 里的最后状态依然是 `BUSY`, `WAITING_FOR_ACK`, `IDLE` 等“活跃（Active）”状态。
*   **结论**：`ahd` 重启时，根本不需要“Daemon Epoch”这种宏观计数器。只需遵循一个核心公理：**重启后，凡是进程没了但 DB 里还是 ACTIVE 状态的，一律视为受害者（意外被杀，进入 CRASHED）；凡是 DB 里已经是 KILLED 的，就是寿终正寝的，直接忽略。** 这正是 ah 现有的 `startup_reconcile` 逻辑，**这个地基是完全正确且健壮的**。

---

### 2. 核心问题修复：保住断点意图 (The Continuity Contract)

目前最大的 Gap 是在 `CRASHED` 状态转换时（`startup_reconcile_phase_c_crash_dead`），把在途的 `DISPATCHED` Job 顺手标 Fail 删了。这导致 Agent 虽然物理上 Resume 了，但在逻辑编排层变成了“没有任务在跑”的无头苍蝇。

**推荐机制：ah-job 层的悬停（Hover）与重抛（Re-throw）。**

*   **设计原则**：对于可恢复的 Provider，在途任务不应 Failed，而应退回为“待恢复（Recovery-Pending）”状态，等待 Agent 物理复活后继续接收其 ACK。
*   **落地动作 [BREAKING]**：
    1.  修改 `src/db/system.rs:779`：在 `mark_dispatched_jobs_failed_for_agent_conn_sync` 之前，增加条件判断：`if !is_recovery_eligible_provider(candidate.agent.provider)`。
    2.  对于 eligible 的 Agent，在途的 `DISPATCHED` Job 保持不动（不置 Fail）。
    3.  当 Orchestrator 发起 `spawn_realign_agent(is_recovery=true)` 且 Agent 重新达到 `IDLE` 或 `BUSY`（比如依靠 Resume 时重放 Transcript 带来的状态跳变）时，Job 会自然地接上之前的 Evidence 判定流程。如果 Provider 不重放 Transcript，我们甚至可以在 Recovery 结束时重发这个 Job（视具体 Provider 能力而定）。

---

### 3. 反孤儿模型的协同（共存契约）

OOM 会通过 `BindsTo` 把所有 agent scope 杀得干干净净。这里的难点不在于杀不干净，而在于 **如何重建** 和 **不误杀**。

*   **并存契约**：
    1.  `BindsTo=ahd.service` 是铁律，绝不修改。这意味着 Daemon 死了，所有物理 Scope 必须陪葬（保证零孤儿的绝对安全底线）。
    2.  `startup_reconcile` 负责“验尸”。看到没心跳的，标为 `CRASHED`。
    3.  重点：**保留 `Sandbox Home`**（PR7 已实现）。只要宿主目录还在，物理进程的消亡就只是一次休眠。
    4.  `reconcile_orphan_scopes_sync`（目前漏接了的那个函数）：应在 `ahd` 启动的最前端接入，但**必须保留现有的过滤白名单**（不碰属于可恢复 Provider 且状态为 CRASHED 的残留 Scope，虽然在 BindsTo 下它们理论上已经被杀了，但为了防御 fallback direct-spawn 的情况，必须过滤）。

---

### 4. 第 7 条补充：Master 进程的命运与重生

*   **现状认知**：Master 是通过 `claude --continue` 外部启动的，它**不是** CCBD 的 Worker，它的生命周期在概念上属于 Client / Harness。
*   **第一性原理判断**：**ahd 绝对不能越俎代庖去复活 Master。**
    *   **理由**：Master 代表的是用户的意图输入端（哪怕它是个 Agent，对 ah 来说它也是 User）。如果 ahd 自动拉起 Master，谁来看它的输出？它的标准流（stdin/stdout）接去哪里？
*   **闭环方案（Scope-out to Client）**：
    *   这必须是一个 **Client-side reconnect**（客户端重连）逻辑。
    *   就像传统的 ssh 掉线一样，Harness 层（或者跑 Master 的那个外部服务）必须自己有监控（比如 Systemd 服务监控这个 `claude` 进程）。
    *   **时序闭环**：ahd OOM -> `BindsTo` 杀掉 ahd, worker, master -> ahd 被 systemd 重启 -> 标 worker 为 CRASHED -> Harness 层检测到 master 死亡，重启 master (`claude --continue`) -> master 启动后，它的第一个动作是连到 ahd，发起 `session.realign` 检查手下 -> ahd 收到 realign 请求，发现 worker 是 CRASHED，执行 `spawn(is_recovery=true)` -> worker 用 `--continue` 复活。
    *   **结论**：把 Master 重生明确划出本机制范畴，交给上层调用者（Client/Harness）。

### 5. 总结：最合理的单一推荐链路

1.  **ahd 被 OOM 杀 -> Systemd 自动拉起 ahd。**（已有的 `Restart=on-failure`）
2.  **Startup Reconcile 验尸**：所有活跃变死的 Agent 转入 `CRASHED`，**但不 Fail eligible 的 DISPATCHED Job。**（修改 `system.rs` 避开 Fail）
3.  **等待上层（Harness）复活 Master**：上层 Harness 检测到 Master 随 ahd 一起死了，重新拉起 Master 进程。
4.  **Master 发起 Realign**：Master 通过 RPC 触发 `session.realign`。
5.  **Recovery Spawn**：ah 看到 `CRASHED` 状态，组装动态 `resume_args`，重新拉起 Worker 进程，连接回旧的 Sandbox Home，并在旧的在途 Job 上继续监听。

此方案模型上没有新增一张表、一个 Enum，仅仅是通过**状态截流（不 Fail 任务）**和**控制反转（等上层复活主控）**，在分布式重启混沌中确立了绝对的时序确定性。  在 OOM 重启后区分“有意 Kill”与“意外 Crash”并续 上断点（Resume）的设计，我从第一性原理出发，提出以下机制架构。

### 1. 第一性原理推 演：如何区分有意与意外？

在无状态的守护进程和无外置 supervisor 的环境下，**不能 依赖守护进程自身的退出标记（Shutdown Reason）**。OOM-killer 随时会发 `SIGKILL( 9)`，根本没有执行 Shutdown Hook 或存库的机会。

因此，**真理（Single Source of Truth）只能存在于被 操作对象的细粒度生命周期状态机中**：
*   **有意 Kill**：是显式的 RPC 请求 （`session.kill`）或级联动作（如 anchor 销毁、master 自然退出）。这种动作有 完整的时间去把 Agent 的 `state` 刷成 `KILLED`。
*   **意外 Crash /  OOM**：是物理层面的猝死。进程消失了，但它在 DB 里的最后状态依然是 `BUS Y`, `WAITING_FOR_ACK`, `IDLE` 等“活跃（Active）”状态。
*   **结论**： `ahd` 重启时，根本不需要“Daemon Epoch”这种宏观计数器。只需遵循一个核心公 理：**重启后，凡是进程没了但 DB 里还是 ACTIVE 状态的，一律视为受害者（意外被 杀，进入 CRASHED）；凡是 DB 里已经是 KILLED 的，就是寿终正寝的，直接忽略 。** 这正是 ah 现有的 `startup_reconcile` 逻辑，**这个地基是完全 正确且健壮的**。

---

### 2. 核心问题修复：保住断点意 图 (The Continuity Contract)

目前最大的 Gap 是在 `CRASHED` 状态转换时（`startup_reconcile_phase _c_crash_dead`），把在途的 `DISPATCHED` Job 顺手标 Fail  删了。这导致 Agent 虽然物理上 Resume 了，但在逻辑编排层变成了“没有任务在跑”的无头苍蝇。 

**推荐机制：ah-job 层的悬停（Hover）与重抛（Re-throw）。**

*    **设计原则**：对于可恢复的 Provider，在途任务不应 Failed，而应退回为 “待恢复（Recovery-Pending）”状态，等待 Agent 物理复活后继续接收其 ACK。
*   **落地 动作 [BREAKING]**：
    1.  修改 `src/db/system.rs:77 9`：在 `mark_dispatched_jobs_failed_for_agent_conn_sync` 之前，增加条件判断： `if !is_recovery_eligible_provider(candidate.agent.provider)`。
    2.  对于 eligible  的 Agent，在途的 `DISPATCHED` Job 保持不动（不置 Fail）。
    3.  当  Orchestrator 发起 `spawn_realign_agent(is_recovery=true)` 且 Agent 重新 达到 `IDLE` 或 `BUSY`（比如依靠 Resume 时重放 Transcript 带来的状态跳变）时，Job 会自然 地接上之前的 Evidence 判定流程。如果 Provider 不重放 Transcript，我们甚至可以在 Recovery 结束时重发这个 Job（ 视具体 Provider 能力而定）。

---

### 3. 反孤儿模型的协同（共存契约）

O OM 会通过 `BindsTo` 把所有 agent scope 杀得干干净净。这里的难点不在于杀 不干净，而在于 **如何重建** 和 **不误杀**。

*   **并存契约**：
    1 .  `BindsTo=ahd.service` 是铁律，绝不修改。这意味着 Daemon 死了，所有物理 Scope 必须 陪葬（保证零孤儿的绝对安全底线）。
    2.  `startup_reconcile` 负责 “验尸”。看到没心跳的，标为 `CRASHED`。
    3.  重点 ：**保留 `Sandbox Home`**（PR7 已实现）。只要宿主目录还在，物理进程的消 亡就只是一次休眠。
    4.  `reconcile_orphan_scopes_sync`（目前 漏接了的那个函数）：应在 `ahd` 启动的最前端接入，但**必须保留现 有的过滤白名单**（不碰属于可恢复 Provider 且状态为 CRASHED 的残留 Scope，虽然在  BindsTo 下它们理论上已经被杀了，但为了防御 fallback direct-spawn 的情况，必须过滤）。

---

###  4. 第 7 条补充：Master 进程的命运与重生

*   **现状认知**：Master 是 通过 `claude --continue` 外部启动的，它**不是** CCBD 的 Worker，它的生命周期在概念 上属于 Client / Harness。
*   **第一性原理判断**：**ahd 绝对不能越俎代庖去复活 Master。 **
    *   **理由**：Master 代表的是用户的意图输入端（哪怕它是个 Agent，对  ah 来说它也是 User）。如果 ahd 自动拉起 Master，谁来看它的输出？它的标准流 （stdin/stdout）接去哪里？
*   **闭环方案（Scope-out to Client）**：
    *   这必须 是一个 **Client-side reconnect**（客户端重连）逻辑。
    *   就像传统的 ssh 掉线一样 ，Harness 层（或者跑 Master 的那个外部服务）必须自己有监控（比如 Systemd 服务监控这个  `claude` 进程）。
    *   **时序闭环**：ahd OOM -> `BindsTo` 杀掉  ahd, worker, master -> ahd 被 systemd 重启 -> 标 worker 为 CRASHED -> Harness  层检测到 master 死亡，重启 master (`claude --continue`) -> master 启动后，它的第一个动作是连 到 ahd，发起 `session.realign` 检查手下 -> ahd 收到 realign 请求，发现  worker 是 CRASHED，执行 `spawn(is_recovery=true)` -> worker 用 `--continue` 复活。
     *   **结论**：把 Master 重生明确划出本机制范畴，交给上层调用者（Client /Harness）。

### 5. 总结：最合理的单一推荐链路

1.  **ahd 被  OOM 杀 -> Systemd 自动拉起 ahd。**（已有的 `Restart=on-failure `）
2.  **Startup Reconcile 验尸**：所有活跃变死的 Agent 转入 `CRASHED`，**但不  Fail eligible 的 DISPATCHED Job。**（修改 `system.rs` 避开 Fail）
3.  ** 等待上层（Harness）复活 Master**：上层 Harness 检测到 Master 随 ahd 一起死了，重新拉起 Master  进程。
4.  **Master 发起 Realign**：Master 通过 RPC 触发 `session.realign `。
5.  **Recovery Spawn**：ah 看到 `CRASHED` 状态，组装动态  `resume_args`，重新拉起 Worker 进程，连接回旧的 Sandbox Home，并在旧的在 途 Job 上继续监听。

此方案模型上没有新增一张表、一个 Enum，仅仅是通过**状态截 流（不 Fail 任务）**和**控制反转（等上层复活主控）**，在分布式 重启混沌中确立了绝对的时序确定性。
completion_reason: hook_after_agent
completion_confidence: exact
updated_at: 2026-06-12T18:00:28.536718+00:00
