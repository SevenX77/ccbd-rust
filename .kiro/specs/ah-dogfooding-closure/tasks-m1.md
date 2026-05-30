# Tasks M1: ah dogfooding closure

## §1 PR scope 与度量目标

M1 scope: 将 design §4 的 `dogfood-1` + `dogfood-2` 合并为首个 ship PR。只做 B2 真 completion path 与 B1 UDS streaming/subscribe; B3 stuck 多信号、B4 slash、B5 health、B6 全量 dogfood 主测留后续。

M1 目标:

- B2: 删除 `tests/ah_full_e2e_main.rs:210-240` 的 `dispatch_and_complete_job` seam; fake provider 输出 `<<ah-idle:job-id=X>>`, 由 `src/agent_io/reader.rs:125-193` 真 reader/MarkerMatcher 驱动 `BUSY -> IDLE`。
- B1: 在现有 UDS JSON-RPC (`src/rpc/mod.rs:23-66`, `src/cli/rpc_client.rs:115-157`) 上新增 `event.subscribe`, 将 `src/orchestrator/pubsub.rs:4-28` in-process 通知推给 master client。
- master client: `src/bin/ah.rs:523-548` 的 `wait_for_job` 从循环 `job.wait(timeout=30)` 改为 subscribe 等终态事件。
- PR-1 regression: `tests/ah_full_e2e_main.rs` 迁移后继续通过, 不再测试直接写 DB 完成 job。

M1 度量:

- `cancel_counter == 0 && capture_counter == 0`。
- fake dispatch 5 RPC 模拟跑通, 无 timeout/error。
- `event.subscribe` 收到 `job_state_change(COMPLETED)` frame, 含 `event_id/kind/agent_id/job_id/state/ts_unix_micro`。
- 错误 job-id marker 不触发完成, 正确 job-id marker 才完成。

## §2 TDD 任务列表

### T1 fake dogfood provider script

文件: 新建 `tests/fixtures/mock_dogfood_provider.sh` (~80-120 LOC)。

依赖: 参考 `tests/fixtures/mock_provider.sh:4-12` 与 `tests/ah_full_e2e_realign_extra.rs:524-580`。

内容:

- 接收 dispatched message, 模拟 LLM 工作, 输出 provider 可见文本。
- 从消息或 env 解析/接收 job_id, 输出 `<<ah-idle:job-id=X>>`。
- 支持 `FAKE_PROVIDER_DELAY_MS` 注入延迟。
- claude/codex/gemini fake provider 默认共用同套脚本; provider 差异留后续。

验收: shell 直接运行能输出 ready prompt; 输入带 job_id 的 message 后 stdout 含 idle marker。

### T2 tests/ah_dogfooding.rs 测试基础设施

文件: 新建 `tests/ah_dogfooding.rs` (~150-250 LOC), `#[ignore]` include-ignored lane。

helper:

- `dogfood_ah_client`: 连接 `ahd.sock`, 发送/接收 JSON-RPC 与 streaming newline frame。
- `InterventionCounters`: 记录 master client 侧 cancel/capture。
- `dispatch_job_via_ah`: 经 master client submit + wait/subscribe, 不调用 seam。
- `install_mock_dogfood_provider`: 安装 T1 fixture 到 PATH。

验收: `cargo test --test ah_dogfooding -- --include-ignored --test-threads=1` 可编译; 实现前红灯必须落在缺 `event.subscribe` 或 marker completion。

### T3 红灯 tests

T3.1 `test_event_subscribe_pushes_idle_frame`

- 流程: spawn fake agent -> submit job -> subscribe `event.subscribe(job_id)` -> fake provider 输出 idle marker。
- 期望: 收到 `job_state_change(COMPLETED)` frame。
- 当前红灯: `src/rpc/router.rs:13-36` 未注册 `event.subscribe`。

T3.2 `test_real_completion_path_no_seam`

- 流程: 派发 job 后只等 fake marker, 不手工写 DB。
- 期望: agent 自然 `BUSY -> IDLE`, job `COMPLETED`。
- 当前红灯: PR-1 seam 仍在, marker job-id 对账未落。

T3.3 `test_zero_cancel_zero_capture_assertion`

- 流程: 跑 5 RPC 模拟: `session.create`, `agent.spawn`, `job.submit`, `event.subscribe`, `session.kill`。
- 期望: `cancel_counter == 0`, `capture_counter == 0`, 全程无 timeout/error。
- 当前红灯: `wait_for_job` 仍是 `job.wait` poll, subscribe 链路不存在。

