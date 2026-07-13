# MD2 Wave-2 候选1 设计稿 · `db::system` 低风险边界拆分

## 0. 任务边界与红线

本稿只覆盖 `src/db/system.rs`(HEAD `b9c9af3`, 3491 行)里的三类低风险搬移:

1. **startup reconcile**:`reconcile_startup*`、`reconcile_active_agents_to_crashed_sync`、orphan scope reconcile、master recovery window startup reconcile、Claude gateway seat rebuild。
2. **system dump**:`system_dump_sync` 与 async wrapper `system_dump`。
3. **runtime sweep**:`sweep_stale_tmux_sockets_sync`、`tmux_sessions_alive`、`remove_agent_sandbox_dir_sync`、`remove_agent_sandbox_dir_preserving_home_sync`。

硬红线:第 4 类 **cascade / master-death cleanup** 保持原地不动、不重命名、不改可见性、不搬测试,包括:

- `cascade_kill_session_agents_sync`(`src/db/system.rs:188`)
- `cascade_kill_session_agents_with_runner_sync`(`src/db/system.rs:412`)
- `cascade_kill_session_agents`(`src/db/system.rs:1224`)
- `cascade_kill_session_agents_for_daemon`(`src/db/system.rs:1235`)
- `clean_worker_runtime_resources_with_runner_sync`(`src/db/system.rs:274`)
- `clean_worker_runtime_resources_sync`(`src/db/system.rs:393`)
- `snapshot_master_death_session_activity`(`src/db/system.rs:216`)
- `stop_session_anchor_for_session_sync`(`src/db/system.rs:552`)
- `session_agent_ids_sync`(`src/db/system.rs:556`)
- `session_agent_ids`(`src/db/system.rs:1256`)

理由不是技术上不能搬,而是语义上不能并入本批次:`d1-target3-master-watch-design-2026-07-13.md` 已把 `db::system::{clean_worker_runtime_resources_sync,cascade_kill_session_agents}`列为 master 自愈 saga 的执行原语,并在 §3.3 说明它们直接关系到"cascade 击败 revive"这类历史败因。触及这组符号等价于触及 master 自愈,必须升级 operator gate,不属于本轮自闭环低风险拆分。

本稿是纯搬移设计:不改变任何函数签名、可见性、返回类型、SQL、分支、日志、错误映射、spawn label、调用点语义。

## 1. 推荐子模块切法

推荐把 `src/db/system.rs` 从单文件实现改成目录门面:

```text
src/db/system.rs              # 门面 + 第 4 类 cascade/master-death cleanup 原地保留
src/db/system/dump.rs         # system dump
src/db/system/startup.rs      # startup reconcile
src/db/system/sweep.rs        # tmux socket + sandbox dir runtime sweep
```

门面 `src/db/system.rs` 增加:

```rust
mod dump;
mod startup;
mod sweep;

pub(crate) use dump::system_dump_sync;
pub use dump::system_dump;

pub(crate) use startup::{
    reconcile_active_agents_to_crashed_sync, reconcile_master_recovery_windows_sync,
    reconcile_orphan_scopes_sync, reconcile_orphan_scopes_with_runner_sync,
    reconcile_startup_sync_with_state_dir,
};
pub use startup::{
    recovery_eligible_orphan_scope_should_be_preserved,
    reconcile_startup, reconcile_startup_with_state_dir, reconcile_startup_with_tmux_socket,
    reconcile_startup_with_tmux_socket_and_gateway,
};

pub use sweep::{
    remove_agent_sandbox_dir_preserving_home_sync, remove_agent_sandbox_dir_sync,
    sweep_stale_tmux_sockets_sync,
};
```

`#[cfg(test)]` helper exports只在门面下按需 re-export,例如:

```rust
#[cfg(test)]
pub(crate) use startup::{
    reconcile_startup_sync,
    reconcile_master_recovery_windows_with_runner_sync,
    reconcile_orphan_scopes_dry_run_enabled,
    reconcile_startup_sync_with_state_dir_and_runner,
};
```

### 为什么是这三个文件

