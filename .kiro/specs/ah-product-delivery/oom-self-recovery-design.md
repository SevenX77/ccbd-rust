# R-A: OOM 自愈 / 有意识重启 设计 (a2 思路 + PM 事实校验)

> 状态: 设计阶段 (SOP-08 §1.1 1c 思路已出, 待 1d a1+a3 audit → 收敛 → 1e formal design → impl)
> 锁定来源: PM 锁定的 net-new "OOM 后有意识重启 + resume 续断点 + 无 orphan"
> 关联 dogfood gap: R-A (无 auto-recovery 触发, 当前需手动 `ah up`)。R-B/R-C 是另一条 (resume-time prompt-scan), 由 `fix/resume-codex-confirm-input-wedge` 分支处理, 是 R-A 落地的前置 (auto-recovery 触发 resume 后, resume 不能 wedge)。

## 1. a2 (gemini 设计顾问) 思路原文要点

**核心: 自主状态协调环 (Autonomous State-Reconciliation Loop)** — 把"恢复"视为状态机的物理补全, 不是补丁。第一性原理: 自愈鲁棒性源于"目标态 vs 观测态"闭环对冲; 当前 ahd 是被动响应型 (依赖 RPC 触发), 自愈需引入主动驱动者。

### A. 触发机制: Orchestrator 驱动 Realignment
- 职责归属: **不**新起 Recovery Worker, **不**在 Health Worker 里直接调恢复。
- 把 `session.realign` 逻辑内核下沉到 `src/orchestrator/mod.rs` 的 `run_once` 循环: 除调度 Job 外, 定期 (或 WAKER 唤醒) 扫描 DB 中 `CRASHED` 且 `is_recovery_eligible` 的 agent → 自动以内部身份发起 realign。

### B. 触发判据 + 退避
- 判据: `state == "CRASHED"` (涵盖 OOM / 非正常退出)。
- 退避: agents 表新增 `retry_count` + `next_retry_at`; 指数退避 (1s/2s/4s), 上限 5 次 → 转 `FAILED` 停止自愈等人工 `ah up`。理由: 防 crash-loop (如 sandbox 权限错导致死循环)。

### C. 反孤儿协调 (难点): "标记保留, 延迟回收"
- 冲突: ahd 重启后 `reconcile_orphan_scopes` 想杀残留 scope, 而 recovery 想复用旧 sandbox home。
- 解法: (1) startup_reconcile 发现 pid 死 → 标 CRASHED, **不标 KILLED**; (2) 改 `reconcile_orphan_scopes`: CRASHED + 可恢复 provider 的 scope **不执行 systemctl stop** (白名单排除); (3) 新 realignment 先 `delete_agent` 级联清旧 scope 再 spawn 新 → 资源物理交替原子。

### D. 场景 C: Master 重生 ("主控墓碑自复活")
- a2 主张: ahd 承担 master 冷启动 — startup_reconcile 发现 master 记录 ACTIVE 但进程死 → ahd 自动调 `spawn_master_pane`; master 复活后其 `ah start/attach` 触发全局 realign。
- **PM 注 (待决策点)**: 这跟之前理解"master 重启由 ahd 之外的 systemd/harness (`claude --continue /remote-control`) 负责"有张力。Master 生命周期归属 = 真设计决策点, audit 重点辩论。可能 goal-level (改变 master 归属), 需谨慎。

### E. 可观测性 ("有意识")
- 新增 `self_recovery_attempt` 事件 (payload: agent_id / reason=OOM_RECOVERY / action / args=["resume", uuid])。
- 用 `state_version` 拦截恢复期间冲突的外部 RPC (如用户手动 `ah stop`)。

## 2. PM 事实校验 (SOP-07 §5, grep cross-check 现有代码)

