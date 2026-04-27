# A-1 议题辩论：持久化方案与数据库选型

## Round 1 - Master Claude 立场

**决策提议**：确认使用 **SQLite + WAL 模式（rusqlite bundled）**。

### 正方推理
1. **ACID + 关系约束是 R-RECONCILE-1 的硬要求**：状态变更与文件系统操作必须在 `BEGIN ... COMMIT` 事务内一致。JSON 无法防文件级竞态，sled 缺少跨集合（多表）的事务级约束。
2. **SQL 表达力对调谐循环至关重要**：执行状态对账需要高频进行集合查询（如找出 `agents` 记录存活但不在 `live_pids` 中的死进程）。SQL JOIN/WHERE 相比 KV 的 prefix scan + 内存合并更高效。
3. **R-OBSERVABILITY-1 实现成本最低**：导出快照仅需执行 SQLite 的 `.dump` 命令。
4. **C-FFI 痛点被 bundled 特性解决**：`rusqlite` 开启 `bundled` 后静态链接 C 源码，产出纯净单二进制文件（符合 B-4 决议），无需目标机器预装 `libsqlite3`。

### 反方立场（sled 纯 Rust 嵌入式 KV）
1. **纯净编译路径**：完全没有 C 依赖。
2. **现代 API**：API 设计符合 Rust 习惯。
*反方代价*：仍处 alpha 阶段（API 会 Breaking），必须手动实现关系索引，ACID 考验不如 SQLite 充分，失去 `sqlite3` 标准 CLI 调试能力。

---

## Round 2 - Gemini 评估

### 1. ORM/Driver 选型：rusqlite vs sqlx vs diesel
- **决议**：认同选择 **`rusqlite`（最薄抽象）**。
- **理由**：`sqlx` 的核心卖点是纯异步和编译期宏检查，但 SQLite 底层是同步文件 I/O。在 `tokio`（B-1 决议）中，最安全的本地 DB 模式是分配一个独立的 MPSC Channel + 专用 OS 线程（或 `tokio::task::spawn_blocking`）运行 `rusqlite` 同步代码，避免阻塞 Async 反应器。对于仅有 4 张核心表的 Daemon，引入宏解析和巨大的异步状态机属于过度设计。

### 2. WAL Pragmas 配置：NORMAL vs FULL
- **决议**：强制使用 **`PRAGMA synchronous = NORMAL;`**。
- **理由**：在 WAL 模式下，`NORMAL` 保证即便应用程序崩溃，数据也绝对不会损坏，且只有在操作系统内核崩溃/突然断电时才会丢失最后几毫秒的事务。考虑到 Agent 输出事件（events）每秒可达几十次，使用 `FULL` 每次强制 fsync 会导致磁盘 I/O 阻塞，拖垮 L2 吞吐量。

### 3. Migration 机制
- **决议**：认同轻量版本化，建议 **编译期嵌入迁移脚本**。
- **机制设计**：不需要 `sqlx-cli` 这种重型外部工具。在 `src/db/migrations/` 下编写 `V1__init.sql`、`V2__add_field.sql`。通过 Rust 的 `include_str!` 宏将 SQL 字符串打包进二进制。在 `ccbd-rust` 启动阶段（`Startup Reconcile` 前），按序读取 `PRAGMA user_version` 并执行增量变更，确保符合 B-4（单二进制）要求。

### 4. 存储位置与测试模式
- **决议**：认同物理路径分治 + 强制支持 **`:memory:`**。
- **理由**：B-5 决议（Mock 测试策略）极其依赖 `:memory:`。测试用例可以在几毫秒内建立纯净的数据库环境验证状态机转移，结束后自动随内存释放，这是保障 TDD 开发体验的基石。

### 5. 数据量估算与性能容量
- **估算场景**：按 1 个活跃系统管理 10 个 Agent，每个产生 10000 events/天（总计 10 万事件），单条事件约 256 字节。
- **容量分析**：全天日志仅约 **25MB - 50MB**。
- **性能结论**：SQLite 能够轻松应对 GB 级别数据和万级 QPS，这完全在 SQLite 的甜点区。为了防止数据无限膨胀，L2 调谐循环只需每天执行一次 `DELETE FROM events WHERE created_at < datetime('now', '-1 day'); PRAGMA wal_checkpoint(TRUNCATE);` 即可维持数据库在极小体积。

---

## 最终决议

**决策结果**：采用 **SQLite + WAL 模式（基于 rusqlite bundled）**，并配套内存测试模式与嵌入式 Migration 机制。

- **核心规范**：
  1. 依赖声明：`rusqlite = { version = "0.32", features = ["bundled"] }`。
  2. 性能调优：连接初始化必须执行 `PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;`。
  3. 执行模型：所有数据库写操作必须放在专用的同步线程或 `spawn_blocking` 中，防止阻塞 `tokio` worker。

**决议日期**：2026-04-26
**达成方式**：Master Claude 提议，Gemini 补强同步执行模型、PRAGMA 配置与容量论证并确认。