- `dump.rs`:只有 DB 只读聚合和 async wrapper,无外部资源副作用,与 startup/cascade 没有业务调用边。独立收益最高、风险最低。
- `startup.rs`:它是一条启动 reconcile 编排链,包含 active agent crash reconcile、orphan scope reconcile、recovery window expiry reconcile、Claude gateway seat rebuild。虽然内部调用到第 4 类 cascade 原语,但调用点本身是 startup 语义,适合由 `startup.rs` 通过门面/兄弟模块引用第 4 类原语。
- `sweep.rs`:第 3 类同时包含 stale tmux socket 和 sandbox dir 删除。两者都是无 DB 事务的 runtime/filesystem 外部资源清理;把它们拆成 `socket.rs` + `sandbox.rs` 会增加模块数量但不会降低耦合,不建议。

不建议命名为 `cleanup.rs`:会和第 4 类 `clean_worker_runtime_resources*` / cascade cleanup 混淆,容易诱导实施者顺手搬 master 自愈原语。`sweep.rs` 明确表达"扫外部资源",避开 master-death cleanup。

## 2. 符号归属清单

### 2.1 `dump.rs`

搬入:

- `system_dump_sync`(`src/db/system.rs:100`, `pub(crate)`)
- `system_dump`(`src/db/system.rs:1220`, `pub`)

依赖:

- `crate::db::{Db, common::{map_db_error, spawn_db}}`
- `crate::error::CcbdError`
- `serde_json::{Value, json}`

不得搬入任何 cascade/startup/sweep 逻辑。

### 2.2 `startup.rs`

搬入:

- startup 私有类型:`DaemonMarkerProvenance`、`DaemonMarker`、`StartupAgentCandidate`、`StartupCrashCandidate`、`StartupAliveIoCandidate`、`StartupClaudeWorkerSeat`、`StartupClaudeSeats`、`StartupMasterRecoveryWindow`(`src/db/system.rs:25-87`, `:682`附近)
- `recovery_eligible_orphan_scope_should_be_preserved`(`src/db/system.rs:93`, `pub`)
- `reconcile_startup_sync`(`src/db/system.rs:570`, `#[cfg(test)] pub(crate)`)
- `reconcile_startup_sync_with_state_dir`(`src/db/system.rs:574`, `pub(crate)`)
- `reconcile_startup_sync_with_state_dir_and_runner`(`src/db/system.rs:587`, private)
- `reconcile_orphan_scopes_sync`(`src/db/system.rs:610`, `pub(crate)`)
- `reconcile_orphan_scopes_dry_run_enabled`(`src/db/system.rs:618`, private; test re-export only if tests need old import)
- `reconcile_orphan_scopes_with_runner_sync`(`src/db/system.rs:622`, `pub(crate)`)
- `reconcile_orphan_scopes_with_marker_sync`(`src/db/system.rs:636`, private)
- `reconcile_master_recovery_windows_sync`(`src/db/system.rs:689`, `pub(crate)`)
- `reconcile_master_recovery_windows_with_runner_sync`(`src/db/system.rs:701`, private; test re-export only)
- `reconcile_master_recovery_windows_with_marker_sync`(`src/db/system.rs:715`, private)
- `unixepoch`(`src/db/system.rs:790`, private)
- `active_session_and_agent_refs_sync`(`src/db/system.rs:797`, private)
- `is_own_ccbd_scope` / `is_orphan_scope`(`src/db/system.rs:844-848`, private)
- `reconcile_active_agents_to_crashed_sync`(`src/db/system.rs:852`, `pub(crate)`)
- `startup_reconcile_phase_*` helpers(`src/db/system.rs:885-1134`, private)
- `open_fifo_for_reattach` / `probe_pid_alive`(`src/db/system.rs:1053-1074`, private)
- async startup wrappers and Claude gateway seat rebuild(`src/db/system.rs:1263-1519`)

`startup.rs` should import sandbox removers from sibling `sweep` through the door kept stable by the parent:

```rust
use super::{remove_agent_sandbox_dir_preserving_home_sync, remove_agent_sandbox_dir_sync};
```

It should import cascade red-line functions only as called dependencies, not move them:

```rust
use super::cascade_kill_session_agents_with_runner_sync;
```

This makes the red-line visible in code review: startup may call the cascade primitive at the existing line-equivalent point, but ownership of cascade remains in `system.rs`.

### 2.3 `sweep.rs`

搬入:

- `remove_agent_sandbox_dir_sync`(`src/db/system.rs:940`, `pub`)
- `remove_agent_sandbox_dir_preserving_home_sync`(`src/db/system.rs:961`, `pub`)
- `sweep_stale_tmux_sockets_sync`(`src/db/system.rs:1521`, `pub`)
- `tmux_sessions_alive`(`src/db/system.rs:1574`, private)

