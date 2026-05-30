# Tasks: ah 全流程 E2E Grand Tour PR-3 (ORPHAN + BUSY + ERROR)

## Scope Guard

PR-3 只落地 ORPHAN + BUSY + ERROR 分支矩阵，新增单文件 `tests/ah_full_e2e_realign_extra.rs` 承载 6 case。

- ORPHAN 覆盖：audit-only 与 force cleanup。
- BUSY 覆盖：真 job 驱动进入 BUSY 后的 `SKIPPED_BUSY` 与 `DRIFT_FORCE_REALIGN`。
- ERROR 覆盖：provider crash detection 正向路径，以及 CRASHED recovery known-gap。
- 不覆盖 master cmd drift，不覆盖 session-level 聚合 status；这两类留 PR-5+。
- 不修 PR-4 recovery src；`case_11_error_recovery_known_gap` 只文档化现状。
- 复用 PR-2 Harness 形态：temp DB、temp state dir、temp project dir、隔离 tmux server、`dispatch` RPC helper、DB query helper。
- 新增 Harness helper：`wait_for_tmux_pane_gone`、`update_agent_state_direct`、`query_agent_last_error`。
- Mock provider 多模式：`GRAND_TOUR_MOCK_BEHAVIOR=BUSY|CRASH`；BUSY 必须先 ready，再收 stdin 后 sleep。
- 物理断言继承 PR-2 `assert_sandbox_file` / `assert_symlink_target` / `query_agent_events` pattern (`tests/ah_full_e2e_drift.rs:197-224`)。
- 真实分支依据：ORPHAN `src/rpc/handlers.rs:523-555`，BUSY `src/rpc/handlers.rs:475-497`，CRASHED `src/db/agents_lifecycle.rs:57-85`。

## 主线矩阵

1. `case_06_orphan_audit_only`: 删除 a2 config -> `session.realign(force=false)` -> RPC ORPHAN -> DB/tmux 不动。
2. `case_07_orphan_force_cleanup`: 同一 ORPHAN -> `session.realign(force=true)` -> state=KILLED -> tmux pane 消失 -> DB 行保留。
3. `case_08_busy_skip`: a1 真 job -> BUSY -> mixed ENV+HOOKS drift -> `SKIPPED_BUSY` -> `drift_skipped` -> pid/pane/hash 不变。
4. `case_09_busy_force_realign`: 沿用 BUSY drift -> force realign -> `DRIFT_FORCE_REALIGN` -> kill/delete/spawn -> 新 pid/pane/hash。
5. `case_10_error_crash_detection`: mock CRASH -> pidfd monitor -> `state=CRASHED` + `error_code=AGENT_UNEXPECTED_EXIT` + `exit_code`。
6. `case_11_error_recovery_known_gap`: CRASHED 同 id realign recovery -> JSON-RPC error `AGENT_ALREADY_EXISTS`，保持 `#[ignore]`。

## T1: Add ignored PR-3 realign extra matrix red-light skeleton

- 文件: Add `tests/ah_full_e2e_realign_extra.rs`
- 依赖: locked `.kiro/specs/ah-full-e2e/pr3-design.md`
- 内容:
  - 新增 `mod common;`
  - 新增 `#[tokio::test(flavor = "multi_thread")] #[ignore] async fn grand_tour_realign_extra_matrix()`
  - 先只 `panic!("red: PR-3 ORPHAN + BUSY + ERROR matrix not implemented")`
  - 文件内预留 case 函数名注释：`case_06_orphan_audit_only` 到 `case_11_error_recovery_known_gap`
  - 顶部 scope 注释明确不覆盖 master cmd drift / session 聚合 status / PR-4 recovery src fix。
- 验收标准:
  - 命令: `CARGO_BUILD_JOBS=1 cargo test --test ah_full_e2e_realign_extra -- --test-threads=1`
  - 绿灯信号: 默认 lane 显示 ignored，不执行主测试。
  - 命令: `CARGO_BUILD_JOBS=1 cargo test --test ah_full_e2e_realign_extra -- --include-ignored --test-threads=1`
  - 红灯信号: test 名出现，失败信息包含 `PR-3 ORPHAN + BUSY + ERROR matrix not implemented`。
