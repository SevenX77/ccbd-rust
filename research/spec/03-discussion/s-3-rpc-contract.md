# S-3：JSON-RPC 契约与错误代码树 (RPC API & Error Codes)

> **设计哲学**：L2 调度层（ccbd-rust）与 L3 编排层之间通过 UNIX Domain Socket (UDS) 通信。契约必须遵循 JSON-RPC 2.0 标准。所有接口设计遵循「面向失败设计」原则，必须显式声明可能抛出的业务错误。

---

## 1. JSON-RPC 2.0 封套标准 (Envelope Standard)

所有请求必须包含 `jsonrpc: "2.0"`。L2 的所有业务错误统一收敛到 `-32000` (Server Error)，并通过 `data` 字段透传强类型的 `error_code`。

```typescript
// 通用错误响应封套
interface JsonRpcErrorResponse {
  jsonrpc: "2.0";
  id: string | number | null;
  error: {
    code: -32000;
    message: string; // 人类可读的简要说明，如 "Agent spawned failed"
    data: {
      error_code: ErrorCodeEnum; // 见第 3 节：错误代码树
      details: Record<string, any>; // 错误上下文，如 { "path": "/bin/bwrap", "exit_code": 1 }
    };
  };
}
```

---

## 2. 方法契约 (Method Definitions)

### 2.1 `session.create`
建立一个用于级联清理（A-6）的会话拓扑根节点。

*   **Request**:
    ```typescript
    {
      "method": "session.create",
      "params": {
        "project_id": string, // 工作区绝对路径的 hash 或名称
        "master_pid": number  // L3 进程的 PID，用于 pidfd 监控
      }
    }
    ```
*   **Response**: `{"session_id": string}`
*   **Errors**: `DB_CONSTRAINT_VIOLATION` (如果 master_pid 不合法或不存在)

### 2.2 `agent.spawn`
在指定 session 下拉起一个带沙盒的 PTY 进程（S-1: `SPAWNING` 状态）。

*   **Request**:
    ```typescript
    {
      "method": "agent.spawn",
      "params": {
        "session_id": string,
        "agent_id": string,   // Caller 决定 ID，满足 R-DISPATCH-1
        "provider": string,   // "gemini", "codex", "claude_code"
        "sandbox_overrides"?: Record<string, any> // [可选] 覆盖 provider 默认挂载
      }
    }
    ```
*   **Response**: `{"state": "SPAWNING"}`
*   **Errors**: `SANDBOX_BWRAP_NOT_FOUND`, `SANDBOX_USER_NS_DISABLED`, `AGENT_ALREADY_EXISTS`

### 2.3 `agent.send`
向 Agent 投递指令流，驱动状态从 `IDLE` 转移到 `BUSY`。

*   **Request**:
    ```typescript
    {
      "method": "agent.send",
      "params": {
        "agent_id": string,
        "text": string,       // 必须以 \n 结尾以触发执行
        "request_id"?: string // [可选] 满足 R-IDEMPOTENCY-1 去重
      }
    }
    ```
*   **Response**: `{"state": "BUSY", "seq_id": number}` (返回事件流的写入位点)
*   **Errors**: `AGENT_WRONG_STATE` (非 IDLE 或 UNKNOWN 时调用), `AGENT_NOT_FOUND`, `IPC_INVALID_REQUEST`

### 2.4 `agent.read` (Pull 模式)
断线重连（R-RECONNECT-1）专用接口，按序拉取增量事件。

*   **Request**:
    ```typescript
    {
      "method": "agent.read",
      "params": {
        "agent_id": string,
        "since_event_id": number // 从 S-2 中 events.seq_id 拉取
      }
    }
    ```
*   **Response**:
    ```typescript
    {
      "events": AgentEvent[], // 见 2.8 节
      "is_truncated": boolean // 如果请求的 ID 已经被 SQLite 归档/删除，返回 true
    }
    ```
*   **Errors**: `AGENT_NOT_FOUND`

### 2.5 `session.subscribe` (Push 模式)
注册长连接监听器。调用后，L2 将源源不断地向当前 Socket 发送 Notification。

*   **Request**: `{"method": "session.subscribe", "params": {"session_id": string}}`
*   **Response**: `{"status": "subscribed"}`

