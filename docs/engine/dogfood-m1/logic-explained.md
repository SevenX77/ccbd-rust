# Dogfood M1 Logic Explained

## §1 PR 范围

M1 把 dogfood-1 与 dogfood-2 合并为首个 ship PR:

- B2 真 completion path: fake provider 输出 `<<ah-idle:job-id=X>>`, ah 从 `output_chunk` 解析 marker, 由状态机驱动 `BUSY -> IDLE` 并完成当前 job。
- B1 UDS streaming/subscribe: 在现有 Unix socket JSON-RPC 上新增 `event.subscribe`, 让 master client 等终态 frame, 不再用 `ah ask --wait` 循环 `job.wait`。
- regression cutover: PR-1/drift/realign/mvp9 依赖的 completion seam 迁移到 marker path。

M1 不实现 stuck push、slash keystroke、health check、多 provider 真 stdout dogfood 主测。这些留给后续 PR。

## §2 新增 / 修改字段与函数

| 字段 / 函数 | 文件:line | 中文 logic 解释 | 类型 / 签名 |
|---|---:|---|---|
| `event.subscribe` RPC method | `src/rpc/router.rs:13-37` | 把 `event.subscribe` 加入 method whitelist, 让普通 JSON-RPC dispatcher 承认这是合法 method。 | `&'static str` |
| `event.subscribe` dispatch arm | `src/rpc/router.rs:73-98` | 非 streaming 单次调用时转到 `handle_event_subscribe`, 用于已终态 job 的 fast path 和测试内 RPC dispatch。 | match arm |
| `EventFrame` | `src/orchestrator/pubsub.rs:4-13` | daemon 内部事件帧结构。字段是 streaming 输出的逻辑 schema: 事件 id、事件 kind、agent、job、state、时间戳、payload。 | `pub struct EventFrame` |
| `event_id` | `src/orchestrator/pubsub.rs:6` | frame 的事件编号。M1 实际 terminal frame 用 job completion seq/time 值填充, 后续可接 SQLite `events.seq_id`。 | `i64` |
| `kind` | `src/orchestrator/pubsub.rs:7` | frame 类型。M1 completion path 使用 `"job_state_change"`。 | `String` |
| `agent_id` | `src/orchestrator/pubsub.rs:8` | frame 关联的 agent id, 由 job row 的 `agent_id` 填入。 | `String` |
| `job_id` | `src/orchestrator/pubsub.rs:9` | frame 关联的 job id。M1 `event.subscribe` 必填 `job_id`, 输出也带同一个 job id。 | `Option<String>` |
| `state` | `src/orchestrator/pubsub.rs:10` | job 或 agent 状态。M1 终态 frame 使用 `COMPLETED/FAILED/CANCELLED`。CLI 接受 `KILLED` 作为兼容终态。 | `Option<String>` |
| `ts_unix_micro` | `src/orchestrator/pubsub.rs:11` | daemon 生成 frame 的 Unix 微秒时间, 为后续 push latency p95 统计预留。 | `i64` |
| `payload` | `src/orchestrator/pubsub.rs:12` | 终态 frame 的业务 JSON。M1 放 `job_id/status/reply_text/error_reason`。 | `Option<Value>` |
| `EVENT_FRAMES` | `src/orchestrator/pubsub.rs:25-28` | typed event bus 的 broadcast channel。M1 已落结构, 但 completion streaming 主路径仍桥接 `JOB_UPDATES`。 | `LazyLock<broadcast::Sender<EventFrame>>` |
| `notify_event` | `src/orchestrator/pubsub.rs:46-48` | 向 typed event bus 发送 frame 的入口。M1 保留给后续 stuck/p95 等事件 fanout。 | `fn(EventFrame)` |
| `subscribe_events` | `src/orchestrator/pubsub.rs:50-52` | typed event bus 订阅入口。M1 未替代旧 `subscribe_job_updates`。 | `fn() -> Receiver<EventFrame>` |
| `event_subscribe_params` | `src/rpc/mod.rs:77-91` | server 读到一行 JSON 后, 先判断 method 是否为 `event.subscribe`; 是则提取 params, 不走 single-response dispatcher。 | `fn(&str) -> Option<Value>` |
| streaming writer branch | `src/rpc/mod.rs:41-58` | UDS connection 命中 `event.subscribe` 后交给 `stream_event_subscribe`, 写一行 newline-delimited frame 后结束当前 stream。 | async server loop |
| `handle_event_subscribe` | `src/rpc/handlers.rs:1011-1016` | 普通 dispatch fast path: 查 job 是否已终态, 已终态返回 frame, 未终态返回 timeout error。 | `async fn(Value, &Ctx) -> Result<Value>` |
| `stream_event_subscribe` | `src/rpc/handlers.rs:1562-1624` | streaming path: 先查 terminal frame; 没有则订阅 `JOB_UPDATES`, 等目标 job 更新为终态后写 JSON line。 | `async fn<W>(Value, &Ctx, &mut W)` |
| `event_frame_for_job` | `src/rpc/handlers.rs:1517-1560` | 从 jobs 表查询指定 job, 只有 `COMPLETED/FAILED/CANCELLED` 才生成 `job_state_change` frame。 | private async fn |
| `rpc_stream_first` | `src/cli/rpc_client.rs:159-186` | CLI streaming client: 连接 `ahd.sock`, 写 JSON-RPC request, 读第一行 frame 并反序列化。 | `pub fn(&Path, &str, Value) -> Result<Value>` |
| `wait_for_job` | `src/bin/ah.rs:523-545` | `ah ask --wait` 的 wait 逻辑: 调 `event.subscribe`, 等 `COMPLETED/FAILED/CANCELLED/KILLED` frame, 返回 frame payload。 | `async fn(&UnixRpcClient, &str)` |
| `extract_ah_idle_marker_job_id` | `src/db/state_machine.rs:435-445` | 从输出文本中找 `<<ah-idle:job-id=` 与 `>>`, 提取中间 job id; 空 id 不接受。 | `pub fn(&str) -> Option<String>` |
| `latest_ah_idle_marker_job_id` | `src/db/state_machine.rs:400-433` | 在当前 dispatched job 的 `dispatched_at_seq_id` 之后扫描该 agent 的 `output_chunk`, 找最后一个 ah idle marker job id。 | private fn |
| marker job-id 对账 | `src/db/state_machine.rs:314-331` | `mark_agent_idle_matched` 真正转 IDLE 前, 若输出里有 ah marker, 必须等于当前 dispatched job id; 不等则 rollback 并保持 BUSY。 | 状态机 guard |
| `insert_event` marker hook | `src/db/events.rs:100-140` | 插入 `output_chunk` 前先检测文本是否含 ah idle marker; 插入后调用状态机完成 job, 并通知 waiters。 | `pub async fn insert_event(...)` |
| `MarkerMatcher` hook | `src/marker/matcher.rs:57-64` | vt100 screen contents 只要含 `<<ah-idle:job-id=`, 就认为屏幕有 completion marker。job-id 是否正确不在 matcher 判定, 在状态机判定。 | `scan(&Parser) -> MatchResult` |
| `emit_provider_idle_marker` | `tests/ah_full_e2e_main.rs:209-236` | PR-1 regression helper: dispatch job 到 BUSY, 插入带 marker 的 `output_chunk`, 等状态机自然回 IDLE。 | test helper |
| `mock_dogfood_provider.sh` | `tests/fixtures/mock_dogfood_provider.sh:1-115` | fake provider: 读一行 message, 解析 job id, 输出 received/working/done, 最后输出 `<<ah-idle:job-id=X>>`。 | bash fixture |
| `dogfood_ah_client` | `tests/ah_dogfooding.rs:159-163` | 测试内 UDS client helper, 指向 `state_dir/ahd.sock`。M1 tests 主要仍用 in-process RPC harness。 | test helper |
| `InterventionCounters` | `tests/ah_dogfooding.rs:165-179` | 测试侧主控介入计数器, 记录 cancel 与 capture 是否被测试 master 主动调用。 | test struct |
| `dispatch_job_via_ah` | `tests/ah_dogfooding.rs:181-186` | 测试 helper: 经 `job.submit` 产生 job id, 不调用旧 seam。 | async test helper |

