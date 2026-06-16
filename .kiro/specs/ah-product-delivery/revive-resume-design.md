# revive/resume 机制正式设计

> SOP-08 §1.1 step 1e。输入: PM 决策、a1 research、a2 思路、1d audit 收敛、master 生命周期契约收敛。本文只定义设计, 不包含实现代码。

## 0. 目标与非目标

目标对齐 PM 三条硬约束:

1. 每个 agent, 含 master, 要有可判定的即时执行状态。
2. 只有任务执行中被 kill 才自动 revive; 空闲被 kill 不自动拉起, 但仍必须清理防孤儿。
3. revive 后恢复 kill 前 session 上下文, 并自动输入“继续”接原活。

非目标:

- 不做整机重启后自动启动 ahd。PM 已明确 ahd 是项目级, 现有 transient `systemd-run --unit=ahd.service --property=Restart=on-failure` 已够。
- 不给 managed master 加端侧 heartbeat/instrumentation。按 a2 推荐, master 执行态由 ActiveWork 代理定义; 该诠释仍需 PM 确认。
- 不新增 `RECOVERED` job 状态; 避免牵动 wait/cancel/dispatch/API 语义。

## 0.5 继承字段表

### `agents`

现有 schema: `src/db/schema.rs:50-66`; Rust struct: `src/db/schema.rs:189-203`。

| 字段 | 类型 | 现状用途 | 本设计 |
|---|---:|---|---|
| `id` | `TEXT PRIMARY KEY` | agent id | 不改 |
| `session_id` | `TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE` | 归属 session | 不改 |
| `provider` | `TEXT NOT NULL` | provider 名 | 不改 |
| `state` | `TEXT NOT NULL` | agent 生命周期/执行态: `IDLE`, `WAITING_FOR_ACK`, `BUSY`, `CRASHED`, `KILLED` 等 | 不改含义; recovery gate 不再只看当前 `CRASHED`, 还读 [NEW] recovery intent |
| `state_version` | `INTEGER NOT NULL DEFAULT 1` | CAS 版本 | 不改; recovery intent 记录 crash 后版本用于审计/幂等 |
| `pid` | `INTEGER` | provider 进程 pid | 不改 |
| `exit_code` | `INTEGER` | crash exit code | 不改 |
| `error_code` | `TEXT` | crash/error reason | 不改 |
| `created_at` | `INTEGER NOT NULL DEFAULT unixepoch()` | 创建时间 | 不改 |
| `sub_state` | `TEXT` | marker/log-event 子状态 | 不改 |
| `config_hash` | `TEXT` | realign/recovery 配置 hash | 不改 |
| `retry_count` | `INTEGER NOT NULL DEFAULT 0` | recovery backoff | 不改 |
| `next_retry_at` | `INTEGER` | recovery backoff | 不改 |
| `retry_exhausted` | `INTEGER NOT NULL DEFAULT 0` | recovery fuse | 不改 |
| `updated_at` | `INTEGER NOT NULL DEFAULT unixepoch()` | 更新时间 | 不改 |

### `jobs`

现有 schema: `src/db/schema.rs:110-128`; Rust struct: `src/db/schema.rs:228-235` 起。状态迁移证据: `src/db/jobs.rs:43`, `src/db/jobs.rs:212`, `src/db/jobs.rs:308`, `src/db/jobs.rs:410`, `src/db/jobs.rs:431`。