依赖:

- `crate::error::CcbdError`
- `std::{io, path::Path, process::Command}`
- `libc::geteuid` under unix cfg

`sweep.rs` must not import DB or cascade symbols. If implementation requires DB after moving, that is evidence the boundary was expanded; stop and re-review.

### 2.4 `system.rs` 门面保留内容

保留:

- top-level `mod` declarations and re-exports
- 第 4 类 cascade/master-death cleanup production code and its private helpers
- `pub(crate) use crate::platform::sys::scope::{ScopeUnit, SystemctlRunner};` can stay in the facade if cascade tests still use `super::ScopeUnit`; startup can import the same types directly or from `super`

第 4 类 tests也留在 `system.rs` 的 `#[cfg(test)] mod tests`,除非单独 operator gate 批准。

## 3. 真实依赖调用边

这部分只列真实函数调用边,不把"同文件"当依赖。

### 3.1 `dump` 与其他类

- `system_dump`(`src/db/system.rs:1220`)调用 `system_dump_sync`(`:100`)。
- `system_dump_sync`不调用 startup/sweep/cascade。

拆后方式:`dump.rs` 内部直接调用同模块 `system_dump_sync`;门面 re-export 保持 `crate::db::system::system_dump` 和 `crate::db::system::system_dump_sync` 旧路径。

### 3.2 `startup` -> `sweep`

- `reconcile_active_agents_to_crashed_sync`(`src/db/system.rs:852`)在 dead agent cleanup 上调用:
  - `remove_agent_sandbox_dir_preserving_home_sync`(`:867`)
  - `remove_agent_sandbox_dir_sync`(`:873`)
- `reconcile_startup_with_tmux_socket_sync_and_runner`(`src/db/system.rs:1333`)在最后调用 `sweep_stale_tmux_sockets_sync(current_socket_name)`(`:1355`)。

拆后方式:`startup.rs` 使用 `super::{remove_agent_sandbox_dir_preserving_home_sync, remove_agent_sandbox_dir_sync, sweep_stale_tmux_sockets_sync}`。`sweep.rs` 不反向引用 `startup.rs`,因此无循环。

### 3.3 `startup` -> 第 4 类 cascade

- `reconcile_master_recovery_windows_with_marker_sync`(`src/db/system.rs:715`)在 expired recovery window 上调用 `cascade_kill_session_agents_with_runner_sync`(`:776`)。

拆后方式:`startup.rs` 使用 `super::cascade_kill_session_agents_with_runner_sync`。这是一条必须保留的语义调用边,不是搬移授权。`cascade_kill_session_agents_with_runner_sync` 继续在 `system.rs` 原地定义,可见性不变。

### 3.4 第 4 类 -> 本轮三类

未发现第 4 类 production code 直接调用 `dump.rs`、`startup.rs`、`sweep.rs` 的目标函数。`rg` 命中只显示测试导入和 startup 调 cascade。

### 3.5 startup 内部自调用链

关键顺序不得改变:

- `reconcile_startup_sync_with_state_dir_and_runner`(`src/db/system.rs:587`)顺序:
  1. `reconcile_active_agents_to_crashed_sync`(`:596`)
  2. `reconcile_master_recovery_windows_with_marker_sync`(`:604`)
  3. `reconcile_orphan_scopes_with_marker_sync`(`:606`)
- `reconcile_startup_with_tmux_socket_sync_and_runner`(`src/db/system.rs:1333`)顺序:
  1. compute socket/daemon marker
  2. `reconcile_active_agents_to_crashed_sync`(`:1350`)
  3. `reconcile_master_recovery_windows_with_marker_sync`(`:1352`)
  4. `reconcile_orphan_scopes_with_marker_sync`(`:1354`)
  5. `sweep_stale_tmux_sockets_sync`(`:1355`)

`startup_reconcile_runs_recovery_windows_before_orphan_scope_cleanup`(`src/db/system.rs:2242`)已经钉住 recovery window reconcile 先于 orphan scope cleanup。搬移不得改变这条顺序。

## 4. 测试搬迁方案

现有 tests 从 `src/db/system.rs:1592` 开始。按本轮边界拆时,测试也按责任搬迁;第 4 类测试留在门面。