- audit 视角:
  - a2 检查 `#[ignore]` 存在，PR-3 不污染默认 cargo test。
  - a3 检查 T1 只建红灯骨架，不提前塞半成品断言。

## T2: Port PR-2 Harness and add PR-3 lifecycle helpers

- 文件: Modify `tests/ah_full_e2e_realign_extra.rs`
- 依赖: T1
- 内容:
  - 复用 PR-2 `Harness` 结构：`Ctx`、`TmuxServerGuard`、temp DB、temp state dir、temp project dir。
  - 复用 `rpc(method, params)`：通过 `ccbd::rpc::router::dispatch` 发送 JSON-RPC；success helper 遇到 `error` 直接 panic。
  - 新增 `rpc_raw(method, params)`：保留 JSON-RPC response，用于 `case_11` 断 error object。
  - 复用 DB helpers：`query_agent_state`、`query_agent_pid`、`query_agent_config_hash`、`query_agent_pane_id`、`query_agent_events`。
  - 新增 `wait_for_tmux_pane_gone(agent_id)`：通过 pane/session 或 pid 轮询，验证物理收割。
  - 新增 `update_agent_state_direct(agent_id, state)`：仅用于 CRASHED recovery known-gap 兜底；不得用于 BUSY 主路径。
  - 新增 `query_agent_last_error(agent_id) -> (Option<String>, Option<i64>)`：读取 `agents.error_code` 与 `agents.exit_code`。
  - 保留 PID race retry pattern：30x100ms 或等价 `wait_until(Duration::from_secs(3))`。
- 验收标准:
  - 命令: `CARGO_BUILD_JOBS=1 cargo test --test ah_full_e2e_realign_extra -- --include-ignored --test-threads=1`
  - 红灯信号: Harness 编译通过，仍停在未实现 case。
  - 绿灯目标: 可以启动 baseline，断 agent session 存在、sandbox dir exists、`agents.state=IDLE`。
- audit 视角:
  - a2 检查 helper 查询真实 schema：`events.event_type` / `events.payload` / `agents.error_code` / `agents.exit_code`。
  - a3 检查 `wait_for_tmux_pane_gone` 是物理断言，不只看 DB state。

## T3: Add fixture builders and multi-mode fake Claude

- 文件: Modify `tests/ah_full_e2e_realign_extra.rs`
- 依赖: T2
- 内容:
  - 新增 `build_realign_extra_ah_toml(project_dir, agents)`：可生成 a1/a2 agent block，并支持删 a2 形成 ORPHAN。
  - 新增 `realign_payload(session_id, master_spec, agents)`：显式构造 `session.realign` payload，包含 `env`、`hooks`、`plugins`。
  - 新增 `run_realign(h, payload, force)`：调用 `session.realign` 并返回 `statuses`。
  - 扩展 `install_fake_claude` 支持 `GRAND_TOUR_MOCK_BEHAVIOR`：
    - 默认/echo：PR-2 行为，ready marker 后 echo 输入并再次输出 prompt。
    - BUSY：先输出 `status Sonnet ... ❯`，等待 stdin；收到 job prompt 后进入 `while true; do sleep 1; done`。
    - CRASH：按 case 需要以非 0 退出，触发 pidfd monitor。
  - BUSY 机制 file:line 注释：`job.submit` 只返回 QUEUED (`src/rpc/handlers.rs:902-932`)，dispatch 发送 prompt (`src/orchestrator/mod.rs:43-50`, `:84-100`)，ACK 后 BUSY (`src/orchestrator/mod.rs:152-164`)。
- 验收标准:
  - 命令: `CARGO_BUILD_JOBS=1 cargo test --test ah_full_e2e_realign_extra -- --include-ignored --test-threads=1`
  - 红灯信号: fixture/mock 编译通过，但 case 尚未完整实现。
  - 绿灯目标: mock 默认模式能到 IDLE；BUSY mode 不会在 init 阶段卡 SPAWNING。
