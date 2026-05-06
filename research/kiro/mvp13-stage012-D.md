# Kiro Design: MVP 13 Stage 0-2 (总闸与架构融合)

## 1. 整体架构图

以下是在 tmux 视图与 systemd cgroup 两个维度上的整体架构设计：

```text
==============================================================================
                          [Systemd Cgroup 层级视图]
==============================================================================
ccb-<project_id>.slice                     <-- 顶层项目总闸 (ccb kill --project 的目标)
  │
  ├── ccbd.service                         <-- 守护进程 (Daemon) 本体
  │
  ├── ccbd-agents.slice                    <-- 代理层隔离 slice
  │     ├── ccbd-agent-a1.scope            <-- Codex 1 (BindsTo ccbd.service)
  │     ├── ccbd-agent-a2.scope            <-- Codex 2 (BindsTo ccbd.service)
  │     ├── ccbd-agent-a3.scope            <-- Gemini (BindsTo ccbd.service)
  │     └── ccbd-agent-a4.scope            <-- Claude (BindsTo ccbd.service)
  │
  └── ccbd-workspace.slice                 <-- 用户工作区隔离 slice
        └── claude-master.scope            <-- Master Claude (BindsTo ccbd.service 或孤立存活，视拉起方式定)

==============================================================================
                          [Tmux Pane 布局视图]
==============================================================================
+-------------------------+--------------------------------------------------+
|                         |                                                  |
|                         |                    %1 (a1: codex)                |
|                         |                                                  |
|                         +--------------------------------------------------+
|     %0 (master)         |                                                  |
|     (30% width)         |                    %2 (a2: codex)                |
|                         |                                                  |
|                         +--------------------------------------------------+
|                         |                    %3 (a3: gemini)               |
|                         |                                                  |
|                         +--------------------------------------------------+
|                         |                    %4 (a4: claude)               |
|                         |                                                  |
+-------------------------+--------------------------------------------------+
```
**命令路径**: `ccb-rust` 裸命令执行时：
1. 检查是否存在活跃 Daemon，无则 `systemd-run` 以后台模式启动 `ccbd.service`。
2. 调用 `agent.spawn` 拉起右侧 4 个 Agent。
3. 当前进程将自身包装进 `claude-master.scope` (或者直接拉起 bash / Master Claude 并 Attach Tmux 第一个 Pane)。

---

## 2. 关键决策对比与推荐

### 2.1 Master Claude 怎么进 Tmux？
* **选项 A (Send-keys 启动)**：Daemon 创建 Session 后，向 `%0` 发送 `claude` 命令。
  - *缺点*：丢失了用户的本地 TTY 环境变量，且无法在退出 Master 时干净地清理外层 Host Shell。
* **选项 B (Host Attach 占位)**：用户在宿主机执行 `ccb-rust`，该命令拉起后台 Daemon，配置好右侧 4 个 Pane 后，将自己当前的终端 Attach 到左侧的 `%0` Pane，然后由该前台进程 `exec` 替换为 Master Claude。
  - *推荐理由*：符合用户的直觉体验。用户敲击命令的终端无缝转变为工作台，按 `Ctrl+D` 退出 Master Claude 时触发 `Tmux` Client Detach，随后触发清理链。

### 2.2 Master Claude 怎么进 Cgroup？
* **选项 A (`cgroup.procs` 注入)**：底层 hack，容易与 Systemd 的内部记账冲突。
* **选项 B (`systemd-run --scope`)**：
  - *推荐理由*：在执行步骤 2.1 选项 B 的 `exec claude` 之前，执行类似 `systemd-run --user --scope --slice=ccb-<project_id>.slice -- claude` 的包装。纯正的 Systemd 管理语义。

### 2.3 BindsTo Target (总闸逻辑核心)
* **选项 A (`BindsTo=ccbd-session-<id>.service`)**：MVP11 Round 2 的逻辑。Daemon 死亡不杀 Agent。
* **选项 B (`BindsTo=ccbd.service`)**：回归 MVP1。
  - *推荐理由*：执行用户的**“清理可靠 > 保留状态”**决议。所有 Agent Scope (`ccbd-agent-*.scope`) 统一添加 `--property=BindsTo=ccbd.service`。Daemon 进程一旦 Panic 或 OOM，Systemd 内核级级联停止所有 Agent 进程。

### 2.4 Multi-Codex 支持：独立还是共享？
* **冲突点**：如果不独立，两只 Codex 会竞争写入 `~/.codex/config.toml` 和 `~/.codex/history`。在同时产生 `ask` 时，底层的 CLI session 可能会覆盖上下文。
* **推荐做法**：**强制独立隔离**。必须修改 `src/sandbox/path.rs`，让每个 Agent 的 Sandbox Home 为 `<state_dir>/sandboxes/<session_id>/<agent_id>`。通过不同的挂载点，Codex 1 和 Codex 2 拥有完全独立的假 HOME 目录。

---

## 3. mvp1 契约 (R-DISPATCH-1 / R-RECONCILE-1) 覆盖性 Verify

