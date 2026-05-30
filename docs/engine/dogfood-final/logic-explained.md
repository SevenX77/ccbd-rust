# Dogfood Final Logic Explained

## §1 PR 范围

M-final 是 ah dogfooding closure 的收尾 PR, 合并 design 中的 B5 与 B6:

- B5 health check: 新建 `src/provider/health_check.rs`, 把 tmux pane alive、provider predicate、completion detector 最近进展三层合成一个健康判断。
- B6 dogfood-8 真 stdout 主测: `tests/ah_dogfooding.rs` 不再用 `insert_event` 伪造 completion, 改为真实 `agent.spawn` 起 bash provider pane, 再让 mock provider stdout 经过 `agent_io::reader` 进入 events/state_machine/pubsub。
- must-fix 收尾: dogfood-8 的 full SOP08 改成单 Harness/session 连续链, 不是多个片段测试拼接。

M-final 不改 M1 reader/marker 对账语义, 不改 M2 stuck schema, 不改 M3a slash writer 行为。

## §2 新增字段与函数

| 字段 / 函数 | 文件:line | 中文 logic 解释 | 类型 / 签名 |
|---|---:|---|---|
| `HealthCheckResult` | `src/provider/health_check.rs:8-12` | 一次健康检查的总结果。`alive=true` 表示三层都没判 dead; `dead_layers` 记录死在哪一层; `last_progress_ts` 给 completion stale 计算使用。 | `pub struct` |
| `HealthCheckResult.alive` | `src/provider/health_check.rs:9` | health check 的布尔结论。只要 tmux/predicate/completion 任一层 dead, 就是 `false`。 | `bool` |
| `HealthCheckResult.dead_layers` | `src/provider/health_check.rs:10` | 死层列表。当前值为 `"tmux"`, `"predicate"`, `"completion"`; push stuck 时转成 `health:<layer>`。 | `Vec<String>` |
| `HealthCheckResult.last_progress_ts` | `src/provider/health_check.rs:11` | 最近一次 output 或 marker 的 Unix 秒。用于判断 completion 是否超过 stuck threshold 没进展。 | `i64` |
| `HealthCheckObservation` | `src/provider/health_check.rs:15-24` | watcher 输入的单 agent 观测快照, 把状态、provider、pane capture、最近 output/marker 时间放到一个结构里。 | `pub struct` |
| `agent_id` | `src/provider/health_check.rs:16` | 被检查的 agent id, 后续 STUCK event 和 DB 状态变更都用它定位 agent。 | `String` |
| `provider` | `src/provider/health_check.rs:17` | provider 名称, 用来取 `get_manifest(provider).init_probe`。 | `String` |
| `state` | `src/provider/health_check.rs:18` | 当前 agent state。只有 `SPAWNING/WAITING_FOR_ACK/BUSY` 这类 active state 才能 escalate。 | `String` |
| `pane_capture` | `src/provider/health_check.rs:19` | tmux capture-pane 的可见文本。provider predicate 在 `SPAWNING` 阶段读取它。 | `String` |
| `pane_capture_ok` | `src/provider/health_check.rs:20` | tmux capture 是否成功。失败直接加入 dead layer `"tmux"`。 | `bool` |
| `last_output_ts` | `src/provider/health_check.rs:21` | 最近 `output_chunk` 时间。没有 output 时是 `None`。 | `Option<i64>` |
| `last_marker_ts` | `src/provider/health_check.rs:22` | 最近 `<<ah-idle:job-id=X>>` marker 时间。优先级高于普通 output。 | `Option<i64>` |
| `now_ts` | `src/provider/health_check.rs:23` | 检查时刻 Unix 秒。测试可注入固定值, watcher 使用当前时间。 | `i64` |
| `health_check_observe` | `src/provider/health_check.rs:26-57` | 纯函数: 从 observation 计算三层健康结果。它不写 DB, 方便 unit tests 锁逻辑。 | `pub fn(&HealthCheckObservation, i64) -> HealthCheckResult` |
| tmux layer | `src/provider/health_check.rs:36-38` | `pane_capture_ok=false` 时记录 `"tmux"`。这代表 pane 不可读或不存在。 | branch |
| provider predicate layer | `src/provider/health_check.rs:40-45` | `SPAWNING` 阶段复用 provider manifest 的 InitProbe predicate; predicate 不通过记录 `"predicate"`。 | branch |
| completion layer | `src/provider/health_check.rs:47-51` | `WAITING_FOR_ACK/BUSY` 且最近 output/marker 超过 threshold, 记录 `"completion"`。 | branch |
| `escalate_health_stuck` | `src/provider/health_check.rs:59-112` | health dead 且 agent active 时, 调 `mark_agent_stuck`, 写 state_change event, 再推 `EventFrame kind:"stuck"`。 | `pub async fn(&Ctx, &HealthCheckObservation, i64) -> Result<usize>` |
| `signal_kinds` | `src/provider/health_check.rs:77-81` | 把 dead layer 翻译成 `health:tmux`, `health:predicate`, `health:completion`, 放进 stuck payload。 | `Vec<String>` |
| health `state_change` payload | `src/provider/health_check.rs:82-90` | 额外写一条带 `HEALTH_CHECK_STUCK`、`job_id`、`signal_kinds`、`elapsed_secs` 的 event, 让 replay path 能解释来源。 | JSON |
| health `EventFrame` | `src/provider/health_check.rs:98-109` | 复用 M2 typed event bus 推 `kind:"stuck"` frame, master client 可用 `event.subscribe` 收到。 | `EventFrame` |
| `health_check_watcher_loop` | `src/provider/health_check.rs:115-121` | daemon 后台 loop。每 tick 跑一次 `health_check_watcher_tick`。 | `pub async fn(Ctx, Duration, Duration)` |
| `health_check_watcher_tick` | `src/provider/health_check.rs:123-154` | 查询 `SPAWNING/WAITING_FOR_ACK/BUSY` agents, capture pane, 查最近 progress, 组装 observation 并调用 escalate。 | private async fn |
| `query_last_progress` | `src/provider/health_check.rs:156-178` | 从 events 表扫描当前 agent 的 `output_chunk`; 记录最近 output 时间和最近 idle marker 时间。 | private async fn |
| `is_active_state` | `src/provider/health_check.rs:180-182` | 判断 state 是否允许 health escalate。 | private fn |
| `is_working_state` | `src/provider/health_check.rs:184-186` | 判断 state 是否需要 completion progress 检查。 | private fn |
| `health_check_watcher_loop` wire | `src/orchestrator/mod.rs:8,28-32` | orchestrator 启动时, 除 M2 `pane_diff_watcher_loop` 外, 同时启动 health watcher, 共用 `resolve_stuck_watch_config()`。 | task wire |
| `mock_dogfood_provider.sh` env | `tests/fixtures/mock_dogfood_provider.sh:8-12` | fake provider 支持 `FAKE_PROVIDER_DELAY_MS`, `FAKE_PROVIDER_STUCK_MS`, `FAKE_PROVIDER_SLASH_ACK_TEXT`, 供 dogfood tests 控制 stdout 行为。 | bash vars |
| `install_mock_dogfood_provider` | `tests/ah_dogfooding.rs:200-226` | 把 fixture 复制到 temp `bin/`, 同时创建 fake `claude` binary, 并设置 PATH/HOME。M-final 真路径使用 bash provider 执行 fixture。 | test helper |
| `spawn_real_dogfood_agent` | `tests/ah_dogfooding.rs:228-251` | 真调用 `agent.spawn` 起一个 bash agent pane, 等 InitProbe 到 IDLE, 再启动 orchestrator task。 | async test helper |
| `enqueue_known_job` | `tests/ah_dogfooding.rs:253-273` | 直接插入一个已知 job id 的 queued job, prompt 是 shell pipeline: `printf job-id | mock_dogfood_provider.sh`。然后 `wake_up` 让 orchestrator 真 dispatch。 | async test helper |
| `run_real_stdout_job` | `tests/ah_dogfooding.rs:308-326` | 等 job 从真实 stdout marker 完成, 再通过 `event.subscribe` 查 `job_state_change(COMPLETED)` frame, 统计 subscribe latency。 | async test helper |
| `shell_quote` | `tests/ah_dogfooding.rs:328-330` | 给 shell pipeline 中的 fixture path 做单引号转义。 | test helper |