## §3 5 个 dogfood test 的 logic

### T3.1 `test_event_subscribe_pushes_idle_frame`

准备状态:

- `Harness::seed_dispatched_busy_job` 创建 session、IDLE agent、job, 然后通过 `dispatch_job_to_agent` 把 job 派成 `DISPATCHED`, agent 置为 `BUSY`。
- test 插入一条 `output_chunk` 事件, 文本含 `<<ah-idle:job-id=dogfood_job_1>>`。

assert:

- `event.subscribe` 不返回 RPC error。
- 返回 frame 的 `kind == "job_state_change"`。
- 返回 frame 的 `state == "COMPLETED"`。

src 逻辑流:

- `events::insert_event` 检测 marker。
- `state_machine::mark_agent_idle_matched` 查询 marker job id 并和当前 dispatched job 对账。
- 对账通过后 job 完成, `notify_job_update(job_id)` 唤醒 waiters。
- `router::dispatch("event.subscribe")` 调 `handle_event_subscribe`, 从 `event_frame_for_job` 返回 terminal frame。

### T3.2 `test_real_completion_path_no_seam`

准备状态:

- 安装 `mock_dogfood_provider.sh` fixture, 但本 M1 regression test 仍用 test `insert_event` 注入 marker 文本。
- seed BUSY job 后插入正确 job-id marker。