| 字段 | 类型 | 现状用途 | 本设计 |
|---|---:|---|---|
| `id` | `TEXT PRIMARY KEY` | job id | 不改 |
| `agent_id` | `TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE` | 归属 agent | 不改 |
| `request_id` | `TEXT` | 幂等请求 id | 不改 |
| `prompt_text` | `TEXT NOT NULL` | 原始任务输入 | 不改; auto-continue 优先复用 interrupted job 的上下文, 不覆盖原 prompt |
| `reply_text` | `TEXT` | 完成/取消回复 | 不改 |
| `status` | `TEXT NOT NULL DEFAULT 'QUEUED'` | `QUEUED` / `DISPATCHED` / `COMPLETED` / `FAILED` / `CANCELLED`; 无 CHECK | 不新增 `RECOVERED`; interrupted `FAILED` job 在 revive 后受控翻回 `QUEUED` 走现有 dispatch |
| `error_reason` | `TEXT` | failed reason | 不改; revive requeue 时追加/替换为 `RECOVERY_REQUEUED` 或保留历史到 recovery intent event |
| `created_at` | `INTEGER NOT NULL DEFAULT unixepoch()` | 创建时间 | 不改 |
| `dispatched_at` | `INTEGER` | 派发时间 | 不改; requeue 时清空 |
| `dispatched_at_seq_id` | `INTEGER` | dispatch event 边界 | 不改; requeue 时清空, 下一次 dispatch 重新写 |
| `completed_at` | `INTEGER` | 终态时间 | 不改; requeue 时清空 |
| `cancel_requested` | `INTEGER NOT NULL DEFAULT 0` | 取消请求标记 | 不改 |
| `requires_physical_evidence` | `INTEGER NOT NULL DEFAULT 0` | 证据约束 | 不改 |
| `requires_test_evidence` | `INTEGER NOT NULL DEFAULT 0` | 证据约束 | 不改 |

### `sessions`

现有 schema: `src/db/schema.rs:8-20`; master runtime 查询/更新由 `src/master_revival.rs:61`, `src/master_revival.rs:156` 等使用。

| 字段 | 类型 | 现状用途 | 本设计 |
|---|---:|---|---|
| `id` | `TEXT PRIMARY KEY` | session id | 不改 |
| `project_id` | `TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE` | 项目 | 不改 |
| `master_pid` | `INTEGER NOT NULL` | managed master pid | 不改 |
| `master_pane_id` | `TEXT` | managed master pane | 不改 |
| `status` | `TEXT NOT NULL DEFAULT 'ACTIVE'` | session 状态 | 不改 |
| `config_hash` | `TEXT` | master config hash | 不改 |
| `master_retry_count` | `INTEGER NOT NULL DEFAULT 0` | master revive backoff/fuse | 不改 |
| `master_next_retry_at` | `INTEGER NOT NULL DEFAULT 0` | master revive backoff | 不改 |
| `master_generation` | `INTEGER NOT NULL DEFAULT 0` | master CAS/fencing generation | 不改 |
| `master_last_exit_reason` | `TEXT` | master exit/revive reason | 不改 |
| `created_at` | `INTEGER NOT NULL DEFAULT unixepoch()` | 创建时间 | 不改 |

### `agent_spawn_specs`

现有 schema: `src/db/schema.rs:70-77` 和 `src/db/recovery.rs:8-17`; snapshot struct: `src/db/recovery.rs:25-37`; persist/query: `src/db/recovery.rs:54`, `src/db/recovery.rs:77`, `src/db/recovery.rs:110`。

| 字段 | 类型 | 现状用途 | 本设计 |
|---|---:|---|---|
| `agent_id` | `TEXT PRIMARY KEY REFERENCES agents(id) ON DELETE CASCADE` | 对应 agent | 不改 |
| `spec_version` | `INTEGER NOT NULL DEFAULT 1` | snapshot 版本 | 不改 |
| `provider` | `TEXT NOT NULL` | provider | 不改 |
| `config_hash` | `TEXT NOT NULL` | expected config hash | 不改 |
| `spec_json` | `TEXT NOT NULL` | serialized `AgentSpawnSpec`: `agent_id`, `provider`, `env`, `hooks`, `plugins`, `sandbox_overrides` | 不改; resume 仍复用该 spawn spec |
| `updated_at` | `INTEGER NOT NULL DEFAULT unixepoch()` | 更新时间 | 不改 |

### [NEW] `agent_recovery_intents`

新增表, 不复用 `events` 作为唯一事实源。理由: recovery gate 需要稳定查询、幂等 claim、可测试的业务字段; `events.payload` 适合审计, 不适合作为恢复决策主表。

