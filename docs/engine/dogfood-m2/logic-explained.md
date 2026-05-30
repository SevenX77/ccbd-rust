# Dogfood M2 Logic Explained

## §1 PR 范围

M2 对应 dogfood-4, 只落 B3 与 C5:

- B3 stuck multi-signal: `pane_diff` 不再只看文本 diff, 增加 content hash、log mtime、Thinking/Spinner/Working 占位信号。
- B3 STUCK push: agent 被标为 `STUCK` 后, 复用 M1 `event.subscribe` / typed `EventFrame` 路径推 `kind:"stuck"` frame。
- C5 配置化: stuck watcher 的 tick 与 threshold 从硬编码 30s/300s 变成 env override。

M2 不改 M1 completion reader / marker path, 不做 B4 slash keystroke, 不做 B5 health check, 不做 B6 真 stdout dogfood 全套。

## §2 新增字段与函数

| 字段 / 函数 | 文件:line | 中文 logic 解释 | 类型 / 签名 |
|---|---:|---|---|
| `compute_content_hash` | `src/pane_diff/mod.rs:237-241` | 把清洗后的 pane 文本算成 `u64` hash。hash 变表示 provider 有真实输出变化, stuck timer 要重置。 | `pub fn(&str) -> u64` |
| `query_log_mtime` | `src/pane_diff/mod.rs:243-247` | 读取 agent log 文件最后修改时间。文件不存在或 metadata 失败返回 `None`, 不 panic。 | `pub fn(&Path) -> Option<SystemTime>` |
| `detect_thinking_spinner` | `src/pane_diff/mod.rs:249-251` | 检测 `Thinking` / `Spinner` / `Working` 等占位文本, 用于 provider-aware stuck 信号。 | `pub fn(&str) -> bool` |
| `AgentDiffState.last_content_hash` | `src/pane_diff/mod.rs:18` | 保存上一次 pane content hash, 当前 hash 变化时认为 agent 仍有进展。 | `Option<u64>` |
| `AgentDiffState.last_log_mtime` | `src/pane_diff/mod.rs:19` | 保存上一次 log mtime, 当前 mtime 变化时认为 agent 仍有进展。 | `Option<SystemTime>` |
| `AgentDiffState.thinking_start` | `src/pane_diff/mod.rs:20` | 记录 Thinking/spinner 第一次出现的时间。只有持续超过 threshold 才参与 stuck。 | `Option<Instant>` |
| `AgentDiffState.last_signal_kinds` | `src/pane_diff/mod.rs:21` | 保存最近一次 stuck 判定使用的信号名, 供测试和 payload 解释。 | `Vec<String>` |
| `PaneDiffObservation.agent_id` | `src/pane_diff/mod.rs:41` | 一次 watcher tick 中被观察的 agent id。 | `String` |
| `PaneDiffObservation.text` | `src/pane_diff/mod.rs:42` | 这次 capture 到的 pane 文本。 | `String` |
| `PaneDiffObservation.log_mtime` | `src/pane_diff/mod.rs:43` | 这次观察到的 log mtime; M2 watcher 主路径暂传 `None`, unit/e2e 可注入。 | `Option<SystemTime>` |
| `PaneDiffObservation.provider` | `src/pane_diff/mod.rs:44` | provider 名称预留字段, M2 先用于结构承载, provider-specific 细化留 B5。 | `Option<String>` |
| `StuckSignal` | `src/pane_diff/mod.rs:47-52` | pane_diff 输出的 stuck 业务结果, 比旧 `stuck_agent_ids` 多带 `signal_kinds` 和 `elapsed_secs`。 | `pub struct` |
| `PaneDiffTickResult.stuck_signals` | `src/pane_diff/mod.rs:57` | watcher tick 的完整 stuck 信号列表, 供 push frame payload 使用。 | `Vec<StuckSignal>` |
| `process_pane_diff_observations` | `src/pane_diff/mod.rs:60-141` | 核心判定函数: 文本有 meaningful diff、hash 变、mtime 变都会重置 timer; 超过 threshold 后输出 `StuckSignal`。 | `pub fn(...) -> PaneDiffTickResult` |
| `pane_diff_watcher_tick` | `src/pane_diff/mod.rs:157-217` | 查询 BUSY agents, capture pane, 调三信号判定; stuck 后标 DB 状态并推 `EventFrame kind:"stuck"`。 | private async fn |
| `resolve_stuck_watch_config` | `src/pane_diff/mod.rs:253-258` | 解析 stuck watcher 配置, 返回 tick 间隔和 stuck threshold。 | `pub fn() -> (Duration, Duration)` |
| `AH_STUCK_TICK_SECS` | `src/pane_diff/mod.rs:255` | stuck watcher tick 秒数 env override。缺省用 30s。 | env var |
| `AH_STUCK_THRESHOLD_SECS` | `src/pane_diff/mod.rs:256` | stuck 判定 threshold 秒数 env override。缺省用 300s。 | env var |
| `env_duration_secs` | `src/pane_diff/mod.rs:260-275` | env parser。只有正整数有效, 非法值 warn 后 fallback 到默认值。 | private fn |
| orchestrator wiring | `src/orchestrator/mod.rs:22-26` | daemon 启动 watcher 时调用 `resolve_stuck_watch_config`, 不再直接传硬编码常量。 | async task setup |
| `StuckOutcome` | `src/db/state_machine.rs:37-43` | `mark_agent_stuck` 的内部结果结构, 带 changes、agent_id、affected job、原状态。旧 caller 仍拿 `usize`。 | `pub struct` |
| `mark_agent_stuck_outcome_sync` | `src/db/state_machine.rs:514-596` | 真正执行 `BUSY/WAITING_FOR_ACK -> STUCK` 的事务, 同时写 `state_change` event payload。 | private fn |
| `mark_agent_stuck` | `src/db/state_machine.rs:705-730` | async wrapper 保持返回 `usize`, 但 changes > 0 时通过 pubsub 推 `kind:"stuck"` frame。 | `pub async fn(...) -> Result<usize>` |
| `EventFrame: Serialize` | `src/orchestrator/pubsub.rs:4-13` | M2 让 typed event frame 可直接转 JSON, streaming stuck path 可以写 newline-delimited frame。 | derive |
| `notify_event` | `src/orchestrator/pubsub.rs:46-48` | 把 typed `EventFrame` 推入 broadcast bus, M2 stuck push 复用它。 | `pub fn(EventFrame)` |
| `handle_event_subscribe` stuck fast path | `src/rpc/handlers.rs:1011-1021` | 普通 RPC dispatch 收到 `event_kind=["stuck"]` 时, 先从 events 表补发最近 stuck frame。 | async handler |
| `stream_event_subscribe` stuck branch | `src/rpc/handlers.rs:1567-1604` | streaming path 支持 stuck: 先 fast path, 再订阅 typed `EVENT_FRAMES`, 收到匹配 frame 就写出。 | async streaming fn |
| `write_event_frame` | `src/rpc/handlers.rs:1637-1651` | 把 JSON frame 写成一行并补 newline。M1 job terminal 与 M2 stuck 共用。 | private async fn |
| `event_kind_includes` | `src/rpc/handlers.rs:1653-1659` | 解析 `event_kind` filter; string/array 都支持, 未填时默认 `job_state_change`。 | private fn |
| `event_frame_matches_filter` | `src/rpc/handlers.rs:1661-1680` | typed frame 的 kind、agent_id、job_id filter 判断。 | private fn |
| `stuck_frame_for_filter` | `src/rpc/handlers.rs:1682-1735` | 从 `events` 表补发 stuck frame; 可按 agent_id 或 job_id 定位, payload 带 `signal_kinds` / `elapsed_secs`。 | private async fn |