### 2.6 异常闭环逃生舱 (issue-1b)
*   **`agent.assert_state`**:
    *   **Params**: `{"agent_id": string, "state": "IDLE", "evidence_id": string}`
    *   **Response**: `{"status": "asserted"}`
    *   **Errors**: `AGENT_WRONG_STATE` (只能在 UNKNOWN 状态调用), `DB_EVIDENCE_NOT_FOUND`
*   **`agent.discard_evidence`**:
    *   **Params**: `{"evidence_id": string}`
    *   **Response**: `{"status": "discarded"}`

### 2.7 观测与清理
*   **`agent.kill`**: `{"agent_id": string}` -> 转入 `KILLED`，发送 SIGKILL。
*   **`system.dump`**: 无参数 -> 导出 `projects`, `sessions`, `agents` 连表全量快照 (R-OBSERVABILITY-1)。

---

### 2.8 Server Push Notifications
当 L3 调用过 `session.subscribe` 后，L2 通过无 `id` 的 JSON-RPC 通知流式推送事件。

```typescript
interface AgentEventNotification {
  jsonrpc: "2.0";
  method: "agent.event";
  params: {
    seq_id: number;
    agent_id: string;
    event_type: "output_chunk" | "state_change" | "delivery_ack";
    payload: any;
  };
}
```
*注：当 Agent 崩溃时（R-DISPATCH-2），L2 推送 `state_change` 到 `CRASHED`，`payload` 中包含 `exit_code` 和 `error_code`。*

---

## 3. 错误代码树 (Error Code Tree)

所有 `error_code` 必须是以下枚举之一。遵循 A-7 决议的 `MODULE_REASON` 格式，只增不减。

### PTY & VT100 层 (PTY_)
*   `PTY_OPEN_FAILED`: 伪终端设备创建失败（可能系统 pty 耗尽）。
*   `PTY_MARKER_TIMEOUT`: `MarkerTimer` 溢出触发 `UNKNOWN` 状态（S-1 状态机）。
*   `PTY_IO_ERROR`: 读取或写入 stdout/stdin 时发生底层 IO 错误。

### 沙盒层 (SANDBOX_)
*   `SANDBOX_BWRAP_NOT_FOUND`: 环境中未安装 `bubblewrap` 且未显式开启 bypass。
*   `SANDBOX_USER_NS_DISABLED`: 宿主机内核不支持或禁用了 unprivileged user namespaces。
*   `SANDBOX_MOUNT_FAILED`: 根据 Profile 挂载卷失败（路径不存在或权限不足）。

### 数据库层 (DB_)
*   `DB_CORRUPTION`: SQLite 文件损坏或 WAL 写入失败。
*   `DB_CONSTRAINT_VIOLATION`: 主外键约束冲突（例如 session_id 不存在）。
*   `DB_EVIDENCE_NOT_FOUND`: 断言或丢弃时提供的 Evidence ID 在库中不存在。

### 生命周期层 (AGENT_)
*   `AGENT_NOT_FOUND`: `agent_id` 未在任何 session 中注册。
*   `AGENT_WRONG_STATE`: 状态机拦截。例如试图向 `BUSY` 或 `CRASHED` 的 Agent 发送 `agent.send`。
*   `AGENT_ALREADY_EXISTS`: 幂等性防护外，试图用相同的 `agent_id` 重新 spawn。
*   `AGENT_SPAWN_TIMEOUT`: 处于 `SPAWNING` 状态太久，未能捕捉到首次 Prompt Marker。
*   `AGENT_UNEXPECTED_EXIT`: 进程在非 `KILLED` 指令下死亡（由 pidfd 唤醒），包含 OOM 或自毁。

### RPC 与 IPC 层 (IPC_)
*   `IPC_INVALID_REQUEST`: JSON-RPC 格式错误或缺少必填字段。
*   `IPC_MALFORMED_JSON`: UDS 流中存在无法解析的断片。

### 调谐层 (RECONCILE_)
*   `RECONCILE_CLEANUP_FAILED`: 尝试回收沙盒或 PTY 句柄时操作系统拒绝。
*   `RECONCILE_STATE_DRIFT`: （内部断言）数据库显示 Running 但 `/proc` 查无此人，已强制转为 Crashed。
