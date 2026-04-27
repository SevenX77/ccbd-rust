# A-7 议题辩论：错误传播与结构化 Code 设计

## Round 1 - Master Claude 立场

**决策提议**：建立「两层错误模型」。内部使用 `thiserror` 强类型 Enum，跨进程边界翻译为带有语义码（error_code）的 JSON-RPC 2.0 结构。

### 正方推理
1. **编译期安全**：内部 Module 使用 `thiserror` 定义枚举，编译器强制 Caller 处理所有错误分支，杜绝静默失败。
2. **跨进程稳定性 (R-ERROR-CODES-1)**：Caller 依赖稳定的 `error_code` 字符串枚举（如 `AGENT_STUCK`）进行逻辑派发，而非解析易变的 `message` 文本。
3. **清晰的演进路径 (R-API-COMPAT-1)**：支持新增错误码。细化错误类型时采用「加新、保留旧」策略，确保老客户端不崩溃。

### 反方立场（单一 String 透传）
1. **极简开发**：一行 `anyhow!("error message")` 搞定。
2. **缺点**：Caller 被迫进入「字符串匹配地狱」，无法稳定区分环境配置错误与业务逻辑超时，且对多语言环境支持极差。

---

## Round 2 - Gemini 评估

### 1. 内部实现：thiserror vs anyhow
- **决议**：Module 边界**强制使用 `thiserror`**。`anyhow` 仅允许在 `main.rs` 或最终的全局异常捕获层作为「万能容器」使用。
- **理由**：L2 调度层内部存在复杂的相互依赖（如 Reconcile 逻辑需要根据 PTY 错误类型决定是重启进程还是标记 Stuck），强类型错误是逻辑自洽的前提。

### 2. RPC 协议层：Code 分配与命名
- **JSON-RPC Code**：统一使用 `-32000` 代表所有 L2 业务错误。
- **Naming**：采用 `MODULE_REASON` 格式的 `SCREAMING_SNAKE_CASE`。例如 `PTY_MARKER_TIMEOUT`。
- **Data Schema**：`details` 字段必须是强 Schema，每个 `error_code` 对应一个固定的 JSON 结构。

### 3. 错误码类目结构 (Initial Categories)

| 模块 (Module) | 错误码前缀 (Prefix) | 候选错误码 (Samples) |
|---|---|---|
| **PTY/VT100** | `PTY_` | `OPEN_FAILED`, `MARKER_TIMEOUT`, `PARSER_DESYNC` |
| **Sandbox** | `SANDBOX_` | `MOUNT_FAILED`, `BINARY_NOT_FOUND`, `OOM_KILLED` |
| **Persistence** | `DB_` | `CORRUPTION`, `LOCK_TIMEOUT`, `CONSTRAINT_VIOLATION` |
| **IPC/RPC** | `IPC_` | `INVALID_REQUEST`, `MALFORMED_JSON`, `CLIENT_DISCONNECTED` |
| **Lifecycle** | `AGENT_` | `SPAWN_TIMEOUT`, `STUCK`, `UNEXPECTED_EXIT` |
| **Reconcile** | `RECONCILE_` | `STATE_DRIFT`, `ORPHAN_RESOURCE`, `CLEANUP_FAILED` |

---

## 最终决议

**决策结果**：采用 **强类型内部 Error + 结构化 RPC Error 翻译层**。

- **核心规范**：
  1. 内部：Module Error 必须派生 `thiserror::Error`。
  2. 边界：RPC 错误结构固定为 `{"code": -32000, "data": {"error_code": "STRING", "details": {}}}`。
  3. 兼容性：`error_code` 字符串枚举只增不减。

**决议日期**：2026-04-26
**达成方式**：Master Claude 提议，Gemini 细化命名规范与类目结构并确认。
