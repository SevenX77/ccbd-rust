# Tasks M2: ah dogfooding closure stuck push

## §1 PR scope 与度量目标

M2 scope: design §4 的 `dogfood-4`。只做 B3 stuck 多信号 + STUCK push event + C5 stuck 阈值配置化; 不碰 M3 的 B4 slash、B5 health check、B6 全量 dogfood 主测。

M2 继承 M1:

- M1 已有 `event.subscribe` RPC、UDS streaming writer、`EventFrame` typed bus, 但当前 `event.subscribe` 主路径只等 `job_state_change` terminal frame。
- M1 已有 `tests/ah_dogfooding.rs` 5 个 ignored tests 与 `InterventionCounters { cancel, capture }`。
- M1 已把 `ah ask --wait` 改成 subscribe, M2 增补 ScheduleWakeup poll counter 断言。

M2 目标:

- B3 三信号 stuck: 现有 pane content diff/hash 继续保留; 新增 log mtime 与 provider-aware Thinking/spinner 信号。
- STUCK push: stuck 成功落库后通过 M1 B1 streaming 推 `EventFrame { kind: "stuck", agent_id, job_id, signal_kinds, elapsed_secs, ts_unix_micro, payload }`。
- C5 配置化: `src/pane_diff/mod.rs:9-10` 的 30s/300s hardcode 改为 config/env 解析; env 为 `AH_STUCK_TICK_SECS` / `AH_STUCK_THRESHOLD_SECS`。

M2 度量:

- `poll_counter == 0`。M1 已锁 `cancel_counter == 0 && capture_counter == 0`。
- stuck event push p95 <= 500ms, 统计 daemon emit stuck frame 到 test master 收到 frame。
- stuck escalate <= 310s; e2e 用 env override 加速到 5-30s, 但逻辑仍覆盖默认 300s + 10s 余量。

## §2 TDD 任务列表

### T1 pane_diff_watcher 扩三信号

文件:
- `src/pane_diff/mod.rs`

依赖:
- 现 hardcode: `src/pane_diff/mod.rs:9-10`。
- 现 tick: `src/pane_diff/mod.rs:73-123`。

内容:
- 新增 `compute_content_hash(pane_content: &str) -> u64`。可用 `std::collections::hash_map::DefaultHasher`; 不引新 crate。
- 新增 `query_log_mtime(agent_log_path: &Path) -> Option<SystemTime>`。路径不存在返回 `None`; metadata error 记录 trace/warn 但不 panic。
- 新增 `detect_thinking_spinner(content: &str) -> bool`。M2 用简单 regex/substring 覆盖 `Thinking`, `Spinner`, `Working`, spinner-only 状态行; provider-specific 精细化留 B5。
- 扩 `AgentDiffState`: `last_content_hash: Option<u64>`, `last_log_mtime: Option<SystemTime>`, `thinking_start: Option<Instant>`, `last_signal_kinds: Vec<String>`。
- 扩 `PaneDiffObservation`: 增加 `agent_id`, `text`, `log_mtime: Option<SystemTime>`, `provider: Option<String>` 或等价字段。
- watcher 规则: 只有三信号都静止才 stuck: content hash 不变, log mtime 不变或不可用, thinking/spinner 持续超过 threshold。
- 输出不再只是 `stuck_agent_ids`; 需要携带 `StuckSignal { agent_id, signal_kinds, elapsed_secs }` 供 T3 push payload 使用。

验收:
- 新增 unit tests: hash 变化重置 timer, mtime 变化重置 timer, thinking 持续超过阈值才 stuck, 三信号静止才 stuck。
- 现有 pane_diff tests 继续绿。

### T2 C5 stuck 阈值配置化

文件:
- `src/pane_diff/mod.rs`
- `src/orchestrator/mod.rs`
- 若现 config 模块适合承载, 可最小新增 `StuckConfig` helper; 不改 ah.toml public schema 也可先用 env-only MVP, 但 tasks/audit 要明确。

内容:
- 新增 `stuck_tick_secs` 默认 30, env `AH_STUCK_TICK_SECS`。
- 新增 `stuck_threshold_secs` 默认 300, env `AH_STUCK_THRESHOLD_SECS`。
- 将 `DEFAULT_WATCH_INTERVAL` / `DEFAULT_STUCK_THRESHOLD` 从直接传入 watcher 改为 `resolve_stuck_watch_config()` 或等价函数。
- `src/orchestrator/mod.rs:22-26` watcher 启动时使用 resolved interval/threshold。
- env parse 非正整数时 fail open 到默认值, 并写 warn。

验收:
- unit test 覆盖默认值、env override、非法 env 回退。
- e2e T4.2 能把 threshold 降到 5s 内验证。

### T3 STUCK push event 集成 M1 B1 streaming

文件:
- `src/db/state_machine.rs`
- `src/pane_diff/mod.rs`
- `src/orchestrator/pubsub.rs`
- `src/rpc/handlers.rs`

内容:
- `mark_agent_stuck` 成功后需要返回 affected job 或由 watcher 查询当前 dispatched job。推荐返回 `StuckOutcome { changes, agent_id, job_id, from_state }`, 保持旧 wrapper 兼容。
- pane_diff watcher 拿到 `signal_kinds` 和 `elapsed_secs`, 构造 `{"kind":"stuck","agent_id":"a1","job_id":"job_x","state":"STUCK","signal_kinds":["hash","mtime","thinking"],"elapsed_secs":31,"ts_unix_micro":1770000000000000}`。

