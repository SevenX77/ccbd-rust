# Design: ah dogfooding closure

## §1 目标与度量

目标: 主控使用 `ah ask --wait` 派发 a1/a2/a3, 在模拟 PR-6 体量的 SOP-08 任务中完成 research -> design -> impl -> e2e -> audit -> report 闭环, 不依赖 ccb ask 的人工介入模式。

验收全部落到 `tests/ah_dogfooding.rs` 的物理断言:

1. 主控介入计数器为 0: `job.cancel` 调用数 = 0, 主控侧 `tmux capture-pane` verify 次数 = 0, `ScheduleWakeup` 式 poll 次数 = 0。
2. push 延迟 p95 <= 500ms: ah daemon 写入/emit job 终态事件到 master client 收到 `EventFrame` 的耗时, e2e 内置 histogram 断言。
3. stuck escalate 延迟 <= 310s: agent 卡住后, `pane_diff`/health check 标记 `STUCK` 到 master client 收到 `stuck` frame, 不超过 300s 阈值 + 10s 余量。
4. slash command 投递成功率 100%: fake claude/codex/gemini 对 `/clear` 或等价映射输出 slash ack, 不被当普通 prompt。
5. 完整 dogfood e2e 跑通 5 个典型 RPC 调用模拟 PR-6 dispatch: dispatch a2 设计、dispatch a1 audit、inject stuck、cancel/kill 终态、slash command, 全程无 timeout/error。

## §2 §0.5 继承字段表

| 项 | 现状 file:line | 设计 |
|---|---|---|
| SQLite STATE 9 个 | `src/db/state_machine.rs:13-26` | 继承不动: `SPAWNING`, `IDLE`, `WAITING_FOR_ACK`, `BUSY`, `PROMPT_PENDING`, `STUCK`, `CRASHED`, `KILLED`, `UNKNOWN` |
| SQLite tables | `src/db/schema.rs:1-100` | 继承 `projects/sessions/agents/events/evidence/jobs/prompt_experience`; B1 事件帧复用 `events` schema, 不新增表 |
| events schema | `src/db/schema.rs:35-45` | 继承 `seq_id/agent_id/request_id/event_type/payload/created_at`; streaming frame 以 `seq_id` 作为 `event_id` |
| jobs schema | `src/db/schema.rs:65-83` | 继承 `status`, `dispatched_at_seq_id`, `completed_at`, `cancel_requested`; B2 job-id marker 只改变完成驱动, 不改字段 |
| RPC method 22 个 | `src/rpc/router.rs:13-36` | 22 个全部继承; `[NEW] event.subscribe` 作为第 23 个 method |
| CLI subcommand 16 个 | research §2.2 | 16 个全部继承: `Ping`, `Version`, `Ps`, `Start`, `Up`, `Ask`, `Pend`, `Cancel`, `Kill`, `Watch`, `Logs`, `Attach`, `Stop`, `Doctor`, `Config`, `Prompt` |
| RPC transport | `src/rpc/mod.rs:23-66`, `src/cli/rpc_client.rs:115-157` | 继承 UDS JSON-RPC transport; B1 只扩长期 streaming response, 不引入 TCP/HTTP/SSE |
| socket path | `src/cli/rpc_client.rs:102-112`, `src/bin/ahd.rs:61-64` | 继承 `state_dir/ahd.sock`; `CCB_SOCKET` 兼容现状, 新设计不新增第二 socket |
| pane_diff config | `src/pane_diff/mod.rs:9-10`, `src/orchestrator/mod.rs:22-26` | `[NEW]` 配置字段 `stuck_tick_secs`, `stuck_threshold_secs`; env override `AH_STUCK_TICK_SECS`, `AH_STUCK_THRESHOLD_SECS` |

## §3 设计组件

### B1 UDS streaming/subscribe RPC method

现状:

- ah daemon 已通过 Unix domain socket 提供 JSON-RPC: `src/rpc/mod.rs:23-66`。
- CLI 通过 `UnixRpcClient` 每次连接、写一行 JSON、shutdown write、读完整 response: `src/cli/rpc_client.rs:115-157`。
- `job.wait` 与 `agent.watch` 目前是短周期等待: `src/rpc/handlers.rs:971-1008`, `src/rpc/handlers.rs:1403-1458`。
- 内部 pubsub 已有 in-process broadcast: `src/orchestrator/pubsub.rs:4-28`。
- `ah ask --wait` 目前循环调用 `job.wait(timeout=30)`: `src/bin/ah.rs:409-430`, `src/bin/ah.rs:523-548`。

新增 RPC method: `event.subscribe`。命名沿用现有 `namespace.verb` 规则, 注册位置为 `src/rpc/router.rs:13-36` method whitelist 与 `src/rpc/router.rs:73-96` dispatch match。

Protocol:

