# PR Report: feat(ah-dogfood): M-final — B5 health + B6 真 stdout dogfood-8 (闭环)

## §1 PR 标题与摘要

PR 标题: `feat(ah-dogfood): M-final — B5 health + B6 真 stdout dogfood-8 (闭环)`

本 PR 是 ah dogfooding closure 的最终收尾: B5 health 三层探测 + B6 dogfood-8 真 stdout 端到端。a3 二次终审指出的命门已补: dogfood-8 不再用 `insert_event` fake completion, 而是起真 tmux/bash agent, 让 mock provider stdout 经过 `agent_io::reader` 驱动 state_machine 和 `event.subscribe`。

## §2 背景与 scope

立项目标是主控用 ah 自驱跑 PR-6 体量 SOP-08 任务, 避免 ccb ask 模式里的 cancel stuck job、loop poll、capture-pane 人工 verify、completion 假阳性等痛点。

M-final scope:

- B5: `src/provider/health_check.rs` 三层健康检查: tmux pane、provider predicate、completion progress。
- B5: orchestrator 启动 health watcher, 与 M2 pane_diff 共用 stuck tick/threshold。
- B6: dogfood-8 主测升级成真 stdout -> reader -> events -> state_machine -> pubsub -> `event.subscribe`。
- must-fix: full SOP08 改成单 Harness/session 连续链。

不在 scope:
- 新增 CLI 子命令或 RPC schema。
- 真 LLM e2e。
- reconnect/backpressure 级别的 event stream 完整协议。

## §3 变更摘要

Commit 范围:

- `162833d test(ah-dogfood): M-final step 3 tests-first — B5 unit 4 + dogfood-8 红灯 3`
- `835f0ca feat(ah-dogfood): M-final step 4 — B5 health 三层 + B6 真 stdout dogfood-8`
- `88999df fix(ah-dogfood): M-final 补 2 must-fix — B6 真 stdout→reader 端到端 + full sop08 单 session`

主要变更:
- `src/provider/health_check.rs`: 新增 `HealthCheckResult`, `HealthCheckObservation`, `health_check_observe`, `escalate_health_stuck`, `health_check_watcher_loop`。
- `src/orchestrator/mod.rs`: 启动 health watcher, 与 pane_diff watcher 共用 `resolve_stuck_watch_config()`。
- `tests/fixtures/mock_dogfood_provider.sh`: 支持 delay/stuck/slash ack env。
- `tests/ah_dogfooding.rs`: 新增 dogfood-8 三测; must-fix 后改成真 agent.spawn + reader path。

必须修复项结果:
- `emit_stdout_marker_via_reader_path` 已删除, dogfood-8 不再直接 `insert_event` completion marker。
- `test_dogfood_e2e_full_sop08_simulation` 改为单 Harness/session: dispatch×5 -> stuck -> slash -> cancel。

## §4 测试与验证

```bash
cargo test --test ah_dogfooding -- --include-ignored --test-threads=1
```

结果: 13 passed。

覆盖:
- M1 5 个 completion/subscribe regression。
- M2 4 个 stuck/p95/poll regression。
- M3a 1 个 slash regression。
- M-final 3 个 dogfood-8 / real stdout / health check tests。

```bash
cargo test --lib health_check -- --test-threads=1
```

结果: 4 passed。

```bash
cargo test --lib pane_diff
```

结果: 16 passed。

```bash
cargo test --test ah_full_e2e_main -- --include-ignored --test-threads=1
```

结果: 4 passed。

```bash
CCB_TEST_SKIP_REAL_PROVIDER=1 cargo test -- --test-threads=1
```

结果: passed。

grep verify:
- `rg "emit_stdout_marker_via_reader_path" tests/ah_dogfooding.rs`: 无输出。
- `grep -n "insert_event" tests/ah_dogfooding.rs`: 只剩 M1/M2 旧测试段, dogfood-8 区域无 `insert_event` completion fake。
- `rg "spawn_real_dogfood_agent|run_real_stdout_job|MOCK_DOGFOOD_PROVIDER=bash" tests/ah_dogfooding.rs`: dogfood-8 真路径落点存在。

## §5 audit 结果

a3 初审: catch 2 must-fix。

- B6 半 seam 没真废: dogfood-8 使用 `insert_event` fake completion。
- full SOP08 是多个 Harness 片段拼接, 不是长 session 连续链。

修复后 a3 二次终审: PASS。结论: 命门真闭合, user 目标真兑现。

主控自审:

- dogfood-8 completion path 已走真 tmux/bash provider stdout。
- reader 真实插入 `output_chunk`。
- state_machine 真实完成 job。
- `event.subscribe` 真实返回 terminal frame。
- 单 session 连续链覆盖 5 dispatch + stuck + slash + cancel + 0 介入。

## §6 已知限制与后续

已兑现:
- 0 cancel / 0 capture / 0 poll。
- push p95 <= 500ms。
- stuck escalate <= 310s。
- slash command 100%。
- 真 stdout reader 端到端 completion。

限制:
- dogfood-8 使用 in-process RPC harness, 不是编译后的 `ah` CLI binary smoke。
- fake provider 是协议层 fake, 不是真 LLM。
- `elapsed_secs` / `signal_kinds` 在部分 M2 state_machine fast path 仍有占位, 不影响命门 closure。

后续 nice-to-have:

- 真 `ah` binary smoke。
- EventFrame 指标真值精细化。
- 外部环境真 LLM dogfood。

## §7 风险 / breaking

无 user-facing BREAKING。

- RPC/CLI/schema 继承不变。
- M1/M2/M3a 既有测试全绿。
- 新 health watcher 只对 active agents 生效, terminal/idle 不触发。
- dogfood-8 使用 bash provider 避开 Claude prompt handler 干扰, 但 stdout/reader/state_machine 链路是生产路径。
