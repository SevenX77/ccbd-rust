# Design: ah 全流程 E2E Grand Tour PR-3 (ORPHAN + BUSY + ERROR)

## §0.5 继承字段表

| 项 | 继承/使用方式 | grep 实证 |
|---|---|---|
| `session.realign.force` | ORPHAN force cleanup 与 BUSY force realign 共用 RPC flag | `src/rpc/handlers.rs:360-362` |
| `RealignAgentParams.env/hooks/plugins` | ah.toml agent 配置进入 realign payload 与指纹计算 | `src/rpc/handlers.rs:340-349`, `:446-452` |
| `agents[i].env -> extra_env_vars` | mock provider 行为变量经 realign spawn 传入进程环境 | `src/rpc/handlers.rs:589-595`, `:676-725` |
| `BUSY` 分支 | `SKIPPED_BUSY` 与 `DRIFT_FORCE_REALIGN` 的真实判别点 | `src/rpc/handlers.rs:475-497` |
| `CRASHED` 持久化 | provider 退出后记录 `state=CRASHED`, `exit_code`, `error_code` | `src/monitor/agent_watch.rs:68-72`, `src/db/agents_lifecycle.rs:57-85` |
| `AGENT_ALREADY_EXISTS` | CRASHED recovery known-gap 的 RPC error code | `src/rpc/handlers.rs:684-685`, `src/error.rs:14-15`, `:64-72` |
| PR-2 Harness helper | 复用事件/FS/symlink 物理断言 | `tests/ah_full_e2e_drift.rs:197-224` |

## §1 第一性原理

PR-3 验证的是响应正确性，而不是 PR-2 的功能正确性。系统已经能在 DRIFT/NEW happy path 中完成重生和扩容；PR-3 关注异常边界：拓扑收缩时 ORPHAN 如何处置，活跃任务与配置调谐冲突时 BUSY 如何跳过或强制剥夺，provider 崩溃后 ERROR/CRASHED 如何被记录，以及已知 recovery gap 如何被可复现地锁住。

## §2 核心机制思路

- **单长 Grand Tour 串接 6 case**: 延续 PR-2 `case_01` 到 `case_05` 的顺序风格 (`tests/ah_full_e2e_drift.rs:623/674/744/823/861`)，新文件承接 `case_06` 到 `case_11`。前序终态作为后序始态，模拟长期运行中的拓扑收缩、忙碌调谐、强制调谐与崩溃记录。
- **Mock provider 多模式**: 扩展 PR-2 `install_fake_claude`。PR-2 fake 已输出 Claude ready marker `status Sonnet ... ❯` (`tests/ah_full_e2e_drift.rs:556-566`)，而 Claude init probe 要求 prompt 与模型标记同时存在 (`src/provider/init_probe.rs:24-31`, `:246-250`)。
- **BUSY 模式顺序**: 不能启动即 `sleep infinity`。正确顺序是：1. mock 先输出 `status Sonnet ... ❯`；2. init probe 命中后 agent 到 IDLE；3. `job.submit` 只写入 QUEUED (`src/rpc/handlers.rs:902-932`)；4. orchestrator dispatch 将 IDLE 转为 WAITING_FOR_ACK 并发送 prompt (`src/orchestrator/mod.rs:43-50`, `:84-100`; `src/db/jobs.rs:212-218`)；5. ACK 稳定窗口后转 BUSY (`src/orchestrator/mod.rs:152-164`)，mock 收到 stdin 后再 sleep，随后触发 `session.realign` 命中 `SKIPPED_BUSY` 或 force realign (`src/rpc/handlers.rs:475-497`)。
- **CRASH 模式**: mock 以非 0 退出触发 pidfd monitor，`mark_agent_crashed_with_exit` 写入 `CRASHED`、`exit_code` 与默认 `AGENT_UNEXPECTED_EXIT` (`src/monitor/agent_watch.rs:68-72`, `src/db/agents_lifecycle.rs:57-85`)。

## §3 关键决策

- **D1: Mixed Drift under BUSY**: `case_08` 使用 ENV + HOOKS 混合漂移，验证 `drift_skipped` payload 至少保留真实 reason/state，而不是只看最终 RPC status。当前 `drift_reason` 是优先级型单 reason (`src/rpc/handlers.rs:656-667`)，测试不应假设多 reason 数组。
- **D2: per-agent control 可选, 不实施**: 虽然 `agents[i].env` 到 `extra_env_vars` 的 wire 已通 (`src/rpc/handlers.rs:589-595`, `:676-725`)，PR-3 不做同 session 多 agent 不同行为并发矩阵。优先单 agent 顺序方案：a1 进入 BUSY -> skip -> force realign，减少调度竞态与测试复杂度。
- **D3: ERROR Recovery known-gap 断言形式**: `case_11_error_recovery_known_gap` 断 JSON-RPC error，不断 `statuses[i].status`。CRASHED agent 被 `running_agent_hashes` 排除后走 NEW spawn 路径，`handle_agent_spawn` 的 `agent_exists` 直接返回 `AgentAlreadyExists` (`src/rpc/handlers.rs:629-637`, `:684-685`)；router 将 Err 包成 JSON-RPC `error` (`src/rpc/router.rs:99-122`, `:134-139`)；最终断 `error.data.error_code == "AGENT_ALREADY_EXISTS"` (`src/error.rs:14-15`, `:64-72`)。该 case 保持 `#[ignore]`，作为 PR-4 修复前的 documents-gap 红灯。

