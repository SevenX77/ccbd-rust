# PR Report: feat(ah-dogfood): M2 dogfood-4 — B3 stuck multi-signal + push + C5 配置化

## §1 PR 标题与摘要

PR 标题: `feat(ah-dogfood): M2 dogfood-4 — B3 stuck multi-signal + push + C5 配置化`

本 PR 落地 ah dogfooding closure 的 M2 子集: B3 stuck 多信号、STUCK push event、C5 stuck 阈值配置化。M1 已把 wait path 切到 `event.subscribe`; M2 在同一条 streaming/event 机制上补 `kind:"stuck"` frame, 并把 stuck watcher 从硬编码 30s/300s 改成可测试的 env override。

## §2 背景与 scope

M1 PR #28 已合 main, 提供:

- `event.subscribe` UDS streaming。
- typed `EventFrame` / pubsub 基础。
- marker 驱动的 completion path。
- 主控 `cancel + capture = 0` 子集指标。

M2 scope:

- B3 三信号: content hash、log mtime、Thinking/spinner。
- B3 STUCK push: `event.subscribe({event_kind:["stuck"]})` 能收到 stuck frame。
- C5 配置化: `AH_STUCK_TICK_SECS` / `AH_STUCK_THRESHOLD_SECS`。
- M2 指标: 0 poll、push p95 < 500ms、stuck escalate < 310s。

不在 scope: B4 slash command keystroke、B5 health check / multi-layer probe、B6 真 stdout dogfood 全套。

## §3 变更摘要

Commit 范围:
- `19f2d56 test(ah-dogfood): M2 step 3 tests-first — T1 pane_diff unit 4 + T4 ah_dogfooding 红灯 4`
- `e302805 feat(ah-dogfood): M2 step 4 src impl — 8 红灯变绿 (T1+T2+T3)`

改动统计:
- Step 3 tests-first: 2 文件, +210 / -3。
- Step 4 src impl: 7 文件, +420 / -45。
- 合计: 约 +630 / -48。

src 变更:
- `src/pane_diff/mod.rs`: 三信号 helper、`AgentDiffState` / `PaneDiffObservation` 扩展、`StuckSignal` 输出、env 配置解析。
- `src/db/state_machine.rs`: `StuckOutcome` 内部结果、stuck state_change payload、typed stuck event push。
- `src/orchestrator/pubsub.rs`: `EventFrame` 支持 serialize。
- `src/rpc/handlers.rs`: `event.subscribe` 支持 `event_kind=["stuck"]` fast path 与 streaming path。
- `src/orchestrator/mod.rs` / `src/state_layout.rs`: watcher 配置接入与小范围兼容调整。
- tests: `pane_diff` 新增 4 个 unit, `ah_dogfooding` 新增 M2 4 个 ignored tests 并扩 `poll` counter。

## §4 测试与验证

```bash
cargo test --test ah_dogfooding -- --include-ignored --test-threads=1
```

结果: 9 passed。覆盖 M1 5 个 + M2 4 个:
- `test_stuck_push_event_via_subscribe`
- `test_stuck_threshold_env_override`
- `test_push_latency_p95_under_500ms`
- `test_zero_schedule_wakeup_poll`

```bash
cargo test --lib pane_diff
```

结果: 16 passed, 含 M2 新增 4 个三信号 unit tests。

```bash
cargo test --test ah_full_e2e_main -- --include-ignored --test-threads=1
```

结果: 4 passed。

```bash
CCB_TEST_SKIP_REAL_PROVIDER=1 cargo test -- --test-threads=1
```

结果: passed。

grep verify:
`rg "AH_STUCK|compute_content_hash|query_log_mtime|detect_thinking_spinner|kind: \"stuck\"|signal_kinds|elapsed_secs" src tests` 覆盖 env、三信号 helper、stuck frame/filter/payload。

## §5 audit 结果

a3 audit: PASS, must-fix 0, nice-to-have 2。

记录的 nice-to-have:
- 流程瑕疵: step 3/4 已跑通, 文档阶段补清楚实际边界。
- `elapsed_secs` / `signal_kinds` 在 DB fast path 中仍是占位字段: `elapsed_secs=0`, `signal_kinds=["state_machine"]`; dogfood-4 收尾或 dogfood-8 需要补 watcher 真值持久化。

主控自审:
M2 未改 M1 reader / marker path; `event.subscribe` job terminal 旧路径保留; SQLite schema 未变; `mark_agent_stuck` 外部签名仍兼容旧 caller。

## §6 已知限制与后续

M2 度量兑现:
- 0 poll: `test_zero_schedule_wakeup_poll` PASS。
- push p95 < 500ms: `test_push_latency_p95_under_500ms` PASS。
- stuck < 310s: env override 5s 路径 PASS, 默认 300s + 10s 预算逻辑保留。

已知限制:
- `elapsed_secs=0` + `signal_kinds=["state_machine"]` 是 DB fast path 占位, M2 度量不依赖它的真值。
- `PaneDiffObservation.provider` 已承载, provider-specific 细化留 B5。
- watcher 主路径当前未把 log file path 接入真实 mtime, unit/e2e 已锁 helper 行为。

后续 PR:

- dogfood-5: B4 slash keystroke。
- dogfood-6: B5 health check / multi-layer probe。
- dogfood-7: tmux scope lifecycle。
- dogfood-8: B6 真 stdout fake provider dogfood 全套, 锁 5 项完整指标。

## §7 风险 / breaking

无 BREAKING。

- RPC / CLI method 形状不变: 继续使用 M1 `event.subscribe`。
- SQLite schema 不变: stuck replay 复用 `events` 表。
- `mark_agent_stuck` 对外返回仍是 `usize`, `StuckOutcome` 是内部 wrapper。
- 旧 `job.wait` / `agent.watch` / job terminal subscribe 路径未移除。

主要风险是 stuck payload 真值尚未完全贯通到 DB replay; 当前已在 report 中作为后续收尾项锁住。