* **Daemon 死 → Cascade CRASHED (R-DISPATCH-1)**：
  - **覆盖性**：完全覆盖。Systemd `BindsTo=ccbd.service` 保证了物理进程的绝对清除。当 Daemon 下次重启执行 `startup_reconcile` (R-RECONCILE-1) 时，会发现数据库里的 `BUSY` Agent 对应的进程已经不存在，将其收敛为 `CRASHED`，并记录事件。
* **零孤儿退出场景**：
  - **覆盖性**：物理清理机制覆盖极佳。但**盲点存在**：如果在 Daemon `OOM Kill` 瞬间，FIFO 管道文件、`/tmp` Sandbox Home 等文件系统残留依靠内核无法自动回收（因为只杀了进程）。这是必须通过 MVP12 的 M12.5 `reconcile` 阶段在下次启动时打扫战场，或者在顶层提供 `ccb-rs clean` 弥补。

---

## 4. multi-codex 调研要点

当前的 ccbd-rust 实现中，缺乏显式的 Provider 数量上限锁，但潜在的假定 `single-codex` 的雷区如下：

1. **Sandbox 挂载点冲突 (`src/sandbox/bwrap.rs`)**: 
   目前 `resolve_sandbox_dir` 的路径生成规则为 `<state_dir>/sandboxes/<agent_id>`。如果两个 Codex 实例的 `agent_id` 分别是 `a1` 和 `a2`，它们在物理盘上是隔离的。**但是**，在 `manifest.rs` 中，Codex 的 `auth_mount_paths` 包含了宿主机的 `.codex` 目录，并将其 `--ro-bind` 到沙盒内。如果 Codex 内部存在写入行为，会因为 `ro-bind` 报错。如果改为 `bind`，则会发生文件锁竞争。
2. **Tmux Pane 布局冲突 (`src/cli/start.rs`)**: 
   `split_plan_for_layout` 的硬编码仅考虑了 4 个 Agent 的切分（Right, Bottom, Bottom）。如果引入 Master Pane 占据左侧 30%，则原有的 Split Plan 必须重写。
3. **实测检验场景**:
   - `a1` 和 `a2` 同时并行 `ccb-rs ask --wait`，验证 PTY FIFO Reader 线程是否串号。
   - `a1` 先进入 `login` 态（触发信赖弹窗），验证是否阻塞 `a2` 的正常加载。

---

## 5. stage 5 路 B vs mvp12-R 废弃 interactive_prompt_handlers 区分

**你的判断完全正确，这不是同一种机制，也不属于重蹈覆辙。**

* **mvp11 的 `interactive_prompt_handlers`**：这是一种**主动干预启动流**的伪装机制。它试图通过硬编码的延时和字符串扫描，盲目地给 Agent 喂按键，以强行催熟一个本不健康的启动过程。
* **MVP13 Stage 5 路 B (Sandbox Onboarding)**：这是一种**针对特定运行环境缺陷的补救策略 (Environment Polyfill)**。在 Sandbox 模式下，我们故意隔离了 Host 的配置文件（导致 Trust 丢失），所以 CLI 必然会弹出询问。此时，写死的回车或选项输入是对**预期内的隔离副作用进行程序化确认**，这应当作为 Sandbox Materialize 阶段的延伸（相当于在拉起 Daemon 前先“伪造”好一个点过同意的配置文件，只是由于黑盒 CLI 配置无文档，只能通过 UI 交互来完成第一次写入）。
* **实施建议**：
  为避免走偏，这套 Onboarding 脚本绝对不能放在 `InitProbe` (Ready 检测) 的主轮询里。应该单独设立一个 `SandboxOnboarder` 模块，在 `agent.spawn` 的极早期（Agent 首次拉起时）执行一次性拦截。完成 onboarding 后，记录标志位（如写入 `.ccb_onboarded`），以后不再触发。

---

## 6. 关键风险 + 反向契约盲点

1. **Master Claude 的存活拖拽风险**: 
   如果 Master Claude 位于 `ccb-<project_id>.slice` 下，当用户敲击 `ccb-rust start` 然后在终端里跑 Master Claude。如果用户在 Master 内部执行了一段耗时 10 分钟的代码生成，此时另一个终端执行了 `ccb-rs kill` 触发了总闸停止，Master Claude 也会被无情切断，用户可能会丢失大量未保存的 Prompt 状态。**建议在终止总闸前发送 SIGTERM 并给予宽限期**。
2. **Tmux 视图所有权与逃逸**:
   在合并视图后，Tmux Session (`ccbd-agents`) 同时承载了前端交互 (Master) 和后台代理 (a1-a4)。如果用户在 Master Pane 内手动执行 tmux 快捷键 (如 `Ctrl+B %` 切分布局，或 `Ctrl+B x` 杀了某个 Pane)，会直接破坏 ccbd-rust 的 DB 布局映射，导致下一次 `agent.ask` 向错误的 TTY 发送命令。
3. **`BindsTo` 反向回滚带来的重连痛点**:
   强绑定 `ccbd.service` 意味着 `ccbd` 的任何网络抖动或微小 Panic 都会带走所有 Agents（清理可靠）。但这与“长驻后台代理”的初衷部分冲突，开发体验可能会变得频繁中断。必须确保 ccbd-rust 的核心极度稳定，不因单次 RPC 解析失败而 Panic。