```json
{"jsonrpc":"2.0","method":"event.subscribe","params":{"agent_id":"a1","job_id":"job_x","event_kind":["job_state_change","stuck"]},"id":1}
```

server 在同一 UDS connection 上持续写 newline-delimited frame, 直到 client 断开或 daemon shutdown:

```json
{"event_id":123,"kind":"job_state_change","agent_id":"a1","job_id":"job_x","state":"COMPLETED","ts_unix_micro":1770000000000000}
```

落点:

- `src/rpc/handlers.rs`: 新增 `handle_event_subscribe`, 从 `events` 表补发 filter 后的新事件, 然后桥接 `orchestrator::pubsub`。
- `src/rpc/mod.rs:41-58`: streaming method 不能走现有 `dispatch -> single String response` 形态; 需要在读到 method 后把 connection 交给 streaming handler。
- `src/orchestrator/pubsub.rs:4-28`: 扩为 typed event bus, 新增 `EventFrame` sender; 现有 `notify_job_update`/`notify_agent_output` 保留兼容。
- `src/bin/ah.rs:523-548`: `wait_for_job` 从 poll `job.wait` 改为 `event.subscribe` + terminal event select; Bash 物理 timeout 只负责断开当前 wait, 不丢 job。

Frame schema 复用 `events` 语义:

- `event_id`: SQLite `events.seq_id` 或内存生成 id; 有 DB event 时必须等于 `seq_id`。
- `kind`: `job_state_change`, `agent_output`, `stuck`, `shutdown`。
- `agent_id`, `job_id`: filter 与终态关联字段。
- `state`: job 或 agent 终态。
- `ts_unix_micro`: daemon emit 时间, 用于 p95 统计。
- `payload`: 原 `events.payload` JSON。

MVP 限制: 单 daemon 进程内 fanout, 单 master client 深度自驱; client 断线清理订阅, 不做跨 daemon replay/reconnect 队列。

### B2 真 completion path

现 test seam:

- `tests/ah_full_e2e_main.rs:210-240` 的 `dispatch_and_complete_job` 手工 dispatch, 手工写 `output_chunk`, 手工 `mark_job_completed`, 手工把 agent 改回 `IDLE`。
- `tests/ah_full_e2e_main.rs:428-430` 明确标注 output_chunk 内容来自 test seam。

设计 cutover:

- `[BREAKING]` 同一 PR 删除 `dispatch_and_complete_job` seam, PR-1 regression 同步迁移到 fake provider 真 marker 路径。
- 真读取路径使用 `src/agent_io/reader.rs:125-149` 保存真实 pane/fifo output chunk。
- completion 检测使用 `src/agent_io/reader.rs:151-193` 的 `MarkerMatcher` scan, 命中后调用 `mark_agent_idle_matched` 并 `notify_job_update`。
- 状态常量与合法活动态继承 `src/db/state_machine.rs:13-26`, `src/db/state_machine.rs:43-48`。

新增 marker 协议:

- fake provider 与 dogfood 测试统一输出 `<<ah-idle:job-id=X>>`。
- parser 加在 `MarkerMatcher` 或 reader scan 层, 但 job_id 对账必须发生在状态机边界: 只允许当前 agent 的 `DISPATCHED` job_id 与 marker job_id 相等时 BUSY -> IDLE。
- claude/codex/gemini 初始都支持 fake marker。真实 provider 自有 idle 文本只作为 provider-aware fallback, 不作为 dogfood e2e 主断言。

状态机效果:

- `WAITING_FOR_ACK` -> `BUSY`: 仍由 orchestrator dispatch/ACK 稳定窗口驱动, 现落点 `src/orchestrator/mod.rs:81-130`, `src/orchestrator/mod.rs:152-170`。
- `BUSY` -> `IDLE`: 由 reader marker 命中驱动, 不允许测试直接写 DB。
- job 完成文本从 marker 前后的 output chunk 聚合, 继续复用现有 jobs/reply 收集路径。

### B3 stuck 多信号 + push escalate

现状:

- `src/pane_diff/mod.rs:9-10` hardcode 30s tick / 300s threshold。
- `src/pane_diff/mod.rs:73-123` 每 tick 查询 BUSY agents, `capture_pane`, 做文本 diff, 超阈值后 `mark_agent_stuck`。
- `src/db/state_machine.rs:435-493` 将 BUSY/WAITING_FOR_ACK CAS 到 `STUCK`, 并写 `state_change` event。
- `src/orchestrator/mod.rs:22-26` 固定启动 pane_diff watcher。

扩展信号:

1. pane content hash: 现有 `sanitize_for_diff`/`is_meaningful_diff` 保留, `AgentDiffState` 增加 `last_content_hash`。
2. log mtime: 新增 provider log path resolver, 记录最后写入时间; 无 log provider 返回 `None`, 不阻塞 hash 信号。
3. provider-aware: 对 codex/gemini/claude 的长时间 `Thinking`/spinner/announce 做专用分类, 避免把假活当实质进展。

