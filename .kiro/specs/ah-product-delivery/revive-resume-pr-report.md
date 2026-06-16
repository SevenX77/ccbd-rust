# revive/resume PR Report

## 1. 目标

这组变更覆盖 `f90d803`, `8c12360`, `45b86a5`, `8087104`。目标是 ah Step-4 master self-heal 的关键闭环: managed master OOM / 意外退出时, 如果 worker 上已有 `DISPATCHED` job, 该 job 不能只停留在 `QUEUED`, 也不能被旧失败补偿永久写成 `FAILED`; master 复活、worker 重建并通过 readiness 后, 原 job 必须被续上并重新派发到新 pane 执行。

## 2. 端到端数据流

1. master death 检测进入 `revive_master_after_exit`。入口先拿 session 级 spawn lock, 然后读取 master 死亡时的 session/worker snapshot: `src/monitor/master_watch.rs:94`, `src/monitor/master_watch.rs:105`, `src/monitor/master_watch.rs:107`。随后 `clean_worker_runtime_resources_sync` 负责清理该 session 下要 reap 的 workers: `src/monitor/master_watch.rs:111`。

2. worker reap 不是普通 kill。`clean_worker_runtime_resources_with_runner_sync` 会停 marker/parser/completion/monitor, 尝试 systemd scope 和 pidfd 清理, 最后对每个 worker 调 `mark_agent_killed_for_master_death_sync`: `src/db/system.rs:226`, `src/db/system.rs:237`, `src/db/system.rs:255`, `src/db/system.rs:297`, `src/db/system.rs:330`。

3. `mark_agent_killed_for_master_death_sync` 和普通 `mark_agent_killed_sync` 的差异是 `capture_revive_intent=true`: `src/db/agents_lifecycle.rs:23`, `src/db/agents_lifecycle.rs:31`, `src/db/agents_lifecycle.rs:49`。它在同一个事务内、`mark_dispatched_jobs_failed_for_agent_conn_sync` 之前捕获 recovery intent: `src/db/agents_lifecycle.rs:61`, `src/db/agents_lifecycle.rs:80`。这么做是必须的, 因为 `agent_recovery_intents.agent_id` 和 `jobs.agent_id` 都依赖 agent 生命周期, master revive 后会 delete/recreate worker; 不先把 job 的值捕到 recovery intent, 后续就可能被 FK cascade 或 delete/reprovision 时序吃掉。

4. captured-value 的 schema 明确保存中断 job 的 `id/status/request_id/prompt_text/cancel_requested/requires_*`: `src/db/recovery.rs:19`, `src/db/recovery.rs:26`, `src/db/recovery.rs:28`, `src/db/recovery.rs:29`, `src/db/recovery.rs:30`, `src/db/recovery.rs:31`, `src/db/recovery.rs:32`。捕获逻辑从当前 dispatched job 读这些列并构造 `CapturedInterruptedJob`: `src/db/agents_lifecycle.rs:198`, `src/db/agents_lifecycle.rs:228`, `src/db/agents_lifecycle.rs:273`。

5. master spawn 成功后, 代码先 best-effort 注入 continue, 再 reprovision workers: `src/monitor/master_watch.rs:330`, `src/monitor/master_watch.rs:343`。reprovision 先把本轮要用的 captured intents 从 DB 读到内存, 再逐个重建 worker: `src/monitor/master_watch.rs:379`, `src/monitor/master_watch.rs:394`, `src/monitor/master_watch.rs:408`。单 worker 重建时, 若旧 row 已是 `KILLED`, 先 delete 旧 agent row, 再用 `spawn_realign_agent(..., is_recovery=true)` 启新 pane: `src/monitor/master_watch.rs:442`, `src/monitor/master_watch.rs:460`, `src/monitor/master_watch.rs:461`, `src/monitor/master_watch.rs:463`。这里的 in-memory captured intents 是 `8c12360` 的关键修正: 不在 reprovision/delete 后再回 DB 重查 intent。

6. readiness gate 复用普通 spawn 路径。普通 agent spawn 先插入 `SPAWNING`: `src/rpc/handlers/agent.rs:230`, `src/rpc/handlers/agent.rs:235`; 注册 reader/pane 后无条件启动 init-probe: `src/rpc/handlers/agent.rs:295`, `src/rpc/handlers/agent.rs:312`。`spawn_realign_agent` 只调用 `handle_agent_spawn_with_recovery` 并刷新 config hash / spawn snapshot, 不直接写 `IDLE`: `src/rpc/handlers/realign.rs:314`, `src/rpc/handlers/realign.rs:322`, `src/rpc/handlers/realign.rs:336`, `src/rpc/handlers/realign.rs:342`。最终只有 init-probe 在 readiness 成立后调用 `mark_agent_idle_matched`, 并对受影响 job 发 `notify_job_update`: `src/provider/init_probe_task.rs:573`, `src/provider/init_probe_task.rs:575`, `src/provider/init_probe_task.rs:576`。

