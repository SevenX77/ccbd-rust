# master OOM revive 与 anchor cascade 协调设计稿

## 结论先行

当前基于 `master_spawn_lock` 的 deferral 已证明能挡住 run2 里的 home-wipe race, 但它只是进程内锁, 不是权威状态。它解决了"锁已经被 master_watch 持有时 session_watch 不级联"这一窄窗口, 没有解决 ahd 重启后锁丢失、anchor 确认早于 master_watch 拿锁的 TOCTOU、以及 revive 失败或卡住后级联是否最终放行的问题。

推荐把 cascade 决策改为 DB 权威状态驱动: master_watch 在同一个持久状态机里 claim master revive window, session_watch 在级联前查询该 window 与 session/master/recovery 状态来判定 `DEFER`、`CASCADE_NOW` 或 `CASCADE_AFTER_TIMEOUT`。进程内锁可以保留为同进程互斥优化, 但不能作为是否保 worker home 的唯一依据。

核心不变量:

1. master 意外死亡后, worker 必须被 master-death cleanup 或最终 anchor cascade reap, 不能无限期抑制级联。
2. ActiveWork master-revive 窗口内, anchor cascade 必须抑制, 保住 recovery-eligible worker home, 让 revived master + `--continue` worker resume/requeue 成功。

推荐新增持久表 `master_recovery_windows`, 或等价地在 `sessions` 增字段。本文推荐新表, 因为它能表达一次 revive attempt 的 phase、deadline、expected/master generation, 并避免把 `sessions.master_last_exit_reason` 这种审计字段扩大成状态机。

## 现状实证

### run2 home-wipe 回归

- `research/ah-master-revival-dogfood/run2-ahd.log:229-236`: master pidfd 先发现 master 死亡, 随后捕获 w1 recovery intent, 执行 ActiveWork worker cleanup, 记录 revive attempt, 写 redispatch marker。
- `research/ah-master-revival-dogfood/run2-ahd.log:238-245`: revived master 注入继续, worker 用 `--continue` 复活, interrupted job 被 requeue。
- `research/ah-master-revival-dogfood/run2-ahd.log:403-408`: 约 5.5s 后 session_watch 仍因 anchor inactive 触发 `ANCHOR_UNIT_STOPPED` cascade, 杀掉刚复活的 w1, dispatch compensation 看到 agent 已变 `KILLED`。

这说明 conflict 不是 master revive 不工作, 而是 session anchor cleanup 在 revive 之后仍能二次级联, 破坏 worker home/pane/job。

### run4 初步修复 PASS

- `research/ah-master-revival-dogfood/run4-PROOF.md:13-18`: ActiveWork 场景下捕获 recovery intent、worker cleanup、master continue 注入、worker reprovision 后 requeue 均在亚秒完成。
- `research/ah-master-revival-dogfood/run4-PROOF.md:20-25`: worker home 连续存在到超过旧的约 7s anchor-inactive 窗口, 同一 transcript 继续增长。
- `research/ah-master-revival-dogfood/run4-PROOF.md:27-33`: 复活后 fresh turn 能回忆 BANANA-42, 证明保 home 让 provider 上下文连续。

结论: deferral 方向正确, 但它需要从内存锁升级为可恢复、可超时、可审计的持久 coordination。

### 当前代码路径

