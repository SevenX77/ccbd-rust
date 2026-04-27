# S-2：SQLite 权威数据源建模 (SoT Schema Design)

> **设计哲学**：关系型数据库是系统状态的唯一事实来源（SoT）。本 Schema 必须为 L2 调度层提供事务级的一致性（针对 S-1 CAS），提供严格排序的时间轴（针对 R-RECONNECT-1），并完美支撑异常反馈闭环（针对 R-STATE-FALLBACK-LOOP）。

## 1. 核心 Schema 设计 (SQL DDL)

**前置约束**：所有数据库连接初始化时，必须执行 `PRAGMA foreign_keys = ON; PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL;`。
建表语句统一采用 SQLite 3.37+ 引入的 `STRICT` 模式，拒绝动态类型推断。

### 1.1 `projects` (工作区表)
物理工作区的逻辑映射，解决不同用户项目间的隔离。

```sql
CREATE TABLE projects (
    id TEXT PRIMARY KEY,
    absolute_path TEXT NOT NULL UNIQUE,
    created_at INTEGER NOT NULL DEFAULT (unixepoch())
) STRICT;
```

### 1.2 `sessions` (会话拓扑表)
承载 L3 (Master Claude) 的生命周期。配合 A-6 决议，如果 `master_pid` 消失，关联的 Agents 必须被清理。

```sql
CREATE TABLE sessions (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    master_pid INTEGER NOT NULL, -- A-6: 用于 pidfd 级联清理的监控目标
    created_at INTEGER NOT NULL DEFAULT (unixepoch())
) STRICT;
```

### 1.3 `agents` (核心状态机表)
S-1 状态机的物理持久化载体。使用 `state_version` 进行乐观锁控制。

```sql
CREATE TABLE agents (
    id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    provider TEXT NOT NULL,         -- 代理类型，例如 'gemini', 'codex'
    state TEXT NOT NULL,            -- S-1 状态：SPAWNING, IDLE, BUSY, UNKNOWN, CRASHED, KILLED
    sub_state TEXT,                 -- 附加标记：Matched, Asserted (针对 IDLE)
    state_version INTEGER NOT NULL DEFAULT 1, -- CAS 并发控制字段
    pid INTEGER,                    -- OS 进程 ID，SPAWNING/CRASHED 时可能为 NULL
    exit_code INTEGER,              -- OS 退出码
    error_code TEXT,                -- A-7 决议的结构化错误，例如 'PTY_MARKER_TIMEOUT'
    created_at INTEGER NOT NULL DEFAULT (unixepoch()),
    updated_at INTEGER NOT NULL DEFAULT (unixepoch())
) STRICT;

-- 索引：加速调谐循环（Reconciliation Loop）查找存活 Agent
CREATE INDEX idx_agents_active ON agents(state) WHERE state NOT IN ('CRASHED', 'KILLED');
```

### 1.4 `events` (事件溯源表)
R-RECONNECT-1 断线重连与 R-IDEMPOTENCY-1 请求幂等性的基石。

```sql
CREATE TABLE events (
    seq_id INTEGER PRIMARY KEY AUTOINCREMENT, -- 严格递增的事件流水号，替代 UUID
    agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    request_id TEXT,                -- 发起请求的幂等性 Key
    event_type TEXT NOT NULL,       -- 枚举：'output_chunk', 'state_change', 'delivery_ack'
    payload TEXT NOT NULL,          -- JSON 文本负载
    created_at INTEGER NOT NULL DEFAULT (unixepoch())
) STRICT;

-- 索引 1：加速 Caller 拉取增量流 (since_event_id)
CREATE INDEX idx_events_agent_seq ON events(agent_id, seq_id);
-- 索引 2：R-IDEMPOTENCY-1 防重放，每个 Agent 唯一，排除 NULL
CREATE UNIQUE INDEX idx_events_idempotent ON events(agent_id, request_id) WHERE request_id IS NOT NULL;
```

### 1.5 `evidence` (异常反馈闭环表)
支撑议题 1b 的 7 步反馈闭环。

