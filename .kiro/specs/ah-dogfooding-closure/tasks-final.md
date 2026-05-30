# Tasks Final: ah dogfooding closure health + full dogfood e2e

## §1 PR scope 与度量目标

M-final scope: design §4 的 `dogfood-6` + `dogfood-8` 合并收尾。只做 B5 health check 与 B6 真 stdout dogfood 主测, 用 dogfood-8 锁最终闭环。

M-final 继承 M1/M2/M3a:

- M1: `event.subscribe`, marker completion, `<<ah-idle:job-id=X>>` job-id 对账, 0 cancel + 0 capture。
- M2: B3 stuck push, `AH_STUCK_TICK_SECS` / `AH_STUCK_THRESHOLD_SECS`, 0 poll, push p95, stuck frame。
- M3a: single-line slash command direct keystroke, fake provider slash ack。

M-final 目标:

- B5: 新建 `src/provider/health_check.rs`, 聚合 tmux pane alive、provider predicate、completion detector 最近进展三层健康度。
- B5: health check 与 M2 pane_diff watcher 共用 tick; 任一层 dead 且 agent 处于 active state, 触发 B3 STUCK escalate。
- B6: `tests/ah_dogfooding.rs` 增 dogfood-8 主测, 使用 `tests/fixtures/mock_dogfood_provider.sh` 真 stdout marker, 不再用 test `insert_event` 注入 marker。
- B6: 验证 stdout -> `agent_io::reader` -> events -> state_machine -> pubsub -> `event.subscribe` 的完整链路。

完整度量:

- `cancel_counter == 0`
- `capture_counter == 0`
- `poll_counter == 0`
- push p95 <= 500ms, M-final 用真 stdout marker path 再验
- stuck escalate <= 310s, 继承 M2 并用 health dead layer 再验
- slash command 投递成功率 100%, 继承 M3a 并在 full e2e 中复验
- 5 个典型 RPC + SOP-08 互动模拟全跑通, 无 timeout/error

## §2 TDD 任务列表

### T1 B5 新建 provider health_check.rs

文件:
- 新建 `src/provider/health_check.rs`
- 更新 `src/provider/mod.rs` 导出模块

依赖:
- `src/provider/init_probe.rs:8-160`: provider-specific readiness predicate。
- `src/provider/init_probe_task.rs:68-110,213-244`: startup probe loop 与 prompt scan 语义。
- `src/agent_io/reader.rs:136-193`: output_chunk insert + marker scan + job completion path。
- `src/db/state_machine.rs:13-26`: active/terminal state 常量。

内容:
- 新增 `HealthCheckResult { alive: bool, dead_layers: Vec<String>, last_progress_ts: i64 }`。
- 新增 `HealthCheckObservation { agent_id, provider, state, pane_capture, pane_capture_ok, last_output_ts, last_marker_ts, now_ts }`。
- layer 1 `tmux_pane_alive`: capture-pane 成功且 pane_capture 可读取。
- layer 2 `provider_predicate`: 对 provider 调 InitProbe predicate; startup state 走严格 readiness, BUSY/WAITING_FOR_ACK 允许 provider 正在工作但不能是 known dead screen。
- layer 3 `completion_progress`: 最近 output_chunk 或 idle marker 时间没有超过 stuck threshold。
- 任一 layer dead 且 agent state active -> result `alive=false`, `dead_layers` 带 `"tmux"|"provider"|"completion"`.
- 启动期用 InitProbe deadline; 工作期用 M2 `stuck_threshold_secs`。

红灯 unit tests:
- `test_health_check_alive_when_all_layers_ok`
- `test_health_check_dead_when_capture_fails`
- `test_health_check_dead_when_provider_predicate_fails_startup`
- `test_health_check_dead_when_completion_progress_stale`

验收:
- `cargo test --lib health_check` 编译红灯后, T5 src 实施转绿。

### T2 B5 orchestrator wire

