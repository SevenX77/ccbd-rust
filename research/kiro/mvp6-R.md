# Kiro Requirements: MVP 6 (拨乱反正 / The Tmux Pivot & CLI Skeleton)

> **文档定位**：本文件是 ccbd-rust 从"纯 L2 守护进程"向"L2+L3 一体化调度治理中枢"演进的首个扩展阶段（MVP 6）官方 R (Requirements) 规格。本阶段是**架构纠偏 MVP**——把 mvp1-5 误用 `portable-pty` 直接管 PTY 的设计回归到 DESIGN.md §1 承诺的"管理挂在 **tmux pane** 里的进程"，并新增 `ccb` 客户端二进制让用户能从命令行接到 daemon。

---

## 0. 立项背景与边界共识

### 0.1 为什么必须做这个 MVP（核心是纠偏）

mvp1-5 实施期间出现了**两个根本性偏离 DESIGN.md 的设计错位**，2026-04-29 一次真实 smoke 测试集中暴露：

1. **Tmux 集成被替换成 portable-pty**：
   - DESIGN.md §1 明确写"L2 调度层管理'**挂在 tmux pane 里的进程**'"——这是核心架构机制
   - mvp1-T.md 决议引入 `portable-pty` crate，此后 mvp1-5 实施全部围绕 portable-pty 展开，**完全没有 tmux 调用**（`grep -rEn 'tmux' src/ Cargo.toml` 0 行返回）
   - 后果：agent 进程被锁在 daemon 内存里，**用户看不到 agent 现场**——彻底丧失旧 ccb 的可观测护城河（旧 ccb 用户随时 `tmux attach` 看 a1/a2/a3 4 个 pane）
   - acceptance 测试用 `mock_agent.sh` 无 PTY 真实交互需求，所以测试全绿但生产语义完全不可用

2. **缺失 `ccb` 客户端二进制**：
   - 旧 Python ccb `lib/cli/services/` 下有 30+ 子命令（`ask` / `pend` / `ping` / `ps` / `kill` / `watch` / `queue` / `doctor` 等），是用户能用的"L3 接入手把手"
   - ccbd-rust 仅有 `ccbd` daemon binary，**没有任何用户能用的命令行工具**——从用户视角看，daemon 跑着但根本不可触达
   - DESIGN.md §1.1 假设 L3 是未来的 Python 仓库——但用户已明确指令"不再保留 Python 调度栈"，CLI 必须随 ccbd-rust 一起出

### 0.2 用户明确指令（驱动本 MVP 的根本要求）

> "我们要做的是替代现在的 ccb 并且比他更好，ccb 有的主要功能至少要一样吧"

也就是 ccbd-rust **不能**只是 L2 daemon——必须把旧 Python ccb 提供给用户的核心能力（4-pane 现场观测 + CLI 命令行工具集 + agent lifecycle 接管）一并实现，直至旧 ccb 可被完全替代退役。

### 0.3 本 MVP **不做**的事（明确 out-of-scope）

为避免 MVP6 摊太大（保持 1-2 天工作量），以下能力**不做**，留给后续 MVP：

- **不修真实 codex/gemini/claude TUI 的 idle marker 检测**——TUI 屏幕中间画 `>_ OpenAI Codex` 这种 prompt 需要新机制（observed completion / session snapshot / structured stream），是 MVP7 范围
- **不做 OAuth/HOME credential 物化**——sandbox 内子进程拿不到宿主已认证 token 是 MVP7 范围
- **不实现 mailbox / queue / async job**——`ccb ask` / `ccb pend` 等高阶命令是 MVP8 范围
- **不做 4-pane 自动布局 / launcher 一键起**——`ccb start` 一行起 daemon + tmux + 4 pane 是 MVP9 范围
- **不动 SQLite schema / 状态机语义**——状态枚举不变，state_change 事件不变
- **不引入新沙盒形态**——bwrap 包裹逻辑保持 mvp2 现状，仅迁移到 tmux 内层调用

### 0.4 与上下游 MVP 的关系

- **依赖 mvp1-5 已完成的全部能力**：SQLite SoT / pidfd 监听 / spawn_blocking 异步硬化 / state machine / evidence loop 等核心机制保留
- **本 MVP 是 MVP7-9 的物理前置**：tmux 集成是 marker 设计 + auth 物化 + mailbox 集成的共同底座
- **R-* 矩阵更新**：R-PTY-1 deprecate；R-TMUX-1 + R-CLI-1 新增

---

## 1. 最小可工作验收标准 (Acceptance Criteria)

MVP 6 验收必须全部通过：

