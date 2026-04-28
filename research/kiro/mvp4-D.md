# Kiro Design: MVP 4 (反思闭环 / The Feedback)

> **文档定位**：本文件是 ccbd-rust MVP 4 阶段的官方 D (Design) 规格。严格基于 mvp4-R.md 边界，为 Codex T 阶段提供无歧义实施蓝图。本阶段把系统从「感知状态机」升级为「具备反思能力的鲁棒系统」——通过激活 evidence 表 + agent.assert_state / agent.discard_evidence / system.dump 三个新 RPC，闭合 R-STATE-FALLBACK-LOOP。

---

## 1. 状态机 Delta

### 1.1 UNKNOWN 出口路径

| 触发源 | 目标状态 | 副作用 |
|---|---|---|
| `agent.assert_state` | IDLE(sub_state=Asserted) | evidence.status='REVIEWED' + l3_asserted_state='IDLE' + state_change reason=`L3_ASSERTED` |
| `agent.send` (新 request_id) | BUSY | 走标准 PENDING→PTY→SENT 状态机；evidence 留 PENDING 不动 |
| `agent.kill` | KILLED | 沿用 MVP2 |
| `pidfd.death` | CRASHED | 沿用 MVP2 |

### 1.2 agent.send 状态校验放宽

mvp3 是 `state == 'IDLE'` 拒非；mvp4 改为 `state IN ('IDLE','UNKNOWN')` 放行：

```rust
// handle_agent_send 内（mvp3-D §1.3 顺序保留）
// 1. 幂等检查（沿用 mvp3）
// 2. state 校验（mvp4 放宽）
let s = query_agent_state(...)?;
if s != "IDLE" && s != "UNKNOWN" {
    return Err(AgentWrongState { current_state: s });
}
// 3. PENDING → PTY → SENT（沿用 mvp1+mvp3）
```

UNKNOWN → BUSY 转移本质是 send 的合法路径，不是新方法。**evidence 表不动**——保留 PENDING 让 L3 后续仍可拿快照分析。

### 1.3 CAS 协议

`agent.assert_state` 的 CAS：必须带 `state='UNKNOWN'` + `state_version=?` 双校验：

```sql
UPDATE agents
SET state = 'IDLE', sub_state = 'Asserted',
    state_version = state_version + 1, updated_at = unixepoch()
WHERE id = ? AND state = 'UNKNOWN' AND state_version = ?
```

CAS 失败（changes==0）说明：a) agent 已被 kill 或 crashed；b) 极小概率 vt100 又命中转 IDLE_Matched。两种情况都拒绝 assert，返回 `AGENT_WRONG_STATE`。

`mark_agent_unknown` 的 CAS（mvp3 既有 + mvp4 改造）：

```sql
UPDATE agents
SET state = 'UNKNOWN', error_code = ?, state_version = state_version + 1, updated_at = unixepoch()
WHERE id = ? AND state IN ('SPAWNING','BUSY') AND state_version = ?
```

WHERE 排除 CRASHED/KILLED/UNKNOWN/IDLE 等已不需要再转 UNKNOWN 的状态。

---

## 2. Schema Delta

### 2.1 evidence 表（MVP4 真正首次创建）

**重要纠正**：mvp1 spec 设计了 evidence 表但 src/db/schema.rs 实际**没有**落地（mvp1+mvp2+mvp3 全部跳过）。MVP4 是 evidence 表的真正首次创建：

```sql
-- MVP4 在 db::init 中新增（用 IF NOT EXISTS 让重启幂等）
CREATE TABLE IF NOT EXISTS evidence (
    id TEXT PRIMARY KEY,
    agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    event_seq_id INTEGER NOT NULL REFERENCES events(seq_id),
    pane_bytes BLOB NOT NULL,           -- 200x200 UTF-8 屏幕快照
    failed_rules TEXT NOT NULL,         -- 当时尝试的 marker 正则数组 (JSON)
    status TEXT NOT NULL DEFAULT 'PENDING',
    -- status 合法值：PENDING / REVIEWED / DISCARDED / SEALED
    -- 不加 CHECK 约束以保留扩展空间
    l3_asserted_state TEXT,
    created_at INTEGER NOT NULL DEFAULT (unixepoch())
) STRICT;

CREATE INDEX IF NOT EXISTS idx_evidence_pending ON evidence(status, created_at) WHERE status = 'PENDING';
CREATE INDEX IF NOT EXISTS idx_evidence_agent ON evidence(agent_id);
```