C5 配置化:

- 新 config 字段: `stuck_tick_secs` 默认 30, `stuck_threshold_secs` 默认 300。
- env override: `AH_STUCK_TICK_SECS`, `AH_STUCK_THRESHOLD_SECS`。
- `src/orchestrator/mod.rs:22-26` 从 hardcoded constants 改为解析后的 config/env duration。

STUCK push:

- `mark_agent_stuck` 成功后必须发 `EventFrame { kind: "stuck", job_id, agent_id, signal_kinds, elapsed_secs, ts_unix_micro }`。
- schema 不新增表; 事件持久化继续使用 `events(event_type='state_change', payload.to='STUCK')`。
- `pane_diff` 需要拿到 affected job_id。若 `mark_agent_stuck` 仍只返回 `usize`, 则在 B3 PR 中扩返回 `(changes, affected_job)` 或在 watcher 中查询当前 dispatched job。

### B4 slash command keystroke

现状: `src/agent_io/writer.rs:6-43` 对所有 send 使用 tmux `load_buffer` + `paste_buffer` + delayed Enter。

scope 收窄:

- message 首字符为 `/` 且单行: 走 keystroke direct send。
- 其他内容: 保持 paste-buffer + Enter 路径不动, 避免多行、大文本、escape 编码风险。

落点:

- `src/agent_io/writer.rs:6-43`: 新增 `send_text_to_pane` 内部分支或拆 `send_slash_command_to_pane`。
- `src/rpc/handlers.rs:1158-1233`: `handle_agent_send` 不直接关心 transport, 只保留 request_id/idempotency/state 语义。
- per-provider mapping: `/clear` / `/new` 映射由 provider manifest 或新 `slash_map` helper 提供, fake provider 在测试中输出 slash ack。

说明: 此项不属于 PR-6 §5 五类介入点的最小闭合, 属 master client 完整性补全。

### B5 multi-layer probe 接入 completion detector

现状:

- `src/provider/init_probe.rs:8-160` 是 startup readiness probe: tmux capture 文本 + provider predicate。
- `src/provider/init_probe_task.rs:68-187` 周期 capture, `STEADY_COUNT=2`, 结合 startup prompt scan。
- `src/provider/init_probe_task.rs:213-244` 接 prompt handler, `src/provider/init_probe_task.rs:274-294` readiness 后标 IDLE。

设计:

- 新建 `src/provider/health_check.rs`。
- health check 聚合三层: tmux pane alive/capture 可用、provider-specific readiness predicate、B2 completion detector 最近进展。
- health check 与 B3 共用 tick。任一层 dead 且 agent 处于 active state 时, 触发 B3 escalate; 对启动期仍使用 InitProbe deadline, 对工作期使用 stuck threshold。

wire:

- `src/orchestrator/mod.rs:22-26` 启动统一 watcher loop: pane diff + health check。
- `src/pane_diff/mod.rs` 保持纯逻辑, 接收 health check 的 provider-aware observation, 输出 `signal_kinds`。
- `src/agent_io/reader.rs:136-193` 更新 completion detector 最近 marker/output 时间, 供 health check 判定。

### B6 e2e dogfooding 测试

新增:

- `tests/ah_dogfooding.rs` (~300-500 LOC), 使用编译后的 `ah` client 跑主路径, 不直接调 handler 替代 master client。
- `tests/fixtures/mock_dogfood_provider.sh` 或扩 `tests/fixtures/mock_provider.sh:4-12`; 支持 `FAKE_PROVIDER_DELAY_MS`, 输出 `<<ah-idle:job-id=X>>`。

可复用:

- PR-1 fixture: `tests/fixtures/mock_provider.sh:4-12`。
- prompt fixture: `tests/fixtures/mock_prompt_provider.sh:22-115`。
- PR-3 fake claude 行为模式: `tests/ah_full_e2e_realign_extra.rs:524-580`。
- PR-3 红灯/额外 e2e 位置与 Harness pattern: `tests/ah_full_e2e_realign_extra.rs:1-24`, `tests/ah_full_e2e_realign_extra.rs:35-90`。

模拟 SOP-08 互动模式的 5 个 RPC 调用:

1. dispatch a2 设计 -> wait -> 收到 IDLE marker -> `job_state_change(COMPLETED)`。
2. dispatch a1 audit -> wait -> 收到 IDLE marker -> `job_state_change(COMPLETED)`。
3. inject stuck -> watcher 标 `STUCK` -> master 收到 `stuck` frame。
4. cancel/kill stuck job -> `KILLED` 或 `CANCELLED` 终态, 队列 head 不堵塞。
5. slash command `/clear` -> fake provider 输出 slash ack。