1. **AC1 双 binary 构建**：`Cargo.toml` 拆分为双 target 构建——`bin/ccbd`（daemon，沿用现状）+ `bin/ccb`（新增的客户端二进制）。`cargo build --release` 后 `target/release/` 下应同时出现 `ccbd` 和 `ccb` 两个可执行文件。CLI 工具用 `clap` 驱动，至少支持两个子命令：
    - `ccb ping` — 通过 UDS 调 daemon `system.dump`，stdout 输出含 `ok=true socket=<path>` 行（含 socket 文件路径），exit 0；daemon 不可达时 stderr 红色提示 + exit 1。**uptime / system.ping 是 nice-to-have，本 MVP 不做**——若 D 文档评估实施成本后决定加，可在不改 R 边界的前提下加（向后兼容扩展）
    - `ccb ps` — 调 `system.dump`，把 sessions / agents / evidence_pending 等格式化为终端表格输出（参考旧 ccb `ps.py` 输出风格）+ 底部 hint 行展示 tmux attach 命令

2. **AC2 portable-pty 完全废弃**：`Cargo.toml` 内 `portable-pty` 依赖**移除**；`src/pty/` 模块下的 `portable_pty::` 调用**全部删除**。判定脚本：

    ```bash
    grep -nE 'portable-pty|portable_pty' Cargo.toml src/ tests/
    # 预期：0 行返回
    ```

3. **AC3 tmux 接管 spawn 物理拓扑**：`agent.spawn` 的内部实现切换为通过 `std::process::Command` 调用 `tmux` 二进制：
    - 在 daemon 启动时（或首次 spawn 时）创建一个 detached tmux session（命名约定 `ccbd-<daemon_pid>` 或 `ccbd-<state_dir_hash>`）
    - 每次 `agent.spawn` 通过 `tmux new-window -d` 或 `respawn-pane` 在该 session 内分配一个 pane，pane 的命令是 `bwrap [...] -- <agent_command>`（保留 mvp2 沙盒包裹）
    - daemon 通过 `tmux display-message -p '#{pane_pid}'` 拿到 agent 的真实 PID，注册到 pidfd watcher
    - daemon 通过 `tmux pipe-pane -O 'cat'` 把 pane 输出 pipe 到一个 fd / 命名管道，由 reader task 持续 drain 进 vt100 parser 触发 `output_chunk` 事件

4. **AC4 用户可观测后门**：spawn 完成后，用户在终端跑 `tmux -L ccbd-<id> attach -t <session_name>`（具体命令由 daemon 启动日志打印 / `ccb ps` 输出展示）能看到 mock_agent 真实运行画面——光标移动、stdin 输入回显、stdout 字节流。这是验证"agent 不再黑盒"的硬指标。

5. **AC5 Lifecycle 保真度**：在 AC4 attach 的 tmux pane 中，用户：
    - 按 `Ctrl+C` 或敲 `exit`（视 mock_agent 行为）杀 agent → daemon 的 pidfd watcher 仍能捕获退出 → agent state 流转至 CRASHED/KILLED → state_change event 写入 events 表
    - mvp1-5 的所有 lifecycle 路径（`agent.kill` / `agent.send` 幂等回放 / `agent.read since=N` polling / SIGKILL→CRASHED / master_death cascade）保持语义不变

6. **AC6 测试矩阵全绿**：现有 `cargo test --quiet` 105+ 测试（mvp2/3/4 acceptance 含 mock_agent + lib unit）必须**重新跑通**——portable-pty 切到 tmux 后，acceptance 测试需相应改造（mock_agent.sh 启动方式变 / PTY 读取改 tmux pipe 读取），但**业务断言不允许放宽**：所有原 AC 行为（state 转移、events 顺序、payload 内容）必须保留。新增 mvp6_acceptance.rs 覆盖 AC2-AC5 的关键路径。

7. **AC7 `ccb ping` / `ccb ps` 端到端 smoke**：手动测试通过——daemon 启动后跑 `ccb ping`，0 秒内返回 `{ok: true, socket: ..., uptime_secs: N}`；跑 `ccb ps`，输出含当前 sessions / agents 的格式化表格。错误路径（daemon 没起）`ccb ping` 应返回明确 err 信息（不 panic、不 hang）。

---

## 2. 状态机激活范围 (Delta)

**核心状态机不变**——SPAWNING → IDLE/BUSY → UNKNOWN → CRASHED/KILLED 转移规则、CAS 协议、evidence 闭环全保留。

**变更的仅是物理拓扑**：
- SPAWNING 阶段：从"daemon fork + portable-pty"变为"daemon → tmux new-window → bwrap → agent"
- output_chunk 来源：从"portable-pty master fd 直接 read"变为"tmux pipe-pane 重定向后 read"
- agent PID 获取：从"Command spawn 后 child.id()"变为"tmux display-message #{pane_pid}"

---

## 3. R-* 需求切割矩阵更新 (Scope Definitions)

### R-PTY-1：Daemon 直接分配 PTY
*   **状态**：🔴 **DEPRECATE**（之前 🟢 Full）
*   **理由**：portable-pty 直接 fork PTY 的方案被废弃。tmux 内部已自带 PTY 管理，daemon 无需重复持有。