M2 当前占位说明:

- `state_machine::mark_agent_stuck` 写入的 `elapsed_secs=0`、`signal_kinds=["state_machine"]` 是兼容 fast path 的占位值。
- `pane_diff_watcher_tick` 直接推送时会把 `StuckSignal.signal_kinds` 与 `elapsed_secs` 写入 frame payload。
- dogfood-4 收尾或 dogfood-8 需要把 DB state_change payload 的 signal 真值和 watcher 真值完全对齐。

## §3 4 个 M2 dogfood test 的 logic

### T4.1 `test_stuck_push_event_via_subscribe`

准备状态:

- `Harness::seed_dispatched_busy_job` 创建 BUSY agent 与 dispatched job。
- 调 `state_machine::mark_agent_stuck` 把 agent 标为 `STUCK`, 并写 `state_change` event。
- 调 `event.subscribe` 参数包含 `agent_id`、`job_id`、`event_kind:["stuck"]`。

assert:

- RPC 无 error。
- frame `kind == "stuck"`。
- frame `state == "STUCK"`。
- frame `job_id == JOB_ID`。
- `payload.signal_kinds` 非空。

src 路径:

- `mark_agent_stuck` 调 `mark_agent_stuck_outcome_sync`。
- `state_change` event payload 写入 `job_id/signal_kinds/elapsed_secs`。
- `handle_event_subscribe` 识别 stuck filter。
- `stuck_frame_for_filter` 从 events 表补发 stuck frame。

