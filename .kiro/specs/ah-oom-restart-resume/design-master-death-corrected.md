# Design: Corrected Master-Death Semantics

## 1. 背景与纠正语义

PR #52 当前实现把 master 非预期退出解释为 revive 事件，并保持既有 workers 不动。这能保住 master conversation，但违背 PM 纠正后的核心目标：master 被杀时，旧 worker/process 必须先清理，避免 zombie/orphan worker 持续堆积。

PM 拍定语义如下，作为本设计唯一标尺：

> master被杀需要分两种情况：a.正在跑任务过程中；b.没有任务待命中；a情况需要自动拉起，b不需要；master被杀不能反转连坐杀掉所有worker的语义，无论是情况a还是b都有可能因为被杀前的任务过程中开了太多进程，进程必须被清理；拉起继续是重新开启，resume后继续，不是worker全程不动。这个策略是为了防止僵尸、孤儿进程。如果因为各种不知名原因bug或者不当操作导致启动了新的master，没有清理老的worker，那么这些worker和进程就会留在那儿越堆越多。

设计目标：

- master 死亡后先清理该 session 的所有 worker runtime resources。
- 清理后再按 master 死亡前的 A/B 快照决定是否 revive master。
- 情况 A：清完旧 worker 后，复用 PR #52 的 CAS/backoff/revive 原语拉起新 master，并依赖 resume/continue 继续会话。
- 情况 B：清完旧 worker 后，不拉起 master；ahd 继续常驻。
- 不再保留“workers untouched”假设。

## 2. 前提与约束

### OOMScore 事实

本设计不修改 OOMScore 配置，只把它作为 master-death 触发概率和恢复边界的前提。

- ahd 受保护：`src/cli/start.rs:42-56` 构造 `systemd-run --unit=ahd.service`，带 `--property=OOMScoreAdjust=-900`。
- master 最容易被杀：`src/sandbox/systemd.rs:99-105` 的 `master_command_with_env` 带 `--property=OOMScoreAdjust=500`。
- worker 默认无正向 OOMScore：`src/sandbox/systemd.rs:13-46` 的 agent `wrap_command` 没有 `OOMScoreAdjust`。
- 这个排序符合 task #25 目标：ahd 常驻受保护，master 更容易被 OOM killer 选中。
- caveat：`src/bin/ah.rs:276-309` 的 direct-spawn fallback 不受 systemd OOMScore 保护。本设计不修这个 fallback。

### 现有原语

保留并复用：

- master pidfd watcher：`src/monitor/master_watch.rs:20-80`。
- master revive CAS/generation/backoff/schema 列：`src/master_revival.rs` 和 `src/db/schema.rs` 的 `master_retry_count` / `master_next_retry_at` / `master_generation` / `master_last_exit_reason`。
- master revive spawn 路径：`src/monitor/master_watch.rs:86-258`，但需要插入 A/B 快照、真 Reap 和时序重排。
- startup reconcile：`src/db/system.rs:363` / `src/db/system.rs:939`，只作为 ahd 重启后的兜底收尸，不替代当场清理。

必须改变：

- `cascade_kill_session_agents_with_runner_sync` 当前在 `src/db/system.rs:151-157` 写死 `UPDATE sessions SET status='KILLED'`。这个行为不能用于 master-death 情况 A，因为 `classify_master_death` 会把非 ACTIVE session 当 IntentionalExit/Stale，导致无法 revive。
- `src/master_revival.rs:331-383` 的 fuse 当前会再次查 worker 并只做 `mark_agent_killed_sync`，这不是“真 Reap”。真 Reap 前置后，fuse 只负责 session FAILED 和报警。
- `auto_shutdown_on_master_exit` 已成为死流水线：`src/monitor/master_watch.rs:27/59/92/250` 还在传递，但没有实际 consumer。后续实现应删除该配置和调用链。

## 3. Master-Death 完整流水线

新流水线：

1. **detect**：master pidfd readiness 触发，沿用 `spawn_master_pidfd_watch_task`。
2. **classify intent**：调用 `classify_master_death(db, session_id, expected_pid, expected_generation)`。只有 `Revive` 进入意外死亡处理；`IntentionalExit` / `Stale` 直接忽略。
3. **enter serialized section**：在当前 `revive_master_after_exit` 中拿 `master_spawn_lock(session_id)` 后继续，避免同一 session 多个 watcher/revive 交错。
4. **A/B 事务快照**：在任何清理前，用 DB 事务计算该 session 是情况 A 还是 B，并暂存本轮应清理的 worker ids。
5. **无条件真 Reap**：无论 A/B，先执行 `clean_worker_runtime_resources(session_id, worker_ids, reason="MASTER_EXIT")`。这一步不改变 `sessions.status`。
6. **分叉**：
   - 情况 A：进入 PR #52 revive 流水线，执行 backoff、claim transition、record attempt、spawn replacement master、register new watcher。
   - 情况 B：不进入 revive，不拉 master，ahd 保持运行。