- `src/monitor/session_watch.rs:17-19`: `should_defer_anchor_cascade_for_master_revive` 只查 `master_revive_in_flight(session_id)`。
- `src/monitor/session_watch.rs:40-58`: anchor inactive 先 debounce 2 次, 每次 2s, 之后才确认。
- `src/monitor/session_watch.rs:65-100`: 确认 inactive 后若 defer 返回 false, 否则调用 `cascade_kill_session_agents(..., "ANCHOR_UNIT_STOPPED")` 并移除 monitor。
- `src/master_revival.rs:410-430`: `master_spawn_lock` 是 `static LazyLock<Mutex<HashMap<String, Arc<AsyncMutex<()>>>>>`; `master_revive_in_flight` 通过 `try_lock().is_err()` 判断锁是否被持有。这是 ahd 进程内状态, ahd 重启后必丢。
- `src/monitor/master_watch.rs:47-78`: master death handler 先拿 `master_spawn_lock`, 再 classify 并进入 revive path。
- `src/monitor/master_watch.rs:433-455`: revive path 先 snapshot ActiveWork/IdleNoWork, 再 `clean_worker_runtime_resources_sync(..., preserve_session_anchor = ActiveWork)`。
- `src/monitor/master_watch.rs:476-548`: ActiveWork 之后才处理 retry backoff、claim generation、record revive attempt/fuse。
- `src/monitor/master_watch.rs:549-711`: 写 redispatch marker、spawn revived master、完成 DB transition、注入继续、reprovision killed workers 并处理 captured intents。
- `src/db/system.rs:364-432`: anchor cascade 会把 session 从 ACTIVE 置 KILLED, stop scopes/anchor, pidfd SIGKILL, 并用普通 `mark_agent_killed_sync` 标 worker KILLED。
- `src/db/system.rs:845-864`: 普通 sandbox cleanup 会删除 sandbox home。
- `src/db/system.rs:866-881`: startup reconcile 对 recovery-eligible dead workers 有 preserving-home variant, 但这不保护 anchor cascade 已经走普通 kill 的场景。
- `src/db/system.rs:521-532` 和 `src/db/system.rs:757-787`: ahd startup reconcile 会 crash dead agents、清 orphan scopes, 但不恢复进程内 master revive lock。
- `src/orchestrator/mod.rs:394-417`: recovery loop 遇到 `REAP_ONLY` 或非 ACTIVE session 的 `REVIVE_IDLE` 会 reap/delete, 说明系统已有"不该 revive 时必须收尸"的方向。

## 现有锁机制缺口

### 1. 锁是易失内存态

`master_spawn_lock` 存在于 `src/master_revival.rs:12-13` 的 static map, `master_revive_in_flight` 只检查该 map 的 async mutex (`src/master_revival.rs:420-430`)。ahd 现在是持久 systemd service, 可被 `Restart=on-failure` 拉起; 一旦 ahd 自己 OOM/restart, 所有 lock state 丢失。

后果:

- 如果 master revive 已经把 worker home 保住并正在 reprovision, ahd 重启后 session_watch 再看到 anchor inactive, 会认为没有 revive in flight 并级联。
- 如果 lock 卡住但 ahd 不重启, session_watch 会无限 defer, 没有 DB deadline 兜底。

### 2. TOCTOU: anchor 确认与 master_watch 拿锁之间

session_watch 的判定发生在 `handle_confirmed_anchor_inactive` 内 (`src/monitor/session_watch.rs:65-78`)。master_watch 拿锁发生在 `handle_master_death_detected` 的开头 (`src/monitor/master_watch.rs:55-57`)。如果 anchor debounce 先结束, 而 master pidfd watch/patrol 尚未进入 handler 或尚未拿锁, session_watch 会看到 no-lock 并执行 cascade。

这在理论上可达:

- session_watch 3s tick + 2x2s debounce 是固定轮询 (`src/monitor/session_watch.rs:9-11`, `src/monitor/session_watch.rs:32-58`)。
- master_watch 有 pidfd path、startup rearm、patrol path; patrol 默认 10s (`src/monitor/master_watch.rs:92-131`, `src/monitor/master_watch.rs:322-328`)。
- 当 pidfd watcher 没有注册、注册失败、或 ahd 重启后只靠 patrol, anchor 可以先于 revive handler 到达 confirmed inactive。

### 3. 泄漏方向: revive 失败或 lock 不释放

当前 lock deferral 没有超时。若 `revive_master_after_exit_locked` 在 await spawn/tmux/systemd 操作时长时间卡住, session_watch 会每轮返回 false, 永不 cascade。若 revive 最终失败但没有把 DB 置 FAILED/放行 cascade, worker runtime 可能成为僵尸。

已有 master retry/fuse 只覆盖进入 `record_master_revive_attempt` 的路径:

- `src/master_revival.rs:239-294`: 记录 retry/backoff, 超过 5 次 fuse。
- `src/master_revival.rs:363-397`: fuse 将 session 置 FAILED 并 stop session anchor。

但当前 worker cleanup 在 retry/claim 之前已经发生 (`src/monitor/master_watch.rs:444-455` 在 `src/monitor/master_watch.rs:476-548` 之前)。如果流程在 cleanup 后、record attempt 前失败, DB 没有明确的 "revive pending/failure deadline" 状态给 session_watch 使用。

### 4. ahd 重启与 startup reconcile 交互

