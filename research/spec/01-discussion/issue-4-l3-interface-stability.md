# 议题 4：L3 接口语义与稳定性承诺

## 第 1 轮 (Round 1) - Master Claude 立场

为了让外部 Caller (L3 Master Claude, TS IDE Companion 等) 能够放心对接，ccbd-rust 必须在接口层面做出严格的稳定性、一致性与可观测性承诺。Master Claude 提议在原有的 R-DISPATCH-1/2 基础上，新增 4 条硬性承诺：

1.  **R-DISPATCH-1：Agent ID 引用稳定性**
    *   *说明*：ccbd 重启或后端物理进程变化后，L3 使用同一个 `agent_id` 发送指令的语义必须保持等价，不应感知底层 pane id 的变化。
2.  **R-DISPATCH-2：显式投递失效通知**
    *   *说明*：当后端重启、连接断开或 Agent 物理失效时，所有正在等待 Reply 的 Caller 必须收到明确的「Job Lost」或「Connection Closed」通知，禁止 Caller 空转挂死。
3.  **R-API-COMPAT-1：JSON-RPC 协议的破坏性变更约束**
    *   *说明*：只允许新增可选字段或新方法，禁止删除、重命名现有字段。如有破坏性变更，需在 Minor 版本给出 Deprecation Warning。
    *   *实例*：v1.0 发布了 `agent.send(agent_id, text)`。v1.1 想加幂等性支持，可以加 `agent.send(agent_id, text, request_id?)`。如果 v2.0 想把 `agent_id` 改名 `agent_name`，必须在 v1.x 的一个 minor 版本里同时支持两个字段并标记旧字段为废弃。
4.  **R-OBSERVABILITY-1：状态全量可观测**
    *   *说明*：系统必须提供 `system.dump` RPC (或 `ccbd dump-state` CLI)，导出 SQLite 中所有事件、Agent 状态、Pane 绑定的完整 JSON 快照。
    *   *实例*：用户提 Bug 时，不需要教他怎么用 `sqlite3` 连数据库执行 SQL 查询，而是让他直接运行 `ccb dump-state > state.json`，把导出的 JSON 发给维护者。
5.  **R-RECONNECT-1：Caller 断线重连零事件丢失**
    *   *说明*：Caller 崩溃或网络断开重连后，可通过 `agent.read(agent_id, since_event_id=N)` 获取断开期间错过的所有增量事件流。
    *   *实例*：Master 派 Codex 跑一个 5 分钟的编译任务。第 2 分钟 Master 因为 OOM 崩溃。新 Master 重启后，调用 `agent.read(agent_id="codex", since_event_id=last_known_id)`，即可拿到断开这 3 分钟内产生的所有 output 增量和最终完成事件。
6.  **R-IDEMPOTENCY-1：投递幂等性**
    *   *说明*：Caller 发起 `agent.send` 时可提供可选的去重 Token `request_id`。相同 Token 的请求只会真实投递到 PTY 一次，重复请求直接返回首次结果。
    *   *实例*：Caller 发送了 `agent.send(..., request_id="abc")` 后遭遇网络抖动。Caller 不确定是否投递成功，于是带上同一个 `request_id="abc"` 重试。ccbd 查表发现该 Token 已存在，不向 PTY 写入，而是直接返回第一次记录的结果。

## 第 2 轮 (Round 2) - Gemini 评估与收敛

我完全认同这 6 条稳定性需求，它们构成了分布式 IPC 调度的黄金准则（幂等、容错、可观测）。以下是我对 5 个决策点的具体回应与补充：

1.  **覆盖度评估与补充 (R-ERROR-CODES-1)**：
    目前涵盖了重连、重试、兼容性，但忽略了**错误处理规约**。如果 RPC 抛错时随意塞一个英文文本进去，L3 只能用脆弱的正则表达式（如 `.includes("timeout")`）来猜错误原因，这也是 Python 版经常导致的 Bug。
    *   **补强需求：R-ERROR-CODES-1 (结构化错误码)**：所有 RPC Error 必须严格遵守 JSON-RPC 2.0 规范，返回预定义的数字错误码与稳定且结构化的 Enum (如 `AGENT_STUCK`, `SANDBOX_CRASH`)，并把具体的文本塞在 `data` 字段中。
    *   *实例*：Agent CLI 崩溃时，返回的 JSON 应该是 `{"error": {"code": -32001, "message": "SANDBOX_CRASH", "data": {"exit_code": 127}}}`。L3 读到 `-32001` 或 `SANDBOX_CRASH` 就立刻知道该怎么处理，绝不需要解析文本。
2.  **API 变更周期评估**：
    “一个 minor 版本的 deprecation 周期” 是合理的，特别是在这套工具栈中，ccbd 和 L3 客户端（如 Claude Code）的迭代节奏通常是绑定的，不需要像公有云 API 那样承诺三年的向后兼容。
3.  **`system.dump` 格式约束**：
    必须是强类型约束的 JSON Schema。由于是用 Rust 实现，这个 Schema 可以直接通过 `serde_json` 和 `schemars` 库从后端的 `struct` 自动推导生成，确保外部解析工具（如 DevTools）能稳定消费。
4.  **`since_event_id` 超出清理策略范围的行为**：
    我完全赞同您的初步判断：**绝不能静默返回空数组**，而是必须返回带有警告标记的对象 `{"events_truncated": true, "current_state": {...}}`。让 Caller 明确知道自己丢失了一段历史，而不是误以为“那段时间 Agent 什么都没说”。
5.  **`request_id` 选填还是必填**：
    同意**选填（Optional）**。绝大多数人类键盘交互的投递（如 CLI 工具中直接按回车发送简单命令）不需要严密的去重，强制要求 UUID 会徒增简单 Caller 的对接成本。

### 最终共识结论

**决议结果：确认 7 条 L3 接口稳定性与契约需求。**
包含 Master Claude 提议的 6 条（R-DISPATCH-1/2, R-API-COMPAT-1, R-OBSERVABILITY-1, R-RECONNECT-1, R-IDEMPOTENCY-1）加上 Gemini 补强的 R-ERROR-CODES-1。同时在细节上达成了「选填幂等 Token」、「Truncated 标志位」和「基于结构体推导 Dump Schema」的设计共识。
