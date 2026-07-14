# revive master readiness gate + zombie detection design

## Conclusion

Revived master recovery must not mark `master_recovery_windows.phase = COMPLETED` only because a tmux pane was spawned. The new success boundary should be:

`COMPLETED <=> revived master is still alive, its pane is observable, and either it acknowledged revive readiness or passed a degraded probe readiness gate before the recovery deadline.`

MVP recommendation:

1. Add a revive-specific readiness wait after `MASTER_RUNNING` and before worker reprovision/requeue completion.
2. Keep the recovery window non-terminal while readiness is pending (`MASTER_RUNNING` or a new non-terminal phase if PM approves a schema change).
3. On readiness success, run worker reprovision/requeue and then mark `COMPLETED`.
4. On readiness timeout/failure, mark the window `FAILED` and route back through the existing master revive retry/fuse machinery; if retry budget is exhausted or deadline is hit, cascade with `MASTER_REVIVE_WINDOW_EXPIRED`.
5. Add a secondary zombie detector for reprovisioned workers stuck in `SPAWNING` after master revive, but treat it as a follow-up guardrail. The readiness gate is the required fix for the permanent master-side zombie root cause.

This preserves cascade-coordination invariant A: every deferral has a deadline and eventually releases cascade. It also preserves invariant B: active-work revive still protects worker home only inside a bounded, non-terminal recovery window.

## Evidence From Current Code

The current revive path reaches a false success state before it has any readiness proof:

- `src/monitor/master_watch.rs:741`: revive obtains `new_pid` from `tmux_server.get_pane_pid(pane)`.
- `src/monitor/master_watch.rs:770`: window phase moves to `MASTER_RUNNING`.
- `src/monitor/master_watch.rs:815`: window phase moves to `WORKERS_REPROVISIONING`.
- `src/monitor/master_watch.rs:818`: workers are reprovisioned.
- `src/monitor/master_watch.rs:839`: `complete_master_recovery_window_for_master_watch(...)` marks the DB window `COMPLETED`.
- `src/monitor/master_watch.rs:845`: only after `COMPLETED`, `spawn_master_confirm_timer(...)` starts.

That confirm timer is not a readiness gate:

- `src/monitor/master_watch.rs:1066`: `spawn_master_confirm_timer` sleeps 60s and then calls `confirm_master_stable`.
- `src/master_revival.rs:296`: `confirm_master_stable` only updates `sessions.master_retry_count = 0` and `master_next_retry_at = 0` if `session_id/master_pid/master_generation` still match. It does not inspect pane output, ack, worker state, or master responsiveness.

Cutover already has the right shape of gate:

- `src/rpc/handlers/sessions.rs:592`: cutover selects `Ack` for Claude and `Probe` otherwise.
- `src/rpc/handlers/sessions.rs:614`: `wait_for_master_readiness` loops until ack/probe succeeds or timeout.
- `src/rpc/handlers/sessions.rs:632`: ack readiness is accepted via DB `ack_ready_at`.
- `src/rpc/handlers/sessions.rs:649`: probe mode captures the master pane.
- `src/rpc/handlers/sessions.rs:657`: probe requires three steady non-empty captures.
- `src/rpc/handlers/sessions.rs:887`: cutover waits for readiness before accepting the new master.
- `.kiro/specs/ah-product-delivery/master-readiness-gate-design.md:24`: ack must match ahd-injected `AH_CUTOVER_ID`.
- `.kiro/specs/ah-product-delivery/master-readiness-gate-design.md:30`: no `ACTIVE-but-unverified`; ack/probe success is required.
- `src/db/master_cutovers.rs:26`: cutover persists `ack_ready_at` and `readiness_mode`.
- `src/db/master_cutovers.rs:190`: `mark_master_cutover_ack_ready` only marks a `VERIFYING` cutover.

Worker readiness and stuck detection are already conceptually available, but they are worker-side:

- `.kiro/specs/ah-product-delivery/revive-resume-design.md:411`: dogfood already found recovery dispatch can race if worker readiness is bypassed.
- `.kiro/specs/ah-product-delivery/revive-resume-design.md:416`: recovery/realign spawn must stay `SPAWNING` until init-probe publishes `IDLE`.
- `src/provider/init_probe_task.rs:301`: provider init-probe uses steady-count readiness matching.
- `src/provider/health_check.rs:37`: health check observes active agents.
- `src/provider/health_check.rs:55`: `SPAWNING` is considered dead at the predicate layer if the provider init probe does not detect readiness.
- `src/provider/health_check.rs:160`: health watcher scans `SPAWNING`, `WAITING_FOR_ACK`, and `BUSY`.

