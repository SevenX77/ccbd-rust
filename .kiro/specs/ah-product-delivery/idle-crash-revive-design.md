# ah 是否应复活「在 IDLE 态意外崩溃」的 worker、如何做

## 结论摘要

建议 ah 区分两类空闲：

1. **干净/预期的空闲**：worker 没有任务、没有异常退出信号，不需要任何恢复动作。
2. **空闲但进程意外崩溃**：worker 之前是 `IDLE`，但 pidfd 已确认进程死亡。这不是“无事可做”，而是拓扑节点意外消失；ah 应复活 worker，以保持声明拓扑完整。

推荐最小方案：在现有 recovery-intent 上新增 `REVIVE_IDLE` action，专门表达“没有 interrupted job 需要 resume/requeue，但需要按 spawn snapshot 重生同一个 worker”。`REVIVE_IDLE` 只覆盖 worker IDLE unexpected crash，不改 master 死亡连坐、不改 active-work `REVIVE + resume/requeue`、不改 session/master watch 已有语义。

本案根因在上游 agy worker 进程崩溃；ah 的职责是韧性兜底，避免 worker 拓扑静默缩水。

## 范围边界

本设计只处理 **worker 在 `IDLE` 态意外崩溃后的复活**。

明确不触碰：

- master 被杀后的连坐清 worker 语义：继续保持“清 worker + 仅在跑任务才 revive/resume”的既有决议。
- `session_watch` 级联删除与 `master_watch` 复活之间的已知竞争。
- `master_watch` 重启后重装探针已修路径。
- master/worker 已有 active-work revive/resume 语义。

## 当前行为分析

### 1. pidfd 确认 worker 进程死亡

`agent_watch` 为每个 worker 进程注册 pidfd watcher：`src/monitor/agent_watch.rs:9`。当 pidfd readable 后，代码先等待 50ms，再用 `kill(pid, 0)` 和 zombie 检查确认进程是否真的死了：`src/monitor/agent_watch.rs:31`、`src/monitor/agent_watch.rs:32`、`src/monitor/agent_watch.rs:95`、`src/monitor/agent_watch.rs:99`。

确认死亡后，watcher 记录日志 `"agent pidfd confirmed dead"` 并读取 exit code：`src/monitor/agent_watch.rs:47`、`src/monitor/agent_watch.rs:49`、`src/monitor/agent_watch.rs:53`。除 `PROMPT_PENDING` 被明确跳过外，它调用 `mark_agent_crashed_with_exit`：`src/monitor/agent_watch.rs:55`、`src/monitor/agent_watch.rs:56`、`src/monitor/agent_watch.rs:68`。

### 2. crash 事务捕获 previous_state 与 recovery intent

`mark_agent_crashed_sync` 在事务里先读 `previous_state`：`src/db/agents_lifecycle.rs:148`，再把 agent 更新为 `CRASHED`：`src/db/agents_lifecycle.rs:156`、`src/db/agents_lifecycle.rs:158`。更新成功后，它调用 `capture_recovery_intent_for_crash`：`src/db/agents_lifecycle.rs:172`、`src/db/agents_lifecycle.rs:178`，然后把 DISPATCHED job 标失败：`src/db/agents_lifecycle.rs:179`。

intent 捕获逻辑会查同一个 agent 的 `session_id/provider/state_version`：`src/db/agents_lifecycle.rs:230`，再查是否存在 `DISPATCHED` job：`src/db/agents_lifecycle.rs:252`、`src/db/agents_lifecycle.rs:257`。

当前 action 判定是：

- `previous_state in (WAITING_FOR_ACK, BUSY)` 或存在 interrupted job：`REVIVE`，见 `src/db/agents_lifecycle.rs:276`、`src/db/agents_lifecycle.rs:279`。
- `previous_state in (IDLE, SPAWNING)`：`REAP_ONLY`，见 `src/db/agents_lifecycle.rs:280`、`src/db/agents_lifecycle.rs:281`。
- 其它状态默认 `REAP_ONLY`：`src/db/agents_lifecycle.rs:283`。

因此，`IDLE + no DISPATCHED job` 的 worker 意外死亡会被持久化为 `REAP_ONLY`。

schema 目前也只允许两种 action：`REVIVE` 和 `REAP_ONLY`，见 `src/db/recovery.rs:68`、`src/db/recovery.rs:117`、`src/db/recovery.rs:119`。