7. **monitor cleanup**：移除旧 generation master monitor key。

重要时序：真 Reap 必须前置到 `src/monitor/master_watch.rs:101` backoff、`:127` claim transition、`:140` record/fuse 之前。这样即使 A 等 backoff、A 熔断、B 不 revive，旧 worker 都已经被清。

## 4. A/B 判定谓词

判定必须基于清理前的事务快照。清理会把 workers 标 KILLED 并注销 runtime，如果清理后再判定会把所有情况错误折叠成 B。

### 情况 A：在跑任务，需要 revive

满足任一条件即为 A：

- session 下任一 worker 处于 active 状态：`SPAWNING` / `WAITING_FOR_ACK` / `BUSY`。现有定义见 `src/db/state_machine.rs:59` 的 `is_active_state`。
- session 下任一 worker 处于 `PROMPT_PENDING`。
- session 下任一 job 处于 `QUEUED` 或 `DISPATCHED`。

`PROMPT_PENDING` 归 A：该状态代表 worker 停泊等待 master 或人类输入，属于未完结在途会话；如果不 revive master，会话上下文无法继续。

`QUEUED` only 归 A：即使所有 worker 当前 IDLE，只要存在 QUEUED job，orchestrator 下一步就要发任务；master 不在会切断后续会话。

### 情况 B：待命无任务，不 revive

同时满足：

- session 下所有 worker 都是 `IDLE`、`CRASHED` 或 `KILLED`。
- session 下没有 `QUEUED` job。
- session 下没有 `DISPATCHED` job。

B 的语义是“真正全空闲”：清 worker 后不拉 master，ahd 继续常驻，等待后续显式启动/操作。

### Query 原语缺口

现有 jobs helper 是 per-agent：

- `src/db/jobs.rs:82` 的 `has_queued_job_sync(conn, agent_id)`。
- `src/db/jobs.rs:437` 的 `query_dispatched_job_for_agent_sync(conn, agent_id)`。

后续实现需要新增 session 级聚合 helper，例如 `snapshot_master_death_session_activity(session_id)`，一次事务内返回：

- `classification: ActiveWork | IdleNoWork`
- `worker_ids_to_reap: Vec<String>`
- 可选诊断计数：active workers、prompt pending workers、queued jobs、dispatched jobs。

该 helper 不应 mutate DB。

## 5. 清理原语设计

新增/抽取 `clean_worker_runtime_resources(session_id, worker_ids, reason, daemon_marker, runner)`。它从 `src/db/system.rs:144` 的 cascade 清理块抽出，但跳过 `src/db/system.rs:151-157` 的 session KILLED update。

### 行为契约

- 不修改 `sessions.status`。
- 对所有 `worker_ids` 做真实 runtime 清理。
- 对 worker DB 状态调用 `mark_agent_killed_sync(db, agent_id, reason)`，使 jobs/events 语义保持现有 lifecycle 行为。
- 清理必须幂等：worker 已 CRASHED/KILLED、pidfd 已消失、scope 已消失、registry 已缺失都不能让整个 cleanup hard fail。
- 返回清理结果摘要：DB killed count、scope stop failures、pidfd kill failures、registry cleanup count 等，便于 master-death path 打日志。

### 真 Reap，不是 DB mark

真 Reap 包含：

- 停 agent systemd scope：现有 `src/db/system.rs:189-191` 通过 `stop_agent_scopes_with_runner` / `stop_session_anchor_with_runner`。
- pidfd SIGKILL fallback：现有 `src/db/system.rs:199` 通过 `monitor::with_borrowed(agent_id, pidfd_send_sigkill)`；`pidfd_send_sigkill` 在 `src/monitor/mod.rs:42`。
- marker timer cancel：现有 `src/db/system.rs:207-208`。
- parser registry remove：现有 `src/db/system.rs:210`，底层 `src/marker/parser_registry.rs:35`。
- agent_io/runtime registry cleanup：`mark_agent_killed_sync` 会走 `src/db/agents_lifecycle.rs:50-52`，进而调用 `agent_io::registry::cleanup_agent_runtime_resources`。
- completion monitor cancel 和 monitor pidfd remove：现有 `src/agent_io/registry.rs:132-137` 已包含 `marker::registry::take`、`completion::registry::cancel`、`parser_registry::remove`、`monitor::remove`。