```sql
CREATE TABLE IF NOT EXISTS agent_recovery_intents (
    agent_id TEXT PRIMARY KEY REFERENCES agents(id) ON DELETE CASCADE,
    session_id TEXT NOT NULL,
    provider TEXT NOT NULL,
    previous_state TEXT NOT NULL,
    crashed_state_version INTEGER NOT NULL,
    interrupted_job_id TEXT,
    interrupted_job_status TEXT,
    interrupted_job_request_id TEXT,
    interrupted_job_prompt_text TEXT,
    interrupted_job_cancel_requested INTEGER,
    interrupted_job_requires_physical_evidence INTEGER,
    interrupted_job_requires_test_evidence INTEGER,
    action TEXT NOT NULL CHECK(action IN ('REVIVE', 'REAP_ONLY')),
    reason TEXT NOT NULL,
    consumed_at INTEGER,
    created_at INTEGER NOT NULL DEFAULT (unixepoch()),
    updated_at INTEGER NOT NULL DEFAULT (unixepoch())
) STRICT;

CREATE INDEX IF NOT EXISTS idx_agent_recovery_intents_action
ON agent_recovery_intents(action, consumed_at, created_at);
```

迁移:

- `db::init` 新增 idempotent DDL。
- 旧库中既有 `CRASHED` agent 没有 intent。迁移后第一次 recovery loop 对缺 intent 的 `CRASHED` eligible agent 按兼容策略处理: emit `self_recovery_attempt` action=`skipped`, reason=`missing_recovery_intent`, 不自动 revive。若需要手工恢复, 用已有 realign/agent.spawn 路径。
- 这是 [BREAKING]: 旧行为 “eligible CRASHED 必 revive” 改为 “必须有 `REVIVE` intent 才 revive”。

## 1. 执行态定义与采集

### Worker

执行中定义:

- `WAITING_FOR_ACK`
- `BUSY`

证据:

- state 常量在 `src/db/state_machine.rs:14-28`。
- 现有 `is_active_state` 包含 `SPAWNING | WAITING_FOR_ACK | BUSY`, 见 `src/db/state_machine.rs:56-60`。本设计收紧 revive gate: `SPAWNING` 是基建, 不算 task executing; `PROMPT_PENDING` 是等待人工/主控, 不算可自动继续的执行中。
- dispatch 会把 `QUEUED` job 改为 `DISPATCHED`, 并通过 `transit_agent_state_conn_sync` 把 agent 转到新状态, 见 `src/db/jobs.rs:210-229`。但 `DISPATCHED` 只是派发事实, 不保证 provider 已物理开始执行。

采集点:

- 必须在 crash-mark 事务内读取 `previous_state` 和当时的 `DISPATCHED` job, 立即写入 `agent_recovery_intents`。
- 原因: crash-mark 后当前 `agents.state` 会变成 `CRASHED`, 且 dispatched job 会变成 `FAILED`, 现场信号会被销毁。

### Master

执行中定义:

- 按 a2 推荐, managed master 的执行态由 ActiveWork 代理定义: session 内存在活跃 worker 或非终态 job。
- 现有实现: `snapshot_master_death_session_activity` 判断 worker state in `SPAWNING | WAITING_FOR_ACK | BUSY | PROMPT_PENDING`, 或存在 `QUEUED | DISPATCHED` job, 见 `src/db/system.rs:188-200`。
- `master_watch` 在 `src/monitor/master_watch.rs:104` 获取 snapshot, 在 `src/monitor/master_watch.rs:125-132` 对 `IdleNoWork` 只 reap 不 revive。

状态:

- 这是按 a2 推荐 + 待 PM 确认的诠释: 对 Master PM, 执行态由且仅由托管资源活跃度定义; 未进入派发阶段的纯端侧规划视为不可靠瞬态, 死即灭。
- 不加 master 端 heartbeat/instrumentation, 不修改 master 的 TUI/提示词协议。

## 2. Worker revive gate [BREAKING]

### 现状