### 3. orchestrator 对 REAP_ONLY 只回收，不重生

orchestrator 每轮扫描 `CRASHED` agent：`src/orchestrator/mod.rs:290`、`src/orchestrator/mod.rs:292`，读取 recovery intent：`src/orchestrator/mod.rs:313`、`src/orchestrator/mod.rs:315`。

若 action 是 `REAP_ONLY`，当前路径直接记录：

`reaping crashed worker without respawn because recovery intent is REAP_ONLY`

对应代码在 `src/orchestrator/mod.rs:342`、`src/orchestrator/mod.rs:343`、`src/orchestrator/mod.rs:346`。随后它清理 runtime resources 并删除 agent row：`src/orchestrator/mod.rs:348`、`src/orchestrator/mod.rs:349`。`delete_agent_sync` 是直接 `DELETE FROM agents WHERE id = ?`：`src/db/agents.rs:162`、`src/db/agents.rs:164`。

结果是：IDLE worker 进程意外死掉后，ah 捕获到 `REAP_ONLY`，执行 cleanup/delete，但不会 respawn；拓扑静默少一个 worker。

## 应不应该复活 IDLE crash worker

应该，但必须限定为“unexpected crash 后的拓扑修复”，不是“空闲也要做恢复任务”。

理由：

- `IDLE` 只说明没有正在执行的 job，不说明 worker 进程可以消失。
- ah 的 worker 拓扑是用户/PM 声明出来的容量与 provider 组合；意外少一个 worker 会改变后续调度能力。
- 当前 `REAP_ONLY` 把“没有 interrupted job 可恢复”和“不需要保留 worker 实体”混为一谈，导致 IDLE crash 被当成正常收尸。
- 上游 agy 崩溃不是 ah 的错，但 ah 可以把它变成可恢复故障，避免用户只有从日志里才知道拓扑缩水。

不应复活的情况仍然存在：

- ah 主动 kill 的 worker。
- master 死亡连坐清理出来的 idle/no-work worker，按既有决议只清理，不 revive。
- session 已经非 ACTIVE 或正在被级联删除。
- spawn snapshot 缺失，无法可靠重建。
- crash-loop 已触发 fuse/backoff 上限。

## 判别机制

### 主动 KILLED vs 凭空消失

现有状态已经提供最小判别基础：

- 主动 kill/master-death cleanup 走 `mark_agent_killed_sync_inner`，写 `KILLED`，并可在 master-death 场景选择性捕获 revive intent：`src/db/agents_lifecycle.rs:31`、`src/db/agents_lifecycle.rs:35`、`src/db/agents_lifecycle.rs:73`、`src/db/agents_lifecycle.rs:75`。
- 意外退出由 `agent_watch` pidfd 死亡确认后走 `mark_agent_crashed_with_exit`，写 `CRASHED` 和 `AGENT_UNEXPECTED_EXIT`：`src/monitor/agent_watch.rs:68`、`src/db/agents_lifecycle.rs:130`、`src/db/agents_lifecycle.rs:135`。
- crash path 不覆盖已是 `KILLED` 的行：`src/db/agents_lifecycle.rs:158` 的 SQL 排除了 `KILLED`。

因此，推荐判别规则：

1. 只有 `agent_watch` 导致 `CRASHED` 且 intent.reason 属于 unexpected-exit path 时，才考虑 idle revive。
2. `previous_state == IDLE` 且无 interrupted job 时，标为 `REVIVE_IDLE`。
3. 主动 kill 或 master-death cleanup 产生的 `KILLED` 不进入 `REVIVE_IDLE`。
4. 若 agent 已被 session/master 级联删除，外键 cascade 删除 intent，orchestrator 不应尝试重生。

如需更强审计，可后续给 intent 增加 `origin` 字段，例如 `AGENT_PIDFD_EXIT`、`MASTER_DEATH_CLEANUP`、`USER_KILL`。但最小方案不要求先加字段；用 state transition source + reason + previous_state 已足够实现窄修复。

### crash-loop 防护

已有 agents 表带有 recovery backoff 字段：`retry_count`、`next_retry_at`、`retry_exhausted`，见 `src/db/schema.rs:65`、`src/db/schema.rs:66`、`src/db/schema.rs:67`。orchestrator 扫描恢复候选时已经跳过 exhausted 或 `next_retry_at` 未到期的 agent：`src/orchestrator/mod.rs:298`、`src/orchestrator/mod.rs:309`。

