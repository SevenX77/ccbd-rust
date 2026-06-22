# master_watch 重启漏装探针修复设计

本方案只修检测层：ahd 重启后重新 arm ACTIVE master 的死亡探针，并加周期巡检兜底；探针/巡检 fire 后仍走现有 master 死亡处理语义，不改变“死后是否连坐 worker、是否复活”的决策。

## 现状链

现有 fire -> 处理链如下：

1. master pidfd watcher 是一次性 wait task。入口 `spawn_master_pidfd_watch_task` 在 `src/monitor/master_watch.rs:30-41`，把 pidfd 包成 `AsyncFd`，等 `readable()`。
2. pidfd ready 后认为 master 退出，记录日志并调用 `classify_master_death(&db, &session_id, expected_pid, expected_generation)`，见 `src/monitor/master_watch.rs:52-60`。
3. `classify_master_death` 读取 `sessions.status/master_pid/master_generation`，只有 `status == ACTIVE`、无 inflight cutover、pid/generation 都匹配时返回 `MasterDeathDecision::Revive`，见 `src/master_revival.rs:61-97`。
4. Revive 分支调用 `revive_master_after_exit(...)`，见 `src/monitor/master_watch.rs:60-77`。
5. `revive_master_after_exit` 先拿 session 级 spawn lock，快照 session activity，然后调用 `clean_worker_runtime_resources_sync(..., "MASTER_EXIT", ...)` 清 worker runtime，见 `src/monitor/master_watch.rs:94-118`。
6. 如果快照分类是 `IdleNoWork`，调用 `mark_session_failed_after_idle_master_death`，不复活 master，见 `src/monitor/master_watch.rs:129-138`。这就是现有 idle master death 语义。
7. 否则按 retry/backoff、CAS claim、记录 revive attempt、写 redispatch marker、spawn replacement master、更新 `sessions.master_pid/master_generation/master_pane_id`，见 `src/monitor/master_watch.rs:139-331`。
8. replacement master 完成后重新注册 pidfd + arm 新 watcher，见 `src/monitor/master_watch.rs:315-331`；随后注入 continue、reprovision worker、requeue interrupted jobs、启动 stable confirm timer，见 `src/monitor/master_watch.rs:332-378`。
9. 无论 Revive/IntentionalExit/Stale，watch task 最后 `remove_master_monitor_key_if_generation_matches`，见 `src/monitor/master_watch.rs:86`。

现有 arm 位置只有：

- master spawn 后且 `arm_revival_watch == true`: `src/rpc/handlers/sessions.rs:394-410` 调 `arm_master_revival_watch`。
- `arm_master_revival_watch` 实际 `pidfd_open`、`monitor::register(master:{session}:{generation})`、`spawn_master_pidfd_watch_task`，见 `src/rpc/handlers/sessions.rs:425-452`。
- cutover `VERIFYING -> ACTIVE` 后 arm: `src/rpc/handlers/sessions.rs:890-900`。
- revive 后 arm: `src/monitor/master_watch.rs:315-331`。

现状 bug 成因也成立：

- ahd startup 入口在 `src/bin/ahd.rs:56-75` 只调用 startup reconcile 后启动 orchestrator。
- `reconcile_startup_with_tmux_socket` 只包 `reconcile_active_agents_to_crashed_sync` + socket sweep，见 `src/db/system.rs:1163-1175`。
- sync reconcile 只处理 agents/scopes，见 `src/db/system.rs:521-532` 和 `src/db/system.rs:757-787`；alive pidfd 重建也只针对 agent，见 `src/db/system.rs:1031-1060`。
- `master_process_is_alive` 只在 cutover readiness 使用，见 `src/rpc/handlers/sessions.rs:455-460`、`src/rpc/handlers/sessions.rs:631-637`。

## 修复点 1: startup 重 arm

建议新增统一检测入口，避免 startup/patrol/pidfd 三条路径分叉：