- 使用 M1 `pubsub::notify_event(EventFrame)` 推 typed frame; 同时保留 `notify_job_update` 兼容旧 waiters。
- `handle_event_subscribe` / `stream_event_subscribe` 支持 filter `{agent_id, job_id, event_kind}`。M2 必须支持 `event_kind=["stuck"]`; 如果 `job_id` 为空, 能按 `agent_id` 等 stuck frame。
- event frame 持久化: 不新增表。继续使用 `events(event_type='state_change', payload.to='STUCK')`; streaming 可先发内存 frame, fast path 可从 events 表补发 stuck frame。
- `ah ask --wait` 收到 stuck frame 不直接返回成功。M2 CLI 行为建议: `wait_for_job` 对 `kind=="stuck"` 返回 typed error 或继续等待由 master 决策; tasks/audit 必须核不把 stuck 当 completed。

验收:
- STUCK event 能被 subscribe 收到。
- `payload.signal_kinds` 与 `elapsed_secs` 非空。
- `job.wait` / `agent.watch` 旧行为不回归。

### T4 红灯 tests

文件:
- 扩 `tests/ah_dogfooding.rs`。

T4.1 `test_stuck_push_event_via_subscribe`

- setup: fake BUSY job + pane/log 静止; env override threshold=10s 或测试内直接调用 tick helper。
- subscribe: `event.subscribe({ "agent_id": AGENT_ID, "job_id": JOB_ID, "event_kind": ["stuck"] })`。
- assert: 收到 frame `kind=="stuck"`, `state=="STUCK"`, `job_id==JOB_ID`, `signal_kinds` 含至少一个信号。
- 红灯原因: 当前 `event.subscribe` 不支持 stuck kind, `mark_agent_stuck` 不 push EventFrame。

T4.2 `test_stuck_threshold_env_override`

- setup: `AH_STUCK_THRESHOLD_SECS=5`, `AH_STUCK_TICK_SECS=1`。
- run: fake provider 静默 6s+ 或直接驱动 watcher ticks。
- assert: agent state 变 `STUCK`, elapsed <= 7s。
- 红灯原因: 当前阈值固定 300s。

T4.3 `test_push_latency_p95_under_500ms`

- setup: 5 个 stuck/completion event samples, 用 `Instant` 记录 emit 前后。
- run: 订阅 frame, 收集 5 个 latency。
- assert: sort 后 p95 <= 500ms。
- 红灯原因: 当前 stuck event 不走 typed streaming; completion path 只单 job terminal, 不覆盖 stuck。

T4.4 `test_zero_schedule_wakeup_poll`

- setup: 扩 `InterventionCounters` 为 `{ cancel, capture, poll }`。
- run: 5 RPC 模拟加入 stuck subscribe: `session.create`, `agent.spawn`, `job.submit`, `event.subscribe(stuck)`, `session.kill`。
- assert: `poll_counter == 0`, 且 M1 的 cancel/capture 仍为 0。
- 红灯原因: 当前 tests 没有 poll counter; stuck path 未验证。

### T5 src 实施

按 T1-T3 最小落地, 让 T4 红灯变绿。

顺序:
1. 先实现 pane_diff 纯函数和 unit tests。
2. 再做 env/config 解析和 orchestrator wiring。
3. 再扩 state_machine stuck outcome 与 EventFrame push。
4. 最后扩 `event.subscribe` filter 和 dogfood e2e。

严禁:
- 不改 M1 已稳定的 `src/agent_io/reader.rs` / `src/marker/matcher.rs`。
- 不把 B4 slash、B5 health、B6 真 stdout dogfood 主测塞进 M2。

### T6 a2 audit + a3 audit

- a2 audit: file:line、frame schema、env config、state_machine affected job、handler filter 是否和 design/tasks 对齐。
- a3 audit: PM 替身 + scope drift, 核 M2 只做 B3/C5, 不扩到 B4/B5/B6。
- round-loop 到 0 must-fix; nice-to-have 明确留 M3 或 dogfood-8。

### T7 docs 同步

文件:
- `docs/engine/dogfood-m2/logic-explained.md`

内容:
- 字段级翻译 `StuckSignal`, `signal_kinds`, `elapsed_secs`, `stuck_tick_secs`, `stuck_threshold_secs`, `AH_STUCK_*`, `event.subscribe(stuck)`。
- 明确 M2 与 M1 差异: M1 terminal job frame, M2 stuck frame。

### T8 PR report

文件:
- `docs/reports/pr-dogfood-m2.md`

内容:
- 背景、scope、变更、测试、audit、风险、后续。
- 明确 M2 指标: 0 poll, stuck push p95 <= 500ms, stuck escalate <= 310s。

## §3 验收门槛

- `cargo test --test ah_dogfooding -- --include-ignored --test-threads=1`: M1 5 + M2 4 = 9 PASS。
- `cargo test --test ah_full_e2e_main -- --include-ignored --test-threads=1`: PASS。
- `CCB_TEST_SKIP_REAL_PROVIDER=1 cargo test -- --test-threads=1`: PASS。
- M2 e2e assert: 收到 stuck push frame, `poll_counter == 0`, push p95 <= 500ms, stuck escalate <= 310s。
- grep verify: `AH_STUCK_TICK_SECS`, `AH_STUCK_THRESHOLD_SECS`, `kind == "stuck"`, `signal_kinds`, `elapsed_secs`, `event.subscribe` stuck filter。

## §4 scope guard

- M2 只改 B3 + C5: pane_diff 多信号、STUCK push、阈值配置化、dogfood stuck tests。
- 不做 B4 slash command keystroke。
- 不做 B5 health check / InitProbe 合并。
- 不做 B6 真 stdout dogfood 全套; 真 fake provider stdout marker 链路留 dogfood-8。
- 不改 M1 reader/marker completion path, 除非 audit 证明 B3 必须读取只读 helper。
