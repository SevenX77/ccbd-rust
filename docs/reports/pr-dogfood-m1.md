# PR Report: feat(ah-dogfood)!: M1 真 completion path + UDS streaming subscribe

## §1 PR 标题与摘要

PR 标题: `feat(ah-dogfood)!: M1 真 completion path + UDS streaming subscribe`

本 PR 落地 ah dogfooding closure 的 M1 子集: B2 真 completion path + B1 UDS streaming subscribe。目标是让 `ah ask --wait` 从“主控轮询等待”走向“daemon 推送终态 frame”, 并把 PR-1 旧 `dispatch_and_complete_job` test seam 切到 marker 驱动的状态机路径。Step 3 的 5 个红灯 dogfood tests 已全部变绿, M1 子集断言主控 `cancel + capture = 0`。

## §2 背景与 scope

立项背景来自 dogfooding closure spec: 用 ah 自驱跑 SOP-08/PR-6 体量任务, 替代 ccb ask 模式中人工 cancel、capture-pane verify、loop poll 等介入点。M1 scope:

- dogfood-1 / B2: 真 completion path, fake provider 输出 `<<ah-idle:job-id=X>>`, 状态机对账后完成 job。
- dogfood-2 / B1: 新增 `event.subscribe`, UDS streaming 终态 frame, `ah ask --wait` 改 subscribe。
- regression cutover: PR-1/drift/realign/mvp9 中受 completion wrapper 影响的测试同 PR 迁移。

不在 scope: B3 stuck multi-signal + push escalate, B4 slash command keystroke, B5 health check / multi-layer probe, B6 全量 dogfood e2e 主测。

## §3 变更摘要

Commit 范围:

- `2180133 test(ah-dogfood): step 3 tests-first 5 红灯 (T1+T2+T3)`
- `eb9147e feat(ah-dogfood)!: step 4 src impl T4.1-T4.8 — 5 红灯变绿`

合计改动:

- Step 3: 2 文件, +344 行。
- Step 4: 14 文件, +581 / -132。
- 合计: 16 文件 touched, 约 +925 / -132。

src 改动: RPC 4 文件, 状态机/marker 3 文件, pubsub 1 文件, CLI 1 文件。

tests/fixture 改动: 新建/扩展 `tests/ah_dogfooding.rs` 5 个 dogfood tests, 新建 `tests/fixtures/mock_dogfood_provider.sh`, 迁移 PR-1/drift/realign/mvp9 regression 的 completion wrapper 调用点。

BREAKING: 删除 `dispatch_and_complete_job` seam。旧测试不再直接 `mark_job_completed` + 改 agent `IDLE`, 改为 marker output -> state_machine。

## §4 测试与验证

```bash
cargo test --test ah_dogfooding -- --include-ignored --test-threads=1
```

结果: 5 passed。

覆盖:

- T3.1 `event.subscribe` 能返回 `job_state_change(COMPLETED)` frame。
- T3.2 marker completion 不依赖旧 seam。
- T3.3 `cancel_counter == 0 && capture_counter == 0`。
- T3.4 PR-1 文件无 `dispatch_and_complete_job`。
- T3.5 错 job-id marker 不误完成, 正 job-id marker 才完成。

```bash
cargo test --test ah_full_e2e_main -- --test-threads=1 --include-ignored
```

结果: passed。

```bash
CCB_TEST_SKIP_REAL_PROVIDER=1 cargo test -- --test-threads=1
```

结果: passed。

grep verify:

- `rg "dispatch_and_complete_job" tests/ah_full_e2e_main.rs`: 无输出。
- `rg "event\\.subscribe" src tests`: router/handler/server/client/test 均有落点。
- `rg "<<ah-idle:job-id|extract_ah_idle_marker_job_id" src tests`: marker parser、状态机对账、tests 均有落点。

## §5 audit 结果

a2 audit: 设计偏移对齐 M1 B1/B2 主线, file:line / RPC method / marker 对账已核, must-fix 0。

a3 audit: PASS, must-fix 0, nice-to-have 2。记录: M1 已用 in-process insert_event 锁状态机, dogfood-8 需补真 stdout reader 链路; dogfood-8 必须使用 `mock_dogfood_provider.sh` 真实 stdout marker, 不能停留在 test `insert_event` 层。

主控自审:

- `event.subscribe` 已注册。
- `ah ask --wait` 已改 `rpc_stream_first("event.subscribe")`。
- `dispatch_and_complete_job` 已从 PR-1 mainline 删除。
- marker job-id mismatch 会保持 BUSY。

## §6 已知限制与后续

M1 子集已完成: `cancel_counter == 0`, `capture_counter == 0`, 5 dogfood red tests -> green。

M2/M3 留项: push p95 <= 500ms, stuck escalate <= 310s, slash command 投递成功率 100%, 0 ScheduleWakeup poll 的更强端到端计数。

半 seam 声明: 当前 regression tests 通过 `events::insert_event` 注入 marker 文本来锁状态机与 event.subscribe 语义。dogfood-8 必须升级为真实 fake provider stdout -> agent_io reader -> events -> state_machine 的端到端链路。

后续 PR:

- dogfood-4: B3 stuck 多信号 + push escalate + C5 配置化。
- dogfood-5: B4 slash keystroke。
- dogfood-6: B5 health check / multi-layer probe。
- dogfood-7: tmux scope lifecycle + tmpdir lifecycle e2e。
- dogfood-8: B6 e2e dogfooding 主测, 锁 5 项完整指标。

## §7 风险 / breaking 影响

BREAKING cutover: `dispatch_and_complete_job` seam 删除影响 PR-1/drift/realign/mvp9 的测试 helper 思路, 本 PR 已同 PR 迁移到 marker output + state_machine pattern。

用户影响: 0。删除的是内部 test seam, 不暴露为 user API。

兼容性: `job.wait`, `agent.watch`, 旧 RPC method, CLI subcommand 形状都保留。

风险: M1 streaming path 只返回首个 terminal frame, 还不是完整多事件 replay/fanout; typed `EventFrame` bus 已落结构, completion 主路径仍桥接 `JOB_UPDATES`; 真 stdout reader 链路尚未被 dogfood-8 锁住, 后续必须补。