## §3 dogfood-8 三测 logic

### `test_dogfood_e2e_full_sop08_simulation`

准备状态:

- `Harness::new` 建临时 DB/state/project。
- `spawn_real_dogfood_agent` 用 `agent.spawn(provider:"bash")` 起真 tmux pane, reader task 和 FIFO 都走生产路径。
- 同一个 Harness/session/agent 贯穿全测。

assert:

- 顺序跑 5 个 `dogfood_final_N` job, 每个都由 `run_real_stdout_job` 等到 `COMPLETED`。
- slash 步通过 `agent.send` 执行 `printf '/clear\n' | mock_dogfood_provider.sh`, 然后在真实 `output_chunk` 里找到 `<<ah-slash-ack:cmd=/clear>>`。
- stuck 步构造 active BUSY job, `escalate_health_stuck` 推 `stuck` frame, `event.subscribe(event_kind:["stuck"])` 能读到。
- cancel 步取消 queued job, 返回 `CANCELLED`。
- `cancel_counter == 0`, `capture_counter == 0`, `poll_counter == 0`。

src/test 流:

`jobs::insert_job` -> `orchestrator::wake_up` -> `orchestrator::run_once` dispatch -> `agent_io::send_text_to_pane` -> bash pane 执行 mock provider -> provider stdout -> FIFO -> `agent_io::reader` -> `events::insert_event(output_chunk)` -> marker hook -> `state_machine::mark_agent_idle_matched` -> job `COMPLETED` -> `event.subscribe` terminal frame。