- 在 `src/monitor/master_watch.rs` 增加 `pub async fn handle_master_death_detected(ctx: &Ctx, session_id: String, expected_pid: i64, expected_generation: i64, master_cmd: String, source: MasterDeathSource) -> Result<(), CcbdError>`。
- 让 `spawn_master_pidfd_watch_task` 在 `src/monitor/master_watch.rs:58-86` 不再自己 classify，而是调用该统一入口。
- 统一入口内部先拿 `master_spawn_lock(&session_id)`，然后在 lock 内重新 `classify_master_death`。只有仍是 `Revive` 才进入现有 `revive_master_after_exit` 的主体逻辑。这样 double-fire 时第二个 caller 会在 lock 内看到 stale/new generation，直接退出，不会二次 cleanup worker。
- 为避免双重 lock，把现有 `revive_master_after_exit` 拆成 `revive_master_after_exit_locked`，或者把 `classify + lock + revive body` 全部收敛进新入口；单次 fire 的业务语义保持原样。

startup re-arm 插入点建议放在 `src/bin/ahd.rs`，因为此处已经有完整 `Ctx`：

- 现有 `ctx` 构造在 `src/bin/ahd.rs:67-73`，`orchestrator::spawn_orchestrator_task(ctx.clone())` 在 `src/bin/ahd.rs:74`。
- 在 `src/bin/ahd.rs:73-74` 之间调用：
  - `master_watch::rearm_active_master_watches_on_startup(&ctx).await`
  - 成功后再启动 orchestrator 和 RPC server。
- 不建议塞进 `src/db/system.rs:1163-1175`，因为当前该函数在 blocking `spawn_db` 内，只适合 DB/文件 reconcile；arm pidfd watch 需要 `Ctx/tmux/env_state/daemon_unit` 并 spawn async task。

`rearm_active_master_watches_on_startup` 设计：

1. 查询所有 `sessions.status = 'ACTIVE'` 且 `master_pid > 0` 的 session，字段至少包括 `id/project_id/master_pid/master_generation/master_pane_id/absolute_path/master_cmd`。
2. 对每个 session 算 key `master_monitor_key(session_id, master_generation)`，见 `src/master_revival.rs:399-407`。
3. 如果 `monitor::contains(key)` 已经为 true，则跳过，防同进程重复 arm。startup 正常是空 registry，但这个 guard 对测试和未来重复调用有价值。
4. `pidfd_open(master_pid)`：
   - 成功：clone fd，`monitor::register(key, pidfd)`，调用 `spawn_master_pidfd_watch_task`。
   - `AgentUnexpectedExit`: 说明 startup 这一刻 master 已死，立即调用 `handle_master_death_detected(..., source=StartupRearmDead)`，走同一条 fire -> 处理路径。
   - 其他错误：记录 warn，不改变 session/worker 状态；周期巡检继续兜底。

必要支撑数据：当前 `sessions` 只保存 `master_pid/master_pane_id/config_hash/master_generation`，没有可逆 `master_cmd`，见 `src/db/schema.rs:8-20`；`config_hash` 由 `ConfigRole::Master { cmd }` 参与计算但不能反推出 cmd，见 `src/provider/fingerprint.rs:23-44`。因此实现前应补一个最小持久化字段：

- 推荐加 `sessions.master_cmd TEXT`，migration 默认 `NULL` 或 `'claude'`。
- 在 master spawn 成功路径记录 `params.cmd`：`src/rpc/handlers/sessions.rs:352-357` 附近目前只更新 config hash，应同时更新 `master_cmd`。
- cutover path 的 `request.master.cmd` 也同样持久化，因为 cutover ACTIVE 后 watcher 也需要同一 cmd，见 `src/rpc/handlers/sessions.rs:797-805`、`src/rpc/handlers/sessions.rs:890-900`。
- 旧库没有 cmd 时可 fallback 到 `"claude"` 或尝试读取 session `absolute_path` 下当前 `ah.toml [master].cmd`；但设计上应把 fallback 明确打 warn，因为这只能 best-effort，不是完全可恢复状态。

## 修复点 2: 周期巡检兜底

只在 startup re-arm 上依赖 pidfd 仍有风险：如果重 arm 失败、registry 被误清、或未来 pidfd task spawn 失败，后续 master 死亡仍会漏。需要周期巡检。

`master_process_is_alive` 目前在 `src/rpc/handlers/sessions.rs:455-460` 是 private helper，建议移动/复制为 `src/monitor/master_watch.rs` 的 public helper，例如：