7. worker 重建后, master revive 用内存里的 captured intents 续 job: `src/monitor/master_watch.rs:352`, `src/monitor/master_watch.rs:608`, `src/monitor/master_watch.rs:622`, `src/monitor/master_watch.rs:630`。DB helper 先尝试把还在表里的 `FAILED` job 改回 `QUEUED`, 写 `RECOVERY_REQUEUED:0`: `src/db/recovery.rs:282`, `src/db/recovery.rs:307`, `src/db/recovery.rs:317`。如果 job row 已不存在, 就用 captured-value 复用原 job id 重新 insert `QUEUED`: `src/db/recovery.rs:327`, `src/db/recovery.rs:340`, `src/db/recovery.rs:341`; insert helper 保存原 `request_id/prompt_text/cancel/evidence` 并写 marker: `src/db/jobs.rs:72`, `src/db/jobs.rs:83`, `src/db/jobs.rs:85`。

8. dispatch loop 只会从可派发 agent 取 pane。进入 send 前会刷新 registry 中可能陈旧的 pane binding: `src/orchestrator/mod.rs:95`, `src/orchestrator/mod.rs:777`, `src/orchestrator/mod.rs:783`, `src/orchestrator/mod.rs:786`, `src/orchestrator/mod.rs:787`。这依赖 registry 的 `update_pane_id`: `src/agent_io/registry.rs:41`。

9. 如果 send 失败, 先走 stale-dispatch fence。`run_once` 在 mark failed 之前调用 `stale_dispatch_failure_already_requeued`: `src/orchestrator/mod.rs:145`, `src/orchestrator/mod.rs:147`。该 helper 重读当前 job; 只有当当前 job 仍属于同 agent、状态已是 `QUEUED`, 且 `error_reason` 是 `RECOVERY_REQUEUED:*`, 才判定旧 in-flight writer 已失去所有权并跳过 destructive failure compensation: `src/orchestrator/mod.rs:929`, `src/orchestrator/mod.rs:936`, `src/orchestrator/mod.rs:947`, `src/orchestrator/mod.rs:949`, `src/orchestrator/mod.rs:958`。

10. stale pane 的 recoverable 防御网只覆盖 marker 标记的 recovered job。pane missing 且 marker 仍在时, 最多把 job 再放回 `QUEUED` 一次并递增 marker attempt: `src/orchestrator/mod.rs:154`, `src/orchestrator/mod.rs:974`, `src/orchestrator/mod.rs:984`, `src/orchestrator/mod.rs:994`, `src/orchestrator/mod.rs:1012`; DB 侧只允许当前 job 是 `DISPATCHED` 且 marker 仍匹配时更新: `src/db/jobs.rs:469`, `src/db/jobs.rs:481`, `src/db/jobs.rs:489`, `src/db/jobs.rs:490`。

11. send 成功后清 marker。dispatch 成功路径检查本次派发快照是否带 `RECOVERY_REQUEUED` marker, 然后调用 `clear_recovered_dispatch_marker`: `src/orchestrator/mod.rs:199`, `src/orchestrator/mod.rs:200`。DB 清理只允许当前 job 是 `DISPATCHED` 或 `COMPLETED` 且 marker 仍匹配: `src/db/jobs.rs:530`, `src/db/jobs.rs:536`, `src/db/jobs.rs:539`, `src/db/jobs.rs:540`。

## 3. Dogfood 钉死的两个根因

readiness-gate 是实现 bug, 不是设计缺陷。设计要求 recovery/realign spawn 和普通 spawn 共享 `SPAWNING -> init-probe -> IDLE` gate; 旧实现的问题是 realign spawn 成功后过早把 agent 置为 `IDLE`, 让 recovered job 抢派到尚未完成 init-probe 的 pane。修法是让 `spawn_realign_agent` 只刷新 hash/snapshot, 不再直接发布 `IDLE`: `src/rpc/handlers/realign.rs:314`, `src/rpc/handlers/realign.rs:336`, `src/rpc/handlers/realign.rs:342`; 真正的 `IDLE` 只从 init-probe 的 `mark_agent_idle_matched` 出来: `src/provider/init_probe_task.rs:573`。

stale-dispatch-fence 也是实现 fencing bug, 不是 recovery requeue 设计缺陷。requeue 设计本身已经把中断 job 恢复成带 `RECOVERY_REQUEUED:*` 的新 lifecycle; 错的是旧 in-flight dispatch writer 在 master death 跨代之后, 仍拿着旧 job 快照执行 send-failure compensation, 把已 requeued 的 job 覆盖成 `FAILED`。修法是在 destructive write 前 compare-before-write: `src/orchestrator/mod.rs:147` 调 fence, `src/orchestrator/mod.rs:936` 重读当前 job, `src/orchestrator/mod.rs:947` 到 `src/orchestrator/mod.rs:960` 确认 recovery 已接管则跳过补偿。

## 4. 保留的 tactical fixes

