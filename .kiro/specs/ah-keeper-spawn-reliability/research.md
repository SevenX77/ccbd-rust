# Research: ah Keeper/Spawn Reliability Status (Task #14)

本文档通过源码实证与架构分析，对 `ah` (ccbd-rust) 当前的守护进程管理与启动可靠性现状进行了评估。在 Round 2 修正中，撤回了关于 `BindsTo` 与 `SQLite` 替代能力的误判，并识别出了单例守卫破坏等致命漏洞。

## 1. 当前 ah 实施现状实证

### 1.1 "Keeper" 概念缺失与单例风险
实证发现 `ah` 架构在“守护进程唯一性”与“自我拉起”上存在结构性缺陷。
- **单例守卫被破坏 (must-fix #3)**: 
    - 实证 `src/rpc/mod.rs:23-24` 执行了 `unlink-before-bind` 逻辑（`if socket_path.exists() { remove_file }`）。
    - **风险**: 第二个 `ccbd` 启动会强行删除并抢占第一个进程的 Socket，导致多个 `ccbd` 并存且旧进程成为无法通过 RPC 控制的孤儿。
- **无自愈能力**: `grep` 未发现任何应用层的 `spawn_loop` 或重启逻辑。

### 1.2 启动调解 (Startup Reconcile) 范围
- **实现位置**: `src/db/system.rs:394-472` (`reconcile_active_agents_to_crashed_sync`)。
- **行为**: 在冷启动时通过 PID 探测将死掉的 Agent 转为 `CRASHED`，并执行 `remove_agent_sandbox_dir_sync` 清理物理残余。
- **局限性**: 仅在启动瞬间生效，不具备运行时的 Lease 续租或 Fencing 能力。

### 1.3 物理绑定 (PR3 systemd BindsTo) 真实边界 (must-fix #1)
- **非决策权威**: `BindsTo` 仅是 **反向级联清理** 机制（Daemon 死则子进程陪葬），不解决“是否应该 Spawn”的决策冲突。
- **条件性生效**: `src/sandbox/systemd.rs:17-29,69-74` 证实该特性依赖于 `daemon_unit` 存在。在 `unsafe_no_sandbox` 或非 systemd 环境下，该防护完全失效。

---

## 2. 行为契约 (R1-R6) 满足度评估

| 需求 ID | 描述 | 现状 | 实证证据 |
| :--- | :--- | :--- | :--- |
| **R1** | **Lease 查权威** | ❌ **失败** | 单例守卫被 `unlink-before-bind` 破坏。无运行时 Lease。 |
| **R2** | **指数退避** | ❌ **未实现** | Agent 崩溃后无拉起，L3 触发的 spawn 无退避逻辑。 |
| **R3** | **熔断+告警** | ❌ **未实现** | 全仓无连续失败计数器，无法阻断 Thrashing。 |
| **R4** | **启动超时回收** | 🟡 **偏乐观** | `init_probe_task.rs:296` 仅改 DB 状态，**不杀** 物理进程。 |
| **R5** | **Ownership 信号** | ❌ **失败** | 没识别冲突作为正向信号，反而被 Socket 抢占逻辑掩盖。 |
| **R6** | **全程可观测** | 🟡 **部分** | 缺失退避、熔断与强制回收的审计事件。 |

---

## 3. 现有 e2e 测试覆盖

- **已覆盖**: `mvp12_r2_dispatcher_lifecycle.rs` 验证了 IDLE 匹配后的正常路径。
- **0 覆盖 (Gap)**:
    - **Thrashing (P0)**: L3 疯狂 spawn 崩溃 Agent 的压力测试。
    - **Double Daemon (P0)**: 启动两个 `ccbd` 抢占 Socket 的冲突测试。
    - **Hung Subprocess (P0)**: readiness 超时后物理进程存活的泄漏测试。

---

## 4. 评估结论: Task #14 必须实施 (必须补强集)

**建议结论: 必须实施 (按 Requirement §6 评级为 must-fix)**

现有架构未能消除 Keeper Spawn Loop 风险，甚至在 Socket 单例守卫上存在退化。为防止生产环境下 OOMD 或 CPU 爆表，必须闭环以下补强。

### 推荐实施路径 (优先级表)：
1.  **[P0] Fix Unlink-before-bind**: 移除 `rpc/mod.rs` 的抢占逻辑，改为 `bind` 失败报错，保护单例。
2.  **[P0] 增加应用层熔断 (Circuit Breaker)**: 在 `agents` 表增加 `consecutive_crash_count`，连续失败 3 次即阻断。
3.  **[P0] 实施物理回收 (Hard Kill)**: 在 `init_probe_task.rs` 超时后，复用/提取 `handle_agent_kill` 路径的物理 kill+cleanup 原语 (src/rpc/handlers.rs:1092)，确保子进程被真实杀除。
4.  **[P0] 指数退避 (Backoff)**: 在 `ah.toml` 增加 `retry_policy`，由 `ccbd` 强制执行 Spawn 冷却。
5.  **[P1] R5 Conflict-as-Signal**: 将 Socket 冲突或 PID 冲突作为正向信号抛回 L3。
