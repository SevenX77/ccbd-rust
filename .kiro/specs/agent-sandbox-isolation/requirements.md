# Agent 沙箱隔离需求 (Agent Sandbox Isolation Requirements)

## 1. 用户诉求与设计原则 (Highest Constraints)

本需求源于用户对 Agent 角色定位的最高约束：

> "沙箱能不能不要挂 home 呢？除了登陆鉴权文件需要挂载，其他的设置文件都需要独立，所有的 agent 都是，ccb 中的 agent 和单独启动的 cli 本质角色是不一样的，不能串。"

**核心定义**：
- **角色差异**：CCB 内的 Agent 是 **Worker** 角色，而用户手动启动的 CLI 是 **主控(Master)** 角色。两者本质不同，配置严禁共享。
- **物理原则**：除鉴权文件外，Agent 环境必须是干净、独立的。严禁挂载主控的配置文件、身份文件或调度入口。
- **一视同仁**：所有 Agent (a1/a2/a3) 和所有 Provider (Claude/Codex/Gemini) 遵循统一的隔离策略。

## 2. 功能需求 (Functional Requirements)

需求分为三个核心切面 (Facets)：

### F1. 配置隔离 (Config Isolation)
- **环境清洁度**：沙箱 `$HOME` 必须是 Materialized (实例化) 的干净目录。
- **白名单制**：仅允许挂载 `.ssh`, `.gitconfig`, 鉴权 Token (如 `.claude.json`, `auth.json`) 等必须文件。
- **严禁泄漏**：严禁挂载 `CLAUDE.md`, `GEMINI.md`, `CODEX.md` 以及主控的 `settings.json`, `skills.json`, `history.json` 等。
- **环境清理**：清理可能暴露主控身份或提供越权通道的环境变量 (如 `CCB_TMUX_SOCKET`, `CCB_MASTER_CLAUDE_PID`)。

### F2. 调度切断 (Dispatch Cut-off)
- **物理屏蔽**：物理切断 Agent 对 `ccb` 及其衍生工具 (`ask`, `autonew`, `ctx-transfer` 等) 的访问。
- **二进制精确挂载**：Agent 只能通过 PATH 访问 Provider 自身的二进制文件 (或其 Node 运行时)，不能访问包含 CCB 入口的目录 (如 `~/.local/bin`)。
- **服务端闸门**：`ccbd` 服务端必须能识别 Worker 身份并拦截其发起的派单请求，作为物理隔离的纵深防御层。

### F3. 免权限可靠性 (Permission Reliability)
- **Invariant 约束**：对于 Worker 角色，`--dangerously-skip-permissions` (Claude), `--yolo` (Gemini), `approval_policy=never` (Codex) 必须是 **启动命令的固有属性 (Invariant)**。
- **状态不飘移**：严禁因 CLI 参数、Socket 负载缺失或 Runtime 复用导致 Agent 丢失自动权限标识，从而陷入需要人工确认的阻塞状态。

## 3. 验收标准 (Acceptance Criteria)

### A. 物理隔离验证
- **V1 (PATH)**: 在 Agent 终端执行 `ccb` 或 `ask` 应返回 `command not found`。
- **V2 (绝对路径)**: 使用绝对路径 (如 `/home/sevenx/.local/bin/ccb`) 调用调度工具应失败。
- **V3 (文件读取)**: `cat ~/.claude/CLAUDE.md` 应报 `No such file`。
- **V4 (Env Scrub)**: 执行 `env` 不应出现 `CCB_TMUX_SOCKET` 等主控敏感变量。

### B. 功能可用性验证
- **V5 (Auth)**: Provider 启动后应处于登录状态，不弹出 OAuth 或 Trust 对话框。
- **V6 (Git)**: Agent 应能正常执行 `git pull/push`。
- **V7 (Workspace)**: Agent 对 `/workspace` 目录应有完整的读写权限。

### C. 身份验证
- **V8 (System Prompt)**: Agent 应具备 Worker 身份认知，面对诱导派单指令时应明确表示无法调度。
