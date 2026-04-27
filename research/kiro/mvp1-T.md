# Kiro Tasks: MVP 1 (I/O 骨架)

> **文档定位**：这是由 Codex 或人类工程师直接执行的任务清单。每个任务必须独立编译、独立验证。禁止跨组跳跃执行。

---

## 1. 任务依赖与执行图谱 (Dependency Graph)

MVP 1 划分为 4 个执行阶段（可分别对应 4 个 PR/Commit 组）。

```mermaid
graph TD
    subgraph G1: 基础设施 (Foundation)
        T1_1(T1.1 环境与依赖配置) --> T1_2(T1.2 错误机制与日志)
    end

    subgraph G2: 数据库层 (Persistence)
        T1_2 --> T2_1(T2.1 SQLite 初始化与迁移)
        T2_1 --> T2_2(T2.2 核心 Schema 表与 CRUD)
        T2_2 --> T2_3(T2.3 启动兜底对账)
    end

    subgraph G3: 进程与 PTY (OS / I/O)
        T2_3 --> T3_1(T3.1 Portable-PTY 进程拉起)
        T3_1 --> T3_2(T3.2 PTY Reader 守护任务)
        T3_1 --> T3_3(T3.3 进程收割守护任务)
    end

    subgraph G4: RPC 层与联调 (API)
        T2_3 --> T4_1(T4.1 UDS 监听与路由)
        T3_3 --> T4_2(T4.2 路由与 Handler 绑定)
        T4_1 --> T4_2
    end
```

---

## 2. 原子任务定义 (Atomic Tasks)

### Group 1: 基础设施 (Foundation)

#### T1.1: 环境与依赖配置
*   **依赖前置**: 无
*   **设计输入**: `mvp1-D.md` (6. 关键依赖, 5. 模块布局)
*   **输出产物**: `Cargo.toml`, `src/main.rs`, `src/env.rs`
*   **执行步骤**:
    1. 如果 `Cargo.toml` 不存在则使用 `cargo init` 初始化，否则在现有基础上补充 `mvp1-D.md` 节 6 指定的全部依赖。
    2. 在 `src/env.rs` 中实现 `resolve_state_dir()` 函数。如果 `CCB_ENV=dev`，返回绝对路径 `.../target/dev_state/`，否则返回 XDG 标准路径。确保目录自动创建。
*   **独立验收**: `CCB_ENV=dev cargo run` 正常运行不报错，且能在 `target/dev_state/` 下看到被自动创建的空目录。

#### T1.2: 错误机制与日志
*   **依赖前置**: T1.1
*   **设计输入**: `mvp1-D.md` (4. 错误代码子集)
*   **输出产物**: `src/error.rs`, `src/main.rs`
*   **执行步骤**:
    1. 在 `main.rs` 中初始化 `tracing_subscriber` (读取 `RUST_LOG` 环境变量，默认 info)。
    2. 在 `src/error.rs` 中定义 `CcbdError` 枚举，派生 `thiserror::Error`。
    3. 声明 `mvp1-D.md` 第 4 节中的 6 个错误变体。
    4. 为 `CcbdError` 实现转化为 JSON-RPC 错误封套的辅助方法 `to_rpc_error() -> serde_json::Value`。
*   **独立验收**: 编写单元测试，断言 `CcbdError::AgentNotFound("ag_1".into()).to_rpc_error()` 的 JSON 结构中包含 `code: -32000` 和 `error_code: "AGENT_NOT_FOUND"`。

---

### Group 2: 数据库层 (Persistence)

#### T2.1: SQLite 初始化与连接池
*   **依赖前置**: T1.2
*   **设计输入**: `mvp1-D.md` (2.1 数据库全局配置)
*   **输出产物**: `src/db/mod.rs`
*   **执行步骤**:
    1. 实现 `db::init(db_path)` 函数，利用 `rusqlite::Connection::open` 打开数据库。
    2. 强制执行 3 个 PRAGMA (`foreign_keys=ON`, `journal_mode=WAL`, `synchronous=NORMAL`)。
    3. 暴露一个简单的线程安全包裹结构 (如 `std::sync::Arc<std::sync::Mutex<Connection>>`)，因为 MVP1 暂时不引入复杂的异步 DB 池。
*   **独立验收**: 编写单元测试，使用 `tempfile::NamedTempFile` 作为 DB 路径验证 `PRAGMA journal_mode` 确实返回 `wal`。`:memory:` 库仅用于基础 CRUD 测试。