文件:
- `src/orchestrator/mod.rs`
- `src/pane_diff/mod.rs` 如需接 health observation, 只做最小字段透传

依赖:
- `src/orchestrator/mod.rs:22-26` 现只启动 `pane_diff_watcher_loop`。
- `src/pane_diff/mod.rs` M2 已输出 `StuckSignal { agent_id, signal_kinds, elapsed_secs }`。

内容:
- 新增统一 watcher loop 或在现 pane_diff watcher tick 中串 health check。
- 共用 M2 `resolve_stuck_watch_config()` 的 tick/threshold。
- 每 tick 对 BUSY / WAITING_FOR_ACK / SPAWNING agents 做 health check。
- health dead -> 调 `mark_agent_stuck` 或等价 B3 escalate, 并通过 `pubsub::notify_event(EventFrame { kind:"stuck", signal_kinds:["health:<layer>"], ... })` 推送。
- health signal 与 pane_diff signal 都进入 `signal_kinds`, 不能覆盖 M2 hash/mtime/thinking 信号。

红灯 e2e:
- T4.3 `test_health_check_dead_layer_escalates_stuck` 先 fail。

验收:
- dead layer 能收到 `stuck` frame。
- M2 `pane_diff` tests 仍 16 PASS。

### T3 B6 mock_dogfood_provider.sh 真 stdout 升级

文件:
- `tests/fixtures/mock_dogfood_provider.sh`

依赖:
- 现 fixture 已支持 stdin message -> stdout `<<ah-idle:job-id=X>>`。
- M3a 已支持 `/clear` -> `<<ah-slash-ack:cmd=/clear>>`。

内容:
- 保持普通 message 真 stdout 输出 `<<ah-idle:job-id=X>>`。
- 明确输出 flush 行, 方便 reader 及时拿到 marker。
- 新增 `FAKE_PROVIDER_STUCK_MS`: 接到 message 后输出 `Thinking...`, sleep 指定毫秒, 可超过 threshold 触发 B3/B5 stuck。
- 新增 `FAKE_PROVIDER_SLASH_ACK`: 默认为 1; 设 0 时 slash ack 关闭, 用于负向测试。
- 支持 `FAKE_PROVIDER_DELAY_MS` 保持原有延迟注入。

验收:
- shell 直接运行: 普通 job 输出 idle marker。
- shell 直接运行: `/clear` 输出 slash ack。
- shell 直接运行: `FAKE_PROVIDER_STUCK_MS=...` 输出 Thinking 并延迟。

### T4 B6 dogfood-8 主测红灯

文件:
- 扩 `tests/ah_dogfooding.rs`

T4.1 `test_dogfood_e2e_full_sop08_simulation`

- setup: 启动 ah daemon/harness, 启动 fake provider 作为真实 agent pane, reader 连接其 stdout/fifo。
- 主控用 dogfood client 跑 5 个典型 RPC:
  1. `session.create`
  2. `agent.spawn`
  3. `job.submit` / `ah ask --wait` 模拟 dispatch a2 design
  4. `event.subscribe` 等 IDLE marker frame
  5. `session.kill`
- 再模拟 SOP-08 互动:
  - dispatch a2 research/design
  - dispatch a1 audit
  - dispatch a3 PM audit
  - inject stuck job -> 收 `stuck` frame
  - slash `/clear` -> 收 slash ack
- assert: 5 个 `<<ah-idle:job-id=X>>` 真 stdout marker 全部经 reader 完成 job。
- assert: `cancel_counter == 0 && capture_counter == 0 && poll_counter == 0`。
- assert: 无 test `insert_event` 注入 completion marker。

T4.2 `test_push_latency_p95_real_stdout`

- setup: fake provider 真实 stdout 输出 5 个 idle marker。
- measure: marker 输出时间到 master client 收到 `job_state_change(COMPLETED)` frame。
- assert: p95 <= 500ms。
- 红灯原因: 现 M1/M2 dogfood tests 多数仍用 `events::insert_event` 半 seam, 未锁真 stdout latency。

