# Research: PR8 - Python ccb 替换可行性与切换设计

本文档评估了 `ccbd-rust` 替换生产环境 Python `ccb` 的可行性，并设计了非破坏性的按项目切换机制。

## 1. 替换可行性评估

### 1.1 命令 Parity (行为对照)
`ccbd-rust` 目前实现了 18 个核心指令/子指令，对比 Python `ccb` 的功能对比如下：

| 命令 | Rust 状态 | 行为差异/缺口 |
|---|---|---|
| `ping` | ✅ Parity | 输出格式略有不同，但逻辑一致。 |
| `ps` | ✅ Parity | Rust 版表格渲染更清晰，支持监控状态显示。 |
| `start` | ✅ Parity | 已支持多 Agent 并发启动及稳定态探测。 |
| `ask` | ⚠️ 弱 Parity | 缺少对 `stdin` 管道输入的自动检测（Python 版支持 `echo "fix" | ccb ask a1`）。 |
| `pend` | ✅ Parity | 逻辑对齐。 |
| `cancel` | ✅ Parity | 逻辑对齐。 |
| `kill` | ✅ Parity | 支持 `--session` 级联清理。 |
| `watch` | ✅ Parity | 支持从特定 `event_id` 开始流式输出。 |
| `logs` | ✅ Parity | 逻辑对齐。 |
| `attach` | ✅ Parity | Rust 版为显式命令，Python 版部分逻辑隐藏在 `start` 后。 |
| `stop` | ✅ Parity | Rust 版特有的优雅停机指令。 |
| `doctor` | ⚠️ 弱 Parity | Rust 版偏向环境自检，Python 版包含诊断包导出。 |
| `config` | ✅ Parity | 支持 `validate`。Rust 独有 `migrate` 引导。 |
| `prompt` | ✅ Parity | 对应 Python 的 `ack` 指令，Rust 版支持三层级联（JSON/DB/LLM）。 |
| `version`| ✅ Parity | 逻辑对齐。 |

**核心缺口 (Missing in Rust)**:
*   `trace`: 追踪任务流向（SCS 强依赖，但在替换 Python ccb 路径上优先级中等）。
*   `resubmit` / `retry`: 任务重试机制。
*   `inbox`: 待处理任务收件箱。
*   `fault`: 故障注入测试工具。

### 1.2 配置兼容性
*   **Python 版**: 使用 `.ccb/ccb.config` (自定义格式)。
*   **Rust 版**: 使用 `ccb.toml` (TOML 格式)。
*   **结论**: 二者文件名不同，可以**在同一项目目录下并存**，互不干扰。这为渐进式替换提供了天然的基础。

### 1.3 Dogfooding 验证 (PR6b)
*   **已验证**: 4-agent (codex/gemini/claude) 混合拓扑、真 LLM 交互、WAITING_FOR_ACK 状态机链路、Tmux 1-Session-per-Agent 物理隔离。
*   **结论**: 核心链路已达到生产替换的稳定性门槛。

---

## 2. 按项目非破坏切换机制设计

### 2.1 路由策略 (CLI Dispatcher)
设计一个轻量级的 `ccb` 包装器（可以是 Shell 脚本或 Rust 编译的通用 Binary）：
1.  **探测**: 检查当前目录及上级目录是否存在 `ccb.toml`。
2.  **分流**:
    *   若存在 `ccb.toml` -> 调用 `ccb-rust`。
    *   若仅存在 `.ccb/ccb.config` -> 调用原始 Python `ccb`。
3.  **降级**: 默认调用 Python `ccb` 以保证存量项目不被破坏。

### 2.2 Daemon 共存性
*   **Socket 路径**:
    *   Python 版: `.ccb/ccbd/ccbd.sock` 或 `/tmp/tmux-<uid>/ccbd-<12位hash>.sock`。
    *   Rust 版: `~/.local/state/ccb-rs/<8位hash>/ccbd.sock`。
*   **Tmux Session**:
    *   Python 版: `ccb-<slug>`。
    *   Rust 版: `agent_<id>`。
*   **结论**: 二者在套接字命名和 Tmux 会话命名上均有显著差异，**支持在同一台机器上同时运行两个版本的 Daemon**。

---

## 3. 结论
`ccbd-rust` 已具备替换生产 Python `ccb` 的核心能力。通过 `ccb.toml` 作为“开关文件”，可以实现按项目的渐进式无缝迁移，风险极低。