#### T2.2: 核心 Schema 表与 CRUD
*   **依赖前置**: T2.1
*   **设计输入**: `mvp1-D.md` (2.2 核心 DDL, 3.2, 3.4 去重)
*   **输出产物**: `src/db/schema.rs`, `src/db/queries.rs`
*   **执行步骤**:
    1. 在 `schema.rs` 定义 `Agent`, `Event`, `Session` 的 Rust struct 映射。
    2. 在 `db::init()` 中写入 D.2.2 节的完整 CREATE TABLE 语句。
    3. 实现基础查询函数：`insert_session`, `insert_agent`, `update_agent_state`, `query_event_by_request_id`。
    4. 实现 `insert_event`，特别捕获 `rusqlite::Error::SqliteFailure` 判断 `UNIQUE` 约束冲突，如果是，返回自定义的 `CcbdError::DuplicateRequest`。
*   **独立验收**: 单元测试中尝试插入两次带有同一个 `request_id` 的 `events`，断言第二次调用返回 `DuplicateRequest` 错误。

#### T2.3: 启动兜底对账 (Startup Reconcile)
*   **依赖前置**: T2.2
*   **设计输入**: `mvp1-R.md` (R-DISPATCH-1), `mvp1-D.md` (1.2 状态机流转限制)
*   **输出产物**: `src/db/queries.rs`, `src/main.rs`
*   **执行步骤**:
    1. 在 `queries.rs` 实现 `reconcile_active_agents_to_crashed()`。
    2. 在一个 SQL 事务中执行：首先找出所有 `state NOT IN ('CRASHED', 'KILLED')` 的 agent_id。
    3. 对于每一个找出的 agent_id，向 `events` 表插入 `event_type='state_change'`，`payload={"from": "...", "to": "CRASHED", "reason": "STARTUP_RECONCILE"}` 的记录。
    4. 执行 SQL: `UPDATE agents SET state = 'CRASHED' WHERE state NOT IN ('CRASHED', 'KILLED')`。
    5. 在 `main.rs` 中，确保在监听 UDS Socket 之前调用此函数，将遗留进程归档。
*   **独立验收**: 单元测试预插一条状态为 `IDLE` 的 agent，调用该函数后查验 DB 断言其状态已变为 `CRASHED`，且 `events` 表中存在对应的 `state_change` 事件。

---

### Group 3: 进程与 PTY (OS / I/O)

#### T3.1: Portable-PTY 进程拉起与管理
*   **依赖前置**: T2.3
*   **设计输入**: `mvp1-D.md` (3.3 agent.spawn, 5. pty/mod.rs)
*   **输出产物**: `src/pty/mod.rs`
*   **执行步骤**:
    1. 创建全局状态 `PTY_MAP: Arc<Mutex<HashMap<String, Box<dyn std::io::Write + Send>>>>`，用于管理 Agent ID 到 `master_writer` 的映射。
    2. 实现 `spawn_agent(provider)` 逻辑。使用 `portable-pty` 创建大小为 24x80 的虚拟终端。
    3. 通过 `slave.spawn_command` 拉起进程，提取 `pid`。
    4. 将 `try_clone_writer()` 存入 `PTY_MAP`。
    5. 返回 `master_reader` 和 `Child` 给外层。
*   **独立验收**: 单元测试中调用该函数拉起 `bash`，能在 `PTY_MAP` 中找到对应的 Writer 并且对 Writer 写入 `exit\n` 后进程正常退出。

#### T3.2: PTY Reader 守护任务
*   **依赖前置**: T3.1
*   **设计输入**: `mvp1-D.md` (3.3 agent.spawn 监控任务)
*   **输出产物**: `src/pty/tasks.rs`
*   **执行步骤**:
    1. 实现异步任务 `spawn_pty_reader_task(agent_id, reader, db_conn)`。
    2. 使用 `tokio::task::spawn_blocking` 包裹同步的 reader 循环（因为 `portable-pty` 的 reader 是阻塞的 io::Read，MVP1 中可简单使用缓冲读取）。
    3. 当读取到字节数组时，将其使用 `String::from_utf8_lossy` 转换为字符串，调用 DB 层的 `insert_event(output_chunk)`。
*   **独立验收**: 单元测试拉起 `bash`，直接通过 T3.1 暴露到 `PTY_MAP` 的 writer 写入 `echo "kiro_test"\n`，休眠 100ms 后查询 SQLite 的 `events` 表，能找到 payload 包含 `kiro_test` 的记录。