Cascade coordination gives the deadline and terminal-state contract this design must preserve:

- `.kiro/specs/ah-product-delivery/master-oom-vs-cascade-coordination-design.md:174`: non-terminal active-work windows defer cascade only while unexpired.
- `.kiro/specs/ah-product-delivery/master-oom-vs-cascade-coordination-design.md:175`: expired windows cascade with `MASTER_REVIVE_WINDOW_EXPIRED`.
- `.kiro/specs/ah-product-delivery/master-oom-vs-cascade-coordination-design.md:199`: if a window expires before `COMPLETED`, mark `FAILED`.
- `.kiro/specs/ah-product-delivery/master-oom-vs-cascade-coordination-design.md:340`: `created_at + 300s` is the hard total cap.

## Readiness Signal

### Recommended Revive Readiness Modes

Revive needs its own readiness gate. It can reuse cutover concepts, but not cutover identity:

| Mode | When | Ready signal | Meaning |
|---|---|---|---|
| `ack` | Managed Claude revive, when ah can inject env/rules/prompt into the revived master | Revived master calls ahd through `CCB_SOCKET` with a revive token/generation | Strong: the master process reached the ah-controlled instruction path and can talk to ahd |
| `probe` | Non-Claude/custom, or MVP before ack plumbing lands | Process alive + pane pid still matches + capture succeeds + non-empty/steady pane output for N samples | Degraded: process is observable and not instantly dead; not proof that PM semantics are loaded |

The ack mode should not reuse `AH_CUTOVER_ID`. Use revive-specific identity:

- `AH_MASTER_RECOVERY_SESSION_ID`
- `AH_MASTER_RECOVERY_GENERATION`
- `AH_MASTER_RECOVERY_TOKEN` or row id
- optional `AH_MASTER_RECOVERY_MARKER` pointing at the existing redispatch marker

The handler should write a persistent DB field, not an in-memory channel, following the pinned cutover decision in `.kiro/specs/ah-product-delivery/master-readiness-gate-design.md:303`. Implementation choices:

- Add columns to `master_recovery_windows`: `readiness_mode`, `ready_at`, `ready_reason`, `readiness_token`.
- Or add a small child table `master_recovery_readiness(session_id, expected_generation, token, mode, ready_at, reason)`.

If schema churn must be minimized, MVP can ship probe-only for revive and reserve ack for the next slice. That is weaker for Claude semantics but still fixes the current false `COMPLETED` because hung panes will not pass stable capture/progress before timeout.

### Probe Definition

MVP probe should be explicit and conservative:

1. Verify `sessions.master_pid == expected_pid` and `sessions.master_generation == expected_generation`.
2. Verify `master_process_is_alive(expected_pid)`.
3. Verify stored `master_pane_id` parses and `tmux get-pane-pid` still equals `expected_pid`.
4. Capture the pane.
5. Require at least 3 consecutive successful captures that are non-empty and either:
   - stable enough to prove the pane is rendering, or
   - changed since spawn to prove progress.

Do not call this "ready" in user-facing logs without mode: log `readiness_mode=probe` and `readiness_strength=degraded`.

### Why `spawn_master_confirm_timer` Is Not Enough

The existing 60s confirm can remain as a retry/backoff cleanup after a successful revive, but it cannot gate `COMPLETED` because:

- it runs after `COMPLETED`;
- it only checks DB pid/generation;
- it resets retry counters but cannot detect first-run onboarding, stdin hangs, bad binary stalls, or a pane that renders no usable state.

The readiness gate must run before `complete_master_recovery_window_for_master_watch`.

## State Machine Change

Current as-built revive sequence:

```text
DETECTED
  -> WORKERS_REAPED
  -> MASTER_SPAWNING
  -> MASTER_RUNNING
  -> WORKERS_REPROVISIONING
  -> COMPLETED
  -> spawn_master_confirm_timer
```

Recommended sequence:

```text
DETECTED
  -> WORKERS_REAPED
  -> MASTER_SPAWNING
  -> MASTER_RUNNING
  -> readiness wait
       success:
         -> WORKERS_REPROVISIONING
         -> worker readiness/requeue gate
         -> COMPLETED
       timeout/failure:
         -> retry if record_master_revive_attempt permits and deadline budget remains
         -> FUSED if retry fuse trips
         -> FAILED + cascade if recovery deadline expires or no retry remains
```

