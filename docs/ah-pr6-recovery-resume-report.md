# ah PR-6 Recovery Resume Report

## §1 PR-6 范围与对齐目标

PR-6 承接 PR-3 的 case_11 ERROR recovery known-gap：PR-3 只记录 `CRASHED` agent 再 realign 会撞 `AGENT_ALREADY_EXISTS`，PR-6 把这条 src 行为修成真实恢复路径。

本 PR 覆盖两件事：第一，`session.realign` 能看见 `CRASHED` agent，并把它作为恢复重启处理；第二，Claude Agent Worker 在恢复时真实追加 `--continue`，让 provider 使用同一个 sandbox home 续接历史会话。

PR-6 不实现 Codex/Gemini resume，这留给 PR-7+；不做 master cmd drift，这属于 PR-5；不做 `ccbd -> ah` 仓库或模块重命名。`KILLED` 不进入 recovery 范围，因为它表示主动终止，不应自动 resume。

## §2 测试拓扑

PR-6 翻转 `tests/ah_full_e2e_realign_extra.rs` 的 case_11：从 `case_11_error_recovery_known_gap` 改为 `case_11_error_recovery` (`tests/ah_full_e2e_realign_extra.rs:1071`)。该 case 仍在 `grand_tour_realign_extra_matrix` 内，接在 case_10 crash detection 之后运行。

测试侧新增 fake Claude marker 行为：`GRAND_TOUR_RESUME_ARG_MARKER` 非空时，fake `claude` 启动会把真实 `"$@"` 写到 marker 文件 (`tests/ah_full_e2e_realign_extra.rs:539-540`)。测试端用 `wait_for_resume_marker` 与 `assert_marker_contains` (`:339-357`) 读取这个文件，确认 `--continue` 传到了物理进程 argv。

PR-6 还新增 lib 单测 `wrap_command_with_recovery_appends_resume_args` (`src/sandbox/systemd.rs:542`)，覆盖四类 provider：Claude recovery 追加 `--continue`；Claude non-recovery 不追加；Bash/Codex/Gemini recovery 也不追加。

case_06-09 不改业务语义。case_11 在设置 recovery marker 前先断言 marker 不存在，避免 IDLE/BUSY drift 路径提前注入 resume。

## §3 实施摘要

- T3 Provider manifest：`ProviderManifest` 新增 `resume_args: &'static [&'static str]` (`src/provider/manifest.rs:11`)；Claude 填 `&["--continue"]` (`:208`)，Bash/Codex/Gemini/fallback 填 `&[]`；`tests/mvp7_acceptance.rs:318` 补字段 fan-out。
- T4 command 构造：`wrap_command` 加 `is_recovery` 参数 (`src/sandbox/systemd.rs:8-15`)，透传到 `command_with_env_prefix` (`:114-131`)；只有 `is_recovery=true` 时才把 `manifest.resume_args` 追加到 provider command 后。
- T5 realign spawn：`spawn_realign_agent` 加 `is_recovery` 参数 (`src/rpc/handlers.rs:608`)，NEW / ordinary drift / CRASHED recovery 三条路径显式传值；公开 `agent.spawn` 仍走 `handle_agent_spawn(..., false)` (`:697`)。
- T6 CRASHED recovery：`running_agent_hashes` SQL 改为只排除 `KILLED` (`src/rpc/handlers.rs:663`)；`handle_session_realign` 在 NO_CHANGE 前优先处理 `running.state == "CRASHED"` (`:468`)；`agent_spawned` payload 增加 `is_recovery` (`:647`)。
- case_11 翻转：恢复后断 `REALIGNED`、`IDLE`、新 PID、marker 含 `--continue`、`agent_spawned.reason=DRIFT_REALIGN` 且 `is_recovery=true` (`tests/ah_full_e2e_realign_extra.rs:1114-1123`)。

## §4 物理断言风格

PR-6 继续沿用 ah full e2e 的物理断言风格。关键不是看测试里拼出来的 JSON，而是让 fake `claude` 进程在启动时记录真实 argv，再由测试读取 marker 文件。

case_11 的对账是四轴：RPC per-agent status 必须是 `REALIGNED`；DB state 必须回到 `IDLE`；PID 必须变化，说明不是复用已死进程；event payload 必须包含 `reason=DRIFT_REALIGN` 和 `is_recovery=true`。

负断言也保留：recovery marker 在 CRASHED recovery 前必须不存在。这样 case_06-09 的 ORPHAN/BUSY/ordinary drift 路径不会因为 `resume_args` 字段存在而误传 `--continue`。

## §5 关键设计决策

- `is_recovery` 严格等于 `running.state == "CRASHED"`，不是“DB 有 row”。这是 1d audit 后的收窄，避免 IDLE/BUSY drift realign 误注入 `--continue`。
- `wrap_command` 的 `is_recovery` 参数落在当前签名的第 5 个业务参数位置，`daemon_unit` 后移；`spawn_realign_agent` 中 `killed_before_spawn` 仍是第 5 参，`is_recovery` 是第 6 参。两个签名独立，含义不混用。
- `agent_spawned.reason` 复用 `DRIFT_REALIGN`，不新增 `RECOVERY` reason。恢复语义用 payload `is_recovery: true` 表示，既保留现有 reason 体系，也给审计/测试明确区分信号。
- `resume_args` 使用 `&'static [&'static str]`，贴合现有 `manifest.command` 静态 slice 风格，避免 `Vec<String>` 带来的不必要分配。
- `agent_exists` 检查不放宽。普通 `agent.spawn` 仍拒绝重复 row；recovery 路径先 `delete_agent`，再以 `is_recovery=true` 进入 spawn。
- `running_agent_hashes` 仍排除 `KILLED`，主动终止不自动 resume。

## §6 验收结果

- `CARGO_BUILD_JOBS=1 cargo check --tests`：0 errors。
- `CARGO_BUILD_JOBS=1 cargo test --workspace --lib`：392 passed, 0 failed, 1 ignored。
- `CARGO_BUILD_JOBS=1 cargo test --test ah_full_e2e_realign_extra grand_tour_realign_extra_matrix -- --include-ignored --test-threads=1 --nocapture`：1 passed, 0 failed；case_06-11 全 PASS，case_11 转绿，完成时间 9.08s。
- `CARGO_BUILD_JOBS=1 cargo test --test ah_full_e2e_realign_extra -- --test-threads=1`：3 passed, 0 failed, 1 ignored。

## §7 LOC

Tests-first commit `97f23fd` 修改 `tests/ah_full_e2e_realign_extra.rs`：179 insertions, 49 deletions。src impl commit `283ecfa` 修改 `src/provider/manifest.rs`、`src/rpc/handlers.rs`、`src/sandbox/systemd.rs`、`tests/mvp7_acceptance.rs`：125 insertions, 8 deletions。

合计新增 304 LOC。分布是 src +125 LOC、test +179 LOC；和 design.md §6 的约 280 LOC 估算同一量级。

## §8 后续 PR 规划

PR-5 继续承接 master cmd drift、`ccbd -> ah` rename / repo rename 等非 worker recovery 工作，不和 PR-6 混在同一个行为修复里。

PR-7+ 可继续补 Codex/Gemini resume。PR-6 已把 manifest 接口留好：未来只需确认各 provider 的真实 resume flag，再给对应 `resume_args` 填值并补物理 argv 测试。
