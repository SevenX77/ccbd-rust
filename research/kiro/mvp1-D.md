# Kiro Design: MVP 1 (I/O 骨架)

> **文档定位**：本文件是 ccbd-rust MVP 1 阶段的官方 D (Design) 规格。严格基于 `mvp1-R.md` 定义的边界，为 Codex 的 T 阶段（Task）提供无歧义的代码实施蓝图。

---

## 1. 状态机精简版 (State Machine)

MVP 1 仅保留最核心的生命周期状态。

### 1.1 状态枚举
*   `IDLE`: Agent 被成功拉起，处于可接收指令状态。
*   `BUSY`: 已通过 `agent.send` 投递指令。（*Carve-out*: 无 vt100 解析，MVP 1 中 `BUSY` 不会自动流转回 `IDLE`）。
*   `CRASHED`: 物理进程通过 `tokio::task::spawn_blocking` 包裹下的 `portable_pty::Child::wait` 探测到退出，或通过 Startup Reconcile 标记。

### 1.2 状态流转限制
在 MVP 1 中，`agent.send` **不以** `state == IDLE` 作为前置拦截条件。即使状态是 `BUSY`，也允许 L3 继续调用 `agent.send`，系统仅进行 PTY 写入和 `events` 记录。
```mermaid
stateDiagram-v2
    [*] --> IDLE: agent.spawn
    IDLE --> BUSY: agent.send
    BUSY --> BUSY: agent.send (MVP1 特权)
    IDLE --> CRASHED: Child::wait 返回 / Startup Reconcile
    BUSY --> CRASHED: Child::wait 返回 / Startup Reconcile
    CRASHED --> [*]
```

---

## 2. SQLite Schema 精简版 (Database Design)

遵循 A-1 决议，裁剪掉 `evidence` 表，保留 `state_version` 等控制字段以保持向后兼容性。

### 2.1 数据库全局配置
初始化 `rusqlite::Connection` 时必须执行：
```sql
PRAGMA foreign_keys = ON;
PRAGMA journal_mode = WAL;
PRAGMA synchronous = NORMAL;
```

### 2.2 核心 DDL
```sql
CREATE TABLE projects (
    id TEXT PRIMARY KEY,
    absolute_path TEXT NOT NULL UNIQUE,
    created_at INTEGER NOT NULL DEFAULT (unixepoch())
) STRICT;

CREATE TABLE sessions (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    master_pid INTEGER NOT NULL, 
    created_at INTEGER NOT NULL DEFAULT (unixepoch())
) STRICT;

CREATE TABLE agents (
    id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    provider TEXT NOT NULL,         
    state TEXT NOT NULL,            -- 仅限: IDLE, BUSY, CRASHED
    state_version INTEGER NOT NULL DEFAULT 1,
    pid INTEGER,                    
    exit_code INTEGER,              
    error_code TEXT,                
    created_at INTEGER NOT NULL DEFAULT (unixepoch()),
    updated_at INTEGER NOT NULL DEFAULT (unixepoch())
) STRICT;

CREATE INDEX idx_agents_active ON agents(state) WHERE state NOT IN ('CRASHED');

CREATE TABLE events (
    seq_id INTEGER PRIMARY KEY AUTOINCREMENT, 
    agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    request_id TEXT,                
    event_type TEXT NOT NULL,       -- 'output_chunk', 'state_change', 'command_received'
    payload TEXT NOT NULL,          -- JSON string, 对于 command_received，需包含 status (PENDING/SENT/FAILED)
    created_at INTEGER NOT NULL DEFAULT (unixepoch())
) STRICT;

CREATE INDEX idx_events_agent_seq ON events(agent_id, seq_id);
-- 用于 R-IDEMPOTENCY-1 的防重放控制
CREATE UNIQUE INDEX idx_events_idempotent ON events(agent_id, request_id) WHERE request_id IS NOT NULL;
```

