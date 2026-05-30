# ah Full E2E PR-3 Report

## §1 PR-3 范围与对齐目标

PR-3 承接 PR-2 DRIFT + NEW 主线之后的异常生命周期分支，覆盖 ORPHAN + BUSY + ERROR。落地对象是 6-case matrix：ORPHAN audit-only、ORPHAN force cleanup、BUSY skip、BUSY force realign、ERROR crash detection、ERROR recovery known-gap。

PR-3 明确不做 PR-4 src fix，不修复 CRASHED agent 的恢复路径；也不覆盖 master cmd drift、session-level aggregate status 等 PR-5+ future scope。该 PR 是 test-only，目标是把当前真实行为锁住，并把已知 gap 用可审计的 JSON-RPC error 断言记录下来。

## §2 测试拓扑

新增 1 个 ignored 测试：`grand_tour_realign_extra_matrix`，位于 `tests/ah_full_e2e_realign_extra.rs`。测试复用 PR-1/PR-2 风格的 in-process RPC harness：temp DB、temp state dir、temp project dir、隔离 tmux server、`dispatch` JSON-RPC helper。

串接顺序是固定长链路，不重置 DB / state / tmux：

1. baseline setup：`session.create` + `session.spawn_master_pane` + `agent.spawn`
2. `case_06_orphan_audit_only`
3. `case_07_orphan_force_cleanup`
4. `case_08_busy_skip`
5. `case_09_busy_force_realign`
6. `case_10_error_crash_detection`
7. `case_11_error_recovery_known_gap`

测试用 fake `claude` binary 注入 temp `PATH`，支持 `ECHO` / `BUSY` / `CRASH` 三种 mock behavior。`BUSY` 保持 pane 存活以验证 skip/force 分支；`CRASH` 先输出 ready marker 再退出，以验证 provider 启动成功后的自然崩溃监控路径。

## §3 实施摘要

- ORPHAN audit-only：删除 `a2` config block 后以非 force realign，断 `status=ORPHAN`、`action=audit_only`、agent 仍 IDLE、PID/pane 不变、无 kill event。
- ORPHAN force cleanup：同一 orphan 以 force realign，断 `status=ORPHAN`、`action=KILLED`、DB row 保留为 KILLED、旧 PID 和 pane 消失、事件 reason 为 `ORPHAN_FORCE_CLEANUP`。
- BUSY skip：通过 `job.submit` + 真 pane prompt 把 `a1` 推到 BUSY，再提交 drift config，断 `status=SKIPPED_BUSY`、`drift_skipped` 事件、PID/pane/hash 不变。
- BUSY force realign：对 BUSY agent 加 force，断 `status=REALIGNED`、新 PID/pane/hash、旧 PID/pane 消失、spawn event reason 为 `DRIFT_REALIGN`、hook symlink 物化正确。
- ERROR crash detection：新增 `a_crash` agent，mock provider ready 后 `sleep 1` 再 `exit 1`，断 realign 返回 `status=NEW`，随后 agent 进入 CRASHED，`last_error_code=AGENT_UNEXPECTED_EXIT`，pane cleanup 完成。
- ERROR recovery known-gap：对 CRASHED `a_crash` 再 realign，断 JSON-RPC error object，`error.data.error_code=AGENT_ALREADY_EXISTS`，且不返回误导性的 `result.statuses[]` success。

## §4 物理断言风格

PR-3 延续 PR-1/PR-2 的“物理断言”风格，不只看 RPC 文本：

- `assert_symlink_target` 使用 `std::fs::read_link`，验证 BUSY force 后 `.claude/hooks/pr3-busy-skip.sh` 的真实 symlink target。
- `.claude/CLAUDE.md` 通过 `assert_sandbox_file` 验证 provider HOME 物化，不只相信 spawn status。
- `wait_for_tmux_pane_gone` 和 PID retry 验证 ORPHAN force、BUSY force、ERROR crash 后的真实 tmux/process cleanup。
- BUSY seam 使用 `state_machine::transit_agent_state` 真 DB state-machine API，reason 固定为 `DISPATCH_ACK_STABLE`，不是裸 SQL update。
- `rpc_raw` 捕获完整 JSON-RPC error object，case_11 断 `error.data.error_code`，避免把 known-gap 包装成成功路径。

## §5 关键发现 / 设计偏离

- case_10 mock CRASH 从 design §2 的“立即 exit 1”调整为 `ready -> sleep 1 -> exit 1`。这是工程必需：立即退出会让 `init_probe` / pidfd monitor attach 还没完成就失败，`session.realign` 无法返回 `statuses[a_crash].status=NEW`。新实现仍是真 crash，只是模拟 provider 启动成功后崩溃。
- case_09 断持久化 spawn event reason 为 `DRIFT_REALIGN`，不是 `DRIFT_FORCE_REALIGN`。真实路径会 delete/reinsert agent；`schema.rs:37` 中 `events.agent_id` 的 `ON DELETE CASCADE` 会清掉旧 row 的 kill event，新 row 可审计事件是 `handlers.rs:623` 附近的 spawn event。
- case_08 BUSY 使用 `transit_agent_state` seam。orchestrator 后台 ACK 稳定窗口 task 在 in-process harness 中不易确定性触发，因此测试手动推进状态；但前置路径是真 `job.submit`、真 `dispatch_job_to_agent`、真 `send_text_to_pane`，BUSY 是任务驱动态，不是 stub bypass。
- case_11 记录 ERROR recovery known-gap：`running_agent_hashes` 排除 CRASHED 节点，随后 `handle_agent_spawn` 命中 existing agent 并返回 `AGENT_ALREADY_EXISTS`。这是真实 src 行为，PR-4 负责修复恢复路径。

## §6 验证

- Host verify 3 次 stable PASS：`CARGO_BUILD_JOBS=1 cargo test --test ah_full_e2e_realign_extra grand_tour_realign_extra_matrix -- --include-ignored --test-threads=1 --nocapture`，完成时间约 8.7s。
- Default lane：`CARGO_BUILD_JOBS=1 cargo test --test ah_full_e2e_realign_extra -- --test-threads=1` -> 3 passed, 1 ignored。
- Commit 序列建议：PR-3 spec lock -> T1 red skeleton -> T2/T3 harness+fixtures -> T4/T5 ORPHAN -> T6/T7 BUSY -> T8/T9/T10 ERROR + report。
- a2 audit focus：ORPHAN audit-only/force 的状态副作用、BUSY skip/force 的 PID/pane/hash 断言、ERROR known-gap 是否锚定 JSON-RPC error。
- a3 audit focus：read_link 物理断言、tmux pane gone retry、case_08 BUSY seam 透明度、case_10 ready-before-crash timing 稳定性。

## §7 PR-4 src fix + PR-5 future scope

PR-4 应聚焦 ERROR recovery src fix：允许 CRASHED agent 在 realign 中走恢复/替换路径，而不是在 `handle_agent_spawn` 上返回 `AGENT_ALREADY_EXISTS`。PR-3 的 case_11 保留当前 known-gap 断言，给 PR-4 提供红绿目标。

PR-5+ future scope 包括 master cmd drift、session-level aggregate status、以及更完整的异常 evidence/prompt 链路。这些能力不进入 PR-3，避免 6-case ORPHAN/BUSY/ERROR matrix 和后续 src 行为修复混在一个测试 PR 中。