```sql
CREATE TABLE evidence (
    id TEXT PRIMARY KEY,
    agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    event_seq_id INTEGER NOT NULL REFERENCES events(seq_id), -- 关联触发 UNKNOWN 的事件
    pane_bytes BLOB NOT NULL,       -- 原始 vt100 内存快照
    failed_rules TEXT NOT NULL,     -- JSON 数组，记录当时尝试失败的规则指纹
    status TEXT NOT NULL DEFAULT 'PENDING', -- PENDING, REVIEWED, DISCARDED
    l3_asserted_state TEXT,         -- 记录 L3 的断言结果
    created_at INTEGER NOT NULL DEFAULT (unixepoch())
) STRICT;

-- 索引：Maintainer 审查队列查询
CREATE INDEX idx_evidence_pending ON evidence(status, created_at) WHERE status = 'PENDING';
```

---

## 2. 关键事务控制实例 (SQL Transactions)

### 2.1 状态机的 CAS 幂等转移 (Push vs Polling)
在 S-1 中定义的竞争条件，物理实现如下。
**防偏航**：必须在同一个事务中更新状态并写入 `events` 表，确保事件流与最终状态严格一致。

```sql
BEGIN IMMEDIATE;

-- 1. CAS 更新：必须检查当前 state 与 version
UPDATE agents 
SET 
    state = 'CRASHED',
    error_code = 'AGENT_UNEXPECTED_EXIT',
    state_version = state_version + 1,
    updated_at = unixepoch()
WHERE 
    id = 'ag_123' AND 
    state_version = 5 AND 
    state NOT IN ('CRASHED', 'KILLED');

-- 2. 仅当上述 UPDATE 成功时（SQLite 返回 changes() == 1），插入对应的状态流转事件
INSERT INTO events (agent_id, event_type, payload) 
VALUES ('ag_123', 'state_change', '{"from": "BUSY", "to": "CRASHED", "reason": "AGENT_UNEXPECTED_EXIT"}');

COMMIT;
```

### 2.2 请求幂等性执行 (R-IDEMPOTENCY-1)
当 L3 发送 `agent.send` 并带有 `request_id` 时：

```sql
BEGIN IMMEDIATE;

-- 1. 尝试插入事件（若 request_id 已存在，触发 UNIQUE 约束冲突，事务报错）
INSERT INTO events (agent_id, request_id, event_type, payload) 
VALUES ('ag_123', 'req_abc987', 'command_received', '{"cmd": "/clear\n"}');

-- 2. 执行状态机流转 (IDLE -> BUSY)
UPDATE agents 
SET 
    state = 'BUSY',
    state_version = state_version + 1,
    updated_at = unixepoch()
WHERE 
    id = 'ag_123' AND state = 'IDLE' AND state_version = 6;

COMMIT;
```
*如果在第一步抛出 `SQLITE_CONSTRAINT_UNIQUE` 异常，Daemon 拦截异常并直接返回对应的最后一次状态给 L3，而不写入 PTY，实现幂等。*

### 2.3 Evidence 记录与密封 (R-STATE-FALLBACK-LOOP)
当 MarkerTimer 溢出触发 `UNKNOWN` 时：

```sql
BEGIN IMMEDIATE;

-- 1. 转移状态
UPDATE agents 
SET state = 'UNKNOWN', state_version = state_version + 1 
WHERE id = 'ag_123' AND state = 'BUSY' AND state_version = 7;

-- 2. 写入事件
INSERT INTO events (agent_id, event_type, payload) 
VALUES ('ag_123', 'state_change', '{"to": "UNKNOWN"}');

-- 3. 提取刚刚插入的 seq_id 作为外键
INSERT INTO evidence (id, agent_id, event_seq_id, pane_bytes, failed_rules)
VALUES ('evi_888', 'ag_123', last_insert_rowid(), X'1B5B324A...', '["rule_v2", "rule_v3"]');

COMMIT;
```

---

## 3. 防偏航架构说明

1. **为什么用 `INTEGER PRIMARY KEY AUTOINCREMENT`？**
   对于事件溯源系统，如果不用 `AUTOINCREMENT`，SQLite 可能会重用被删除的 ID（`ROWID` 回绕）。使用它可以确保 `seq_id` 绝对递增，满足 R-RECONNECT-1 中 L3 仅靠一个 `since_event_id` 指针即可无缝拉取全部断线事件。
2. **容量与清理边界（Retention）**
   `PRAGMA foreign_keys = ON` 结合 `ON DELETE CASCADE` 让调谐循环只需执行 `DELETE FROM sessions WHERE id = ?` 即可瞬间自动清理下属的 `agents`, `events`, `evidence`。这对于维护高性能的 SQLite 数据库至关重要。