T3.4 `test_pr1_regression_still_green`

- 流程: 迁移 `tests/ah_full_e2e_main.rs` 后跑 PR-1 grand tour。
- 期望: 旧生命周期仍绿, grep 不再出现 `dispatch_and_complete_job`。
- 当前红灯: 旧测试仍依赖 seam。

T3.5 `test_marker_job_id_对账`

- 流程: fake provider 先输出错误 job-id marker, 再输出正确 job-id marker。
- 期望: 错误 marker 不触发 `BUSY -> IDLE`; 正确 marker 才完成 job。
- 当前红灯: `MarkerMatcher` 还没有 job-id 对账语义。

### T4 src 实施

T4.1 `tests/ah_full_e2e_main.rs`: 删除 `dispatch_and_complete_job` 及 `output_chunk` seam 断言; 旧 cases 改为 fake dogfood provider 真 marker path。

T4.2 `src/rpc/router.rs:13-36,73-96`: 注册 `event.subscribe` whitelist + dispatch。

T4.3 `src/rpc/handlers.rs`: 新建 `handle_event_subscribe`, 支持 filter `{agent_id, job_id, event_kind}`, 先补发 DB filter 后事件, 再桥接 pubsub。

T4.4 `src/rpc/mod.rs:41-58`: 读到 `event.subscribe` 后进入 streaming writer, newline-delimited frame 持续写到 client 断开。

T4.5 `src/orchestrator/pubsub.rs:4-28`: 扩 typed `EventFrame` sender; 保留 `notify_job_update`/`notify_agent_output` 兼容旧 `job.wait`/`agent.watch`。

T4.6 `src/agent_io/reader.rs:136-193`: marker 命中后解析 job_id; 只允许当前 dispatched job 匹配时调用 `mark_agent_idle_matched`; mismatch 不转 IDLE。

T4.7 `src/bin/ah.rs:523-548`, `src/cli/rpc_client.rs:115-157`: `wait_for_job` 改为 `event.subscribe` + select 终态事件; client timeout 只断 wait, 不取消 job。

T4.8 PR-1 regression 同步: `tests/ah_full_e2e_main.rs` 旧 cases 改用 `mock_dogfood_provider` 和真 marker completion。

T4 验收:

- T3.1-T3.5 全绿。
- `cargo test --test ah_full_e2e_main -- --include-ignored --test-threads=1` 绿。
- grep 无 `dispatch_and_complete_job`; 普通 JSON-RPC method 行为不变。

### T5 a2 audit + a3 audit

- a2 audit: src/tests vs `design.md`/`tasks-m1.md`, 核 file:line、RPC schema、EventFrame 字段、job-id 对账。
- a3 audit: PM 替身 + drift check, 核 M1 没扩进 B3/B4/B5/B6。
- round-loop 直到 0 must-fix; nice-to-have 明确留后续 PR 或转任务。

### T6 docs 同步

文件:

- `docs/engine/dogfood-m1/logic-explained.md`。
- `mvp0-alignment.md` 若存在相关 ah dogfood 段则同步。

内容: 按 `[[logic-explained-is-code-translation]]` 字段级翻译 `event.subscribe`, `EventFrame`, marker job-id 对账, PR-1 seam cutover。

验收: a1/a3 audit docs 与 src 行为一致。

### T7 PR report

文件: `docs/reports/pr-dogfood-m1.md`。

内容: 背景、变更、测试、风险、后续 M2/M3; 明确 M1 指标 `cancel=0`, `capture=0`, 5 RPC 模拟跑通。

验收: report 可直接作为 PR 描述基础。

## §3 验收门槛

- `cargo test --test ah_dogfooding -- --include-ignored --test-threads=1` 全绿。
- `cargo test --test ah_full_e2e_main -- --include-ignored --test-threads=1` 全绿。
- `cargo test` 全 suite 全绿。
- M1 e2e assert: `cancel_counter == 0 && capture_counter == 0`。
- 5 RPC 模拟跑通无 timeout/error。
- grep verify: 无 `dispatch_and_complete_job`; 有 `event.subscribe`; 有 `<<ah-idle:job-id=`.

## §4 design.md §4 同步

- 删除 design §4 表中的 `dogfood-3 | B | 合并到 dogfood-2, 不单拆 | - | -` 行。
- 总量表述改为 `7 个独立 PR`, 避免 `8 PR` 与实际独立 PR 数歧义。
- M1 解释为首个 ship PR 合并 design 中 A1 + A2: B2 真 completion path + B1 UDS streaming/subscribe。