assert:

- agent state 最终为 `IDLE`。
- job status 最终为 `COMPLETED`。
- 不调用 `dispatch_and_complete_job` 旧 seam。

src 逻辑流:

- `insert_event` 写入 output。
- marker hook 调状态机。
- 状态机从 output events 里取 marker job id。
- marker job id 等于当前 dispatched job id, 允许 `BUSY -> IDLE` 并完成 job。

### T3.3 `test_zero_cancel_zero_capture_assertion`

准备状态:

- seed BUSY job。
- 插入正确 marker output。
- 创建 `InterventionCounters`。

assert:

- `event.subscribe` 不报错。
- `cancel_counter == 0`。
- `capture_counter == 0`。

src 逻辑流:

- completion 仍由 marker path 完成。
- wait 使用 `event.subscribe` frame, 不需要 master side `job.cancel`。
- 测试 master 不执行 `tmux capture-pane` verify, capture counter 保持 0。

### T3.4 `test_pr1_regression_still_green`

准备状态:

- 读取 `tests/ah_full_e2e_main.rs` 源码。

assert:

- 文件中不再包含 `dispatch_and_complete_job`。

src 逻辑流:

- PR-1 旧 helper 被 `emit_provider_idle_marker` 替代。
- `emit_provider_idle_marker` 不直接 `mark_job_completed`, 而是插入 marker output 后等状态机返回 IDLE。

### T3.5 `test_marker_job_id_对账`

准备状态:

- seed BUSY job `dogfood_job_1`。
- 先插入 `<<ah-idle:job-id=WRONG>>`。
- 再插入 `<<ah-idle:job-id=dogfood_job_1>>`。

assert:

- 错 marker 后 agent 仍为 `BUSY`。
- 对 marker 后 agent 变为 `IDLE`。

src 逻辑流:

- `extract_ah_idle_marker_job_id` 提取 `WRONG`。
- 状态机查当前 dispatched job id 是 `dogfood_job_1`。
- mismatch 时 rollback, 不完成 job。
- 第二个 marker 匹配后才允许完成。

## §4 跟现有架构兼容性

- 9 个 SQLite agent state 不动: `SPAWNING/IDLE/WAITING_FOR_ACK/BUSY/PROMPT_PENDING/STUCK/CRASHED/KILLED/UNKNOWN`。
- SQLite schema 不动: M1 没新增 table/column, marker 通过既有 `events.payload` 与 `jobs.dispatched_at_seq_id` 对账。
- RPC 旧方法保留: `job.wait`, `agent.watch`, `job.cancel`, `agent.read` 等都保留原行为。
- RPC method 数从 22 增到 23: 新增 `event.subscribe`。
- CLI subcommand 不动: `ah ask --wait` 内部 wait 机制变更, 用户命令形状不变。
- UDS transport 不变: 仍是 `state_dir/ahd.sock`, 只在 `event.subscribe` method 上允许 newline-delimited frame。
- pubsub 兼容: `JOB_UPDATES`/`AGENT_OUTPUT` 仍服务 `job.wait`/`agent.watch`; `EventFrame` typed bus 已预留, 不破坏旧通知。
- test seam cutover 是内部 BREAKING: 删除 `dispatch_and_complete_job`, 对用户 API 无影响。

## §5 不在 M1 的部分

- B3 stuck multi-signal + push escalate 留 dogfood-4: `pane_diff` hash/mtime/provider-aware 与 `stuck` frame 尚未落地。
- B4 slash command keystroke 留 dogfood-5: `/clear` 等 slash command 仍未改成 direct keystroke path。
- B5 multi-layer probe + completion detector wire 留 dogfood-6: startup `InitProbe` 尚未和 completion detector/health check 合并。
- B6 dogfooding e2e 主测留 dogfood-8: 必须使用真实 `mock_dogfood_provider.sh` stdout marker 经 agent_io reader 进入 DB, 不能停留在 test `insert_event` 层。
- push latency p95、stuck <= 310s、slash 100% 三个指标留 M2/M3。
- M1 的 0 cancel + 0 capture 只锁主控侧介入; daemon 内自动 watcher capture 不属于人工 verify。