#### T3.3: 进程收割守护任务
*   **依赖前置**: T3.1
*   **设计输入**: `mvp1-D.md` (3.3 agent.spawn 监控任务)
*   **输出产物**: `src/pty/tasks.rs`
*   **执行步骤**:
    1. 实现异步任务 `spawn_child_wait_task(agent_id, mut child, db_conn)`。
    2. 使用 `tokio::task::spawn_blocking` 包裹，并在其内部调用 `portable_pty::Child::wait(&mut child)` 阻塞等待进程退出。
    3. 当等待返回时，抓取 exit_code。调用 DB 层方法将对应 Agent 的状态更新为 `CRASHED`，同时在 `events` 插入类型为 `state_change` 的事件。
    4. 从 `PTY_MAP` 中移除对应的 writer。
*   **独立验收**: 单元测试中，外部直接 kill 掉 child 进程。等待几毫秒后查询 DB，断言该 Agent 的状态变为 `CRASHED` 且 `PTY_MAP` 已清理。

---

### Group 4: RPC 层与联调 (API)

#### T4.1: UDS 监听与基础路由
*   **依赖前置**: T2.3
*   **设计输入**: `mvp1-R.md` (AC 1), `mvp1-D.md` (3.1 封套标准)
*   **输出产物**: `src/rpc/mod.rs`, `src/rpc/router.rs`
*   **执行步骤**:
    1. 使用 `tokio::net::UnixListener` 在 `target/dev_state/ccbd.sock` 建立监听。启动前如果文件已存在，先 `fs::remove_file`。
    2. 实现一问一答的 Socket Frame 处理（按行读取 `\n` 分隔的 JSON）。
    3. **安全性防偏航**：Socket 文件创建后必须确保其具备 `chmod 0600` 权限（可通过 umask 设置或显式 chmod）。
    4. 解析输入的 JSON-RPC 2.0 格式。如果解析失败，回复 `IPC_INVALID_REQUEST`。
*   **独立验收**: 启动程序，在终端运行 `socat - UNIX-CONNECT:target/dev_state/ccbd.sock`，输入 `{"method": "not_exist"}`，能收到正确的 `-32000` 错误包装格式。

#### T4.2: 路由绑定与 4 大 Handler
*   **依赖前置**: T4.1, T3.3
*   **设计输入**: `mvp1-D.md` (3.2-3.5)
*   **输出产物**: `src/rpc/handlers.rs`, `src/rpc/router.rs`
*   **执行步骤**:
    1. 实现 `handle_session_create`，调用 `db::insert_session` (接收 `absolute_path` 参数)。
    2. 实现 `handle_agent_spawn`，按序调用 `db` 和 `pty::spawn_agent`，并触发后台的 reader 和 wait tasks。
    3. 实现 `handle_agent_send`，**严格按 D §3.4 的 PENDING → PTY → SENT/FAILED 状态机**：
        - 3a. 前置校验：`query_agent(agent_id)` 不存在或 state=CRASHED 返回 `AGENT_NOT_FOUND`；`PTY_MAP` 中没有 writer 返回 `PTY_IO_ERROR`
        - 3b. 原子 INSERT `command_received` payload `{status:"PENDING"}`，依赖 `idx_events_idempotent` UNIQUE 索引防并发
        - 3c. UNIQUE 冲突时，调 `query_event_by_request_id`，根据已有事件的 `payload.status` 分支：`SENT` 或 `PENDING` 视为成功幂等返回当前 agent state；`FAILED` 返回 `PTY_IO_ERROR { existing_seq_id }`，**不掩盖之前的失败**
        - 3d. 占坑成功 → 写 PTY → 在 `BEGIN IMMEDIATE` 事务内 UPDATE `events.payload` 为完整新 JSON（`status=SENT` 或 `FAILED`，**不用 `json_set`** 以避开 JSON1 扩展依赖），写成功时同时 UPDATE `agents.state='BUSY'` + `state_version+1`；写失败仅更新 status=FAILED 不动 agents.state，最后返回 `PTY_IO_ERROR`
        - 3e. **TOCTOU 防 panic**：写 PTY 时必须在同一次 `pty_map.lock()` 内做 `get_mut` 并处理 `None`（child_wait_task 可能在 3a 和 3d 之间清理了 writer），**禁止 `unwrap` 或 `expect`**；writer 缺失视为 `BrokenPipe` 走 FAILED 分支落库，不要让进程 panic
    4. 实现 `handle_agent_read`，执行带 `since_event_id` 的 SQLite 查询并序列化返回。
    5. 将这 4 个方法注册进 `router.rs`。
*   **独立验收**: 整个 MVP 1 总验收测试。

---

## 3. MVP 1 总验收追溯表 (Traceability Matrix)