---

## 3. RPC 契约与核心逻辑 (Core RPCs)

### 3.1 封套标准 (Envelope)
所有请求/响应都封装在 JSON-RPC 2.0 中。MVP 1 **不包含** `session.subscribe` 推送通知，所有通信仅为 Request-Response 模式。

### 3.2 `session.create`
*   **Params**: `{"project_id": "proj_123", "absolute_path": "/path/to/project", "master_pid": 999}`
*   **Returns**: `{"session_id": "sess_abc"}`
*   **逻辑伪代码**:
    ```rust
    // 确保项目存在 (upsert)
    INSERT OR IGNORE INTO projects (id, absolute_path) VALUES (?, ?);
    // 生成 uuid v4 作为 session_id
    let session_id = uuid::Uuid::new_v4().to_string();
    INSERT INTO sessions (id, project_id, master_pid) VALUES (?, ?, ?);
    return session_id;
    ```

### 3.3 `agent.spawn`
*   **Params**: `{"session_id": "sess_abc", "agent_id": "ag_1", "provider": "bash"}`
*   **Returns**: `{"state": "IDLE"}`
*   **逻辑伪代码**:
    ```rust
    // 1. DB 校验
    if session_exists(session_id) == false { return Err(DB_CONSTRAINT_VIOLATION); }
    
    // 2. 使用 portable-pty 拉起进程 (防偏航：不使用 bwrap)
    let pty_system = native_pty_system();
    let pty_pair = pty_system.openpty(PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 })?;
    
    let mut cmd = portable_pty::CommandBuilder::new(provider);
    let mut child = pty_pair.slave.spawn_command(cmd)?;
    
    let pid = child.process_id().unwrap();
    let master_writer = pty_pair.master.try_clone_writer()?;
    let master_reader = pty_pair.master.try_clone_reader()?;
    
    // 3. 落盘初始状态
    INSERT INTO agents (id, session_id, provider, state, pid) 
    VALUES (agent_id, session_id, provider, 'IDLE', pid);
    
    // 4. 注册后台监控任务 (I/O 读取与退出捕获)
    spawn_pty_reader_task(agent_id.clone(), master_reader); // 不断将 stdout 写入 events(output_chunk)
    
    // 注意：portable_pty::Child::wait 是阻塞操作，必须包入 spawn_blocking
    spawn_child_wait_task(agent_id.clone(), child); // wait() 返回时，更新 db 状态为 CRASHED
    
    // 5. 保存 Writer 句柄用于发送
    pty_map.lock().unwrap().insert(agent_id, master_writer);
    
    return {"state": "IDLE"};
    ```