- `pub fn master_process_is_alive(pid: i64) -> bool`
- 内部仍用 `monitor::pidfd_open(pid as i32).is_ok()`；不要发送信号。

巡检插入点：

- `src/orchestrator/mod.rs:36-52` 已经 spawn 了 pane_diff、health_check、prompt_pending watcher。
- 新增类似 `master_watch_patrol_loop(ctx.clone(), interval)` 的 tokio task。
- 不能只在 `run_once` 顶部加逻辑，因为主 orchestrator loop 在 `src/orchestrator/mod.rs:53-59` 每次 `run_once` 后等待 `WAKER.notified()`；没有新 job/event 时不会自然 tick。
- 为测试方便，同时提供 `pub(crate) async fn patrol_active_masters_once(ctx: &Ctx) -> Result<usize, CcbdError>`，可由 loop 调，也可由单测直接调。

巡检算法：

1. 频率建议 5-30s，可用 env `AH_MASTER_WATCH_PATROL_SECS`，默认 10s；不复用 stuck watch threshold，避免和 worker stuck 语义绑定。
2. 查询 ACTIVE sessions 的 `master_pid/master_generation/master_cmd`。
3. 对每行：
   - 如果 `master_pid <= 0`，跳过或 warn。
   - 如果 `monitor::contains(master_monitor_key(...))` 为 false，先尝试补 arm 一次；补 arm 成功后继续。
   - 如果 `master_process_is_alive(master_pid)` 为 false，调用 `handle_master_death_detected(..., source=Patrol)`。
4. 巡检本身只做检测和路由，不直接修改 session/worker 状态。

为什么它是主保险：

- pidfd task 是一次性 async wait，ahd 重启旧 task 必然消失。
- startup re-arm 只覆盖启动时刻；巡检覆盖“启动后才发现没有 watcher”与“watcher 未成功安装”的情况。

## tests-first

建议先红灯这些测试：

1. `src/monitor/master_watch.rs::startup_rearm_active_master_registers_watch_and_later_exit_routes_existing_path`
   - seed ACTIVE session，`master_pid/master_generation/master_cmd` 指向一个真实 `sleep 0.2` child。
   - seed BUSY worker + DISPATCHED job，复用 `test_master_watch_revives_active_session_on_master_exit` 的断言风格，见 `src/monitor/master_watch.rs:1749-1825`。
   - 调 `rearm_active_master_watches_on_startup(&ctx)` 后断言 `monitor::contains(master:{session}:{generation})`。
   - child exit 后断言 monitor key 消失、worker 被 KILLED、session 仍 ACTIVE 且 generation 前进/新 master spawn marker 出现。

2. `src/monitor/master_watch.rs::startup_rearm_dead_master_immediately_routes_existing_path`
   - seed ACTIVE session，`master_pid` 用已死 pid 或超大 pid。
   - seed active work。
   - 调 startup rearm。
   - 断言不只是返回错误，而是进入 `handle_master_death_detected`: worker cleanup/revive 或 IdleNoWork failed 行为与 `active_work_master_death_reaps_worker_revives_master_and_requires_redispatch_marker`、`idle_master_death_reaps_without_revive` 一致，见 `src/monitor/master_watch.rs:995-1208`。

3. `src/monitor/master_watch.rs::patrol_detects_dead_active_master_when_monitor_missing`
   - seed ACTIVE session with master pid/generation/cmd。
   - 不注册 `master:{session}:{generation}` key，模拟 ahd 重启后没装探针或 registry 丢失。
   - kill child 后调用 `patrol_active_masters_once(&ctx)`。
   - 断言进入同一处理路径，并返回 detected count = 1。

4. `tests/r1_master_exit_shutdown.rs::ahd_restart_rearms_inherited_active_master_then_later_exit_is_detected`
   - 基于现有 e2e `active_master_raw_exit_reaps_old_worker_then_revives_master`，见 `tests/r1_master_exit_shutdown.rs:469-531`。
   - 流程改为：启动 ahd -> create session/spawn master/spawn worker/submit job -> 只终止 ahd 进程但保留 tmux master/worker -> 重启 ahd 使用同一 state_dir -> kill inherited master pane pid。
   - 断言 worker runtime 被 reaped/replaced，master generation 前进，status 仍 ACTIVE，daemon 不退出。
   - 这是本 bug 的最小真实回归。