放在 `src/db/schema.rs` 的 SCHEMA SQL 模板里跟其它 4 张表一起，db::init 时一次性 CREATE。`IF NOT EXISTS` 让对已有 dev_state 数据库（无 evidence 表）也能补建。

### 2.2 agents 表保持

不动 agents schema。`sub_state='Asserted'` 与 `'Matched'` 共存（同字段不同值），CAS 正确流转。

---

## 3. RPC 契约 Delta

### 3.1 `agent.assert_state` (新增)

强制把 agent 从 UNKNOWN 拉回 IDLE_Asserted。

* **Params**: `{"agent_id": "ag_1", "state": "IDLE", "evidence_id": "evi_..."}`
* **Returns**: `{"state": "IDLE", "sub_state": "Asserted"}`
* **可能 error_code**: `AGENT_NOT_FOUND` / `AGENT_WRONG_STATE` / `DB_EVIDENCE_NOT_FOUND` / `IPC_INVALID_REQUEST` (state 字段非 "IDLE")

* **执行逻辑伪代码**:
    ```rust
    // 0. 入参校验
    if params.state != "IDLE" {
        return Err(IpcInvalidRequest { details: "assert_state only accepts state=IDLE" });
    }

    // 1. 事务开启
    BEGIN IMMEDIATE;

    // 2. 验证 evidence 存在 + 归属正确（防越权）
    let evi = SELECT id, agent_id, status FROM evidence WHERE id = ?;
    if evi.is_none() || evi.agent_id != params.agent_id {
        ROLLBACK; return Err(DB_EVIDENCE_NOT_FOUND);
    }

    // 3. 读 agent 当前 state + state_version
    let (current_state, version) = SELECT state, state_version FROM agents WHERE id = ?;
    if current_state.is_none() {
        ROLLBACK; return Err(AGENT_NOT_FOUND);
    }
    if current_state != "UNKNOWN" {
        ROLLBACK; return Err(AGENT_WRONG_STATE { current_state });
    }

    // 4. CAS 转移 agent
    let changes = UPDATE agents
        SET state='IDLE', sub_state='Asserted', state_version=state_version+1, updated_at=unixepoch()
        WHERE id=? AND state='UNKNOWN' AND state_version=?;
    if changes == 0 {
        // 极端竞态（vt100 在事务边界刚抢先转 IDLE_Matched）
        ROLLBACK; return Err(AGENT_WRONG_STATE { current_state: <重读> });
    }

    // 5. 标记 evidence
    UPDATE evidence SET status='REVIEWED', l3_asserted_state='IDLE' WHERE id=?;

    // 6. 插入 state_change
    INSERT INTO events (agent_id, event_type, payload)
    VALUES (?, 'state_change',
        json!({"from":"UNKNOWN","to":"IDLE","sub_state":"Asserted","reason":"L3_ASSERTED","evidence_id":?}).to_string()
    );

    COMMIT;

    return Ok({"state":"IDLE", "sub_state":"Asserted"});
    ```

### 3.2 `agent.discard_evidence` (新增)

标记 evidence 无效或敏感数据（不影响 agents 状态）。

* **Params**: `{"evidence_id": "evi_..."}`
* **Returns**: `{"status": "DISCARDED"}`
* **可能 error_code**: `DB_EVIDENCE_NOT_FOUND`
* **逻辑**: 单 UPDATE：
    ```sql
    UPDATE evidence SET status='DISCARDED' WHERE id=? AND status NOT IN ('DISCARDED');
    ```
    changes==0 → 返回 `DB_EVIDENCE_NOT_FOUND`（包括"已是 DISCARDED 重复调"，按 mvp4-R AC3 语义这条算成功幂等返回；实施时按方便选）。

### 3.3 `system.dump` (新增)

全量快照，调试期用。