### 4.1 留在 `system.rs` 的第 4 类 tests

这些测试覆盖 master-death/cascade/runtime cleanup red-line,留在原地:

- `master_death_snapshot_active_states_are_active_work`(`:1726`)
- `master_death_snapshot_prompt_pending_is_active_work`(`:1748`)
- `master_death_snapshot_queued_only_idle_worker_is_active_work`(`:1768`)
- `master_death_snapshot_dispatched_job_is_active_work`(`:1788`)
- `master_death_snapshot_all_idle_or_dead_without_jobs_is_idle_no_work`(`:1813`)
- `master_revive_deliberate_cascade_kill_does_not_capture_revive_intent`(`:1844`)
- `clean_worker_runtime_resources_keeps_session_active_and_marks_worker_killed`(`:1881`)
- `clean_worker_runtime_resources_clears_runtime_registries_without_systemd`(`:2295`)
- `clean_worker_runtime_resources_kills_agent_tmux_session_before_returning`(`:2327`)
- `clean_worker_runtime_resources_records_scope_failure_and_uses_pidfd_fallback`(`:2396`)
- `clean_worker_runtime_resources_stops_matching_scope_on_success`(`:2436`)
- `clean_worker_runtime_resources_preserves_anchor_for_active_work_revive`(`:2470`)
- `clean_worker_runtime_resources_stops_anchor_when_not_preserved`(`:2503`)
- `clean_worker_runtime_resources_is_idempotent_for_already_cleaned_worker`(`:2534`)
- `clean_worker_runtime_resources_degrades_when_scope_and_pidfd_cleanup_fail`(`:2579`)
- `test_cascade_kill_session_agents_counts_active_only`(`:2626`)
- `test_cascade_kill_session_agents_cleans_closed_session`(`:2659`)
- `test_cascade_kill_session_agents_stops_matching_agent_scopes`(`:2698`)
- `test_cascade_kill_session_agents_skips_anchor_when_daemon_marker_absent`(`:2743`)

### 4.2 搬到 `startup.rs` 的 tests

这些测试直接覆盖 startup reconcile / recovery window startup reconcile / orphan scope reconcile / Claude gateway seat rebuild:

- `startup_reconcile_preserves_unexpired_window_after_ahd_restart`(`:1930`)
- `startup_reconcile_expires_old_window_and_cascades_workers`(`:1972`)
- `startup_reconcile_alive_verifying_window_does_not_complete`(`:2020`)
- `startup_reconcile_expires_window_when_master_dead`(`:2081`)
- `startup_reconcile_preserves_window_when_master_dead_and_unexpired`(`:2139`)
- `startup_reconcile_leaves_terminal_window_untouched`(`:2198`)
- `startup_reconcile_runs_recovery_windows_before_orphan_scope_cleanup`(`:2242`)
- `test_reconcile_startup_crashes_dead_pid_and_fails_dispatched_job`(`:2775`)
- `test_reconcile_dead_pid_removes_materialized_home_but_alive_reattach_preserves_it`(`:2850`)
- `test_reconcile_startup_keeps_alive_pid_active`(`:2904`)
- `startup_reconcile_restores_claude_gateway_master_and_worker_idempotently`(`:2933`)
- `test_reconcile_startup_preserves_prompt_pending_agent`(`:3024`)
- `test_reconcile_startup_retains_live_agent_scope_and_reaps_orphan`(`:3151`)
- `test_reconcile_orphan_scopes_never_stops_foreign_marker_overlapping_agent_ids`(`:3203`)
- `test_reconcile_startup_respects_marker_ownership_for_overlapping_agent_ids`(`:3237`)
- `test_reconcile_startup_with_ambient_marker_never_stops_scopes`(`:3284`)
- `test_reconcile_startup_from_env_marker_is_ambient_and_never_stops_scopes`(`:3315`)
- `test_reconcile_orphan_scopes_skips_foreign_daemon_scopes`(`:3352`)
- `test_reconcile_orphan_scopes_dry_run_does_not_stop`(`:3368`)
- `test_reconcile_orphan_scopes_force_mode_stops`(`:3384`)
- `test_reconcile_orphan_scopes_defaults_to_real_stop`(`:3401`)
- `test_reconcile_orphan_scopes_dry_run_escape_hatch`(`:3412`)
- `test_reconcile_orphan_scopes_skips_known_agent_with_stale_marker`(`:3426`)
- `test_reconcile_orphan_scopes_keeps_active_session_scope`(`:3455`)
- `test_reconcile_orphan_scopes_handles_missing_systemctl_gracefully`(`:3476`)