| 验收标准 (AC) | 依赖的最后 Task | 验收测试方法 (供 T5.1 整体跑通) |
| :--- | :--- | :--- |
| **AC1: 环境隔离** | T4.1 | 设置 `CCB_ENV=dev` 启动，查验 `target/dev_state/` 下是否有 `ccbd.sock` 与 `ccbd.sqlite`。 |
| **AC2: 拓扑创建** | T4.2 | 外部脚本 `socat` 发送 `session.create` 拿到 session_id，发送 `agent.spawn` 参数为 `bash`。使用 `ps aux` 能看到系统拉起了真实的 `bash`。 |
| **AC3: I/O与幂等**| T4.2 | 发送 `agent.send(request_id="A", "echo 1\n")`。随后重复发一次。检查 DB 中的 `events` 表，确保 `command_received` 只插入了一条。 |
| **AC4: 断点拉取** | T4.2 | 发送 `agent.read(since_event_id=0)`，能成功读取到包含 `1` 换行的 `output_chunk` payload 数组。 |
| **AC5: 生命捕获** | T4.2 | 手工 `kill -9 <bash_pid>`。发送 `agent.read`，获取到最新的 `event_type="state_change"`，内容指出 `to: CRASHED`。 |

---

## 4. 执行节奏与工时估算建议

建议将以上代码实现分为 3 个 PR/Commit 进行合并，以便进行增量 Code Review。

1. **Commit 1 (G1 + G2): "Setup Project & SQLite SoT"**
    *   **范畴**: T1.1 - T2.3。完成数据底座和环境隔离。
    *   **代码量**: 约 300-500 行。属于 CRUD 和模板代码，风险低。
2. **Commit 2 (G3): "Portable-PTY Process Management"**
    *   **范畴**: T3.1 - T3.3。完成 PTY 读写与子进程生命周期绑定。
    *   **代码量**: 约 200-300 行。
3. **Commit 3 (G4): "UDS RPC Server & Handlers"**
    *   **范畴**: T4.1 - T4.2。将底层逻辑暴露给 UDS。
    *   **代码量**: 约 300-400 行。需处理大量 JSON 拆装包。

---

## 5. 核心风险点提示 (Risks & Mitigations)

1. **SQLite 并发锁 (Database Locked)**
    *   *风险*: `PtyReaderTask` 极高频地（每毫秒）将 Chunk 写入 SQLite，同时 UDS 也在查询。由于使用同一文件，可能触发 `database is locked`。
    *   *兜底*: T2.1 中已指定 `WAL` 模式。要求在 Rust 代码连接数据库时，使用 `busy_timeout` PRAGMA（例如设为 5000ms），允许 SQLite 自动重试。
2. **PTY 阻塞读取 (Reader Blocking)**
    *   *风险*: `portable-pty` 的 `try_clone_reader` 产出的是阻塞式的 `std::io::Read`。如果在主 `tokio` worker 里直接 `.read()` 会导致整个异步反应器停转。
    *   *兜底*: 在 T3.2 中明确指出，必须用 `tokio::task::spawn_blocking` 包裹 PTY 的 `read` 循环。
3. **僵尸进程 (Zombie Processes)**
    *   *风险*: 如果 ccbd-rust Daemon 崩溃退出，通过 `spawn` 拉起的子进程不会自动死掉。
    *   *说明*: 在 MVP 1 中（因为没有引入 MVP 2 的 Systemd 包装），这是**预期行为**。开发测试期间，需要手工 `kill` 遗留的 bash 进程。不必在 MVP 1 的代码中写复杂的 `Drop` 钩子。
4. **UDS Socket 越权访问**
    *   *风险*: `ccbd.sock` 创建后如果权限为 `0777`，可能被非 Owner 写入，导致注入攻击。
    *   *兜底*: 在 T4.1 明确规定创建后必须执行 `chmod 0600`。
5. **PTY 乱码与膨胀**
    *   *风险*: CLI 有时会输出包含无效 UTF-8 的二进制颜色转义符，导致 `String::from_utf8` Panic。
    *   *兜底*: `PtyReaderTask` 必须使用 `String::from_utf8_lossy` 处理输出的 chunk 数据。
6. **守护进程重启状态丢失**
    *   *风险*: 内存中 `PTY_MAP` 丢失，导致原本处于 `IDLE` 的进程变为幽灵进程且无法接受指令。
    *   *兜底*: 已经通过引入 T2.3 `Startup Reconcile` 将所有 Active Agent 在启动时清退为 `CRASHED`，强制保证 L3 能观测到失效。