T4.3 `test_health_check_dead_layer_escalates_stuck`

- setup: 构造 active agent + dead health layer, 或 fake provider pane/capture fail。
- run: health watcher tick。
- assert: agent -> `STUCK`, `event.subscribe(event_kind:["stuck"])` 收 frame, payload `signal_kinds` 含 `health:<layer>`。
- 红灯原因: `src/provider/health_check.rs` 不存在。

验收:
- Step 3 期望 M1+M2+M3a 10 PASS + M-final 3 FAIL。

### T5 src 实施

顺序:
1. 实现 `src/provider/health_check.rs` 纯函数和 unit tests。
2. 接 `src/provider/mod.rs`。
3. 接 orchestrator watcher, 共用 M2 tick/threshold。
4. 扩 fixture stuck/slash/env 模式。
5. 升级 dogfood-8 harness 到真 stdout -> reader -> DB -> state_machine path。
6. 确认 T4.1-T4.3 全绿。

严禁:
- 不改 M1 marker 对账语义, 只读/复用。
- 不改 M2 event.subscribe schema, 只复用 stuck frame。
- 不改 M3a writer slash transport, 只复验。
- 不引新 crate。

### T6 a3 audit

audit 焦点:
- B5 三层是否真实接入: tmux pane alive / provider predicate / completion progress。
- B5 是否任一 dead active agent 都触发 B3 STUCK escalate。
- B6 是否真 stdout marker 进入 `agent_io::reader`, 没用 test `insert_event` 偷懒。
- dogfood-8 是否覆盖 5 RPC + stuck + slash + 0 cancel/capture/poll。
- push p95 是否在真 stdout path 上统计。

验收:
- 0 must-fix 后进入 docs/report。

### T7 docs 同步

文件:
- `docs/engine/dogfood-final/logic-explained.md`

内容:
- 字段级翻译 `HealthCheckResult`, `HealthCheckObservation`, health layers, orchestrator wire, fake provider stdout mode, dogfood-8 tests。
- 明确 M-final 如何闭合 design §1 五项指标。

### T8 PR report

文件:
- `docs/reports/pr-dogfood-final.md`

内容:
- 背景、scope、变更、测试、audit、风险、后续。
- 明确这是 dogfooding closure 最终 PR, M1/M2/M3a/M-final 指标全部汇总。

## §3 验收门槛

- `cargo test --test ah_dogfooding -- --include-ignored --test-threads=1`: M1+M2+M3a 10 + M-final 3 = 13 PASS。
- `cargo test --lib pane_diff`: M2 16 PASS。
- `cargo test --lib health_check` 或等价 health_check unit tests: PASS。
- `cargo test --test ah_full_e2e_main -- --include-ignored --test-threads=1`: 4 PASS。
- `CCB_TEST_SKIP_REAL_PROVIDER=1 cargo test -- --test-threads=1`: full suite PASS。
- dogfood-8 assert: 5 RPC 全跑通, 真 stdout marker 全部完成, stuck frame 收到, slash ack 收到。
- instrumentation assert: `cancel_counter == 0`, `capture_counter == 0`, `poll_counter == 0`。
- latency assert: 真 stdout path push p95 <= 500ms。
- stuck assert: health dead layer -> STUCK frame <= 310s, e2e 可用 env override 加速。

## §4 scope guard

- M-final 只做 B5 + B6 dogfood-8。
- 不重构 RPC/CLI/schema。
- 不改 M1 reader/marker completion 语义, 除非只加 last-progress timestamp 字段且 audit 证明 B5 必需。
- 不改 M2 pane_diff/state_machine/pubsub schema, 只补 health signal source。
- 不改 M3a slash writer 行为。
- 不引新 crate。
- 不做 Web UI/stdout 增强; 本 PR 专注 headless dogfood closure。