* **Params**: `{}`
* **Returns**:
    ```json
    {
      "projects": [...],
      "sessions": [...],
      "agents": [{"id","session_id","provider","state","sub_state","state_version","pid","exit_code","error_code","created_at","updated_at"}, ...],
      "evidence_pending": [{"id","agent_id","status","created_at"}, ...]
    }
    ```
    `pane_bytes` 与 `failed_rules` **不**在响应里（避免 BLOB 撑爆 JSON-RPC 单帧）；L3 需要看现场则用 sqlite3 CLI 直读 `target/dev_state/ccbd.sqlite` 的 evidence 表。

* **逻辑**: 4 个独立 `SELECT` 拼装 JSON 返回。read-only 不需事务。

### 3.4 `agent.send` 状态校验放宽

handle_agent_send 改 mvp3 的 state 校验逻辑：

```diff
- if s != "IDLE" {
+ if s != "IDLE" && s != "UNKNOWN" {
      return Err(AgentWrongState { current_state: s });
  }
```

注意保留 mvp3 的「先幂等检查 → 后 state 校验」顺序。

---

## 4. Evidence 写入算法（mark_agent_unknown 改造）

mvp3 既有 `mark_agent_unknown(db, agent_id, reason)` 是 stub 只转 state；mvp4 改为完整版**事务**：

```rust
pub fn mark_agent_unknown(
    db: &Db,
    agent_id: &str,
    reason: &str,
    pane_bytes: Vec<u8>,        // mvp4 新加：vt100 屏幕快照
    failed_rules: serde_json::Value,  // mvp4 新加：当时 marker regex 列表
) -> Result<usize, CcbdError> {
    let mut conn = db.conn();
    let tx = conn.transaction()?;

    // 0. 读当前 state + version（用于 CAS）
    let (current_state, version) = SELECT state, state_version FROM agents WHERE id = ?;
    if current_state.is_none() { tx.rollback()?; return Ok(0); }

    // 1. CAS 转 UNKNOWN（必须先做 CAS，再做后续 SEAL/INSERT；
    //    否则 CAS 失败时 SEAL 已动旧 evidence 是 dirty 行为）
    let changes = tx.execute(
        "UPDATE agents SET state='UNKNOWN', error_code=?,
         state_version=state_version+1, updated_at=unixepoch()
         WHERE id=? AND state IN ('SPAWNING','BUSY') AND state_version=?",
        (reason, agent_id, version),
    )?;
    if changes == 0 {
        tx.rollback()?;
        return Ok(0); // CAS 失败：agent 已是终态或并发转移
    }

    // 2. SEAL 之前的 PENDING evidence（在 CAS 成功之后才执行）
    tx.execute(
        "UPDATE evidence SET status='SEALED' WHERE agent_id=? AND status='PENDING'",
        [agent_id],
    )?;

    // 3. 写 state_change 事件，记下 seq_id 给 evidence 外键用
    tx.execute(
        "INSERT INTO events (agent_id, event_type, payload) VALUES (?, 'state_change', ?)",
        (agent_id, &json!({"to":"UNKNOWN","reason":reason,"from":<current_state>}).to_string()),
    )?;
    let event_seq_id: i64 = tx.last_insert_rowid();

    // 4. 写 evidence 现场
    let evidence_id = format!("evi_{}", uuid::Uuid::new_v4().simple());
    tx.execute(
        "INSERT INTO evidence (id, agent_id, event_seq_id, pane_bytes, failed_rules)
         VALUES (?, ?, ?, ?, ?)",
        (&evidence_id, agent_id, event_seq_id, pane_bytes, failed_rules.to_string()),
    )?;

    tx.commit()?;
    Ok(changes)
}
```

**关键时序**：步骤顺序是 CAS → SEAL → state_change → INSERT evidence，CAS 失败时 ROLLBACK 不动 evidence 旧数据。

---

## 5. system.dump 算法