## §4 测试矩阵 (6 Cases + 4 维联合断言)

每个正向 case 必须覆盖 RPC Result、SQLite 状态/事件、OS/Tmux 进程或 pane、FS Sandbox/Provider 物理副作用中适用的四维断言。

1. `case_06_orphan_audit_only`: 去掉 ah.toml 中 a2，`force=false`；断 RPC 返回 ORPHAN，DB 不删除、不 KILL，tmux pane 仍存在。
2. `case_07_orphan_force_cleanup`: 同一 ORPHAN 场景 `force=true`；断 `ORPHAN_FORCE_CLEANUP`、state 变 KILLED、tmux pane 消失、agent_io runtime 清理。注意 DB 行保留为 KILLED，不物理删除 (`src/db/agents_lifecycle.rs:50-51`)。
3. `case_08_busy_skip`: a1 经真 job 驱动进入 BUSY，改 ENV+HOOKS 后 `force=false`；断 `SKIPPED_BUSY`、`drift_skipped` 事件、pid/pane/hash 不变。
4. `case_09_busy_force_realign`: 沿用 BUSY 漂移态，`force=true`；断 `DRIFT_FORCE_REALIGN` kill + delete + respawn (`src/rpc/handlers.rs:493-505`)，新 pid/pane/hash，旧 pid 消失。
5. `case_10_error_crash_detection`: mock `GRAND_TOUR_MOCK_BEHAVIOR=CRASH`；断 provider 非 0 exit 被 monitor 记录为 `CRASHED`，`error_code=AGENT_UNEXPECTED_EXIT`，`exit_code` 落 DB，runtime registry 清理。
6. `case_11_error_recovery_known_gap`: 对 CRASHED 同 id 触发 realign recovery；断 JSON-RPC error `AGENT_ALREADY_EXISTS`，不产生误导性的 `statuses[]` 成功项，保持 `#[ignore]`。

## §5 物理断言风格细化

- **Tmux/OS**: `wait_for_tmux_pane_gone(agent_id)` 不能只查 DB state；必须查 pane/session 或 pid 消亡，覆盖 ORPHAN force 与 BUSY force 的物理收割。
- **SQLite**: 新增 `update_agent_state_direct` 只作为 CRASHED 兜底，不作为 BUSY 主路径；BUSY 主路径必须经 `job.submit -> dispatch -> ACK -> BUSY`。
- **FS Sandbox**: 复用 PR-2 `assert_sandbox_file` / `assert_symlink_target` 形态 (`tests/ah_full_e2e_drift.rs:211-224`)；BUSY mixed drift 可以继续验证 hooks symlink 在 force realign 后落到 sandbox。
- **RPC**: per-agent status 只用于 ORPHAN/BUSY success result；case_11 必须走 JSON-RPC `error` 对象。

## §6 实施切片与工程量

- **文件命名**: 新建 `tests/ah_full_e2e_realign_extra.rs`，单文件承载 ORPHAN + BUSY + ERROR 6 case，避免扩大 PR-2 文件。
- **Harness 扩展**: 约 100-130 LOC，包括 `wait_for_tmux_pane_gone`、`update_agent_state_direct`、`query_agent_last_error`、mock behavior env helper。
- **Mock 扩展**: 约 40-60 LOC，支持 `GRAND_TOUR_MOCK_BEHAVIOR=BUSY|CRASH`；BUSY 必须先 ready 后 stdin sleep。
- **Case 实施**: 约 450-520 LOC。总计预计 600-750 LOC，比 research 的 550 LOC 略高，因为拆出 `case_10_error_crash_detection` 并保留 `case_11` known-gap。

## §7 决议汇总 (Finalized)

- **CI Lane**: `#[ignore]` Grand Tour lane；本地命令使用 `CARGO_BUILD_JOBS=1 cargo test --test ah_full_e2e_realign_extra -- --include-ignored --test-threads=1`。
- **范围锁定**: PR-3 只覆盖 ORPHAN、BUSY、ERROR crash detection、ERROR recovery known-gap。不修 PR-4 recovery src，不处理 master cmd drift，不处理 session-level 聚合 status；这些留 PR-5+。
- **关键吸收项**: M1 已拆 6 case；M2 已规定 BUSY ready -> stdin sleep；N1 已降级 per-agent control 为可选不实施；N2 已写死 case_11 JSON-RPC error code `AGENT_ALREADY_EXISTS`。
