# ah Dogfooding Closure Summary

## §1 User 立项目标

> 主控用 ah 派 a1/a2/a3 自驱跑一个完整 SOP-08 任务 (PR-6 体量), 整个过程不撞 ccb 痛点 (无 cancel stuck job / 无 loop poll 干等 / 无 capture pane 人工 verify). e2e 测试本身 dogfood: e2e 测试用 ah master client 替代 ccb ask 来 dispatch agents.

> 用户参与只有需求挖掘和目标敲定. 实施全是主控+agents 按 SOP-08 13 步自驱闭环 (research→design→impl→e2e→不闭合再回 research→闭合才请 user squash merge).

## §2 闭环成果

| PR | scope | 度量 |
|---|---|---|
| M1 (#28) | B2 真 completion + B1 UDS streaming | `event.subscribe` 替代 wait poll 基础; 0 cancel + 0 capture 子集 |
| M2 (#29) | B3 stuck 多信号 + STUCK push + C5 配置化 | + 0 poll + push p95 < 500ms + stuck < 310s |
| M3a (#30) | B4 slash command keystroke | + slash 投递 100% |
| M-final | B5 health 三层 + B6 真 stdout dogfood-8 | + 真 reader 端到端命门闭合 |

四段 PR 串起来后, ah dogfooding closure 从“研究设计”走到“测试锁定 + src 实施 + 终审 must-fix 修复”。

## §3 7 项度量全兑现

| 度量 | 状态 | 测试证据 |
|---|---|---|
| 主控 cancel = 0 | ✅ | `test_zero_cancel_zero_capture_assertion`, `test_dogfood_e2e_full_sop08_simulation` |
| 主控 capture-pane verify = 0 | ✅ | `InterventionCounters.capture_count() == 0` |
| 主控 ScheduleWakeup poll = 0 | ✅ | `test_zero_schedule_wakeup_poll`, full SOP08 `poll_count() == 0` |
| push p95 <= 500ms | ✅ | `test_push_latency_p95_under_500ms`, `test_push_latency_p95_real_stdout` |
| stuck escalate <= 310s | ✅ | `test_stuck_threshold_env_override`, `test_health_check_dead_layer_escalates_stuck` |
| slash 投递 100% | ✅ | `test_slash_command_keystroke_delivery`, full SOP08 slash ack |
| 5 RPC / SOP-08 模拟跑通 | ✅ | `test_dogfood_e2e_full_sop08_simulation` 单 session 连续链 |

验证命令:

```bash
cargo test --test ah_dogfooding -- --include-ignored --test-threads=1
cargo test --lib health_check -- --test-threads=1
cargo test --lib pane_diff
cargo test --test ah_full_e2e_main -- --include-ignored --test-threads=1
CCB_TEST_SKIP_REAL_PROVIDER=1 cargo test -- --test-threads=1
```

全部通过。

## §4 命门闭合声明

B6 是本 spec 的命门: 证明 ah 不撞 ccb completion 痛点, 必须验证真 stdout -> reader -> completion, 不能只写 DB。

a3 初审指出 dogfood-8 曾经有半 seam: `emit_stdout_marker_via_reader_path` 名字像 reader path, 实际仍 `insert_event(output_chunk)`。M-final must-fix 已删除该 helper, 当前 dogfood-8 使用:

1. `agent.spawn(provider:"bash")` 真起 tmux pane。
2. queued job prompt 执行 `mock_dogfood_provider.sh`。
3. mock provider 真 stdout 输出 `<<ah-idle:job-id=X>>`。
4. `agent_io::reader` 从 FIFO 捕获 stdout 并写 `output_chunk`。
5. marker hook + state_machine 完成 job。
6. `event.subscribe` 返回 `job_state_change(COMPLETED)` frame。

因此“ah 不撞 ccb completion 痛点”现在是真命题, 不是 test fake。a3 二次终审结论: PASS, 命门真闭合。

## §5 Scope 与边界

本 closure 验的是 daemon 侧生产链路和 master wait 模式:

- tmux pane / FIFO / reader 是真路径。
- events / state_machine / pubsub / event.subscribe 是真路径。
- stuck / slash / cancel 都在同一个 dogfood-8 session 里复验。

当前仍保留的工程边界:

- dogfood-8 用 in-process RPC harness, 不是真 `ah` CLI binary smoke。
- fake provider 是协议层 fake, 不是真 Claude/Codex/Gemini LLM。
- 部分 M2 `elapsed_secs` / `signal_kinds` fast path 仍可进一步精细化。

这些是 nice-to-have, 不阻塞 closure。

## §6 后续建议

1. 真 `ah` CLI binary smoke: 用编译后的 `ah start/ask --wait/stop` 跑一个小型 dogfood lane。
2. EventFrame 指标真值精细化: 所有 stuck source 的 `elapsed_secs` / `signal_kinds` 都从真实 watcher 传入。
3. 真 LLM e2e: 在有 OAuth/token 的外部环境跑非 CI 验收。

## §7 最终结论

ah dogfooding closure 已完成。

主控可以用 ah 的 event-driven wait、stuck push、slash keystroke、health check 和真 reader completion path, 自驱模拟 PR-6 体量 SOP-08 流程。核心痛点从人工介入变成可测试、可回归的 daemon 行为。