startup reconcile 能把 dead active agents 标 CRASHED 并对 recovery-eligible provider 保 home (`src/db/system.rs:757-787`), 也能清 orphan scopes (`src/db/system.rs:521-532`, `src/db/system.rs:599-637`)。但它没有 master-revive window 概念:

- 不会把 "master death cleanup 已经开始, worker home 应暂保" 持久化。
- 不会在窗口超时后主动放行 anchor cascade 或调用 master fuse cleanup。
- active refs 会把 ACTIVE session 下 CRASHED recovery-eligible agents 视作 live refs 以保护 scopes (`src/db/system.rs:639-683`), 但 KILLED worker 和 KILLED/FAILED session 不受保护。

### 5. 多次 anchor-inactive 事件缺少统一结果

当前 defer 返回 false 后 loop 继续, 下轮再从头探测。没有记录 "本次 anchor inactive 已因 revive window defer 到 deadline X"。如果 revive 成功但 anchor unit 永久 inactive, 后续轮次如何结束取决于内存锁是否还在, 不是 DB 状态。

## 权威状态设计

### 新增持久状态: `master_recovery_windows`

推荐新增表:

```sql
CREATE TABLE master_recovery_windows (
    session_id TEXT PRIMARY KEY REFERENCES sessions(id) ON DELETE CASCADE,
    expected_pid INTEGER NOT NULL,
    expected_generation INTEGER NOT NULL,
    claimed_generation INTEGER,
    phase TEXT NOT NULL CHECK(phase IN (
        'DETECTED',
        'WORKERS_REAPED',
        'MASTER_SPAWNING',
        'MASTER_RUNNING',
        'WORKERS_REPROVISIONING',
        'COMPLETED',
        'FAILED',
        'FUSED'
    )),
    active_work INTEGER NOT NULL,
    defer_until INTEGER NOT NULL,
    created_at INTEGER NOT NULL DEFAULT (unixepoch()),
    updated_at INTEGER NOT NULL DEFAULT (unixepoch()),
    completed_at INTEGER
);
```

Minimum viable fields are `session_id`, `expected_pid`, `expected_generation`, `phase`, `active_work`, `defer_until`. `claimed_generation` and timestamps make stale/fence logic auditable and testable.

Alternative: add columns to `sessions`:

- `master_recovery_phase`
- `master_recovery_expected_generation`
- `master_recovery_defer_until`

I do not recommend this as the first implementation because `sessions` already mixes runtime, retry, and audit fields; a table lets us expire/delete rows without overloading `master_last_exit_reason`.

### State ownership

Only master_watch writes the window state, except a startup reconcile/fuse helper may expire stale rows. session_watch only reads it and may call a single helper to convert expired windows into cascade/fuse outcome under transaction.

Proposed writer points:

1. In `handle_master_death_detected`, after classify returns `Revive` but before long async work, insert/upsert `DETECTED(active_work = unknown or snapshot pending, defer_until = now + T)`.
2. In `revive_master_after_exit_locked`, after `snapshot_master_death_session_activity`, update:
   - ActiveWork: `WORKERS_REAPED`, `active_work=1`, `defer_until = now + T`.
   - IdleNoWork: no deferral window needed; mark `FAILED` as today and let cleanup proceed.
3. Before spawning master: `MASTER_SPAWNING`.
4. After `complete_claimed_master_transition`: `MASTER_RUNNING`.
5. During worker reprovision: `WORKERS_REPROVISIONING`.
6. After workers reprovisioned and requeue handled: `COMPLETED`, then clear row or leave completed for audit with `completed_at`.
7. On unrecoverable/fused failure: `FAILED` or `FUSED`, set session FAILED and allow cascade/reap.

### Session_watch authoritative decision

Replace `should_defer_anchor_cascade_for_master_revive(session_id)` with a DB-backed helper:

```rust
enum AnchorCascadeDecision {
    Defer { until: i64, phase: String },
    CascadeNow { reason: &'static str },
}
```

Inputs:

- `sessions.status`
- `sessions.master_pid`, `sessions.master_generation`, `master_retry_count`, `master_next_retry_at`, `master_last_exit_reason`
- optional `master_recovery_windows` row
- current unix time
- existence of active work/recovery intents if no window exists yet

Decision table:

| DB state | Decision | Rationale |
|---|---|---|
| session missing or status not ACTIVE | `CascadeNow` no-op/idempotent | session is already terminal or gone |
| active window exists, `active_work=1`, phase in `DETECTED/WORKERS_REAPED/MASTER_SPAWNING/MASTER_RUNNING/WORKERS_REPROVISIONING`, `now <= defer_until` | `Defer` | ActiveWork revive window; preserve worker home |
| active window exists but `now > defer_until` | `CascadeNow("MASTER_REVIVE_WINDOW_EXPIRED")` and mark window FAILED | no infinite suppression |
| window phase `COMPLETED` | `CascadeNow` only if session still has unhealthy anchor and no live master; otherwise stop watching/re-arm anchor | revive finished; stale anchor must not keep indefinite defer |
| window phase `FAILED/FUSED` | `CascadeNow` | recovery failed; anti-orphan wins |
| no window, current session row still matches expected dead master and active work snapshot says ActiveWork | create short `DETECTED` window, return `Defer` | closes TOCTOU before master_watch has lock |
| no window and no active work | `CascadeNow` | idle/no-work master death should reap, not revive |

The "no window + ActiveWork" branch is the key TOCTOU close. It must be implemented in a transaction that both snapshots active work and creates the window if the session is still ACTIVE. That makes session_watch itself capable of reserving the recovery window before master_watch catches up. master_watch later observes the same window and continues/claims it, or stale detection expires it.

### Timeout and fallback

Recommended MVP values:

- `MASTER_RECOVERY_CASCADE_DEFER_SECS = 90` default.
- Configurable env override for dogfood: `AH_MASTER_RECOVERY_CASCADE_DEFER_SECS`.
- Minimum clamp 10s, maximum clamp 300s.

Why 90s:

- run4 successful path finished within subsecond for master revive and worker reprovision (`run4-PROOF.md:13-18`).
- existing master stable confirm sleeps 60s (`src/monitor/master_watch.rs:880-892`), so 90s gives room for slow spawn plus confirm without indefinite leak.
- master retry backoff can delay spawn (`src/monitor/master_watch.rs:476-501`); if PM wants backoff retries to hold home across attempts, defer_until should be extended by each recorded retry but still capped by a global max.

Fallback rule:

1. If window expires before `COMPLETED`, mark window `FAILED`, set `sessions.master_last_exit_reason='REVIVE_WINDOW_EXPIRED'`.
2. Call cascade with reason `MASTER_REVIVE_WINDOW_EXPIRED` or allow current anchor cascade to proceed with that reason.
3. Do not direct-delete recovery intents; let FK cascade follow agent/session cleanup.
4. Emit event/log with phase, expected_generation, defer_until, and worker count.

This satisfies invariant A: every deferral has a deadline and eventually releases cascade.

### Relationship with existing master retry/fuse

Do not remove `master_retry_count` and fuse. The window coordinates anchor cascade; retry/fuse coordinates master spawn attempts.

Recommended integration:

- `record_master_revive_attempt` extends `defer_until` only when it returns `Spawn`; extension should be min(`now + defer_secs`, `created_at + max_total_secs`).
- `MasterReviveAttemptDecision::Fused` updates window `FUSED` before returning.
- `confirm_master_stable` can clear completed windows for the same generation.
- If ahd restarts and sees active window with `now <= defer_until`, startup rearm/patrol should continue master recovery. If `now > defer_until`, startup reconcile should expire it and cascade.

## Invariants proof

### A. master 死后最终 reap worker 防僵尸

Paths:

- IdleNoWork: master_watch already marks session FAILED and does not revive (`src/monitor/master_watch.rs:466-475`). The design does not add deferral for this branch.
- ActiveWork success: workers are first killed/reprovisioned by master_watch (`src/monitor/master_watch.rs:444-455`, `src/monitor/master_watch.rs:690-711`), and window completes.
- ActiveWork failure/hang: DB window expires, session_watch/startup reconcile marks FAILED/KILLED and runs cascade. No in-memory lock can suppress beyond deadline.
- ahd restart: window survives in DB; startup reconcile can either resume recovery or expire it. Lock loss no longer means "forget".

### B. revive 窗口内抑制级联

The preservation decision is keyed to `active_work=1` and unexpired recovery phase, not a best-effort lock. During that window:

- session_watch returns `Defer`.
- cascade_kill_session_agents is not called for `ANCHOR_UNIT_STOPPED`.
- worker home is preserved long enough for master_watch to reprovision using recovery intent and spawn spec.

Only ActiveWork gets this treatment. Idle/no-work workers remain under the existing PM decision: master death reaps them and does not revive them.