已有 claim 也带 CAS 和 backoff gate：`try_claim_agent_recovery_sync` 要求 `state = 'CRASHED'`、`state_version` 匹配、未 exhausted、且 `next_retry_at` 到期：`src/db/recovery.rs:565`、`src/db/recovery.rs:573`、`src/db/recovery.rs:576`、`src/db/recovery.rs:577`、`src/db/recovery.rs:578`、`src/db/recovery.rs:579`。

恢复失败时已有指数-ish backoff：第 1 次 1s、第 2 次 2s、之后 4s，最多 5 次后 exhausted，见 `src/db/recovery.rs:604`、`src/db/recovery.rs:605`、`src/db/recovery.rs:609`、`src/db/recovery.rs:614`。

推荐复用这套机制，不另建 idle 专用计数器。`REVIVE_IDLE` respawn 成功则清 backoff；respawn 失败则 restore `CRASHED` row 并记录 backoff，和现有 `REVIVE` 一致。

需要注意：如果 respawn 成功后新进程立即再次 crash，当前 row 会被 delete/insert，retry_count 是否能跨代保留取决于现有恢复实现。若现有成功 respawn 会清零，PM 需要决定是否接受“只防 spawn 失败 loop”，还是要求“成功后短时间再 crash 也累计 fuse”。推荐 MVP 先复用已有 backoff；若 dogfood 证明 agy 会成功 spawn 后秒崩，再补“rolling crash window”。

## 与现有 recovery-intent 的关系

不建议把 IDLE crash 直接改成现有 `REVIVE`。

原因：

- `REVIVE` 当前还承载“有 active work/interrupted job，需要 resume/requeue”的语义。
- `requeue_interrupted_job_from_captured_intent_sync` 对非 `REVIVE` 直接跳过：`src/db/recovery.rs:323`、`src/db/recovery.rs:327`。如果 IDLE crash 复用 `REVIVE`，虽然没有 interrupted job 时也会跳过，但语义上容易让后续实现误认为需要 resume。
- 既有文档/测试里 `REAP_ONLY` 表达“只清理不复活”，不能继续拿它表达“无 job 但要重生”。

推荐新增 action：

```text
REVIVE_IDLE
```

语义：

- worker unexpected crash。
- `previous_state == IDLE`。
- 没有 interrupted job。
- 目标是恢复声明拓扑，不做 job requeue，不做 provider resume prompt。

orchestrator 行为：

- `REAP_ONLY`：保持现状，cleanup + delete，不 respawn。
- `REVIVE`：保持现状，从 snapshot respawn，并按 captured interrupted job 做 resume/requeue。
- `REVIVE_IDLE`：走 snapshot respawn，但传入的 intent 不触发 requeue；respawn 后 agent 应回到正常 spawn/ready/idle 流程。

schema 需要把 action CHECK 从 `('REVIVE', 'REAP_ONLY')` 扩展到 `('REVIVE', 'REVIVE_IDLE', 'REAP_ONLY')`，并更新 enum parser。

## 风险分析

### crash-loop

风险：agy 上游持续崩溃，ah 不断复活，形成日志/CPU/进程风暴。

控制：

- 复用 `retry_count/next_retry_at/retry_exhausted`。
- 复用 `try_claim_agent_recovery_sync` 的 backoff gate。
- 保持最大 5 次 fuse，或由 PM 决定 IDLE revive 是否使用更低上限。
- 事件里明确标注 `action=REVIVE_IDLE`，便于 dogfood 识别上游 crash-loop。

### double-spawn

风险：多个 orchestrator tick 或并发 watcher 同时看到同一个 `CRASHED` agent，重复 spawn。

控制：

- 必须继续先 `try_claim_agent_recovery_sync`，依赖 `state_version` CAS：`src/orchestrator/mod.rs:380`、`src/orchestrator/mod.rs:381`、`src/db/recovery.rs:577`。
- respawn 前确认 spawn snapshot 存在，当前已有 `has_snapshot` gate：`src/orchestrator/mod.rs:353`、`src/orchestrator/mod.rs:355`。
- delete + respawn 失败后的 restore 逻辑要继续保留 captured intent；现有失败分支已 restore 并重新 persist intent：`src/orchestrator/mod.rs:448`、`src/orchestrator/mod.rs:456`、`src/orchestrator/mod.rs:457`、`src/orchestrator/mod.rs:459`。