If PM accepts a schema phase addition, add `MASTER_VERIFYING` between `MASTER_RUNNING` and `WORKERS_REPROVISIONING`. It makes logs and startup reconcile clearer:

```sql
phase IN (..., 'MASTER_RUNNING', 'MASTER_VERIFYING', 'WORKERS_REPROVISIONING', ...)
```

If PM wants minimal schema change, keep `MASTER_RUNNING` as the non-terminal readiness-wait phase. The key invariant is that `COMPLETED` is not written until readiness succeeds.

## Timeout Coordination

The readiness timeout must be derived from the recovery window, not independent from it.

Recommended formula at the point `MASTER_RUNNING` is entered:

```text
remaining = min(window.defer_until, window.created_at + 300) - now
readiness_timeout = clamp(configured_revive_readiness_timeout, 10s, 60s)
effective_timeout = min(readiness_timeout, remaining - cleanup_margin)
cleanup_margin = 5s
```

Defaults:

- `AH_MASTER_REVIVE_READINESS_TIMEOUT_SECS = 30`
- clamp `10..=60`
- never exceed `defer_until - 5s`

Rationale:

- successful dogfood revive paths complete in seconds;
- cutover can default to 120s because it is an explicit user-initiated handoff and not protecting in-flight worker homes;
- revive readiness is on the anti-orphan critical path, so it must fit inside the 90s default cascade window;
- the 300s hard cap remains the absolute safety rail.

If `remaining <= cleanup_margin`, skip readiness wait and mark `FAILED` or let deadline expire immediately; do not write `COMPLETED`.

## Timeout Outcome: Retry Then Cascade

Recommended failure behavior:

1. Readiness timeout/failure before retry fuse:
   - mark phase `FAILED` for the current attempt only if no immediate retry is possible, or introduce `MASTER_VERIFYING_FAILED` if PM accepts per-attempt detail;
   - call existing `record_master_revive_attempt` path for retry/backoff;
   - keep the recovery window non-terminal while retry is scheduled, but never beyond `created_at + 300s`.
2. Fuse reached:
   - mark window `FUSED`;
   - keep existing session `FAILED` / stop-anchor behavior.
3. Deadline reached:
   - mark window `FAILED`;
   - cascade with `MASTER_REVIVE_WINDOW_EXPIRED`.

This is preferable to immediate cascade on the first readiness timeout because it reuses existing retry/fuse semantics for transient bad starts, but still preserves invariant A through the window deadline.

## Active Zombie Detection

Readiness gate fixes the master-side false success. It does not fully cover workers that were reprovisioned but remain stuck.

MVP guardrail:

- After readiness success and worker reprovision starts, record `reprovision_started_at`.
- Before marking `COMPLETED`, check that reprovisioned workers that matter for active-work recovery have left `SPAWNING`/`SPAWNING_INTERVENTION` or have a bounded worker readiness timer still running.
- If any recovered worker is still `SPAWNING` after `worker_reprovision_timeout = min(30s, remaining - cleanup_margin)`, mark recovery `FAILED` or leave non-terminal until deadline; do not mark `COMPLETED`.

Follow-up active detector:

- A periodic watcher queries sessions with `master_recovery_windows.phase = WORKERS_REPROVISIONING`.
- For workers in `SPAWNING` / `WAITING_FOR_ACK` / `BUSY`, reuse `provider::health_check` observations.
- If all workers are healthy or have moved to dispatchable states, allow completion.
- If any worker has dead tmux/predicate/completion layers beyond threshold, mark the recovery failed and let retry/cascade policy run.

This should not replace the main readiness gate in MVP. It is a second line of defense for `project_ah_revival_produces_zombie_agents`.

## Integration Boundaries

Do not regress these existing mechanisms:

- Cascade coordination remains authoritative. `master_recovery_windows` still bounds cascade suppression; `COMPLETED` remains terminal only after real success.
- Cutover readiness remains independent. Cutover continues to use `master_cutovers`, `AH_CUTOVER_ID`, and `master.ack_ready`; revive should borrow the pattern but not share cutover rows.
- Active-work revive remains active-work only. Idle/no-work master death still reaps and must not be converted into revive.
- `REVIVE_IDLE` worker behavior is unchanged.
- Existing worker recovery/requeue remains after master readiness. The change is sequencing: do not reprovision/requeue workers behind a master that has not passed readiness.
- `master_spawn_lock` remains an intra-process spawn serialization optimization; it is not the readiness source of truth.