- recovery loop 在 `src/orchestrator/mod.rs:219-225` 查询全部 `CRASHED` agent。
- 只 gate provider eligibility, 见 `src/orchestrator/mod.rs:227-230`。
- 随后检查 backoff/snapshot/CAS, 见 `src/orchestrator/mod.rs:231-289`。

### 命门

crash-mark 顺序:

1. `src/db/agents_lifecycle.rs:75-82` 读 `previous_state`。
2. `src/db/agents_lifecycle.rs:83-88` 更新 `agents.state = 'CRASHED'`。
3. `src/db/agents_lifecycle.rs:99-105` 调 `mark_dispatched_jobs_failed_for_agent_conn_sync`。
4. `src/db/jobs.rs:425-434` 把该 agent 的 `DISPATCHED` job 改成 `FAILED`。
5. `src/db/agents_lifecycle.rs:106-117` 插 state_change event。

因此 recovery 扫描时再查 `DISPATCHED` 是死路; 在途 job 信号已经消失。必须在 crash-mark 事务内保存 recovery intent。

### 设计

在 `mark_agent_crashed_sync` 同一事务内:

1. 读 `previous_state`。
2. 查询当前 `DISPATCHED` job: 复用 `query_dispatched_job_for_agent_sync`。
3. 计算 action:
   - `previous_state in ('WAITING_FOR_ACK', 'BUSY')` 或存在 interrupted `DISPATCHED` job: `REVIVE`。
   - `previous_state in ('IDLE', 'SPAWNING')` 且无 interrupted job: `REAP_ONLY`。
   - `KILLED` 不进入 crash path; 保持现有语义。
4. upsert `agent_recovery_intents`。
5. 继续现有 `mark_dispatched_jobs_failed_for_agent_conn_sync`。
6. 插现有 state_change event, 并额外插一条 `recovery_intent_recorded` event 作为审计, event 不是 gate 的唯一数据源。

Recovery loop 变化:

- `run_recovery_once_with_respawn` 对每个 `CRASHED` candidate 先读取 `agent_recovery_intents`。
- 无 intent: skipped `missing_recovery_intent`, 不 revive。
- intent.action = `REAP_ONLY`: 执行收尸清理, 标 consumed, 不 revive。
- intent.action = `REVIVE`: 继续现有 snapshot/backoff/CAS/delete-then-spawn 路径。

收尸清理:

- 不复活的 idle-crashed worker 不能只 `delete_agent`; 必须 reap 遗留 runtime。
- 复用 `clean_worker_runtime_resources_sync` 的核心能力, 当前实现会:
  - 清 marker/parser/completion registry, `src/db/system.rs:235-249`。
  - stop matching systemd scopes, `src/db/system.rs:252-284`。
  - fallback pidfd SIGKILL, `src/db/system.rs:294-317`。
  - remove monitors/registries, `src/db/system.rs:330-334`。
- 需要抽一个 agent-level cleanup helper, 避免调用现有函数时把 CRASHED 改成 KILLED 或误 stop session anchor。建议形态:
  - `clean_agent_runtime_resources_sync(db, session_id, agent_id, reason, daemon_marker)`。
  - 内部复用 scope/pidfd/registry 清理逻辑。
  - 对 `REAP_ONLY` 的 `CRASHED` agent: cleanup 后 `delete_agent`, intent 因 FK cascade 删除或先 consumed 后删除。

### 语义影响

- [BREAKING] `IDLE` worker OOM/crash 后不再自动 revive。
- [BREAKING] `PROMPT_PENDING` worker crash 不自动 revive; 这是等待人工/主控输入, 不能盲目继续。
- [BREAKING] 缺 recovery intent 的旧 `CRASHED` row 不再自动 revive。

## 3. Worker resume + auto-continue

### Resume

现有 provider resume 参数:

- eligible providers: `codex`, `claude`, `antigravity`, 见 `src/provider/manifest.rs:27-29`。
- Claude recovery args: `--continue`, 见 `src/provider/manifest.rs:31-34`。
- Codex recovery args: 读 `.codex/sessions/rollout-*.jsonl` session id, fallback `resume --last`, 见 `src/provider/manifest.rs:40-58` 和 `src/provider/manifest.rs:129-203`。
- Antigravity recovery args: conversation id 或 fallback `--continue`, 见 `src/provider/manifest.rs:62-84`。