`src/master_revival.rs:380` 只 `mark_agent_killed_sync(..., "MASTER_REVIVE_FUSED")` 的旧 fuse 逻辑不能作为 master-death cleanup 的标准，因为防僵尸命门是进程和 registry 真清理，而不只是 DB 标 KILLED。

### 下架顺序

清 worker 时必须先从内存态下架，再停 pane/scope：

1. 对 agent_io/marker/parser/completion/monitor registry 做下架或 cancel。
2. 再 stop systemd scope。
3. 再 pidfd SIGKILL fallback。
4. 再 kill tmux agent session/pane 和清 sandbox runtime。
5. 最后或过程中执行 `mark_agent_killed_sync`，但要避免它内部 cleanup 与前置 cleanup 冲突；重复 cleanup 必须幂等。

理由：`src/orchestrator/mod.rs:78-128` 和 `src/rpc/handlers/ack.rs:33-51` 会基于 pane/parser registry 继续 capture 或 fallback。如果先杀 pane 再 remove registry，短窗口内可能把 capture failure 转成 STUCK/CRASHED 噪声，并造成 I/O 报警风暴。

### no-systemd 主路径

systemd scope stop 不是唯一前提。无 systemd 或 unsafe/no-sandbox 下仍必须执行：

- registry 下架；
- pidfd SIGKILL fallback；
- tmux agent session/pane cleanup；
- DB `KILLED` lifecycle transition；
- sandbox/fifo/runtime cleanup。

如果没有 daemon marker 或 systemd runner 不可用，只跳过 scope stop，不跳过其他 cleanup。

### 双重失败降级

如果 scope stop 和 pidfd SIGKILL 都失败，记录 ERROR，不是普通 warning。日志必须明确：

- agent_id/session_id；
- scope stop error；
- pidfd kill error；
- cleanup 已从 DB/registry 下架；
- 残留 worker 交给 startup reconcile / OS 兜底。

这不是“无害”。这是低概率 degraded mode。继续 revive 的理由是优先恢复 master，但文档和日志都必须诚实标注残留风险：老 worker 可能继续占用资源或执行外部副作用，直到进程退出、ahd 重启 reconcile 或 OS 清理。

## 6. Backoff 与 Fuse 衔接

当前 revive 路径顺序是：

- `src/monitor/master_watch.rs:98` 拿 spawn lock；
- `src/monitor/master_watch.rs:101` backoff；
- `src/monitor/master_watch.rs:127` claim transition；
- `src/monitor/master_watch.rs:140` record revive attempt，可能触发 fuse；
- `src/monitor/master_watch.rs:204` spawn replacement master。

新顺序：

1. 拿 spawn lock。
2. A/B 事务快照。
3. 真 Reap。
4. 如果 B：返回，不 revive。
5. 如果 A：检查 backoff；如果需要 sleep，worker 已经清完，等待期间不会继续孤儿化。
6. backoff 醒来后重新 classify，仍为 Revive 才继续。
7. claim transition。
8. record revive attempt。
9. 若 Spawn：拉起 replacement master。
10. 若 Fused：只标 session FAILED、记录 ERROR、清 session anchor，不再杀 worker。

`fuse_session_after_master_revive_exhausted` 后续应瘦身：

- 保留 retry_count 查询和 ERROR。
- 保留 `sessions.status='FAILED'`、`master_last_exit_reason='FUSED'`。
- 移除或改为幂等 no-op 的 worker kill 循环。
- 增加 session anchor cleanup，避免 failed session anchor 残留。

## 7. 防孤儿纵深

当场清是主路径：

- ahd 存活时，master pidfd watcher 立即执行 A/B 快照和真 Reap。
- 情况 A/B 都先清 worker。
- 情况 A 才 revive master。

startup reconcile 是次级兜底：

- `src/db/system.rs:363` 的 orphan scope reconcile 负责 ahd 重启后的系统级收尸。
- `src/db/system.rs:939` 的 `reconcile_startup_with_tmux_socket` 在 ahd startup 时运行。
- 它覆盖 ahd 与 master 同时死亡、当场 cleanup 没机会执行、或双重失败后残留 scope 的恢复窗口。

边界：startup reconcile 不能替代 master-death 当场清理。PM 目标是防止 worker 越堆越多；只靠 ahd 重启才清理会让常驻 ahd 场景泄漏长期存在。

## 8. 删除项

删除 `auto_shutdown_on_master_exit` 相关死流水线：

- 配置字段：`src/cli/config.rs:55`。
- RPC 参数传递和 master watcher 参数：`src/monitor/master_watch.rs:27/59/92/250`。
- 相关测试断言需同步移除或改写。