## Implementation Sketch

Phase R1: DB/readiness plumbing.

- Add readiness metadata to recovery windows or a child table.
- Add `begin_master_recovery_readiness_wait_sync`, `mark_master_recovery_ready_sync`, and `fail_master_recovery_readiness_sync`.
- Add a pure helper that computes `effective_timeout(now, defer_until, created_at, configured_timeout)`.

Phase R2: revive readiness wait.

- Extract cutover probe logic into a shared `master_readiness` helper that accepts `pane_id`, `expected_pid`, `mode`, timeout, and tmux capture callback.
- Add revive ack handler only if PM chooses ack MVP; otherwise use probe-only and log degraded readiness.
- Move `complete_master_recovery_window_for_master_watch` after readiness success and worker readiness/requeue success.
- On timeout, route through retry/fuse/deadline policy; never mark `COMPLETED`.

Phase R3: zombie detector.

- Add a bounded worker reprovision readiness gate before `COMPLETED`.
- Add periodic observation for `WORKERS_REPROVISIONING` windows if needed.
- Emit structured events for `MASTER_REVIVE_READINESS_TIMEOUT`, `MASTER_REVIVE_WORKER_ZOMBIE`, and `MASTER_REVIVE_PROBE_DEGRADED`.

## Tests

Unit tests:

1. `revive_readiness_probe_requires_alive_matching_pid_and_pane`: pid/generation/pane mismatch fails.
2. `revive_readiness_probe_accepts_three_steady_nonempty_captures`: degraded probe succeeds.
3. `revive_readiness_probe_times_out_before_defer_deadline`: timeout is clamped below `defer_until`.
4. `revive_completion_waits_for_readiness`: `MASTER_RUNNING` does not become `COMPLETED` until readiness succeeds.
5. `revive_readiness_timeout_does_not_complete_window`: timeout leaves window non-terminal or marks `FAILED`, never `COMPLETED`.
6. `revive_readiness_timeout_records_retry_or_fuse`: integrates with existing retry/fuse decisions.
7. `revive_worker_spawning_timeout_blocks_completed`: reprovisioned worker stuck in `SPAWNING` prevents `COMPLETED`.
8. `startup_reconcile_expired_master_running_window_cascades_when_master_not_ready`: verifies cascade-coord invariant A remains intact.

Integration/e2e tests for later dogfood:

1. Success: kill active-work master; revived master passes readiness; workers resume; window becomes `COMPLETED`.
2. Hung master: revived pane process stays alive but never produces ready signal; window does not become `COMPLETED`; retry/fuse or deadline cascade reaps workers.
3. Bad binary: spawned master exits before readiness; retry/fuse path triggers, no permanent home preservation.
4. Worker zombie: master ready but worker remains `SPAWNING`; completion is withheld and deadline eventually reaps.
5. Ahd restart during readiness wait: DB window remains non-terminal; startup reconcile either resumes readiness/retry or expires by deadline.
6. Provider matrix: Claude ack path, custom/bash probe path, and probe timeout.

## PM Decisions Needed

1. **Revive MVP readiness mode**: probe-only first, or add revive ack in the same implementation slice?
2. **Revive ack identity**: add readiness fields to `master_recovery_windows`, or create a child table?
3. **Schema phase**: add `MASTER_VERIFYING`, or reuse `MASTER_RUNNING` for the readiness-wait phase?
4. **Timeout default**: approve `AH_MASTER_REVIVE_READINESS_TIMEOUT_SECS=30`, clamp `10..=60`, effective timeout bounded by `defer_until - 5s`.
5. **Timeout outcome**: approve retry-then-cascade, rather than immediate cascade on first readiness timeout.
6. **Worker zombie detector scope**: include worker `SPAWNING` readiness gate in MVP, or ship master readiness first and do worker zombie detection as follow-up?
7. **Probe semantics wording**: approve `readiness_mode=probe` as degraded readiness in logs/UI.

## Recommended PM Defaults

- MVP readiness signal: probe-only if implementation size matters; ack-capable if this is the reliability milestone.
- Timeout: 30s default, hard bounded by the current recovery window deadline.
- Timeout outcome: retry while retry/fuse budget and window budget remain; otherwise `FAILED`/`FUSED` and cascade.
- Zombie detector: add the pre-`COMPLETED` worker readiness check if cheap; defer periodic detector to a follow-up.