恢复进程仍走现有 `spawn_realign_agent(..., is_recovery=true)`, 见 `src/orchestrator/mod.rs:208-216` 和 `src/orchestrator/mod.rs:296-325`。

### Auto-continue

不在 `state_machine` 中做:

- `mark_agent_idle_matched_outcome_sync` 是同步 DB 事务, 见 `src/db/state_machine.rs:292-299`。
- `mark_agent_idle_log_event_outcome_sync` 也是同步 DB 事务, 见 `src/db/state_machine.rs:447-459`。
- writer 是 async tmux IO, 见 `src/agent_io/writer.rs:15-60`。DB 层无 `Ctx` / pane / tmux context。

落点:

- 放在 orchestrator recovery post-ready 路径。
- `spawn_realign_agent` 当前 spawn 后会更新 agent `IDLE` 并持久化 snapshot, 见 `src/rpc/handlers/realign.rs:314-343`。
- recovery loop 在 `recover_crashed_agent_from_snapshot_with_respawn` 调用 respawn 后再 `apply_recovery_spawn_result`, 见 `src/orchestrator/mod.rs:324-340`。
- 在 `apply_recovery_spawn_result` 成功分支 emit recovered 事件后, 调度 `auto_continue_recovered_job(ctx, agent_id, intent)`。

Auto-continue 行为:

1. 用 delete-then-spawn 之前已捕获并传入的 intent 值读取 interrupted job 的 captured fields；不 DB 重读, 因 FK CASCADE 会随 delete crashed row 级联删 intent, 且 `jobs.agent_id ON DELETE CASCADE` 也会删掉原 job 行。
2. 如果 job 仍是 `FAILED` 且 error_reason 来自该 crash, 原子改回:
   - `status='QUEUED'`
   - `error_reason=NULL`
   - `completed_at=NULL`
   - `dispatched_at=NULL`
   - `dispatched_at_seq_id=NULL`
   - `cancel_requested=0`
3. 如果 job 行已被 delete-then-spawn 级联删除, 用 captured job value 复用原 `job_id` 重建 `QUEUED` job。复用原 id 便于保持 watch/idempotency 语义和审计连续性; 不引入 snapshot 表或 trigger。
3. 不直接调用 writer 注入自由文本, 而是 `wake_up()` 让现有 dispatch loop 消费 queued job。
4. 现有 dispatch 会通过 `send_text_to_pane_with_options` 把原 `prompt_text` 发给 pane, 见 `src/orchestrator/mod.rs:121-139`。

为什么不新增 `RECOVERED`:

- `job.wait` 把 `COMPLETED | FAILED | CANCELLED` 视为终态, 见 `src/rpc/handlers/jobs.rs:88-100`。
- cancel 只理解 `QUEUED` / `DISPATCHED` / terminal, 见 `src/rpc/handlers/jobs.rs:104-145`。
- dispatch 查询只消费 `QUEUED`, 见 `src/db/jobs.rs:195-215`。
- `RECOVERED` 会成为新非终态, 需要改 API/等待/取消/索引/事件语义。翻回 `QUEUED` 能直接复用现有 dispatch。

“输入继续”的解释:

- 对 worker, provider recovery args 先恢复 session 上下文; 重新派发 interrupted job 的原 prompt 是系统可验证、幂等的继续动作。
- 若后续 PM 要字面发送 “继续”, 可在 requeued prompt 前加 provider-specific continuation prefix, 但本设计不默认篡改用户原 prompt。

## 4. Master revive seed [BREAKING]

### 现状