### 3.4 `agent.send`
*   **Params**: `{"agent_id": "ag_1", "text": "ls\n", "request_id": "req_1"}`
*   **Returns**: `{"state": "BUSY", "seq_id": 101}`
*   **可能的 error_code**: `AGENT_NOT_FOUND` / `PTY_IO_ERROR` / `DB_CONSTRAINT_VIOLATION`
*   **逻辑伪代码（PENDING → PTY 写 → SENT/FAILED 原子状态机；含 FAILED 重试语义）**:
    ```rust
    // 0. 前置校验：防 PENDING 孤儿事件 + 防 PTY_MAP.unwrap() panic
    let agent_row = query_agent(agent_id)?;
    match agent_row {
        None => return Err(AGENT_NOT_FOUND),
        Some(row) if row.state == "CRASHED" => return Err(AGENT_NOT_FOUND),
        _ => {}
    }
    if !pty_map.lock().unwrap().contains_key(agent_id) {
        return Err(PTY_IO_ERROR);  // writer 已被 child_wait_task 清理 → 进程死了
    }

    // 1. 原子占位（依赖 UNIQUE INDEX(agent_id, request_id) 防并发重复）
    let initial_payload = serde_json::json!({"cmd": text, "status": "PENDING"}).to_string();
    let res = execute(
        "INSERT INTO events (agent_id, request_id, event_type, payload) VALUES (?, ?, 'command_received', ?)",
        (agent_id, request_id, &initial_payload),
    );

    if let Err(SqliteError::UniqueConstraint) = res {
        // 重放：根据已有事件的最终 status 决定返回语义，绝不把 FAILED 当成功
        let existing = query_event_by_request_id(agent_id, request_id)?;
        let status = existing.payload_json["status"].as_str().unwrap_or("PENDING");
        match status {
            "SENT" | "PENDING" => {
                // PENDING 也按"已投递"返回（首个 in-flight 请求会最终落到 SENT/FAILED；
                // 同一 caller 不会真的并发发同一个 request_id 两次）
                let agent_state = query_agent(agent_id)?.unwrap().state;
                return Ok({"state": agent_state, "seq_id": existing.seq_id});
            }
            "FAILED" => {
                // 上次明确投递失败：不掩盖，让 caller 知道并能用新的 request_id 重试
                return Err(PTY_IO_ERROR { existing_seq_id: existing.seq_id });
            }
            _ => unreachable!("unknown command_received status: {}", status),
        }
    }
    let seq_id = res.unwrap_inserted_rowid();

    // 2. 写入 PTY（在同一次 lock 内处理 writer 消失的 TOCTOU 边界——
    //    child_wait_task 可能在 step 0 和这里之间已移除 writer；不能 expect/unwrap，
    //    必须把 writer-not-found 当作 PTY_IO_ERROR 让 step 3 落 FAILED）
    let write_result: std::io::Result<()> = {
        let mut map = pty_map.lock().unwrap();
        match map.get_mut(agent_id) {
            Some(writer) => writer.write_all(text.as_bytes()),
            None => Err(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "PTY writer disappeared between pre-check and write",
            )),
        }
    };

    // 3. 事务内更新事件 status 和 agent state
    //    避免 SQLite json_set（依赖 JSON1 扩展），改为 Rust 侧重新序列化整 payload 后 UPDATE
    let final_status = if write_result.is_ok() { "SENT" } else { "FAILED" };
    let final_payload = serde_json::json!({"cmd": text, "status": final_status}).to_string();
    execute("BEGIN IMMEDIATE");
    execute("UPDATE events SET payload = ? WHERE seq_id = ?", (&final_payload, seq_id));
    if write_result.is_ok() {
        execute(
            "UPDATE agents SET state = 'BUSY', state_version = state_version + 1, updated_at = unixepoch() \
             WHERE id = ? AND state != 'CRASHED'",
            agent_id,
        );
    }
    execute("COMMIT");

    if write_result.is_err() { return Err(PTY_IO_ERROR); }
    Ok({"state": "BUSY", "seq_id": seq_id})
    ```

### 3.5 `agent.read`
*   **Params**: `{"agent_id": "ag_1", "since_event_id": 100}`
*   **Returns**: `{"events": [...], "is_truncated": false}`
*   **逻辑伪代码**:
    ```rust
    if not_exists_in_agents(agent_id) { return Err(AGENT_NOT_FOUND); }
    
    // 1. 查询增量事件
    let rows = query("SELECT seq_id, event_type, payload FROM events WHERE agent_id = ? AND seq_id > ? ORDER BY seq_id ASC", (agent_id, since_event_id));
    
    // 2. 组装返回 (前端通过发现 event_type='state_change', payload={"to":"CRASHED"} 来感知进程死亡)
    return {"events": rows, "is_truncated": false};
    ```

---

## 4. 错误代码子集 (Error Codes)

在 `src/error.rs` 中使用 `thiserror` 定义枚举。暴露给 RPC 时的 `error_code` string 仅限以下子集：