## 已拍板 (PINNED 2026-06-29 — PM + 主控联合复核;实现按此为准,不再 re-ask)

核心 APPROVE:`COMPLETED` 必须前置于 revive master 就绪门(在 `complete_master_recovery_window_for_master_watch` 之前)。两不变量为硬约束:A(就绪期 window 非终态 + deadline 到期必 cascade 防僵尸);B(home 只在 bounded 非终态窗内护)。

7 决策点(主控独立票,override 了 PM 初版的 1/3/6):
1. **就绪信号 — claude 必须 ack-capable**。理由(主控复核实证):cutover probe(sessions.rs:649-657)= 连续 3 次"非空+与上次完全相同"capture,是**内容盲**的,分不开「ready master 停在 /remote-control」与「卡 first-run 主题向导」(皆稳定非空屏)。真僵尸(project_ah_revival_produces_zombie_agents)= revived claude 卡向导 = 稳定非空屏 → probe 会**误判 ready** → 永久僵尸。只有 ack(revived master 真跑到 /remote-control、用 revive-specific token 回呼 ahd)能证语义加载。**probe 仅作 non-claude / fallback;若因体量先发 probe-only,绝不可据此判 FINDING 关闭**(probe 只挡死/空 pane,不挡稳定卡住屏)。ack 身份用 revive-specific(`AH_MASTER_RECOVERY_*`),不复用 `AH_CUTOVER_ID`。
2. **就绪字段** — 加列到 `master_recovery_windows`(readiness_mode/ready_at/ready_reason/readiness_token),不另建子表(1:1 最简,跟"窗即恢复状态"现模式一致)。
3. **schema phase** — 加 `MASTER_VERIFYING`(MASTER_RUNNING 与 WORKERS_REPROVISIONING 之间),让 startup-reconcile/log 对"等 readiness"无歧义(见点2)。
4. **timeout** — 默认 `AH_MASTER_REVIVE_READINESS_TIMEOUT_SECS=30`,clamp 10..=60,硬限 `defer_until-5s`。master-readiness 段 + worker-readiness 段各自 bound 在"其时刻 remaining"内,总和须落进窗(设计已是)。
5. **超时动作** — retry-then-cascade(复用 record_master_revive_attempt / fuse);retry 全程 bound 在 `created_at+300s` 硬帽。绝不在就绪超时时写 COMPLETED。
6. **worker 僵尸门** — MVP **纳入** pre-`COMPLETED` 的 worker readiness 门(reprovision 后卡 SPAWNING → 不 COMPLETED,挡 dogfood 场景2 的 a1 卡 SPAWNING);周期 detector 留 follow-up。
7. **probe 语义** — 标 `readiness_mode=probe` / `readiness_strength=degraded`。

实现期必须钉死的 3 条(主控复核补充):
- **点2 / P4b 回归点(必随门改)**:加门后"master 活"≠"ready"。Phase-4b(ddbb8a9)startup reconcile 的「活→complete」分支**必须改**:活+窗 COMPLETED→ok;活+窗在 MASTER_VERIFYING(非终态)→**重启 readiness 等待(重 arm probe/ack),不 complete**;过期→cascade。否则 ahd 在就绪等待中重启会把 hung master 的窗跨重启误 COMPLETED。列为 required 回归点(对应 test#8 / e2e#5)。
- **点1 回归保护(必加 e2e)**:补 1 条 e2e —— revived claude master 停在"稳定渲染但语义未加载"屏(模拟向导)→ 验 **probe 会误过、ack 会挡**。不加这条,probe 的内容盲缺口就没有回归保护。
- **点3 / branch 收敛(release 协调,merge owner 定)**:预防修复 c7f14a4(re-materialize 沙箱 HOME / theme-wizard zombie)+ c9706a0(codex version.json 跳 update)在 `feat/revive-skip-interactive-gates`,**不在** `feat/ahd-persistent-service` HEAD。本门是"检测"(漏网者 deadline+cascade 兜),那两个是"预防"(revived master 压根不撞向导)。真闭合 revival 可靠性需两者收敛进同一发布线——cherry-pick / 合并时机由 merge owner(用户)在 merge 时定;不阻塞本门实现。

实现节奏:a1 按 phase(R1 DB plumbing → R2 readiness wait+ack/probe+gate COMPLETED+P4b 改+worker 门 → R3 周期 detector follow-up)严格 TDD;a1 作者,审计 PM+主控(a4 登出);每 phase commit 主控独立验;dogfood e2e 主控隔离跑。