| a2 引用 | 现状 verify |
|---|---|
| `run_once` 扩展点 | ✅ `src/orchestrator/mod.rs:51 async fn run_once` |
| `realign_agent_internal` | ⚠️ 该确切名不存在; 实际 = `src/rpc/handlers/realign.rs:48 handle_session_realign` / `:268 handle_agent_realign` / `:290 spawn_realign_agent`。能力真实, 命名 nit。下沉到 orchestrator 需重构 realign 内核为可内部调用 (现在是 RPC handler) |
| `is_recovery_eligible_provider` | ✅ `src/provider/manifest.rs:27`。**关键: `src/db/system.rs:45` 已有 `agent_state == "CRASHED" && is_recovery_eligible_provider(provider)` 谓词** — 判据已存在, 不用新造 |
| `state_version` | ✅ agents 表已有 (`ack.rs`, `integration.rs:352` 用它做 CAS update) |
| `reconcile_orphan_scopes` | ✅ `system.rs:361 reconcile_orphan_scopes_sync` / `:373 _with_runner_sync` (含 dry_run); 另有 `:531 reconcile_active_agents_to_crashed_sync`, `:564 startup_reconcile_phase_prompt_pending_preserve` 等 phased 函数 |
| `startup_reconcile` | ✅ `system.rs:536+` phased (phase_a select / phase_b probe_pids / phase_c crash_dead / phase_d reregister_alive) |
| `spawn_master_pane` | ✅ `handle_session_spawn_master_pane` (RPC handler, router 注册) |
| `retry_count` / `next_retry_at` | ❌ 全 src 无 — 真 net-new schema (a2 已标"引入") |

**结论**: 思路落点基本扎实, 引用的扩展点/谓词真实存在; CRASHED+eligible 判据已现成 (system.rs:45)。需整合大量现有 reconcile 机制 (非 greenfield)。命名 nit (realign 内核现是 RPC handler, 下沉需重构为内部可调)。backoff 两字段真新增。

## 3. 待 audit 焦点 (1d, a1 工程 + a3 PM 替身)
1. **场景 C master 重生归属** — ahd 自复活 master vs systemd/harness 负责。是否 goal-level? (PM 重点)
2. realign 内核下沉 orchestrator 的重构代价 (现是 RPC handler, 跟 run_once 的 async ctx 兼容?)
3. 反孤儿白名单改动跟现有 `startup_reconcile_phase_prompt_pending_preserve` / `reconcile_active_agents_to_crashed_sync` 的关系 (会不会重复/冲突?)
4. backoff 状态机 (CRASHED → 重试 → FAILED) 跟现有 state machine 转换的兼容 + state_version CAS
5. R-C (resume wedge) 是 R-A 前置 — auto-recovery 触发 resume 后必须不 wedge, 顺序: R-C 先落地

## 4. a3 audit (1d, PM 替身) + PM 收敛裁定 — 2026-06-11

a3 grep/read 实证 audit (job_4915e9893f67), 结论: **不能直接进 1e formal design**, 4 个阻塞项, 解掉后核心 (worker 自动 realign) 可行。

### MF1 (must-fix, 证据 High × 影响 High × 置信 A): config-blindness — 自动 realign 地基缺失
- `load_project_config`/`find_config` (读 ah.toml) **只在 `src/cli/`** (up.rs:15-17, config.rs:92); **daemon/orchestrator 全程不读 ah.toml** (grep 实证)。
- `handle_session_realign`(realign.rs:48-67) 的 master/agents (env/hooks/plugins) 从 **RPC params 反序列化** — params 是 `ah up` (CLI) 读 ah.toml 后发来的。
- agents 表只有 `provider`+`config_hash` (hash 不能反推配置), **无 env/hooks/plugins 列** (PR-7 实证)。
- ⇒ **orchestrator 在 ahd 内部构造不出 `RealignAgentParams`** → auto-realign 建不起来, 或空配置 spawn = 静默坏恢复。**这正是当前恢复必须手动 `ah up` 的根因** (ah up 携带配置)。a2 设计提出去手动触发, 却没解掉让它手动的约束。
- **PM 裁定**: 真阻塞。两候选 (design 必须二选一并写进 formal design):
  - **(a)** ahd 恢复时从 `sessions` 表的 `absolute_path` 读 ah.toml → 构造 RealignAgentParams。**注意: 这反转 PR-7 刻意保持的 daemon config-blind 设计** (PR-7 选 operator-triggered 的根本原因)。需处理 ah.toml 已变 / 路径不可达。
  - **(b)** spawn 时把完整 spawn spec (env/hooks/plugins) 持久化进 DB → orchestrator 从 DB 读。schema 变更更大但自洽, 不让 daemon 读项目文件。
  - 决策性质: **工程/设计** (autonomous recovery 怎么拿到 config), 非 goal-level (goal=autonomous 不变)。R-C 落地后 a1+a2 收敛 a vs b (self-drive)。