```rust
pub fn handle_system_dump(db: &Db, _params) -> Result<Value, CcbdError> {
    let conn = db.conn();
    let projects = conn.prepare("SELECT id, absolute_path, created_at FROM projects")?
        .query_map([], |row| ...)?.collect::<Result<Vec<_>, _>>()?;
    let sessions = conn.prepare("SELECT id, project_id, master_pid, created_at FROM sessions")?
        ...;
    let agents = conn.prepare(
        "SELECT id, session_id, provider, state, sub_state, state_version, pid, exit_code, error_code, created_at, updated_at FROM agents"
    )?...;
    let evidence_pending = conn.prepare(
        "SELECT id, agent_id, status, created_at FROM evidence WHERE status = 'PENDING' ORDER BY created_at DESC LIMIT 100"
    )?...;
    Ok(json!({"projects":projects,"sessions":sessions,"agents":agents,"evidence_pending":evidence_pending}))
}
```

read-only 多个 SELECT 不需事务。`evidence_pending` 限 100 条防过大。

---

## 6. 错误码 Delta

`src/error.rs` 新增 1 个变体：

- `DbEvidenceNotFound { details: String }` → error_code `"DB_EVIDENCE_NOT_FOUND"`，data.details 含 evidence_id 或 "agent_id mismatch" 说明

`to_rpc_error` 同 mvp1-D §4 风格：code -32000 + error_code + data.details。

---

## 7. 模块布局 Delta

```
src/
├── db/
│   ├── queries.rs       // [修改] mark_agent_unknown 改造为完整事务版（加 pane_bytes / failed_rules 参数）
│   │                    // [新增] update_evidence_status / query_evidence_by_id / handle_system_dump 用的查询 helper
│   ├── schema.rs        // [修改] db::init 加 CREATE INDEX IF NOT EXISTS idx_evidence_agent
│   └── mod.rs           // [复用]
├── marker/
│   └── timer.rs         // [修改] timer 超时回调拿 vt100 屏幕 snapshot + failed_rules 传给 mark_agent_unknown
│                        //         需要 marker::registry 提供 with_parser_borrow 这种 helper 让 timer task 拿到 parser 引用
│                        //         或者更简单：spawn_marker_timer_task 入参增加 parser_handle: Arc<Mutex<vt100::Parser>> 直接 move 给 task
├── rpc/
│   ├── handlers.rs      // [新增] handle_agent_assert_state / handle_agent_discard_evidence / handle_system_dump
│   │                    // [修改] handle_agent_send 第二步 state 校验从 == "IDLE" 改为 IN ("IDLE","UNKNOWN")
│   └── router.rs        // [修改] 注册 "agent.assert_state" / "agent.discard_evidence" / "system.dump" 三个新方法
├── error.rs             // [修改] 加 DbEvidenceNotFound 变体
├── monitor/             // [复用 MVP2]
├── sandbox/             // [复用 MVP2]
└── ...
```

---

## 8. evidence pane_bytes 序列化策略

* **提取点**：marker timer task 超时时，调 `parser.screen().contents()` 拿 String（200x200 字符 UTF-8）。
* **打包**：直接 `String::into_bytes()` 作 BLOB 入库。**不**做 gzip/zstd 压缩——200x200 ≈ 40-80KB 不大，压缩节省的存储不抵失去 sqlite3 CLI 直接 `cat` 看的便利。
* **存活时间**：未来可能加 retention（如 evidence DISCARDED 后 N 天清），mvp4 不做（沿用 ON DELETE CASCADE 让 agents 删时一起删）。

### 8.1 timer task 与 parser 共享方案

`marker::timer.rs` 的 spawn_marker_timer_task 接收 `parser_handle: Arc<Mutex<vt100::Parser>>`（mvp3 reader task 持有的同一份），timer 超时调 `parser_handle.lock().screen().contents().into_bytes()` 拿 snapshot。注意 lock 持有时间极短（一次 contents() 拷贝），不会跟 reader task 长时间冲突。

`failed_rules` 参数由 timer task 调用方静态构造：MVP4 阶段就一组 hardcoded regex，`json!(["[\\$#>✦]\\s*$"]).to_string()` 即可。后续 MVP 如果让 marker rule 可配置再动。

---

## 9. 依赖 Delta

* **无新增依赖**。
* `uuid` (mvp1 已有) 用于生成 `evidence.id`。
* `serde_json` (mvp1 已有) 用于序列化 `failed_rules` / `payload`。