## 与已落地机制的边界

### REVIVE_IDLE

Do not change `REVIVE_IDLE`. `idle-crash-revive-design.md:16-23` explicitly scopes it to worker unexpected IDLE crash and excludes master-death cleanup. The current orchestrator only allows `REVIVE_IDLE` when session is ACTIVE (`src/orchestrator/mod.rs:405-417`). This design preserves that boundary.

### Active-work REVIVE + resume/requeue

Do not change provider resume/requeue semantics:

- master-death worker kill captures recovery intent before marking KILLED (`src/db/agents_lifecycle.rs:49-72`).
- captured intent is read before reprovision (`src/monitor/master_watch.rs:823-849`).
- worker reprovision uses recovery path and captured intent (`src/monitor/master_watch.rs:804-821`).
- requeue/reinsert happens during worker recovery path (`src/orchestrator/mod.rs:516-532`) and related recovery atomicity design remains separate.

This design only prevents session_watch from deleting the home underneath that flow.

### Master self-switch / cutover

Do not reinterpret cutover as revive. `classify_master_death` already treats in-flight master cutover as intentional exit (`src/master_revival.rs:86-91`). This design keeps that rule: when cutover state is active, no master recovery window should be created.

The cutover incident findings remain a separate readiness/reap gate issue (`research/ah-master-death-cutover-incident/findings.md:16-26`). This design should not make cutover reap more permissive.

### Startup reconcile

Startup reconcile needs one new step:

1. Before or after `rearm_active_master_watches_on_startup`, scan unexpired master recovery windows.
2. If active and not expired, rearm/route master watch for that session.
3. If expired, mark window FAILED and run cascade/fuse path.

Do not broaden orphan scope cleanup. `reconcile_orphan_scopes_sync` should continue to use DB live refs (`src/db/system.rs:599-637`); the new window should only influence whether a session/agent is considered live during the defer window if needed.

## Implementation sketch

Phase 1: DB helpers only.

- Add schema/table and helpers:
  - `begin_master_recovery_window_sync`
  - `update_master_recovery_phase_sync`
  - `complete_master_recovery_window_sync`
  - `decide_anchor_cascade_sync`
  - `expire_master_recovery_window_sync`
- Make helpers transactional and generation-fenced.

Phase 2: master_watch writes windows.

- After classify Revive, call begin helper before long async work.
- Update phases around worker cleanup, spawn attempt, complete transition, worker reprovision, complete/fail.
- On every early return after a claimed window, mark FAILED/FUSED or COMPLETED as appropriate.

Phase 3: session_watch reads windows.

- Replace lock check with DB decision.
- Keep `master_spawn_lock` for intra-process spawn serialization only.
- On `Defer`, log phase/deadline and continue loop.
- On expired, call cascade with explicit reason.

Phase 4: startup reconcile.

- Add recovery-window reconcile to resume or expire windows across ahd restart.

## Test strategy

Unit tests:

1. `anchor_decision_defers_unexpired_activework_window`: ACTIVE session + window `WORKERS_REAPED`, `active_work=1`, future deadline -> no cascade.
2. `anchor_decision_expires_window_and_cascades`: same but past deadline -> cascade reason `MASTER_REVIVE_WINDOW_EXPIRED`, window FAILED.
3. `anchor_decision_creates_detected_window_for_toctou`: no window, session still ACTIVE with dead expected master and ActiveWork snapshot -> creates `DETECTED` and defers.
4. `anchor_decision_does_not_defer_idle_no_work`: no active workers/jobs -> cascade.
5. `master_watch_updates_window_phases`: simulate revive path and assert phase sequence through completed.
6. `master_watch_failure_marks_window_failed`: spawn/reprovision error does not leave indefinite active window.
7. `startup_reconcile_preserves_unexpired_window_after_ahd_restart`: no in-memory lock, DB window still causes defer/rearm.
8. `startup_reconcile_expires_old_window`: expired window leads to final reap.
9. `cutover_inflight_does_not_create_recovery_window`: preserves self-switch boundary.
10. `idle_worker_master_death_reap_only`: master death with IdleNoWork still reaps, no `REVIVE_IDLE`.

E2E/manual dogfood:

1. ActiveWork master kill: worker home exists past old anchor debounce; revived worker resumes transcript and requeued job completes.
2. Force master revive spawn failure: home preserved only until deadline; after deadline session/worker are reaped and no provider process remains.
3. ahd restart during revive window: restart ahd after worker cleanup but before reprovision; unexpired DB window prevents anchor cascade and recovery resumes or expires deterministically.
4. Idle/no-work master kill: no revive, workers reaped, no home preservation beyond normal cleanup.
5. Self-switch/cutover: cutover active state prevents recovery window creation.

## PM 决策点

1. **持久状态载体**: 推荐新表 `master_recovery_windows`; 备选为 `sessions` 增列。
2. **默认 deferral timeout**: 推荐 90s, env override `AH_MASTER_RECOVERY_CASCADE_DEFER_SECS`, clamp 10s..300s。
3. **总窗口上限**: 推荐 `created_at + 300s` 硬上限, 即使 master retry backoff 延长也不能超过。
4. **过期动作**: 推荐 mark window FAILED + cascade reason `MASTER_REVIVE_WINDOW_EXPIRED` + session terminalization; PM需确认状态用 `FAILED` 还是 `KILLED`。
5. **TOCTOU 分支是否允许 session_watch 创建 recovery window**: 推荐允许, 但只在 ACTIVE + ActiveWork snapshot + no cutover in-flight 时。
6. **审计留存**: 推荐 completed/failed window 行保留到 session 结束; 备选完成后删除, 只靠 events/log。
7. **startup reconcile 优先级**: 推荐先处理 recovery windows, 再 ordinary orphan scope cleanup, 防止恢复窗口内 scopes 被误判 orphan。

## 已拍板 (PINNED 2026-06-29 — PM + 主控联合复核 APPROVE,实现按此为准,不再 re-ask)

设计方向 APPROVE:用 DB 权威 `master_recovery_windows` 状态机替代易失内存锁 `master_spawn_lock`(后者 ahd 重启即丢 + TOCTOU + 无超时泄漏致 worker 僵尸)。两不变量为硬约束:A=每个 defer 有 deadline、过期必放 cascade(防僵尸);B=仅 `active_work=1` 未过期窗口内抑制级联保 home。

7 决策点:
1. **载体**: 新表 `master_recovery_windows`(不污染 sessions 审计字段;FK `ON DELETE CASCADE` 随 session 清)。
2. **默认 deferral**: 90s,env override `AH_MASTER_RECOVERY_CASCADE_DEFER_SECS`,clamp 10–300s。
3. **总上限**: `created_at + 300s` 硬上限,master retry backoff 延长 `defer_until` 也不得超过——这是不变量 A 的命门,必须有。
4. **过期动作**: window 置 **FAILED** + cascade reason **`MASTER_REVIVE_WINDOW_EXPIRED`**(两样都不能少)。session 终态优先 **FAILED**(语义=recovery 失败,与现有 fuse 一致;agent 仍由 cascade 置 KILLED)。若 FAILED 与 cascade 的 KILLED 写法难调和,KILLED 亦可接受——选实现更干净者,但 window=FAILED + reason 必须落,且须确认下游对该终态 session 无误判。
5. **TOCTOU 分支**: 允许 session_watch 在 `session ACTIVE + ActiveWork snapshot + no cutover in-flight` 时创建 `DETECTED` window——闭合「anchor 确认早于 master_watch 拿锁」竞态的命门,必须实现。
6. **审计留存**: completed/failed window 行保留到 session 结束(靠 FK cascade 清)。
7. **startup reconcile 顺序**: 先处理 recovery windows,再 ordinary orphan scope cleanup。

实现期必须钉死的 3 条(主控复核补充):
- **非-Revive 立即 override**: master_watch 若 classify 出 IdleNoWork / cutover-in-flight,必须**立即 expire/override** session_watch 抢先建的 window,不得干等 90s 才放 cascade。
- **generation fencing 是命门**: `expected_generation/claimed_generation` 是 ahd 重启正确性的根本——所有 helper 必须 generation-fenced + 事务化,绝不对 stale 代 window 动作。
- **race-safe upsert**: session_watch 的 create-on-TOCTOU 与 master_watch 的 begin 必须 idempotent / race-safe(同事务 insert-or-upsert,两个 writer 并发抢建不得产生双窗口或丢更新)。

实现节奏:a1 按 4 phase 严格 TDD(先写 10 单测红 → 实现绿),每 phase 可 commit;a1 是作者,审计走 PM + 主控(a4 登出);每 commit 主控独立验;5 个可靠性 e2e dogfood 由主控亲自隔离安全跑。