### SF1 (should-fix): §D master 重生 — 砍出 R-A, 作 goal-level 升级 user
- a3 + PM 事实校验 + 项目 CLAUDE.md 三方一致: master = `claude --dangerously-skip-permissions --continue /remote-control` (config.rs/systemd.rs 实证), 外部 harness/systemd 启动, **不是 CCB worker** (master 环境无 `CCB_CALLER_ACTOR`)。ahd 自动 spawn master = **ahd 接管 master 生命周期 = 根本归属变更 + 产品决策**。
- **PM 裁定**: **从 R-A 砍掉**。R-A 核心 = worker 自动 realign (§A/B/E)。master 归属是 goal-level 决策 (尤其关联 Step-4 master 自换 ah), **flag 给 user 异步拍, 不阻塞 worker recovery 推进**。

### SF2 (should-fix): §C 反孤儿 ~90% 是已 merged 的 PR-7, 别重做
- §C(2) orphan 白名单排除 CRASHED+recoverable: `active_session_and_agent_refs_sync`(system.rs:419-446) **已有** `OR (agents.state='CRASHED' AND provider IN codex/claude/antigravity)`。
- §C(1) 标 CRASHED 不标 KILLED + 保留 home: `reconcile_active_agents_to_crashed_sync`(system.rs:545) `is_recovery_eligible_provider` 分支 **已有**; 谓词 system.rs:45 已在。
- §C(3) 先 delete_agent 再 spawn: realign.rs CRASHED 分支 **已是**。
- **PM 裁定**: formal design **引用 PR-7 机制 + 只列残留 delta**, 不重新设计。

### SF3 (should-fix, 置信 B): backoff 终态 FAILED 会让 agent 彻底无法恢复
- realign **只对 `state=="CRASHED"` 触发恢复** (realign.rs, PR-7 实证), 不处理 FAILED。退避耗尽标 FAILED 后人工 `ah up` 也救不回 → 跟"等人工 ah up"兜底矛盾。
- **PM 裁定**: 终态保持 `CRASHED` + 加 `retry_exhausted` 标志 (ah up 仍可手动重置), 或扩 realign 处理 FAILED。formal design 明确。

### 确认可行 (非阻塞)
- realign 内核下沉 run_once: `spawn_realign_agent` 已是 `async fn(&Ctx,...)`, run_once 持 `&Ctx`, **async/ctx 兼容**。卡点是 MF1 配置来源, 不是 handler→内部调用重构。
- backoff 两字段 (retry_count/next_retry_at) additive; `state_version` CAS 已存在 (ack.rs/integration.rs:352) 可拦截恢复期外部 RPC 冲突。
- §E `self_recovery_attempt` 事件 additive, 符合"有意识"。

### PM 收敛后下一步 (R-C 落地后)
1. a1+a2 收敛 MF1 (a vs b) — 工程 self-drive, round-cap 2。
2. master 重生 (SF1) flag user 异步拍 (goal-level), 不阻塞。
3. a1 主笔 1e formal design: 引用 PR-7 (SF2) + backoff 终态 CRASHED+flag (SF3) + 选定的 config 方案 (MF1)。
4. tests-first impl → re-dogfood OOM 自愈。

## 5. 关联
- R-B/R-C: `fix/resume-codex-confirm-input-wedge` 分支 (a1)
- dogfood 证据: `.kiro/specs/ah-dogfooding-closure/acceptance-matrix.md` Step-3 Case A
- handoff: `.kiro/specs/ah-product-delivery/handoff-prompt.md`

## 6. MF1 收敛裁定 — 2026-06-12 (a1 工程 + a2 设计独立汇聚 → 方案 b)