`test_reconcile_crashed_agent_removes_sandbox_dir`(`:2804`) straddles startup and sweep:入口是 `reconcile_active_agents_to_crashed_sync`,断言是 cleanup side effect。建议放在 `startup.rs`,因为被测 public surface 是 startup reconcile;它通过 `super::remove_agent_sandbox_dir_sync` 验证跨模块调用边仍工作。

### 4.3 搬到 `sweep.rs` 的 tests

这些测试直接覆盖 sandbox remover:

- `test_remove_agent_sandbox_dir_removes_materialized_home`(`:2828`)

建议新增一个 compile-level behavior test放在 `sweep.rs` 或门面 tests:

```rust
#[test]
fn facade_reexports_sweep_stale_tmux_sockets_sync_old_path_compiles() {
    let f: fn(Option<&str>) -> Result<usize, crate::error::CcbdError> =
        crate::db::system::sweep_stale_tmux_sockets_sync;
    let _ = f;
}
```

这条测试不需要真实 tmux socket;它的目标是钉住旧路径 `crate::db::system::sweep_stale_tmux_sockets_sync` 仍可用。

### 4.4 `dump.rs` tests

当前 `system.rs` inline tests未直接覆盖 `system_dump_sync`。本轮可以不补行为测试,但建议新增一个轻量 compile-level facade test:

```rust
#[test]
fn facade_reexports_system_dump_sync_old_path_compiles() {
    let f: fn(&crate::db::Db) -> Result<serde_json::Value, crate::error::CcbdError> =
        crate::db::system::system_dump_sync;
    let _ = f;
}
```

如果实施者认为这会暴露 `pub(crate)` 测试路径细节,可只依赖 `cargo test --lib db::system` 编译门。行为层不要求新增 dump 断言,因为本批是搬移不是功能补测。

## 5. 零回归论证

参照 target3 设计稿的结构,本轮先明确"当前防护在哪",再说明"搬移如何保证不变"。

### 5.1 当前行为防护

- 旧调用路径已经被外部代码使用:
  - `src/bin/ahd.rs:91` 调 `db::system::reconcile_startup_with_tmux_socket_and_gateway`
  - `src/bin/ahd.rs:278` 调 `db::system::remove_agent_sandbox_dir_sync`
  - `src/rpc/handlers/system.rs:7` 调 `system_dump`
  - `src/rpc/handlers/agent.rs:11/:860` 调 `remove_agent_sandbox_dir_sync`
  - `src/rpc/handlers/master_cutover.rs:12/:100/:111` 调 `remove_agent_sandbox_dir_sync` / `session_agent_ids`
  - `src/rpc/handlers/sessions.rs:10/:103/:106/:157/:160` 调 `remove_agent_sandbox_dir_sync` / `session_agent_ids`
  - `src/agent_io/registry.rs:177` 调 `crate::db::system::remove_agent_sandbox_dir_sync`
- startup 顺序已有测试钉住:`startup_reconcile_runs_recovery_windows_before_orphan_scope_cleanup`(`src/db/system.rs:2242`)。
- recovery window expired 后 cascade 的状态变化已有测试钉住:`startup_reconcile_expires_old_window_and_cascades_workers`(`src/db/system.rs:1972`)。
- sandbox 删除行为已有测试钉住:`test_remove_agent_sandbox_dir_removes_materialized_home`(`src/db/system.rs:2828`)。

### 5.2 拆分影响

本拆分是 Rust module relocation:

- 函数体逐字搬移,不改 SQL、不改 `params!` 顺序、不改 `TransactionBehavior::Immediate`、不改 filesystem paths、不改 `Command::new("tmux")` 参数。
- public / crate-public surface 通过门面 re-export 保持旧路径。调用方仍写 `crate::db::system::*`,不要求任何生产调用点改 import。
- async wrapper 的 `spawn_db` label保持原字符串,例如 `"system::system_dump"`、`"system::reconcile_startup"`。
- 第 4 类 cascade/master-death cleanup不搬,因此 target3 设计稿 §3.3 依赖的 `db::system` master 自愈原语路径仍稳定。

### 5.3 保证机制

