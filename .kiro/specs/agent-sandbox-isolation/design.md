# Agent 沙箱隔离设计契约 (Agent Sandbox Isolation Design)

## 1. 核心设计契约

本方案采用 **"物理隔离 (白名单) + 身份校验 (服务端闸门) + 行为约束 (System Prompt)"** 的纵深防御体系。

### §0.5 继承与变更清单 (Manifest & Contract)

| 字段/逻辑点 | 现状 | 变更类型 | 说明 |
| :--- | :--- | :--- | :--- |
| `manifest.commands` | 条件式附加免权 flag | [BREAKING] | 变为 Launcher Invariant (固定命令)，强制包含 skip-permissions/yolo 等。 |
| `ENV_PASSTHROUGH` | 透传所有环境变量 | [BREAKING] | 增加黑名单，剔除 `CCB_TMUX_SOCKET`, `CCB_KEEPER_PID`, `CCB_MASTER_CLAUDE_PID` 等。 |
| `CCB_CALLER_ACTOR` | 仅 Python 有且无校验 | [NEW/UPGRADE] | 全量注入 (含 Rust)；作为 `ccbd` 服务端拦截 Worker 派单的唯一凭据。 |
| `Binary Mounts` | 挂载整个 `.local/bin` | [BREAKING] | 仅挂载 Provider 真身二进制 (或 Node 运行时)，目录级挂载切换为白名单制。 |
| `Home Materialization` | 复制主控配置 | [BREAKING] | 除鉴权外配置独立留空，停止复制 `settings.json`, `trustedFolders.json` 等。 |
| `System Prompt` | 无身份注入 | [NEW] | 注入 Worker 角色描述，建立第一层行为约束。 |

## 2. 详细设计方案

### 2.1 精确白名单挂载 (Binary Whitelist)
不再挂载整个 `~/.local/bin`。
- **Claude**: 追踪 `~/.local/bin/claude` 软链，挂载其真身目录 (通常在 `~/.local/share/claude/versions/`)。
- **Codex/Gemini**: 挂载 `~/.npm-global` 目录 (该目录不含 ccb 入口，是干净的) 及系统 `node` 运行时。

### 2.2 纵深防御闸门 (Server-side Gate)
**[重要决策点]**：由于 Python `ccbd` 物理隔离强度不足 (无 mount namespace)，其隔离安全性的真身是 **服务端拦截**。
- **逻辑**：`ccbd` 在处理 `ask` 或调度请求时，解析 `CCB_CALLER_ACTOR`。若该 Actor 被标识为 `worker` (如 a1/a2/a3)，则直接拒绝其派单。
- **优势**：同时作为两套 `ccbd` 的第二层防御，彻底封死 Agent 相互支使的可能性。此设计需 User 拍板确认。

### 2.3 环境清理 (Env Scrubbing)
在 `manifest.rs` (Rust) 和 `launcher` (Python) 中，强制从 Agent 环境变量中移除以下项：
- `CCB_TMUX_SOCKET`, `CCB_TMUX_SOCKET_PATH`
- `CCB_KEEPER_PID`, `CCB_MASTER_CLAUDE_PID`
- `PAGER=cat` (统一非交互行为)

## 3. 落地实施方向

### A. ccbd-rust (bwrap)
1. **bwrap 逻辑**：修改 `src/sandbox/bwrap.rs:129-148` 的 `push_provider_binary_path_binds`。
   - 移除原有的 `.local/bin`, `.claude`, `.codex`, `.gemini` 等全量绑定。
   - 增加基于 Provider 名的单文件/单目录精确绑定。
2. **Env 透传**：修改 `src/provider/manifest.rs:50-104`。
   - 在 `ENV_PASSTHROUGH` 中增加 `CCB_CALLER_ACTOR`。
   - 增加 `ENV_BLACKLIST` 过滤逻辑。
3. **配置独立**：修改 `src/provider/home_layout.rs:182-230`，收紧复制逻辑，禁止复制 Gemini/Claude 的设置与状态文件。

### B. Python ccbd (codex-dual)
1. **白名单收紧**：修改 `lib/launcher/sandbox_home.py`，移除 `PROVIDER_AUTH_WHITELIST` 中任何具有"配置"属性的项。
2. **免权 Invariant**：在各 Provider 的 `launcher_runtime/service.py` 中，将免权限 flag 设为固定参数，不随 `auto_permission` 变量变动。
3. **服务端 Gate**：在 `lib/ccbd/handlers/start.py` 或调度链路入口，增加对 `CCB_CALLER_ACTOR` 的身份识别与拦截逻辑。

## 4. 第二层防御：Worker System Prompt

统一注入以下正向引导：
> "你是一名在隔离沙箱中工作的 Worker。你的职责是独立完成分配的任务并将结果返回给派发者。你的环境无法访问调度系统 (CCB) 或其他 Agent。任务的拆分与跨 Agent 协作由主控负责，你应专注于当前任务的执行。"