- audit 视角:
  - a2 检查 BUSY mode 顺序是 ready -> stdin -> sleep，不是启动即 sleep。
  - a3 检查 mock 不访问网络，不依赖真实 Claude。

## T4: Case 06 ORPHAN audit-only

- 文件: Modify `tests/ah_full_e2e_realign_extra.rs`
- 依赖: T3
- 内容:
  - baseline 启动 a1 + a2，记录 a2 state、pid、pane、config_hash。
  - 修改 ah.toml / realign payload，只保留 a1，删除 a2。
  - 调 `session.realign(force=false)`。
  - 在 `statuses[]` 中按 `agent_id == "a2"` 找 per-agent entry，断 `status == "ORPHAN"`。
  - 断 a2 DB 行仍存在且 state 不变；pid/pane 仍存在；无 `agent_killed` reason=`ORPHAN_FORCE_CLEANUP`。
  - PID/pane 检查使用 retry，避免读取瞬时 tmux 状态。
- 验收标准:
  - 命令: `CARGO_BUILD_JOBS=1 cargo test --test ah_full_e2e_realign_extra -- --include-ignored --test-threads=1`
  - 红灯信号: `force=false` 误杀 a2、DB state 改为 KILLED、或 RPC 无 ORPHAN。
  - 绿灯信号: ORPHAN audit-only 无副作用。
- audit 视角:
  - a2 检查 ORPHAN 路径使用 `src/rpc/handlers.rs:523-555` 的 per-agent status。
  - a3 检查不把 master/session aggregate status 当作 agent status。

## T5: Case 07 ORPHAN force cleanup

- 文件: Modify `tests/ah_full_e2e_realign_extra.rs`
- 依赖: T4
- 内容:
  - 沿用 T4 的 a2 ORPHAN 状态，调用 `session.realign(force=true)`。
  - 在 `statuses[]` 中找 a2，断 `status == "ORPHAN"` 且 `action == "KILLED"`。
  - 断 a2 state 变 `KILLED`；DB 行保留，不断物理删除。
  - 断存在 `agent_killed` event，payload reason=`ORPHAN_FORCE_CLEANUP`。
  - 用 `wait_for_tmux_pane_gone("a2")` 断 tmux pane/session 物理消失。
  - 说明 cleanup 来自 `mark_agent_killed` 后的 `cleanup_agent_runtime_resources` (`src/db/agents_lifecycle.rs:50-51`)。
- 验收标准:
  - 命令: `CARGO_BUILD_JOBS=1 cargo test --test ah_full_e2e_realign_extra -- --include-ignored --test-threads=1`
  - 红灯信号: state 未 KILLED、pane 未消失、或错误断言 DB 行删除。
  - 绿灯信号: ORPHAN force cleanup 通过 DB + event + tmux 三层断言。
- audit 视角:
  - a2 检查 reason 字符串为 `ORPHAN_FORCE_CLEANUP` (`src/rpc/handlers.rs:528-539`)。
  - a3 检查明确 DB row retained，避免回退到已否定的“DB 删除”假设。

## T6: Case 08 BUSY skip

- 文件: Modify `tests/ah_full_e2e_realign_extra.rs`
- 依赖: T5
- 内容:
  - 确保 a1 为 IDLE，记录 old_pid、old_pane、old_hash。
  - 通过 `job.submit` 给 a1 派发 prompt，使 orchestrator 真 dispatch 到 pane。
  - mock BUSY mode 收 stdin 后 sleep；等待 DB state 到 `BUSY`。
  - 构造 mixed ENV+HOOKS drift，并调 `session.realign(force=false)`。
  - 在 `statuses[]` 中找 a1，断 `status == "SKIPPED_BUSY"`。
  - 断新增 `drift_skipped` event，payload 至少包含 `state == "BUSY"` 和 reason。
  - 断 pid/pane/hash 不变，旧进程仍存活。
- 验收标准:
  - 命令: `CARGO_BUILD_JOBS=1 cargo test --test ah_full_e2e_realign_extra -- --include-ignored --test-threads=1`
  - 红灯信号: BUSY 是 SQL stub 注入、未出现 `drift_skipped`、或 `force=false` 仍 kill/spawn。
  - 绿灯信号: 真 Job 驱动 BUSY 后 skip 分支稳定通过。