5. `src/monitor/master_watch.rs::pidfd_and_patrol_double_fire_only_handles_once`
   - 注册 pidfd watcher，同时在 child exit 后手动调用 patrol。
   - 断言 master_generation 只前进一次，worker cleanup/requeue 不重复，retry_count 不重复增加。
   - 对照现有 `spawn_master_pane_does_not_arm_revival_watch_before_active`，见 `src/rpc/handlers/sessions.rs:1309-1346`，确保 VERIFYING 前仍不 arm。

## 风险幂等

- double-arm 风险:
  - 同进程重复调用 startup rearm 可通过 `monitor::contains(master_monitor_key(session, generation))` 跳过。
  - 但 `monitor::register` 只是替换 fd，见 `src/monitor/mod.rs:64-71`；它不会取消旧 tokio task。因此不要用“覆盖 register”当 double-arm 策略。

- pidfd 与 patrol double-fire:
  - 当前代码 classify 在 watcher 里，cleanup 在 `revive_master_after_exit` 里；如果两条路径都在旧 generation 上进入 revive，存在重复 cleanup 风险。
  - 修复必须把“拿 session lock + 重新 classify”放进统一 `handle_master_death_detected` 入口，并在 cleanup 前执行。
  - 第二个 fire 在 lock 内应看到 pid/generation 已不匹配，返回 Stale，不进入 `clean_worker_runtime_resources_sync`。

- startup dead 与 patrol 同时触发:
  - startup rearm 在 `src/bin/ahd.rs:73-74` 之间、orchestrator 启动前执行，可避免 startup 阶段与 patrol 并发。
  - patrol loop 在 orchestrator 启动后再开始。

- master_cmd 恢复:
  - 不保存 master_cmd 就不能可靠复用现有 revive path，因为 `revive_master_after_exit` 必须传 `master_cmd` 来 spawn replacement，见 `src/monitor/master_watch.rs:270-290`。
  - `config_hash` 不是可逆配置存储；必须补持久化字段或专门 master runtime spec。

- 不碰死亡语义:
  - `IdleNoWork -> FAILED/no revive` 仍由 `src/monitor/master_watch.rs:129-138` 决定。
  - `ActiveWork -> cleanup + revive + reprovision/requeue` 仍由 `src/monitor/master_watch.rs:107-378` 决定。
  - 本设计只增加检测入口和调用来源，不引入新的 cascade 决策。

## 读过的文件

- `/tmp/research-masterwatch-fix.md`
- `src/monitor/master_watch.rs`
- `src/master_revival.rs`
- `src/rpc/handlers/sessions.rs`
- `src/db/system.rs`
- `src/db/schema.rs`
- `src/db/sessions.rs`
- `src/monitor/mod.rs`
- `src/orchestrator/mod.rs`
- `src/bin/ahd.rs`
- `src/provider/fingerprint.rs`
- `src/cli/start.rs`
- `tests/r1_master_exit_shutdown.rs`
- `tests/r2_master_scope_spawn.rs`

## 跑过的 grep / shell 命令

- `sed -n '1,260p' /tmp/research-masterwatch-fix.md`
- `rg --files`
- `rg -n "master_process_is_alive|arm_revival_watch|spawn_master_watch_task|master_watch|MasterDeath|revive|reap|orchestrator|run_once|tick|ACTIVE|master_pid" src tests`
- `rg -n "reconcile_startup_with_tmux_socket|reconcile_startup_sync_with_state_dir|startup reconcile|reconcile_startup" src/db/system.rs src/bin/ahd.rs tests`
- `rg -n "master_monitor_key|remove_master_monitor_key_if_generation_matches|confirm_master_stable|query_master_runtime" src/master_revival.rs src/monitor/master_watch.rs src/rpc/handlers/sessions.rs src/orchestrator/mod.rs`
- `rg -n "master_cmd|cmd:|Master.*cmd|ConfigRole::Master|master.*config|config_hash|sessions.*master|CREATE TABLE sessions|master_" src/db src/rpc src/cli src/master_revival.rs src/provider`
- 多次 `nl -ba ... | sed -n ...` 精确核对上文引用行号。