原子 marker 保留。`RECOVERY_REQUEUED:<attempt>` 不是长期状态, 而是 recovered dispatch 的短生命周期所有权标记: `src/db/jobs.rs:9`, `src/db/jobs.rs:19`, `src/db/jobs.rs:23`。它让 stale fence、stale pane retry、成功 send 清理都能用同一个 DB 可见事实判断当前 lifecycle, 避免只靠内存状态。

pane-refresh guard 保留。`resolve_current_dispatch_pane` 会在 dispatch 前用 tmux live pane 修正 registry 中的旧 pane id: `src/orchestrator/mod.rs:777`, `src/orchestrator/mod.rs:783`, `src/orchestrator/mod.rs:786`, `src/orchestrator/mod.rs:787`。它覆盖真实 registry mismatch, 但不替代 fence; 因为 fence 处理的是旧 writer 跨 lifecycle 的所有权问题, pane refresh 只修当前 writer 的目标 pane。

## 5. 测试与 dogfood 实证

关键单测:

- `master_revive_worker_reprovision_requeues_captured_interrupted_job`: `src/monitor/master_watch.rs:1349`。覆盖 master revive 完整路径中 worker reprovision 后, captured interrupted job 被恢复为 `QUEUED`, prompt_text 保留。
- `master_revive_recovered_job_waits_for_realign_readiness_before_dispatch`: `src/monitor/master_watch.rs:1453`。覆盖 realign/recovery spawn 后仍处于 `SPAWNING`/`SPAWNING_INTERVENTION`, init-probe 之后才 `IDLE`, recovered job 随后进入 `BUSY` 并清 marker。
- `master_revive_recovered_job_survives_stale_pane_dispatch_and_retries_new_pane`: `src/monitor/master_watch.rs:1579`。覆盖 recovered job 派到 stale pane 时不会直接永久失败, 而是受限 retry。
- `master_revive_stale_inflight_dispatch_failure_does_not_overwrite_requeued_job`: `src/monitor/master_watch.rs:834`。覆盖旧 in-flight dispatch 失败不能覆盖已经 recovery-requeued 的 job, 后续能重新派发并清 `RECOVERY_REQUEUED` marker。

已跑过的验证命令:

- `CARGO_BUILD_JOBS=1 cargo test --release --lib master_revive -- --test-threads=1`: `20 passed; 0 failed; 1 ignored`。
- `CARGO_BUILD_JOBS=1 cargo test --release --lib -- --test-threads=1`: `644 passed; 0 failed; 3 ignored`。
- marker-clear targeted regression: `1 passed; 0 failed; 646 filtered out`。

dogfood 实证:

- S1: worker crash/recovery 回归仍可复活 worker 并恢复中断 job。
- S2: idle/no-work master death 不制造无意义 redispatch。
- S3: managed master OOM during in-flight dispatch, 0.5s 与 2.0s enter-delay 窗口都能把 recovered job 续到 `BUSY` / `WAITING_FOR_ACK`, 且旧 writer 不再把 job 覆盖成 `FAILED`。

诚实标注: drift-realign 没有单独 true-scope dogfood; 它通过共享的 `spawn_realign_agent -> handle_agent_spawn_with_recovery -> init-probe` 结构覆盖 readiness 语义。

## 6. 诚实边界

未额外实现 cleanup-cancel old in-flight writer。当前闭环靠 DB owner/marker fence 保证旧 writer 的失败补偿不能破坏 recovered lifecycle; 旧 writer task 本身仍可能跑到 send failure 分支, 但 destructive write 前会重读并退出。

`RECOVERY_REQUEUED:*` 复用 `jobs.error_reason` 作为短生命周期 marker, 不是独立列。优点是迁移小、所有 dispatch 路径立刻可见; 代价是 report/排障时要区分 transient recovery marker 和真正失败原因。

marker 是 dispatch-success 时清, 不是进入 `WAITING_FOR_ACK` 前的外部可见强同步屏障。成功 send 路径在看到 `RECOVERY_REQUEUED:*` 后调用 `clear_recovered_dispatch_marker`: `src/orchestrator/mod.rs:199`, `src/orchestrator/mod.rs:200`; DB 侧只清当前 `status IN ('DISPATCHED', 'COMPLETED')` 且 marker 匹配的 job: `src/db/jobs.rs:530`, `src/db/jobs.rs:536`, `src/db/jobs.rs:539`, `src/db/jobs.rs:540`。因此 `QUEUED -> DISPATCHED -> (send 成功 clear) -> WAITING_FOR_ACK/BUSY` 中存在短暂可观测窗口: 采样恰好落在 clear commit 前可能看到 `WAITING_FOR_ACK` 旁边仍有 marker。这是设计接受的 transient marker, 不是逻辑残留; clear 发生在 job 进入 `COMPLETED` 前, 不会把 marker 滞留到终态污染排障。

stale pane retry 是 defense-in-depth, 当前限制为一次: `src/db/jobs.rs:10`, `src/orchestrator/mod.rs:994`。如果 tmux pane registry 长时间不一致, 第二次仍会按普通失败处理, 避免无限重试遮蔽真实环境故障。