- `revive_master_after_exit` 不做 conversation seed; 它写 redispatch marker, 设置 `AH_REDISPATCH_MARKER`, 复用/创建 master sandbox home, 然后 spawn, 见 `src/monitor/master_watch.rs:207-264`。
- marker 当前只是文件提示, 内容写 `redispatch_required: true`, 见 `src/monitor/master_watch.rs:496-520`。
- cutover seed 是 `seed_claude_project_conversation(old_home, master_home, cwd, handoff_path)`, 从 old home 复制 Claude project conversation 到 master home, 见 `src/master_cutover.rs:88-120`。
- cutover 调用处有 `request.old_home`, 见 `src/rpc/handlers/sessions.rs:529-575`。

### 设计

Revive 时 master 已死, 没有独立 `old_home`; 现有 `master_sandbox_home` 就是唯一会话上下文来源。因此:

- 不直接复用 `seed_claude_project_conversation` 的 old_home -> master_home copy 签名。
- 保留并升级现有 redispatch marker, 让它成为 machine-readable continue marker。
- 继续复用 existing master home; 不复制、不覆盖 `.claude/projects`。

建议 marker 形态:

```json
{
  "session_id": "...",
  "expected_pid": 123,
  "observed_generation": 5,
  "revived_generation": 6,
  "worker_ids_to_reap": ["a1"],
  "redispatch_required": true,
  "continue_required": true,
  "continue_instruction": "继续。检查 AH_REDISPATCH_MARKER 中的 worker/job 状态, 对被中断且已恢复的任务继续推进; 不要重复已完成工作。",
  "interrupted_jobs": [
    {"agent_id": "a1", "job_id": "job_...", "status_before_reap": "DISPATCHED"}
  ]
}
```

接线:

- `snapshot_master_death_session_activity` 当前只返回 classification + worker ids, 见 `src/db/system.rs:166-222`。扩展为同时返回 active/queued/dispatched job 摘要, 或新增查询 helper。
- `revive_master_after_exit` 在 worker cleanup 后, transition claim 前后均可写 marker; 保持现有 `AH_REDISPATCH_MARKER` env 注入, 见 `src/monitor/master_watch.rs:245-249`。
- revived master 的 command 已带 `AH_STATE_DIR`, `CCB_SOCKET`, `AH_MASTER_ROLE`, 见 `src/monitor/master_watch.rs:238-244`。继续保留。

是否自动向 master pane 输入:

- 对 managed master,默认 master cmd 是 `claude --dangerously-skip-permissions --continue /remote-control`, 见 `src/cli/config.rs:169-170`。它应恢复 Claude 会话。
- 在 master spawn 成功并 pidfd 注册后, ahd 可用 tmux writer 向 master pane 注入 `continue_instruction`。这不是 state_machine 层; 接点在 `revive_master_after_exit` 成功 spawn 后, `spawn_master_pidfd_watch_task` 后、`reprovision_declared_workers_after_master_revive` 前后均可。
- 注入必须 best-effort; 失败保留 marker + warn, 不阻断 master revive。

与 worker reprovision:

- 现有 master revive 会重新预建 declared workers 并恢复 sandbox_overrides snapshot, 见 `src/monitor/master_watch.rs:327-370`。
- 对被 master death 级联 KILLED 的 workers, 仍使用该 reprovision 机制; interrupted jobs 的恢复按 §3 requeue, 不新增 `RECOVERED`。

## 5. 端到端时序

### Worker OOM/crash 线

1. agent pidfd watch 确认进程死亡。
2. `mark_agent_crashed_sync` 同事务:
   - 读 `previous_state`。
   - 查 interrupted `DISPATCHED` job。
   - 写 `agent_recovery_intents`。
   - 标 `CRASHED`。
   - 将 interrupted `DISPATCHED` job 标 `FAILED`。
   - 插 state_change + recovery_intent_recorded events。
3. orchestrator recovery tick 扫 `CRASHED`。
4. 读 intent:
   - `REAP_ONLY`: clean runtime, delete row, consumed/skipped event。
   - `REVIVE`: 检查 provider eligibility/backoff/spawn snapshot/CAS。