*   **DB_CONSTRAINT_VIOLATION**: DB 外键错误或非 idempotency 导致的 Unique 约束错误。
*   **AGENT_NOT_FOUND**: `agent_id` 在 DB 中未找到，或 state=CRASHED。
*   **AGENT_ALREADY_EXISTS**: 试图用相同的 ID 重复 spawn。
*   **PTY_OPEN_FAILED**: `openpty()` 或 `spawn_command()` 失败。
*   **PTY_IO_ERROR**: `agent.send` 往 PtyMaster 写入失败（如进程管道已断裂、TOCTOU 时 writer 消失、或前次同 request_id 已 FAILED）。
*   **IPC_INVALID_REQUEST**: 缺失必填字段，或者无法解析 JSON。

**关于 `CcbdError::DuplicateRequest`**：T2.2 中 `insert_event` 检测到 UNIQUE 冲突时返回的 `CcbdError::DuplicateRequest` 是 **DB 层内部 sentinel**，**不映射为 RPC error_code**。`agent.send` handler 捕获该 sentinel 后按 D §3.4 step 1 的 status 分支路由（SENT/PENDING 成功幂等返回，FAILED 返回 `PTY_IO_ERROR`），caller 永远看不到 `DUPLICATE_REQUEST` 字符串。

---

## 5. Rust 模块布局 (Crate Layout)

MVP 1 推荐的源码目录结构规划如下：

```
src/
├── main.rs         // 入口：读取 CCB_ENV，执行 Startup Reconcile，启动 UDS server
├── env.rs          // 配置：基于 XDG 解析 socket 和 sqlite 的绝对路径
├── error.rs        // 统一错误管理 (thiserror 定义及 JSON-RPC 格式化)
├── db/
│   ├── mod.rs      // SQLite 连接池管理与 Migration (include_str!)
│   ├── schema.rs   // 映射 agents, sessions 等表的 Rust Struct
│   └── queries.rs  // 具体的 SQL 执行逻辑 (upsert_event, update_state 等)
├── rpc/
│   ├── mod.rs      // UNIX Domain Socket 监听循环与 JSON 拆包
│   ├── router.rs   // JSON-RPC 2.0 路由派发
│   └── handlers.rs // session.create, agent.spawn 等方法的具体实现
└── pty/
    ├── mod.rs      // 管理存活进程的 PtyMaster (Writer) 句柄 Map (Arc<Mutex<HashMap>>)
    └── tasks.rs    // spawn_pty_reader_task (读 PtyMaster Reader) 与 spawn_child_wait_task (等 exit)
```

---

## 6. 关键依赖与 Feature 声明 (Dependencies)

在 `Cargo.toml` 中必须包含且仅限以下核心运行依赖（`[dev-dependencies]` 不受"仅限"约束，按测试需要扩展）：

```toml
[dependencies]
# 异步运行时
tokio = { version = "1.36", features = ["rt-multi-thread", "macros", "net", "process", "io-util", "sync"] }
# 序列化与 JSON-RPC 基础
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
# SQLite 持久化 (必须 bundled 保证无外部依赖)
rusqlite = { version = "0.32", features = ["bundled"] }
# 强类型错误传播
thiserror = "1.0"
# 日志观测体系
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
# 路径规范解算
directories = "5.0"
# PTY 分配层 (A-3 决议)
portable-pty = "0.8.1"
# session_id 生成 (用于 D §3.2 session.create)
uuid = { version = "1", features = ["v4"] }

[dev-dependencies]
# 文件型 SQLite 测试（用于验证 WAL 模式，:memory: 不能可靠开启 WAL）
tempfile = "3"
```

**关于 SQLite JSON 函数**：D §3.4 的 `agent.send` 不使用 SQLite `json_set`（避免依赖 JSON1 扩展的可用性），改为在 Rust 侧读出 payload → 重新序列化 → 整字段 UPDATE。如果未来需要 SQL 内 JSON 操作，再显式给 rusqlite 添加 `"functions"` 或 `"json"` feature。
