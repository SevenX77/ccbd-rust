# Agent 沙箱隔离现状调研 (Agent Sandbox Isolation Research)

## 1. 隔离机制现状

目前 CCB 存在两套并行开发的 `ccbd` 实现，其隔离强度与机制存在显著差异：

### A. ccbd-rust (基于 bwrap)
- **机制**：利用 Linux `bwrap` (Bubblewrap) 创建 Mount Namespace，实现真正的物理隔离。
- **现状**：Agent HOME 被映射到独立的 `/home/agent` 目录，并按 Manifest 声明进行白名单挂载。
- **问题**：当前挂载逻辑过于宽松，将包含 `ccb` 的 `.local/bin` 和包含 `CLAUDE.md` 的 `.claude` 等目录整块挂入，导致物理隔离失效。

### B. Python ccbd (codex-dual)
- **机制**：基于 `HOME` 环境变量重定向及符号链接 (Symlink) 模拟沙箱。
- **现状**：无 Mount Namespace，物理隔离等级较低。依赖 `PATH` 过滤来隐藏主控工具。
- **问题**：无法防御绝对路径调用。Agent 理论上可以访问整个宿主文件系统。

## 2. 核心隔离缺口 (7 Gaps)

经过 a1/a2/a3 三轮审计，识别出以下 7 个关键安全缺口：

### G1. 调度入口泛滥 (Dispatch Entry Explosion)
- **实证**：`~/.local/bin` 中不仅有 `ccb`，还有 `ask`, `autonew`, `ctx-transfer`, `claude-ccb-orchestrator` 等多个可发起调度或控制 Scope 的入口。
- **风险**：仅黑名单屏蔽 `ccb` 无法阻止 Agent 通过 `ask a1 ...` 发起派单。

### G2. Python 侧物理隔离缺失 (Python Sandbox Weakness)
- **实证**：`lib/launcher/sandbox_home.py` 仅进行软链映射。
- **风险**：绝对路径调用 (如 `/home/sevenx/.local/bin/ccb`) 可直接绕过 `PATH` 过滤，物理切断在 Python 侧不成立。

### G3. TMUX 与主控环境泄漏 (Env Leaks)
- **代码位置**：`src/provider/manifest.rs:50-104` (`ENV_PASSTHROUGH`)。
- **风险**：`CCB_TMUX_SOCKET`, `CCB_KEEPER_PID` 等变量被原样透传。Agent 可通过 `tmux send-keys` 跨沙箱注入指令，或获取主控拓扑。

### G4. Rust 侧多余挂载 (Rust Excess Binds)
- **代码位置**：`src/sandbox/bwrap.rs:129-148` (`push_provider_binary_path_binds`)。
- **风险**：该函数不仅挂载了 `.local/bin`，还同步挂载了 `.claude`, `.codex`, `.gemini`, `.claude.json` 到沙箱绝对路径。这导致主控配置文件 (含 `CLAUDE.md`) 对 Agent 物理可读。

### G5. 身份识别缺失鉴权 (Caller ID without Gate)
- **现状**：`CCB_CALLER_ACTOR` 当前仅作为"归属标签"，且 Rust 侧 (`ENV_PASSTHROUGH`) 尚未透传。
- **风险**：缺乏服务端鉴权闸门，无法根据 Caller 身份拦截 Worker 的非授权请求。

### G6. Provider 异构性 (Provider Heterogeneity)
- **实证**：Claude 是独立二进制 (在 `~/.local/share/claude`)；而 Codex/Gemini 是 Node.js 包 (在 `~/.npm-global`)。
- **风险**：统一的"单文件挂载"策略对 Node.js Provider 不适用，它们依赖整个 npm package 目录及 `node` 运行时。

### G7. 权限标识漂移 (Permission Flag Drift)
- **实证**：Python 侧 `command.auto_permission` 是 transient (瞬时) 的。
- **风险**：在项目恢复 (Restore) 或 Socket 调用路径下，Agent 极易丢失免权限 flag，导致自动化中断。

## 3. 调研结论

物理隔离不能仅靠"隐藏"，必须切换到"白名单制"（仅挂载 Provider 及其运行时）。同时，由于 Python 侧物理隔离能力的天然局限，必须引入 **服务端 Caller 身份校验** 作为全量防御闸门。