原因：

- PR #52 已删除 `schedule_daemon_shutdown_if_idle`，当前 `src/` 无实际 consumer。
- 情况 B 不关停 ahd。ahd 是常驻 hypervisor，继续提供监控、RPC 和 startup reconcile 兜底。
- PM 只要求 B 不拉 master，没有要求关 ahd；恢复 daemon shutdown 属于 scope creep。

## 9. PR #52 原语处置表

| 原语/假设 | 处置 | 说明 |
| --- | --- | --- |
| `classify_master_death` active pid/generation 门控 | 保留 | 仍用于区分 unexpected / intentional / stale watcher。 |
| `master_generation` CAS claim | 保留 | 仍防止 stale watcher 或双 revive。 |
| retry/backoff columns | 保留 | 情况 A revive 仍需要 bounded retry/backoff。 |
| replacement master spawn | 保留并前移 cleanup | 继续用 captured `master_cmd` 和 existing home/env 逻辑。 |
| `master_last_exit_reason` | 保留 | 可继续记录 OOM_OR_CRASH/FUSED 等原因。 |
| fuse worker kill loop | 改 | 真 Reap 前置后，fuse 不再负责杀 worker。 |
| cascade kill whole session | 拆分 | 提取 worker cleanup，不改 `sessions.status`。 |
| workers untouched | 反转 | 彻底作废；master death 必须先 reap workers。 |
| auto shutdown on master exit | 删除 | 情况 B 不关 ahd。 |

## 10. 测试计划

后续 reimpl 必须 test-first 覆盖以下轴。

### 纯 DB / helper 测试

- A/B snapshot：active states `SPAWNING` / `WAITING_FOR_ACK` / `BUSY` 判 A。
- A/B snapshot：`PROMPT_PENDING` 判 A。
- A/B snapshot：仅 `QUEUED` job 且 workers IDLE 判 A。
- A/B snapshot：`DISPATCHED` job 判 A。
- A/B snapshot：workers 全 IDLE/CRASHED/KILLED 且零 QUEUED/DISPATCHED 判 B。
- session 级 helper 一次事务返回 classification 和 worker_ids_to_reap。

### cleanup helper 测试

- `clean_worker_runtime_resources` 不修改 `sessions.status`。
- cleanup 标 worker KILLED、失败 dispatched jobs、写 state_change event。
- cleanup cancel marker registry、remove parser registry、cancel completion registry、remove monitor pidfd。
- no-systemd 路径仍走 pidfd/tmux/registry/DB cleanup。
- scope stop 失败时 pidfd fallback 被调用。
- scope stop + pidfd fallback 双失败记录 ERROR，仍完成 DB/registry 下架。

### master watch / revive 测试

- 情况 A：master raw exit 后先 reap old workers，再 revive master generation，旧 worker 不保持 alive。
- 情况 A backoff：即使 backoff sleep，worker 已先 reap。
- 情况 A fuse：worker 已先 reap，fuse 只 FAILED session 和清 anchor。
- 情况 B：master raw exit 后 reap workers，不 revive master，session 不被错误标 KILLED，ahd 继续常驻。
- intentional session.kill/system.shutdown：不触发 corrected master-death revive。
- stale generation watcher：不清理新 generation 的 workers。

### R1 RE-cutover

`tests/r1_master_exit_shutdown.rs` 当前断言的是旧 #52 “workers untouched” 契约，必须 cutover 到纠正语义：

- master 死亡后旧 worker 被真 Reap。
- 情况 A 拉起新 master；后续新 worker 是重新开启/恢复后的 worker，不是旧 worker 全程不动。
- 情况 B 不拉 master。
- 情况 B 清 worker 后 ahd 仍常驻，不因 idle master death 自杀。
- 测试命名必须反映 corrected semantics，避免继续出现 `shuts_down_daemon` 与 body 语义相反的问题。

## 11. 已知 Caveats

- direct-spawn fallback 无 `OOMScoreAdjust=-900` 保护；本设计只记录，不修。
- A/B 判定仍是从 worker/job 状态推断 master 是否“在跑任务”。这是当前 schema 下的 operationalized contract，不是单独 master busy 字段。
- 双重清理失败继续 revive 是 degraded mode，可能留下真实残留进程；startup reconcile/OS 是兜底，不是主路径。
- 自定义 master command 可能不含 `--continue`。默认 `src/cli/config.rs:177` 是 `claude --dangerously-skip-permissions --continue /remote-control`，但 custom command 的 resume 能力由用户配置承担。本设计不强制改写 custom master command。
