# Inventory: ccbd-rust core-fixes 现状代码地图

本文件由 a3 (Claude) 在 2026-05-08 撰写,纯**事实清单 + file:line 锚点**,不含方案建议。供 a2 (Gemini) 写 `research.md` / `design.md` 时直接 jump-to-source。

不写"应该怎么改",只写"当前是什么、在哪、什么 commit 加的"。

---

## 0. 顶层架构事实 (Updated)

| 事实 | 位置 |
|---|---|
| ccbd 二进制入口 | `src/bin/ccbd.rs:13` `main()` |
| ccb-rust CLI 二进制入口 | `src/bin/ccb-rust.rs:101` `main()` |
| RPC handler 总表 | `src/rpc/router.rs` (350 行) + `src/rpc/handlers.rs` (2058 行) |
| state_dir 解析 (XDG-only,不和 project_root 关联) | `src/env.rs:3-18` `resolve_state_dir()` |
| 共享 tmux session 名称 (legacy) | `src/tmux/mod.rs` (已删除 `SESSION_NAME` 常量,仅 `src/cli/doctor.rs` 保留字符串用于迁移检测) |
| 项目配置文件结构 | `src/cli/config.rs:8-48` `ProjectConfig / MasterConfig / AgentConfig` (LayoutConfig 已删除) |
| 当前 agent state 集合 | `SPAWNING`, `WAITING_FOR_ACK` (新), `BUSY`, `IDLE`, `STUCK`, `UNKNOWN`, `CRASHED`, `KILLED` |
| 状态机转换入口 | `src/db/state_machine.rs` `transit_agent_state_sync` (原子更新状态并插入 state_change 事件) |
| 调度事务入口 | `src/db/jobs.rs` `dispatch_job_to_agent_sync` (原子合并 claim + metadata 写入) |

---

## 1. R1: 进程生命周期追踪 + 物理隔离 (Bug A & F)

### 1.1 当前 tmux 调用模型 (1-Session-per-CLI)

* **Session 命名**: 弃用共享 `ccbd-agents`。Agent 使用 `agent_<id>`, Master 使用 `master_<project_id>`。
* **Server 创建 / Session 锁定**: `src/tmux/session.rs:55-88` `ensure_session_sync` — `tmux new-session -d -s <name> -c <cwd> -x 150 -y 60` 并配合 `set-option window-size manual`。
* **清理路径**:
  * `kill_session_sync` `src/tmux/session.rs` (新): 封装 `tmux kill-session -t <name>`。
  * `cleanup_agent_runtime_resources` (`src/agent_io/registry.rs:50`): 调用 `kill_session_sync`。
* **Layout 移除**: `apply_layout`, `LayoutKind`, `SplitSpec` 已在 commit `0e38fe8` 中全量删除。

### 1.2 物理隔离代码点

| 调用点 | 用法 | file:line |
|---|---|---|
| `ensure_session(agent_session, …)` | agent.spawn 入口 | `src/rpc/handlers.rs:350` |
| `ensure_session(master_session, …)` | master 创建前 | `src/rpc/handlers.rs:222` |
| `kill_session(name)` | daemon shutdown | `src/bin/ccbd.rs:122` |

---

## 2. R2: 状态机防抖 + "双保险" (Bug D & E)

### 2.1 状态机原子性

* **状态转换**: 必须通过 `transit_agent_state_sync`。该函数在同一事务内完成 `UPDATE agents` 和 `INSERT events (state_change)`，解决观测链路断裂。
* **调度原子化**: `dispatch_job_to_agent_sync` 合并了 Job 认领与元数据写入。tmux IO (`send_text_to_pane`) 在事务外部执行。

### 2.2 防抖与 ACK

* **WAITING_FOR_ACK**: 指令发送后立即切入。
* **ACK 落地**: `spawn_new_capture_seed` 轮询间隔降至 50ms。

---

## 3. R3: CWD + 沙盒挂载校准

### 3.1 路径传导

* **absolute_path**: `Session` struct 已扩充 `absolute_path` 字段。
* **CWD 强制**: Master 与 Agent 启动时均强制使用 `session.absolute_path` 作为 tmux `-c` 参数。

---

## 4. R4: ccb.toml 配置传导

### 4.1 B2 配置边界

* **master.enabled**: 控制是否启动 Master 进程。若为 false，则无 Master 监控。
* **auto_shutdown_on_master_exit**: 控制 Master 退出后是否杀 Daemon。属于联动策略，与进程启动解耦。

---

## 5. 备注 — 历史遗留追溯
* **共享 Session 逻辑**: 曾广泛存在于 `src/rpc/handlers.rs` 的 `ensure_session` 和 `spawn_window` 调用中，现已全部通过 `agent_session_name` 和 `master_session_name` 实现了 1v1 物理隔离。
* **Layout RPC**: `session.apply_layout` 及其配套的 `session_window_target` 已彻底移除。
* **状态机转换**: 原有的散落在各处的裸 `update_agent_state_sync` 已由 `transit_agent_state_sync` 统一接管，确保状态与事件的一致性。