### 与 master_watch/session_watch 的竞争

风险：master 死亡或 session 级联删除期间，IDLE worker crash 也被捕获，idle revive 抢在级联清理之前 spawn 新 worker。

控制：

- `REVIVE_IDLE` 只在 session 仍为 ACTIVE 时执行；如果 session 已 KILLED/DELETING/非 ACTIVE，跳过或转 `REAP_ONLY`。
- 不修改 master-death cleanup。master 被杀仍按既有决议连坐清 worker，且 idle/no-work worker 不 revive。
- 如果 session_watch 已 cascade 删除 agent row，`agent_recovery_intents.agent_id` 的 FK `ON DELETE CASCADE` 会清掉 intent：`src/db/recovery.rs:56`，orchestrator 查询不到行即不 spawn。
- 如果 race 中 CAS 失败，按现有 `"recovery CAS lost"` 路径继续跳过：`src/orchestrator/mod.rs:388`、`src/orchestrator/mod.rs:391`。

## 推荐的最小改动方案

1. 扩展 recovery-intent action：
   - schema CHECK 增加 `REVIVE_IDLE`。
   - `RecoveryIntentAction` enum 增加 `ReviveIdle`。
   - `as_db_str/from_db_str` 支持新值。

2. 修改 intent 捕获：
   - 在 `capture_recovery_intent_for_crash` 中，当 `previous_state == IDLE` 且 `interrupted.is_none()`，写 `REVIVE_IDLE`。
   - `SPAWNING` 是否也纳入先不做，保持 `REAP_ONLY`，避免把未 ready 的半初始化 worker 纳入本案。

3. 修改 orchestrator 恢复分支：
   - `REAP_ONLY` 保持现状。
   - `REVIVE` 保持现状。
   - `REVIVE_IDLE` 走同一个 snapshot respawn/CAS/backoff 管线，但不 requeue interrupted job。
   - respawn 前增加 session ACTIVE gate，避免和 session/master teardown 竞争。

4. 事件与日志：
   - 复活成功/失败事件里显式带 `action=REVIVE_IDLE` 或 `recovery_kind=idle_topology_restore`。
   - 保留 crash exit_code/error_code，便于证明是上游 agy crash。

5. 测试：
   - `IDLE + no DISPATCHED + pidfd crash` 捕获 `REVIVE_IDLE`。
   - `REVIVE_IDLE` respawn 成功后不 requeue job。
   - `REVIVE_IDLE` respawn 失败后 backoff 增长。
   - `REAP_ONLY` 仍 cleanup/delete，不 respawn。
   - session 非 ACTIVE 时不 idle revive。
   - master-death cleanup 的 idle worker 仍不 revive。

## 需要 PM 拍板的决策点

1. **是否接受 IDLE unexpected crash 自动复活为产品语义**：推荐接受，定义为拓扑完整性修复，不定义为 job resume。
2. **action 命名**：推荐 `REVIVE_IDLE`；备选 `RESTORE_IDLE` 或 `RESPAWN_ONLY`。关键是不要复用 `REVIVE` 或 `REAP_ONLY`。
3. **是否只覆盖 `IDLE`**：推荐 MVP 只覆盖 `IDLE`，不覆盖 `SPAWNING`。`SPAWNING` 崩溃可能代表初始化失败，复活策略和 crash-loop 风险不同。
4. **crash-loop fuse 是否沿用现有 5 次**：推荐先沿用；若 agy 秒崩频繁，再补 rolling window。
5. **session 非 ACTIVE 时的行为**：推荐直接 skip/REAP_ONLY，不做 revive。
6. **是否增加 intent.origin 字段**：推荐 MVP 不加，后续为了审计再加；当前可用 pidfd crash path + `reason` + `previous_state` 实现。

## 最小推荐方案

采用 `REVIVE_IDLE`。

它把当前混在 `REAP_ONLY` 里的两种含义拆开：

- `REAP_ONLY` = 只收尸，拓扑也移除。
- `REVIVE_IDLE` = 无 job 可恢复，但 worker 是意外死亡，需要按 snapshot 重生以恢复拓扑。

实现上最大化复用现有 `CRASHED -> CAS claim -> snapshot respawn -> backoff/fuse` 管线，只在 action 判定和 orchestrator 分支上做窄改动，并加 session ACTIVE gate。这样不会触碰 master 连坐、session_watch/master_watch 竞争修复、或 active-work revive/resume 语义。