### `test_push_latency_p95_real_stdout`

准备状态:

- 同样先 `spawn_real_dogfood_agent`。
- 连续跑 5 个 `latency_stdout_N` job。

assert:

- 每个 job 先经过真实 stdout marker 完成。
- 完成后立即 `event.subscribe` 取 terminal frame。
- 5 个样本排序后 p95 <= 500ms。

说明:

这个测试不再测 standalone child process stdout, 而是锁住 reader/state_machine/pubsub/replay 真实链路的 terminal frame 获取延迟。

### `test_health_check_dead_layer_escalates_stuck`

准备状态:

- seed 一个 BUSY job。
- 构造 `HealthCheckObservation { pane_capture_ok:false, state:"BUSY" }`。

assert:

- `health_check_observe` 返回 dead layer `tmux`。
- `escalate_health_stuck` 返回 changes=1。
- `event.subscribe(event_kind:["stuck"])` 返回 stuck frame。
- frame payload 里有 `health:` 开头的 signal。

src 流:

`health_check_observe` -> `mark_agent_stuck` -> 额外 `state_change` event -> `pubsub::notify_event(EventFrame kind:"stuck")` -> `handle_event_subscribe` stuck fast path。

## §4 兼容性

- M1 completion 语义未改: marker 仍是 `<<ah-idle:job-id=X>>`, job-id 对账仍在状态机边界。
- M2 stuck 语义未改: health 只是新增 signal source, 继续复用 `event.subscribe` 的 `kind:"stuck"`。
- M3a slash writer 未改: full SOP08 中 slash ack 是对既有 `/clear` 能力的回归验证。
- SQLite schema 未改: health/stuck 仍写 `events` JSON payload。
- RPC/CLI 形状未改: dogfood-8 使用现有 `agent.spawn`, `agent.send`, `job.cancel`, `event.subscribe`。

## §5 后续

- 真 `ah` binary smoke: 当前 dogfood-8 在测试内用 in-process RPC harness, daemon 侧 tmux/reader/state_machine 真路径已验; CLI binary smoke 可作为外部验收补充。
- `elapsed_secs` / `signal_kinds` 真值精度: health path 已写真 health signal, M2 state_machine fast path仍有部分占位, 属监控精度收尾。
- 真 LLM e2e: 当前 fake provider 是协议层可重复测试, 真 LLM 留外部环境验收。