5. `REVIVE` 时 delete crashed row, `spawn_realign_agent(is_recovery=true)`。
6. provider 用 resume args 恢复上下文。
7. spawn 成功后 emit recovered。
8. `auto_continue_recovered_job` 把 interrupted `FAILED` job 翻回 `QUEUED`, 清 dispatch/completion 字段, `wake_up()`。
9. 普通 dispatch loop 把原 prompt 发回 pane, 任务重新进入 `WAITING_FOR_ACK/BUSY`。

### Managed master OOM/crash 线

1. `master_watch` pidfd 确认 master 死亡。
2. `snapshot_master_death_session_activity` 判断 ActiveWork/IdleNoWork。
3. 先 cleanup/reap workers: scope stop + pidfd SIGKILL + registry cleanup + KILLED, 见 `src/db/system.rs:224-354`。
4. `IdleNoWork`: 不 revive master, 退出。
5. `ActiveWork`: claim master transition + backoff/fuse。
6. 写升级后的 `AH_REDISPATCH_MARKER` / continue marker。
7. 复用现有 `master_sandbox_home`, 注入 `AH_STATE_DIR`, `CCB_SOCKET`, `AH_MASTER_ROLE`, `AH_REDISPATCH_MARKER`。
8. spawn revived master, register pidfd watch。
9. best-effort 注入 `continue_instruction` 到 master pane。
10. reprovision declared workers from `agent_spawn_specs`, 包含 `sandbox_overrides`。
11. interrupted jobs 由 recovery intent/requeue 路径继续。

## 6. 缺陷定性、三轴与风险

### 设计缺陷 vs 实现缺陷

| 项 | 定性 | 证据 | 影响 | 置信 |
|---|---|---|---|---|
| Worker eligible CRASHED 无 gate revive | 设计缺陷 / [BREAKING] 契约重画 | `src/orchestrator/mod.rs:219-230` | 高 | 高 |
| crash-mark 销毁 previous state / DISPATCHED 信号 | 命门实现缺陷, 必须先补数据模型 | `src/db/agents_lifecycle.rs:75-105`, `src/db/jobs.rs:425-434` | 高 | 高 |
| Master revive 不 seed/不 consume marker | 补全实现 | `src/monitor/master_watch.rs:207-264`, `src/monitor/master_watch.rs:496-520` | 高 | 高 |
| Auto-continue 缺失 | 补全实现 | writer 存在 `src/agent_io/writer.rs:15`; orchestrator dispatch 存在 `src/orchestrator/mod.rs:121-139` | 高 | 高 |
| Master 执行态用 ActiveWork 代理 | 设计选择, 待 PM 确认诠释 | `src/db/system.rs:188-200`, `src/monitor/master_watch.rs:125-132` | 中高 | 中高 |

### 风险

- Recovery intent 与 crash-mark 必须同事务提交, 否则 gate 会出现 CRASHED 但无 intent 的不确定状态。
- Requeue `FAILED` job 必须验证 error_reason/job id 与 intent 匹配, 防止用户已观察到失败后被意外重跑。
- Auto-continue 可能重复提交非幂等任务。设计上只对 crash-time interrupted job 生效, 并用 request_id/job_id 维持唯一性; 仍需测试覆盖。
- Master continue 注入是 best-effort, provider TUI 未 ready 时可能吞输入; 必须保留 marker 作为兜底凭证。
- `REAP_ONLY` cleanup 抽 helper 时不能误用 session-level cleanup, 避免 stop session anchor 或 KILL 其他 workers。
- 旧 `CRASHED` row 没有 intent 不自动 revive 是 [BREAKING] 但保守; 需要 release note。

## 6.5 1f AUDIT 收敛 (a3, 2026-06-16)

a3 1f 净判定: **design 忠实落实 1d 收敛 (3 must-fix 全落 + 防孤儿 gap 已补 + master 偷换概念已转待 PM), 无 drift, file:line 准确 → 可进 tests-first。** 2 点 clarify + 1 nit, 不构成回 1e:

- **[clarify, 进 tests-first 钉死]** intent-capture 时序命门: `agent_recovery_intents.agent_id` 是 FK ON DELETE CASCADE; REVIVE 走 delete-then-spawn (§3 step5 删 crashed agent row) → **级联删 intent**。auto-continue 在 respawn 之后用 `interrupted_job_id`, **必须用 delete 前捕获的 intent 值传入, 严禁 DB 重读** (重读会查不到→静默 no-op→不 requeue→PM #3 失效)。§3 落点已按值传参 (`auto_continue_recovered_job(ctx, agent_id, intent)`), 但 §3 行为 step1 "读取 agent_recovery_intents" 措辞要改成 "用已捕获的 intent 值"。tests-first 必须有一条断言: delete-then-spawn 后 auto-continue 仍能拿到 interrupted_job_id 并 requeue。
- **[round-2 三方收敛]** job 行同样被 `jobs.agent_id ON DELETE CASCADE` 删除, 只捕获 `interrupted_job_id` 不足以 requeue。最终方案升级为 captured-job-value struct lift: crash-mark 同事务内捕获 interrupted job 的 `job_id/request_id/prompt_text/cancel_requested/evidence flags` 等重建字段, REVIVE 成功后按 captured value 复用原 job id 重建 `QUEUED` job。明确不使用额外 snapshot 表或 trigger, 避免 orphan snapshot 泄漏和隐式 DB side-effect。
- **[PM-pending #2, 待 PM 拍, 不阻塞 worker 实施]** PM #3 "输入继续" 诠释: (a) worker = 翻回 QUEUED 重派**原 prompt** (provider --continue 已恢复上下文, 幂等可验) 还是字面注入"继续"? design 选前者 (更鲁棒幂等); (b) master continue 注入是 **best-effort** (失败留 marker+warn, 非保证闭合, 依赖注入成功 OR master prompt 主动读 marker)。这两点呈 PM 确认诠释。
- **[nit]** §2.3 把 `PROMPT_PENDING → REAP_ONLY` 列入 intent 计算, 但 crash-mark (agents_lifecycle.rs:88 `WHERE state NOT IN (...'PROMPT_PENDING')`) 根本不覆盖 PROMPT_PENDING (changes=0 不进 intent 计算)。措辞冗余非错, 实施时去掉该行。

### PM-pending 汇总 (待 PM 拍, 均不阻塞 worker 机制实施)
1. **master 执行态诠释** (§1 Master / §0 非目标): ActiveWork = master ground truth (零代码改) vs 加 master 端 instrumentation。已发 PM 通知。
2. **"输入继续" 诠释** (上条): worker 重派原 prompt vs 字面"继续"; master best-effort 非保证。

## 7. Test-first 拆解

1. DB schema:
   - 新库有 `agent_recovery_intents` 和索引。
   - 旧库迁移后可查询新表。
2. crash intent:
   - `BUSY + DISPATCHED` crash 写 `REVIVE` intent, 保存 previous_state/job_id, job 仍被标 `FAILED`。
   - `WAITING_FOR_ACK + DISPATCHED` 同上。
   - `IDLE + no DISPATCHED` crash 写 `REAP_ONLY`。
   - `PROMPT_PENDING` 不被 crash 覆盖; 若进入 cleanup path, 不自动 revive。
3. worker gate:
   - `CRASHED + REVIVE intent` 走 respawn。
   - `CRASHED + REAP_ONLY intent` 只 cleanup/delete, 不 respawn。
   - `CRASHED + missing intent` skipped。
4. job requeue:
   - respawn recovered 后 interrupted `FAILED` job 翻回 `QUEUED`, 清 dispatch/completion 字段。
   - 普通 `FAILED` job 不被翻回。
5. master revive:
   - `IdleNoWork` 只 reap 不 revive。
   - `ActiveWork` 写 upgraded marker, env 注入 `AH_REDISPATCH_MARKER`, spawn master。
   - 不调用 old_home -> master_home copy seed。
   - best-effort continue 注入失败不阻断 revive。
6. end-to-end dogfood:
   - worker OOM during BUSY: revive + original job requeued/dispatched。
   - worker OOM while IDLE: cleanup + no revive。
   - managed master OOM with DISPATCHED job: workers reaped, master revived, marker present, worker reprovisioned, interrupted job requeued。