### R-TMUX-1：Tmux Pane 托管（MVP6 新增）
*   **状态**：🟡 **Partial**
*   **定义**：所有 agent 进程必须运行在 daemon 控制的 tmux pane 内（detached session）。daemon 通过 tmux CLI 完成 spawn / kill / pipe / capture 控制。
*   **本期范围**：仅做"detached 后台托管 + pipe-pane 读流"。多窗口复杂 UI 布局（4-pane 排版 / 状态行 / window naming）留 MVP9。

### R-CLI-1：客户端骨架（MVP6 新增）
*   **状态**：🟢 **Full（骨架级）**
*   **定义**：新增独立 `ccb` 二进制，通过 UDS 连 daemon，至少支持 `ping` / `ps`。后续高阶命令（`ask` / `pend` / `watch` / `kill` / `queue`）留 MVP8。

### R-OBSERVABILITY-1：状态全量可观测
*   **状态**：从 🟢 In-scope（mvp4）升级为"全维度可观测"
*   **MVP6 新增维度**：用户可手动 `tmux attach` 物理观察 agent 现场（不仅靠 events 表的逻辑流）

### 其他 R-* 矩阵
本 MVP **不动**：R-DISPATCH / R-ISOLATION / R-RECONCILE / R-API-COMPAT / R-RECONNECT / R-IDEMPOTENCY / R-ERROR-CODES / R-STATE-FALLBACK-LOOP / R-RUNTIME-1 / R-MODULARITY-1 / R-PTY-EXEMPT-1（注：mvp5 PTY exempt 在 MVP6 后自然失效，因为不再有"sync DB 调用在 spawn_blocking 闭包内的合法例外"——tmux 模式下 reader 改 tokio::AsyncRead pipe 直接 async）

---

## 4. 范围分阶段（实施视角）

为降低 MVP6 一次性 PR 风险，建议分三个子阶段推进，每阶段独立可回滚：

| 子阶段 | 内容 | 安全检查点 |
|---|---|---|
| **G6.0：CLI 骨架先出**（半天）| 加 `bin/ccb.rs` 二进制 + clap dispatch + UDS client + `ping` / `ps` 子命令；daemon 不动 | `ccb ping` / `ccb ps` 跑通；`cargo test` 全绿（不影响 daemon 测试）|
| **G6.1：Tmux 包装层**（半天）| Rust 写一套强类型 tmux wrapper（`new_session` / `new_window` / `respawn_pane` / `display_message #{pane_pid}` / `pipe_pane` / `kill_pane`），独立模块 `src/tmux/`，单元测试覆盖每个命令构造 | `cargo test tmux::tests` 全绿；wrapper 调真 tmux 二进制能成功创建 + 销毁 detached session |
| **G6.2：Spawn 外科手术替换**（半到一天）| 切除 `src/pty/` 内 portable-pty 调用 + 改 `handle_agent_spawn` 走 tmux wrapper + 改 reader task 走 pipe-pane；mvp2/3/4 acceptance 改造 | `cargo test` 全绿（含 mvp6_acceptance）；AC4 手动 attach 验证 |

子阶段失败可独立回滚：G6.0 失败仅删 `bin/ccb.rs`；G6.1 失败仅删 `src/tmux/`；G6.2 失败回退到 G6.1 末态。

---

## 5. 非验收点（永不做或后期 MVP）

- ❌ codex/claude/gemini TUI 真 idle 检测（→ MVP7）
- ❌ OAuth token / HOME mount 物化（→ MVP7）
- ❌ `ccb ask` / `ccb pend` / `ccb watch` 高阶命令（→ MVP8）
- ❌ mailbox / queue / async job 系统（→ MVP8）
- ❌ `ccb start` launcher / 4-pane 自动布局（→ MVP9）
- ❌ 多 master 协调 / per-project ccbd 实例隔离（保留 mvp1-5 现状）
- ❌ Web UI / dashboard（永不做）

---

## 6. 跟前后 MVP 的接口约束

- **JSON-RPC schema 不允许破坏**：所有 mvp1-5 已实现的 RPC 方法（`session.create` / `agent.spawn` / `agent.send` / `agent.read` / `agent.kill` / `agent.assert_state` / `agent.discard_evidence` / `system.dump`）请求/响应字段保持兼容。MVP6 仅可能新增 `system.ping`（如设计需要），不动既有方法签名
- **SQLite schema 不允许破坏**：sessions / agents / events / evidence 四张表字段保持。新增列必须 nullable 不破坏 mvp1-5 INSERT 语句
- **mvp5 spawn_blocking 事务边界保留**：所有 8 条事务路径（D §3.5）的单 spawn_blocking 闭包内单 transaction 边界不允许拆分
- **错误码兼容**：现有 `error_code` 集合保留；MVP6 仅可能新增 `TMUX_NOT_FOUND` / `TMUX_COMMAND_FAILED` 等本期相关错误码

---

## 7. 验收脚本（H 类辅助）

D 文档提供具体的 grep 命令模板和实测 cargo test 输出格式。R 文档只规定**判定标准**，不规定**判定脚本**。