R-C 已落地 (PR #41 合入 main 594ef15), R-A 前置满足。派 a1 (工程) + a2 (设计) 独立收敛 MF1。
**两方从不同角度独立汇聚到同一结论: 选方案 (b) — spawn spec 持久化进 DB 作 SSOT**, round-1 即收敛, 无 must-fix 残留 (SOP §1.05 不开 round-2)。

### 一致结论 (b): DB 持久化 resolved spawn spec, 不让 ahd recovery 读 ah.toml
- **a2 第一性原理**: 自愈系统 (k8s 式) 的 desired state 必须由控制器**闭环拥有**且持久化。方案 (a) 让 ahd 的 desired state "寄生"在 ah.toml (外部、可能已改、可能不可达)。方案 (b) 让 DB 成为真正 SSOT, ahd 仅凭 DB + sandbox 现场即可重建逻辑连接。
- **a1 工程语义**: R-A 是故障自愈 → 应按 **"崩前配置快照"** 恢复同一 agent; **"恢复时最新 ah.toml" 是配置 rollout, 应继续由 operator 触发 `ah up`**。方案 (a) 反转 PR-7 config-blind + 引入 ah.toml 已变 / 路径不可达 / worktree 切换 / 配置临时非法时静默用错配置等失败模式。
- **共同提出的 hybrid (恰好一致)**: 保留 `ah up` 作唯一显式刷新通道 — `ah up`/session.realign 把最新 ah.toml 转成新 DB snapshot; auto-recovery 用旧 snapshot, 显式 `ah up` 刷新。"自动恢复=复原, 显式更新=升级", 架构自洽。

### 落地契约 (1e formal design 锁定项)
- **存储**: 新增 additive 表 `agent_spawn_specs` (a1 形态, 优于在 agents 表堆 JSON 列 — 保持 agents 表精简 + `ON DELETE CASCADE` 自动清 + `spec_version` 留演进):
  ```sql
  CREATE TABLE IF NOT EXISTS agent_spawn_specs (
    agent_id TEXT PRIMARY KEY REFERENCES agents(id) ON DELETE CASCADE,
    spec_version INTEGER NOT NULL DEFAULT 1,
    provider TEXT NOT NULL,
    config_hash TEXT NOT NULL,
    spec_json TEXT NOT NULL,   -- resolved spec: provider+env+hooks+plugins, 足以重建 RealignAgentParams
    updated_at INTEGER NOT NULL DEFAULT (unixepoch())
  ) STRICT;
  ```
  实证: agents 表 (schema.rs:18-31) 现只有 provider/state/state_version/config_hash, 无 env/hooks/plugins; grep 全 src 无任何 spawn-spec 持久化 → 真 net-new。
- **写入点**: 初始 `handle_agent_spawn` 成功后写; `spawn_realign_agent` 成功后覆盖; `ah up`/session.realign 是唯一把最新 ah.toml 转成新 snapshot 的路径。
- **无 snapshot 的旧 CRASHED agent**: **不猜空配置恢复** → 记 `self_recovery_attempt{action:"skipped", reason:"missing_spawn_spec"}`, 保留 CRASHED 等人工 `ah up`。DB snapshot staleness 是**设计语义** (按崩前配置恢复), 不是 bug。
- **原子性/CAS**: orchestrator run_once 读 CRASHED+eligible+snapshot+state_version → `WHERE id=? AND state='CRASHED' AND state_version=?` 抢占恢复资格; CAS 失败跳过 (state_version 已存在, schema.rs:23)。
- **backoff 终态 (SF3)**: 保持 `CRASHED` + `retry_exhausted` 标志 (人工 `ah up` 仍可救), **不转 FAILED** (realign 只处理 CRASHED, FAILED 会让人工也救不回)。
- **反孤儿 (SF2)**: 引用已 merged 的 PR-7 机制 (system.rs:45 谓词 / :419-447 orphan 白名单 / :545 CRASHED 保留 home), 只列残留 delta, 不重做。

### 下一步
- a1 主笔 1e formal design (锁定上述契约) → a3 audit 1f (round-cap 2) → tests-first → impl → re-dogfood OOM 自愈。
- **SF1 master 重生**: 仍是唯一 goal-level escalation, flag user 异步拍, 不阻塞 worker recovery。

## 7. 正式设计 (1e)

本设计只覆盖 **worker auto-recovery**: ahd/orchestrator 自动恢复 `CRASHED` 且 recovery-eligible 的 worker。**不覆盖 master 重生**; master 生命周期归属仍是 SF1 的 goal-level 决策, 异步 flag 给 user, 不阻塞 R-A。

### 7.1 目标语义
- **恢复=按崩前快照复原**。auto-recovery 使用 DB 中持久化的 resolved spawn spec, 不读恢复时的最新 `ah.toml`。
- **配置刷新=显式 operator 行为**。`ah up`/`session.realign` 仍是唯一把最新 `ah.toml` 转成新运行态 snapshot 的通道。实证: CLI 在 `src/cli/up.rs:13-18` 调 `find_config`/`load_project_config`, 并在 `src/cli/up.rs:31-50` 把 master/agents 的 `env/hooks/plugins` 发给 `session.realign`; config 读取函数在 `src/cli/config.rs:92-109`。
- **daemon 保持 config-blind**。ahd recovery 不从 `sessions.absolute_path` 读项目文件, 避免 `ah.toml` 已变、路径不可达、worktree 切换、配置临时非法时把故障恢复变成隐式配置 rollout。

### 7.2 `agent_spawn_specs` 表

在 `src/db/schema.rs` 的 `agents` 表后、`events` 表前加入 additive 表。现状 `agents` 只有 `provider/state/state_version/config_hash` 等字段, 无 `env/hooks/plugins` (`src/db/schema.rs:18-31`); `Agent` struct 也只暴露 `config_hash`, 无 spec 字段 (`src/db/schema.rs:145-158`)。

最终 DDL:

```sql
CREATE TABLE IF NOT EXISTS agent_spawn_specs (
  agent_id TEXT PRIMARY KEY REFERENCES agents(id) ON DELETE CASCADE,
  spec_version INTEGER NOT NULL DEFAULT 1,
  provider TEXT NOT NULL,
  config_hash TEXT NOT NULL,
  spec_json TEXT NOT NULL,
  updated_at INTEGER NOT NULL DEFAULT (unixepoch())
) STRICT;
```

`spec_json` 序列化 **resolved agent spec**:

```json
{
  "agent_id": "a1",
  "provider": "codex",
  "env": {},
  "hooks": {},
  "plugins": []
}
```

字段依据:
- `RealignAgentParams` 需要 `agent_id/provider/env/hooks/plugins` (`src/rpc/handlers/realign.rs:28-38`)。
- `spawn_realign_agent` 把这些字段映射成 `handle_agent_spawn_with_recovery` 的 `agent_id/provider/extra_env_vars/hooks/plugins` (`src/rpc/handlers/realign.rs:290-317`)。
- `handle_agent_spawn_with_recovery` 解析 `session_id/agent_id/provider`, `hooks/plugins`, `extra_env_vars` (`src/rpc/handlers/agent.rs:41-58`; `src/rpc/handlers/params.rs:25-38`)。

`agent_spawn_specs.provider` 和 `config_hash` 是冗余索引/审计字段; recovery 重建 params 以 `spec_json` 为准, 但查询和诊断可直接看列。迁移在 `src/db/mod.rs:init` 的现有 migration 序列中加入 `migrate_agent_spawn_specs(&conn)?`, 放在 `migrate_agents_config_hash(&conn)?` 之后即可 (`src/db/mod.rs:50-61`)。现有 migration 模式是单函数执行 DDL/ALTER, 如 `migrate_sessions_config_hash` 和 `migrate_agents_config_hash` (`src/db/mod.rs:100-115`)。

旧库迁移不 backfill。没有 snapshot 的旧 `CRASHED` agent 不猜空配置恢复, 只记录 skipped 事件并等待人工 `ah up` 刷新 snapshot。

### 7.3 Snapshot 写入点

1. **初始 spawn**: `handle_agent_spawn` 进入 `handle_agent_spawn_with_recovery` (`src/rpc/handlers/agent.rs:37-45`)。在 tmux/systemd spawn 成功、`insert_agent` 成功、`compute_config_hash` 完成并 `update_agent_config_hash` 成功后写 `agent_spawn_specs` (`src/rpc/handlers/agent.rs:202-223`)。写入内容使用最终 `spawn_env_vars` + `extensions.hooks/plugins`, 因为 `spawn_env_vars` 已包含 home materialization extra env (`src/rpc/handlers/agent.rs:79-99`)。
2. **realign/recovery spawn**: `spawn_realign_agent` 成功调用 `handle_agent_spawn_with_recovery` 后, 当前会再写 expected hash 并置 `IDLE` (`src/rpc/handlers/realign.rs:298-317`)。在 `update_agent_config_hash` 成功后覆盖 `agent_spawn_specs`, 使用传入的 `RealignAgentParams` 和 `expected_hash`。
3. **`ah up/session.realign` 路径**: `handle_session_realign` 从 RPC params 反序列化 master/agents (`src/rpc/handlers/realign.rs:48-65`), 对每个 agent 计算 expected hash (`src/rpc/handlers/realign.rs:133-141`)。凡是 NEW/REALIGNED/forced drift 后实际改变运行态的 agent, 都通过 `spawn_realign_agent` 覆盖 snapshot; audit-only/unchanged 不改 snapshot。CRASHED 分支已有 delete 后 respawn (`src/rpc/handlers/realign.rs:156-166`)。

写入原则: **spawn 成功后尽力写 snapshot, snapshot 写失败优雅降级**。
- spawn 本身失败 (tmux/systemd/`insert_agent`/provider materialization 等) 仍按现有错误路径处理, 不写 snapshot。
- 如果 agent 已成功起来, 但 `agent_spawn_specs` 写入失败: 记录 error log, **不回滚/不杀掉已正常运行的 worker**。这与相邻 `update_agent_config_hash(...).await?` 的直接传播模式不同, 但 R-A 的恢复地基是 additive; 一次瞬时 snapshot 写抖动不应让 working agent 变脆。
- 此时 DB 中存在 agent 但缺 snapshot 是合法降级状态, 等同旧库迁移后未 backfill 的 agent: 该 agent 下次若变为 `CRASHED`, 会自然走 §7.4 的 missing-snapshot skip, 记录 `self_recovery_attempt{action:"skipped", reason:"missing_spawn_spec"}`, 等人工 `ah up` 补 snapshot。
- 只有 snapshot 写成功才算该 agent 具备 auto-recovery 地基。

### 7.4 Orchestrator `run_once` 自动恢复环

扩展点是 `src/orchestrator/mod.rs:51 async fn run_once`。当前 `run_once` 已不是单纯 "查 IDLE 后 dispatch": IDLE for-loop 先 `has_queued_job` peek (`src/orchestrator/mod.rs:55-58`), 再检查 pane/parser (`src/orchestrator/mod.rs:60-90`), 通过 `run_dispatch_guard` (`src/orchestrator/mod.rs:92-101`), 最后 `dispatch_queued_job` 并发送 prompt (`src/orchestrator/mod.rs:103-138`)。新增恢复环必须**追加在这条含 dispatch-guard 的 IDLE for-loop 之后**, 与 dispatch-guard 共存, 不替换既有 dispatch 路径:

1. `run_once` 先完整保留现有 IDLE job dispatch / pane-parser readiness / dispatch-guard 一轮。
2. 同一次 `run_once` 末尾最多处理 **一个** recovery candidate; 如成功/失败产生状态变化则返回 `did_work=true` 并 `wake_up()`。
3. recovery 查询条件: `agents.state='CRASHED'`, `is_recovery_eligible_provider(provider)` 为真 (`src/provider/manifest.rs:27-29`; 现有同义谓词 `src/db/system.rs:41-46`), `agent_spawn_specs` 存在, `retry_exhausted=0`, `next_retry_at IS NULL OR next_retry_at <= unixepoch()`。
4. 对候选同时读 `state_version`。CAS 抢占:

```sql
UPDATE agents
SET state_version = state_version + 1,
    updated_at = unixepoch()
WHERE id = ?
  AND state = 'CRASHED'
  AND state_version = ?
  AND (retry_exhausted = 0 OR retry_exhausted IS NULL)
  AND (next_retry_at IS NULL OR next_retry_at <= unixepoch());
```

CAS 只抢占, 不把状态改出 `CRASHED`, 保持 SF3 的人工 `ah up` 可救语义。现有 state_version CAS 模式可复用, 例如 `src/db/state_machine.rs:112-147` 先读 version 再 `WHERE state_version = ?`, 以及 `src/db/state_machine.rs:862-872` 同时约束状态和值。

调用序:
- 查询 candidate + snapshot。
- CAS 成功后, 从 `spec_json` 重建 `RealignAgentParams`。
- emit `self_recovery_attempt{action:"started"}`。
- 调用下沉后的内部 realign primitive: 等价于 `delete_agent` (`src/db/agents.rs:140-145`) + `spawn_realign_agent(..., killed_before_spawn=true, is_recovery=true)`。现有 CRASHED realign 已是 delete 后 respawn (`src/rpc/handlers/realign.rs:156-158`); `spawn_realign_agent` 已支持 `is_recovery=true` (`src/rpc/handlers/realign.rs:290-317`)。
- 成功后清零 backoff 字段, 覆盖 snapshot, emit `self_recovery_attempt{action:"recovered"}`。

错误分支:
- **CAS 失败**: 不报错, 不 emit failure; 说明用户/其他 loop 已改变状态或版本。加 debug log, 例如 `"recovery CAS lost, external state change"`, 方便诊断为何没有自动恢复。
- **snapshot 缺失**: 不 CAS, emit `self_recovery_attempt{action:"skipped", reason:"missing_spawn_spec"}`, 保持 `CRASHED` 等人工 `ah up`。
- **respawn 失败**: 保持 `CRASHED`, 增加 backoff, emit `self_recovery_attempt{action:"failed"}`。不得转 `FAILED`。

### 7.5 Delete-then-spawn 丢失窗口处置

现有 manual realign 的 CRASHED 分支是 `delete_agent` 后 `spawn_realign_agent` (`src/rpc/handlers/realign.rs:156-158`), force drift 分支也有 `mark_agent_killed` → `delete_agent` → `spawn_realign_agent` (`src/rpc/handlers/realign.rs:206-213`)。这在手动路径里已存在; auto-recovery 会更频繁触发, 若 delete 成功但 spawn 失败, agent row 会消失, 后续 recovery 无法再选中。

R-A 选择**最务实的 restore-on-failure 包装**, 不做 spawn-before-delete:
- spawn-before-delete 会撞 `agent_id` 唯一性/`agent_exists` 检查 (`src/rpc/handlers/agent.rs:59-60`), 还会同时存在同名 scope/home 的资源冲突; 外部 tmux/systemd spawn 也无法和 SQLite delete 放进一个真正原子事务。
- auto-recovery 内部 primitive 在 delete 前先持有 `session_id/agent_id/provider/config_hash/spec_json/backoff` 内存快照。
- delete 后调用 recovery spawn; 如果 spawn 成功, 按 §7.3 覆盖 snapshot 并清 backoff。
- 如果 delete 成功但 spawn 失败, 立即重建最小 agent row: 同 `agent_id/session_id/provider`, `state='CRASHED'`, `pid=NULL`, `config_hash` 来自 snapshot, retry/backoff 按失败规则更新, 并 upsert 原 `agent_spawn_specs`。随后 emit `self_recovery_attempt{action:"failed"}`。这样即使物理 respawn 失败, DB 仍保留可重试的 CRASHED agent, 不会蒸发。
- 若最小 row 重建也失败, 记录 error/critical log; 这是 DB 层不可恢复异常, 但正常实现必须覆盖该测试。

### 7.6 Backoff 字段与状态流转

字段放在 `agents`, 不放 `agent_spawn_specs`:
- backoff 是 agent 生命周期/状态机属性, 与是否有 snapshot 无关。
- 查询候选需要和 `state/state_version/provider` 同表过滤, 放 `agents` 简化 CAS。
- `agent_spawn_specs` 可随 agent delete cascade 清理; retry 历史应跟 agent row 当前状态一致。

新增 columns:

```sql
ALTER TABLE agents ADD COLUMN retry_count INTEGER NOT NULL DEFAULT 0;
ALTER TABLE agents ADD COLUMN next_retry_at INTEGER;
ALTER TABLE agents ADD COLUMN retry_exhausted INTEGER NOT NULL DEFAULT 0;
```

指数退避:
- 第 1 次失败后 `retry_count=1`, `next_retry_at=now+1s`。
- 第 2 次失败后 `retry_count=2`, `next_retry_at=now+2s`。
- 第 3 次失败后 `retry_count=3`, `next_retry_at=now+4s`。
- 第 4/5 次继续 cap 到 `4s`。
- 第 5 次失败后 `retry_exhausted=1`, `next_retry_at=NULL`, state 仍为 `CRASHED`。

成功 recovery 和显式 `ah up/session.realign` 成功后都重置 `retry_count=0,next_retry_at=NULL,retry_exhausted=0`。终态按 SF3 保持 `CRASHED + retry_exhausted`, **不转 FAILED**; 因为现有 realign CRASHED 分支只处理 `running.state == "CRASHED"` (`src/rpc/handlers/realign.rs:156-158`), 转 FAILED 会让人工 `ah up` 也绕不开。

### 7.7 `self_recovery_attempt` 事件

事件使用现有 `insert_event` (`src/db/events.rs:192-198`)。payload 统一 JSON:

```json
{
  "agent_id": "a1",
  "reason": "OOM_RECOVERY",
  "action": "started|recovered|failed|skipped",
  "args": ["resume", "..."],
  "retry_count": 1,
  "next_retry_at": 1710000000,
  "state_version": 7,
  "error": "..."
}
```

`args` 记录 provider recovery args; provider 侧已由 `compute_recovery_args` 根据 provider 生成 resume/continue 参数 (`src/provider/manifest.rs:31-35`)。skip 分支必须包含 `reason:"missing_spawn_spec"`:

```json
{
  "agent_id": "a1",
  "reason": "missing_spawn_spec",
  "action": "skipped",
  "retry_count": 0
}
```

### 7.8 SF2 反孤儿: 只列残留 delta

不重做 PR-7 反孤儿机制, 正式设计只依赖现有行为:
- recovery-eligible 判据已有 `agent_state == "CRASHED" && is_recovery_eligible_provider(provider)` (`src/db/system.rs:41-46`)。
- orphan live refs 已把 `CRASHED` 且 provider in `codex/claude/antigravity` 保留 (`src/db/system.rs:419-447`)。
- startup reconcile 对 recovery-eligible dead agent 保留 home (`src/db/system.rs:531-555`)。

R-A 残留 delta 只有: orchestrator 在保留的 CRASHED/home/scope 基础上自动触发 realign; 不改 orphan 白名单语义。

### 7.9 测试计划大纲

**单元测试**
- schema/migration: 新库创建 `agent_spawn_specs`; 旧库 migration 后有表和 agents backoff columns; `ON DELETE CASCADE` 生效。
- migration 幂等: `CREATE TABLE IF NOT EXISTS` 和 `add_column_if_missing` 风格 migration 跑两次不报错 (`src/db/mod.rs:167-182`)。
- snapshot persistence: 初始 spawn 成功后写 spec; spawn 失败不写; snapshot 写失败不回滚已成功 spawn 的 agent; realign/recovery 成功后覆盖 spec; `spec_json` 能反序列化重建 `RealignAgentParams`。
- backoff/CAS: CRASHED+version 匹配才抢占; version 变化时跳过; 真并发模拟 orchestrator recovery 与外部 `ah up`/`ah stop` 同时改同一 CRASHED agent, 验证 winner/loser 只有一个生效; 失败按 1/2/4/4/4 秒递增; 第 5 次失败置 `retry_exhausted` 且仍为 `CRASHED`。

**集成测试**
- `run_once` 有 queued job 时仍 dispatch, 同轮最多处理一个 recovery, 不饿死 job dispatch。
- CRASHED+eligible+snapshot 到期: delete old agent, recovery spawn, 写新 snapshot, 清 backoff, 记录 `self_recovery_attempt`。
- CRASHED+eligible 但无 snapshot: 不 spawn, 保持 CRASHED, 记录 skipped/missing_spawn_spec。
- respawn 失败: 保持 CRASHED, 增加 backoff, 不转 FAILED。
- delete 成功但 recovery spawn 失败: 重建最小 CRASHED agent row + 恢复 snapshot, 后续 run_once 可再次重试。
- 显式 `ah up/session.realign` 成功后刷新 snapshot 并清 retry_exhausted。

**dogfood / e2e**
- 人工制造 codex/claude/antigravity worker OOM 或非正常退出: startup reconcile 标 CRASHED 并保留 home; ahd 无需手动 `ah up` 自动 recovery; worker 通过 provider resume/continue 接续。
- 修改 `ah.toml` 后不运行 `ah up`, 再触发 OOM: recovery 使用旧 snapshot, 不吃最新文件改动。
- 运行 `ah up` 后再触发 OOM: recovery 使用新 snapshot。
- crash-loop 场景达到 5 次后保持 CRASHED+retry_exhausted, 人工 `ah up` 可恢复。