instrument:

- cancel counter: 统计 test master client 主动调用 `job.cancel` 次数。
- capture counter: 统计 test master client 主动执行 `tmux capture-pane` 次数; daemon 内部 watcher capture 不算人工 verify。
- poll counter: 禁止 `wait_for_job` 循环 `job.wait(timeout=30)`。
- push latency histogram: 不引入新生产 crate; e2e 内 `Vec<Duration>` 排序算 p95。生产已有 `tracing-subscriber` (`Cargo.toml:21`), 无 `metrics`/`hdrhistogram`。
- stuck latency timer: 从 fake provider 停止输出/mtime 到 `stuck` frame 到达。

## §4 实施计划

| PR | 组 | 名 | LOC | depends |
|---|---|---|---|---|
| dogfood-1 | A1 | B2 真 completion path + PR-1 regression | 600-1000 | - |
| dogfood-2 | A2 | B1 UDS streaming/subscribe + master client | 400-700 | dogfood-1 |
| dogfood-4 | C | B3 stuck 多信号 + push escalate + C5 配置化 | 300-500 | dogfood-2 |
| dogfood-5 | D | B4 slash keystroke | 200-300 | independent |
| dogfood-6 | E | B5 multi-layer probe + completion detector wire | 400-600 | dogfood-1, dogfood-4 |
| dogfood-7 | F | tmux scope lifecycle + tmpdir lifecycle e2e | 200-400 | independent |
| dogfood-8 | G | B6 e2e dogfooding 主测 | 300-500 | dogfood-1..7 部分子集 |

总量: 7 个独立 PR, ~2400-4000 LOC。若把 B1 的 frame protocol/backpressure 做完整 reconnect, 总量可接近 research §9 的 2900-4500 LOC 上界。

Milestones:

- M1 (PR 1-2): A 组完成。跑 step 10 子集 e2e: 0 cancel + 0 capture 断言。闭合 = 充分非必要, 闭合则 stop。
- M2 (+PR 4): C 组完成。新增 0 poll + push p95 < 500ms 断言。
- M3 (+PR 5-7 并行): D/E/F 完成。新增 stuck < 310s + slash 投递 + scope lifecycle 断言。
- M4 (PR 8): G dogfooding test 落盘, 锁全部指标作 regression。

## §5 nice-to-have

1. queue GC: B3 标 `STUCK` 后或 job `KILLED` 后, 队列 head 自动出队让新 job 进入; 落点 `src/orchestrator/mod.rs` queue management 与 `src/db/jobs.rs` claim 逻辑。
2. C3 指标精确化: 0 cancel 表示 0 盲目 cancel; stuck 真触发后主控可基于 B1 推送的 STUCK 事件决策 cancel, 不算人工探测。
3. C3 指标精确化: 0 capture-pane verify 指主控不手工 capture; daemon 内 `pane_diff`/InitProbe 的 capture 是自动观测, 允许。
4. C3 指标精确化: 0 ScheduleWakeup poll 指 master client 不轮询 `job.wait`; UDS subscribe keepalive 不算 poll。
5. C4 vs B4 一致性: idea §C4 的“默认 keystroke”被本 design 收窄为“只有 `/` 开头且单行的 slash command 走 keystroke”。

## §6 风险与缓解

1. 风险: ANSI 字符干扰 IDLE marker。缓解: `agent_io` 已有 `vt100` parser (`src/agent_io/reader.rs:125-164`), marker scan 在 parser 后执行。
2. 风险: Unix socket streaming 缓冲区溢出。缓解: MVP 单 master client, bounded broadcast, 慢 client 断开并要求重新订阅。
3. 风险: Bash tool 10min 超时。缓解: `ah ask --wait` 支持 async + `ah pend <job_id>` 接力; server 侧 job 不因 client 断线取消。
4. 风险: fake provider marker 太理想化。缓解: fake marker 只锁协议; provider-aware fallback 另测真实 UI 文本。
5. 风险: B2 移除 `dispatch_and_complete_job` 造成 PR-1 回归。缓解: dogfood-1 同 PR 迁移 PR-1 cases 到真 marker path。
6. 风险: UDS subscribe streaming 改动 `src/rpc/mod.rs` 单 response 架构, 涉 frame protocol、断线检测、backpressure。缓解: MVP 单连接、newline-delimited frame、client 断即清, 不做 reconnect。

## §7 不在 scope

1. 真 LLM 交互: e2e 仅限 fake provider 协议层验证, 真 Claude/Gemini/Codex 留外部验收。
2. 多主控并发: 保持 master 与 ah daemon 1:1 物理绑定, 不处理多主控抢占。
3. Web UI/stdout 增强: 本设计专注 headless master client 闭环。
4. 跨机调度: 维持 localhost 调度边界, 不引入远程 transport。