1. **门面 re-export 先行**:先创建子模块并 re-export旧符号,再搬函数体。编译失败会直接暴露漏 export。
2. **单向依赖**:
   - `dump.rs` 不依赖其他子模块。
   - `sweep.rs` 不依赖其他子模块。
   - `startup.rs` 只依赖 `super::sweep` re-export 和门面原地 cascade 原语。
   - `system.rs` 作为父模块可同时看见子模块与原地 cascade,不会形成 Rust module cycle。
3. **第 4 类红线审查**:实施 diff 中出现 `cascade_kill_session_agents*`、`clean_worker_runtime_resources*`、`snapshot_master_death_session_activity`、`session_agent_ids*` 的 `pub`/签名/文件归属变化即判定越界。
4. **测试随责任移动**:测试移动只调整 `use super::...` 路径,不得把测试断言改弱。对跨模块调用边,保留至少一条 startup 入口触发 sweep 行为的测试(`test_reconcile_crashed_agent_removes_sandbox_dir`)。

### 5.4 失效模式与回滚

可能失效模式:

- 漏 re-export导致外部旧路径编译失败。
- 将 private helper错误提升为 `pub(crate)` 或 `pub`,扩大 API。
- 将第 4 类 cascade helper顺手搬走,污染 master 自愈 gate。
- startup 与 sweep互相 import造成循环或不必要耦合。
- 测试搬迁时把 `#[serial_test::serial(global_env)]`、`#[tokio::test]` 或 unix cfg 丢失。

回滚行为:

- 因为本批是纯新增子模块 + 函数搬移,若出现行为风险,整 PR 可机械回滚回单文件 `system.rs`。
- 不允许在同一 PR 中用"顺手修 bug"抵消拆分失败;行为 bug单开后续任务。

## 6. 实施者验收锚点

最低验收:

1. `cargo test --lib db::system` 通过。
2. `cargo test --all-targets` 通过。
3. 新增行为保持/编译保持测试至少一条:
   - 推荐名:`facade_reexports_sweep_stale_tmux_sockets_sync_old_path_compiles`
   - 断言目标:旧路径 `crate::db::system::sweep_stale_tmux_sockets_sync` 可赋给原签名 `fn(Option<&str>) -> Result<usize, CcbdError>`。
4. 保留并通过既有关键测试:
   - `startup_reconcile_runs_recovery_windows_before_orphan_scope_cleanup`
   - `startup_reconcile_expires_old_window_and_cascades_workers`
   - `test_reconcile_crashed_agent_removes_sandbox_dir`
   - `test_remove_agent_sandbox_dir_removes_materialized_home`
   - 第 4 类 red-line tests清单中的 cascade/clean_worker tests。
5. `rg -n "pub .*cascade_kill|pub .*clean_worker|pub .*session_agent_ids|snapshot_master_death" src/db/system.rs src/db/system` 显示第 4 类符号仍由 `src/db/system.rs` 拥有。

## 7. 覆盖范围、最弱区域、o1 反方处理

实际覆盖:

- 读并核对了 brief、`src/db/system.rs` 目标生产符号、测试名/行号、外部旧路径调用点、target3 设计稿的零回归论证结构。
- brief 提到的 `wave2-plan-draft-2026-07-13.md` 在当前工作区不存在;本稿使用现有 `md2-plan-2026-07-13.md`、`c2-layer4-5-audit-2026-07-13.md`、`d1-target3-master-watch-design-2026-07-13.md` 和源码实测补齐依据。

最弱区域:

- 没有逐行重读全部 1900 行测试体;测试归属基于测试名、入口函数和抽样断言。
- 未运行构建/测试,因为本任务是设计稿执笔且不改 `.rs`。
- `system_dump_sync` 现有 inline test缺口未补行为断言;本稿只建议 compile-level facade test,不把补功能测试作为强制。

o1 反方处理:

- 本任务未附新的 o1 divergence brief。可视为已采纳的关键反方是 target3 设计稿里的 master 自愈红线:任何 cascade/master-death cleanup 搬移都会扩大到 operator gate,所以本稿明确把第 4 类留原地。
- 驳回的潜在弱方案:`cleanup.rs` 统一承接 sweep + cascade。理由:命名会把无 DB/filesystem sweep 和 master 自愈 cleanup 混在一起,增加实施越界概率,与本轮低风险纯搬移目标冲突。
