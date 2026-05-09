# Requirements Document

## Introduction

**Feature/Initiative**: `ccbd-rust` Core Fixes (Daemon Stabilization Phase)

**Background**: `ccbd-rust` 被定位为连接 L3 代理人与底层的 Agent Hypervisor，负责在终端（tmux）内安全、稳定地拉起和管理原生大模型 CLI（Claude Code / Codex 等）。然而，在初步重写后，系统暴露出 7+1 个致命的物理级 Bug，导致主控经常假死、状态机错乱，以及沙盒和执行目录彻底错位。

**本次 Spec 旨在**：不改变“保留 VT100 双保险”和“强沙盒隔离”的既定架构，彻底根除生命周期追踪、终端读写竞争、以及工作目录飘逸的物理级 Bug。

## Requirements

### Requirement 1: 绝对可靠的进程生命周期追踪与物理隔离 (Fix Bug A & F)
**Status**: Implemented. 已完成从共享 Session 到 `agent_<id>` / `master_<project_id>` 的 1v1 隔离转换。

**Objective:** 彻底抛弃旧的“单 Session 多 Pane”拥挤模式，转向“One Tmux Session per CLI”的 1v1 纯后台隔离架构。这能从物理上杜绝串台、焦点丢失、以及被外部设备 attach 时的 PTY 尺寸干扰（导致标志位被硬换行截断）。

#### Acceptance Criteria
1. **独立后台 Session**: When 拉起新的 Agent CLI 时，the system shall 使用 `tmux new-session -d -s agent_<UUID>` 创建一个独立的后台 Session，而不是在现有的 Session 中切分 Pane。
2. **纯净清理**: When 任务完成或异常中断，the system shall 仅需执行 `tmux kill-session -t agent_<UUID>` 即可将该 CLI 及其衍生进程干净利落地销毁，不再需要复杂的 `waitid` 捕捉。
3. **尺寸锁定防干扰**: The system shall 在创建后台 Session 时，利用 tmux 特性锁定其内部 PTY 尺寸（如 150 宽），确保底层 VT100 解析器读取的字符流永远不会被硬换行截断。

### Requirement 2: 状态机防抖与“双保险”协同 (Fix Bug D & E)
**Status**: Implemented. 已引入 `WAITING_FOR_ACK` 状态，并实现了 `transit_agent_state_sync` 原子事务入口。

**Objective:** 解决刚发出指令后，`MarkerMatcher` 瞬间匹配到残留的终端标志位，导致状态机刚开始就立刻结束的竞态条件。

#### Acceptance Criteria
1. **引入等待确认态 (WAITING_FOR_ACK)**: When 主控发送指令后，The system shall 立刻将状态机切换为 `WAITING_FOR_ACK` 或类似的防抖态。
2. **视觉变化强校验**: In `WAITING_FOR_ACK` state, the system shall 要求 `PaneDiffWatcher` 必须检测到终端画面发生实质性更新（或设置强制的最小时间窗口，如 500ms），证明 LLM 已经开始响应。
3. **安全恢复匹配**: Only after 确认终端已经发生滚动或更新后，the system shall 才允许 `MarkerMatcher` 重新开始捕获“完成标志符”（如 `=== DONE ===`）。

### Requirement 3: 工作目录 (CWD) 与沙盒路径的绝对校准 (Fix Master CWD Bug, Bug C & G)
**Status**: Implemented. `absolute_path` 传导链已打通。

**Objective:** 彻底解决目前 `ccbd-rust` 启动后，无论是主控还是沙盒 Agent，统统飘逸到用户的根目录 (`~`) 或系统 state 目录的致命问题，确保它们永远运行在目标项目的根目录。

#### Acceptance Criteria
1. **主控 CWD 修正**: When `ccbd-rust` 启动并为 Master 准备环境时，the system shall 必须将 Master 进程（通常是跑着 Remote Control 的 Claude Code）的 CWD 精确指向目标 Project Root，**绝对不能是 `~`**。
2. **独立 Session CWD 传递**: When 调用 tmux 派生新 Session 时，the system shall 在 `tmux new-session -c <DIR>` 参数中，明确传入目标工程目录。
3. **bwrap 沙盒挂载修正 (Bug C)**: When 启用沙盒模式拉起 Agent 时，The system shall 确保 bwrap 的 `--bind <真实路径> /workspace` 参数中，传入的是确切的 Project Root，保证 Agent 在沙盒内有正确的文件操作权限。

### Requirement 4: CLI 启动命令与参数的完整透传 (Fix Bug B)
**Status**: Implemented. `sh -lc` 透传机制已固化。

**Objective:** 解决 `ccb.toml` 配置文件中对于 Master 的启动命令解析不够灵活的问题。

#### Acceptance Criteria
1. **多参数展开**: When 读取配置文件中的 `[master] cmd` 时，The system shall 能够正确解析并传递带参命令。例如将 `"claude"` 自动展开/支持 `"claude --dangerously-skip-permissions --continue /remote-control"`，确保 Master 在远程模式下无需手动确认权限即可自启动。