- audit 视角:
  - a2 检查 BUSY 主路径不能用 `update_agent_state_direct`。
  - a3 检查 `drift_reason` 是单 reason (`src/rpc/handlers.rs:656-667`)，不要断多 reason 数组。

## T7: Case 09 BUSY force realign

- 文件: Modify `tests/ah_full_e2e_realign_extra.rs`
- 依赖: T6
- 内容:
  - 沿用 T6 的 BUSY + mixed drift 始态，记录 old_pid、old_pane、old_hash。
  - 调 `session.realign(force=true)`。
  - 在 `statuses[]` 中找 a1，断 `status == "REALIGNED"`。
  - 断 `agent_killed` event 包含 `DRIFT_FORCE_REALIGN`，对应 `src/rpc/handlers.rs:493-505`。
  - 等新 a1 回到 IDLE；断 new_pid/new_pane/new_hash 与旧值不同。
  - 用 retry 断 old_pid 或 old_pane 消失；用 FS 断 mixed hooks symlink 在 sandbox 重新物化。
- 验收标准:
  - 命令: `CARGO_BUILD_JOBS=1 cargo test --test ah_full_e2e_realign_extra -- --include-ignored --test-threads=1`
  - 红灯信号: force 未重生、reason 不是 `DRIFT_FORCE_REALIGN`、旧 pane 未消失、或 hash 未更新。
  - 绿灯信号: BUSY force realign 完成 kill/delete/spawn，并且新 agent 可用。
- audit 视角:
  - a2 检查 force 分支不是 `SKIPPED_BUSY`，而是 destructive realign。
  - a3 检查 PID race retry pattern 保留，不做一次性 `/proc` 断言。

## T8: Case 10 ERROR crash detection

- 文件: Modify `tests/ah_full_e2e_realign_extra.rs`
- 依赖: T7
- 内容:
  - 使用 mock `GRAND_TOUR_MOCK_BEHAVIOR=CRASH` 启动一个新 agent，例如 `a_crash`。
  - CRASH mode 必须真进程非 0 退出；主路径不得使用 `update_agent_state_direct` 设 CRASHED。
  - 等 pidfd monitor 写入 DB，断 `state == "CRASHED"`。
  - 用 `query_agent_last_error` 断 `error_code == Some("AGENT_UNEXPECTED_EXIT")`，`exit_code` 非空或符合实际非 0 exit。
  - 断 runtime registry/pane 清理；可用 pane_id 不存在或 `wait_for_tmux_pane_gone`。
  - 记录该 case 已验证自然 crash path，T9 才允许 SQL 兜底构造 CRASHED 始态。
- 验收标准:
  - 命令: `CARGO_BUILD_JOBS=1 cargo test --test ah_full_e2e_realign_extra -- --include-ignored --test-threads=1`
  - 红灯信号: 只用 SQL stub、state 未 CRASHED、error_code 未落 DB、或 runtime 未清理。
  - 绿灯信号: provider crash detection 通过真实进程退出链路。
- audit 视角:
  - a2 检查 `src/monitor/agent_watch.rs:68-72` 和 `src/db/agents_lifecycle.rs:57-85` 被真实覆盖。
  - a3 检查 CRASH mode 不输出 misleading ready 后马上被当作 IDLE 成功。

## T9: Case 11 ERROR recovery known-gap

- 文件: Modify `tests/ah_full_e2e_realign_extra.rs`
- 依赖: T8
- 内容:
  - 新增 `#[ignore]` case 或在 grand tour 内明确标注 known-gap 子段；注释写明 “documents known gap, PR-4 src fix”。
  - 对 CRASHED 同 id 构造 realign recovery。优先复用 T8 的 CRASHED agent；如生命周期不稳定，允许 `update_agent_state_direct(agent_id, "CRASHED")` 兜底。
  - 调 `rpc_raw("session.realign", payload)`，不得使用 success-only `rpc` helper。
  - 断 response 是 JSON-RPC `error` object，不是 `result.statuses[]`。
  - 断 `error.data.error_code == "AGENT_ALREADY_EXISTS"`。
  - 断错误来源：`running_agent_hashes` 排除 CRASHED (`src/rpc/handlers.rs:629-637`) 后走 NEW spawn，`handle_agent_spawn` agent_exists 报错 (`src/rpc/handlers.rs:684-685`)。