### T4.2 `test_stuck_threshold_env_override`

准备状态:

- 保存旧 `AH_STUCK_THRESHOLD_SECS` / `AH_STUCK_TICK_SECS`。
- 设置 threshold=5s, tick=1s。
- 构造 `PaneDiffObservation { text:"Thinking...", log_mtime:None }`。

assert:

- `resolve_stuck_watch_config` 返回 threshold 5s。
- 第一次 observation 不 stuck。
- 6 秒后的同一 observation 触发 stuck。
- 标记后 agent state 为 `STUCK`。
- 测试耗时 <= 7s。

src 路径:

- `resolve_stuck_watch_config` 读 env。
- `process_pane_diff_observations` 初始化 `thinking_start`。
- 第二次 tick 中 hash/mtime 不变且 elapsed 超过 threshold, 输出 `stuck_agent_ids`。
- `mark_agent_stuck` 写 DB 状态。

### T4.3 `test_push_latency_p95_under_500ms`

准备状态:

- 循环 5 个 Harness 样本。
- 每个样本 seed BUSY job, 记录 `emitted_at`。
- 调 `mark_agent_stuck` 后立即 `event.subscribe(stuck)`。

assert:

- 5 个 stuck subscribe 都无 error。
- latency 排序后 p95 <= 500ms。

src 路径:

- `mark_agent_stuck` 同步写 events 表并推 typed event。
- `event.subscribe(stuck)` fast path 可从 DB 补发, streaming path 可从 typed bus 收 frame。
- test 统计 emit 到 master 收到 frame 的总延迟。

### T4.4 `test_zero_schedule_wakeup_poll`

准备状态:

- `InterventionCounters` 扩 `poll`。
- seed BUSY job, 标 stuck, 订阅 stuck frame。
- 最后调用 `session.kill` 清理。

assert:

- `event.subscribe(stuck)` 无 error。
- `cancel_count == 0`。
- `capture_count == 0`。
- `poll_count == 0`。

src 路径:

- M2 stuck 等待走 `event.subscribe`。
- 测试 master 不调用 ScheduleWakeup poll。
- M1 的 cancel/capture 计数仍保持 0。

## §4 跟 M1 与现有架构兼容

- M1 reader / matcher completion path 未改: `src/agent_io/reader.rs` 与 `src/marker/matcher.rs` 不在 M2 改动范围。
- M1 `event.subscribe` streaming 路径复用: M2 只新增 stuck kind branch, job terminal branch 保留。
- pubsub 兼容: `JOB_UPDATES` / `AGENT_OUTPUT` 旧 channel 不动, M2 使用 `EVENT_FRAMES` typed bus。
- SQLite schema 不动: stuck replay 继续查 `events(event_type='state_change')` 与 JSON payload。
- RPC method 不新增: 继续用 M1 的 `event.subscribe`, 只是扩 `event_kind=["stuck"]` filter。
- `mark_agent_stuck` 对外签名保持 `Result<usize, CcbdError>`, 内部通过 `StuckOutcome` 保留 affected job 信息。
- `job.wait` / `agent.watch` 旧行为不变。

## §5 不在 M2 的部分

- B4 slash command keystroke 留 dogfood-5: `/clear` 等单行 slash command 还未切 direct keystroke。
- B5 `health_check.rs` / multi-layer probe 留 dogfood-6: InitProbe 尚未和 completion detector 合并。
- B6 真 stdout fake provider 全套留 dogfood-8: 需要真实 `mock_dogfood_provider.sh` stdout 经 agent_io reader 进入 events。
- `elapsed_secs` / `signal_kinds` 真值完全持久化留 dogfood-4 收尾或 dogfood-8: 当前 DB fast path 仍有 `elapsed_secs=0` 与 `["state_machine"]` 占位。
- queue GC 与 stuck 后自动出队不在 M2。