- 验收标准:
  - 命令: `CARGO_BUILD_JOBS=1 cargo test --test ah_full_e2e_realign_extra -- --include-ignored --test-threads=1`
  - 红灯信号: 错误地断 `statuses[].status`、未断 `AGENT_ALREADY_EXISTS`、或 case 未标 known-gap ignore。
  - 绿灯信号: known-gap 可复现，且不会被误报为 recovery 成功。
- audit 视角:
  - a2 检查 RPC error 包装来自 `src/rpc/router.rs:99-122`, `:134-139`。
  - a3 检查 error code 来自 `src/error.rs:14-15`, `:64-72`，不是字符串猜测。

## T10: Local run notes and final grep guard

- 文件: Modify `tests/ah_full_e2e_realign_extra.rs` comments only if needed; no docs outside this task unless review later要求
- 依赖: T9
- 内容:
  - 在测试文件顶部保留本地运行命令与 scope guard。
  - 明确默认 lane 不运行 ignored Grand Tour。
  - 明确 PR-3 不修 src，不处理 master cmd drift / session 聚合 status。
  - 保留 grep guard 覆盖 6 case、3 helper、mock mode、known-gap JSON-RPC error。
- 验收标准:
  - Rust PR-3 ignored lane: `CARGO_BUILD_JOBS=1 cargo test --test ah_full_e2e_realign_extra -- --include-ignored --test-threads=1`
  - Rust PR-3 default lane: `CARGO_BUILD_JOBS=1 cargo test --test ah_full_e2e_realign_extra -- --test-threads=1`
  - Grep guard:
    - `rg -n "case_06_orphan_audit_only|case_07_orphan_force_cleanup|case_08_busy_skip|case_09_busy_force_realign|case_10_error_crash_detection|case_11_error_recovery_known_gap" tests/ah_full_e2e_realign_extra.rs`
    - `rg -n "wait_for_tmux_pane_gone|update_agent_state_direct|query_agent_last_error" tests/ah_full_e2e_realign_extra.rs`
    - `rg -n "GRAND_TOUR_MOCK_BEHAVIOR|BUSY|CRASH|AGENT_ALREADY_EXISTS|rpc_raw" tests/ah_full_e2e_realign_extra.rs`
    - `rg -n "master cmd drift|session-level|PR-5|PR-4 src fix" tests/ah_full_e2e_realign_extra.rs`
  - 绿灯信号: 6 case 矩阵可按设计执行；默认 lane skipped；无新 failed。
- audit 视角:
  - a2 检查 final grep 覆盖 case/helper/mock/scope/known-gap。
  - a3 检查 PR-3 没有偷偷修 src，也没有扩到 PR-5 范围。

## PR-3 Final Verification

- `CARGO_BUILD_JOBS=1 cargo test --test ah_full_e2e_realign_extra -- --include-ignored --test-threads=1`
- `CARGO_BUILD_JOBS=1 cargo test --test ah_full_e2e_realign_extra -- --test-threads=1`
- `rg -n "case_06_orphan_audit_only|case_07_orphan_force_cleanup|case_08_busy_skip|case_09_busy_force_realign|case_10_error_crash_detection|case_11_error_recovery_known_gap" tests/ah_full_e2e_realign_extra.rs`
- `rg -n "wait_for_tmux_pane_gone|update_agent_state_direct|query_agent_last_error" tests/ah_full_e2e_realign_extra.rs`
- `rg -n "GRAND_TOUR_MOCK_BEHAVIOR|SKIPPED_BUSY|DRIFT_FORCE_REALIGN|AGENT_ALREADY_EXISTS|rpc_raw" tests/ah_full_e2e_realign_extra.rs`
- `rg -n "master cmd drift|session-level 聚合|PR-4 src fix|PR-5" tests/ah_full_e2e_realign_extra.rs`
