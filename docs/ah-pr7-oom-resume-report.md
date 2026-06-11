# ah PR-7 OOM Resume Report

## §1 PR-7 范围与对齐目标

PR-7 承接 PR-6 的 Claude `--continue` recovery 基础，把目标扩成 OOM 后 ahd 能有意识重启、worker 能 resume 续断点。范围包含四条主线：Codex/Antigravity 的 provider 原生 resume，ahd 自身 user systemd 自愈，CRASHED recoverable worker 的 sandbox home 保留，以及 `ah start` 对既有可恢复 session 的可达性修正。

PR-7 的恢复触发者是 operator 运行 `ah up`，或 revised `ah start` 发现同 project ACTIVE session 后复用并走 `session.realign`。这不是 daemon 内部全自动 re-spawn：daemon 当前 config-blind，`agents` 表只有 `id/session_id/provider/state/state_version/pid/exit_code/error_code/created_at/sub_state/config_hash/updated_at` 等运行态字段，没有 env/hooks/plugins 列 (`src/db/schema.rs:18-30`)；而 realign 所需的 env/hooks/plugins 来自 CLI 读取 ah.toml 后传入 `session.realign` (`src/cli/up.rs:32-49`)。如果 ahd 自行 re-spawn，只能空配置恢复，属于静默坏恢复。全自动 daemon 恢复和持久化完整 spawn spec 因此 DEFER 到另案，且会是更大的 [BREAKING]。

明确非目标：Gemini resume DEFERRED，`compute_recovery_args("gemini", ...)` 返回空 (`src/provider/manifest.rs:31-35`)；`KILLED` 不进入 recovery，主动终止/管理型退出仍可清理 home；daemon 自动重生 master 与 daemon 自动 realign 已拒绝，原因同 config-blind，master pane 创建也依赖 CLI 传入 cmd/hooks/plugins。

## §2 测试拓扑

PR-7 的 tests-first 主体是 `tests/pr7_tests_first.rs`，共 27 个 `pr7_` 测试，覆盖 design §5 的 7 组。

- Provider recovery args：Codex 精确 UUID 使用真实 `.codex/sessions/.../rollout-*.jsonl` 首行 `session_meta.payload.id` 格式 (`tests/pr7_tests_first.rs:174`)，非 `session_meta` 负向回退 `resume --last` (`tests/pr7_tests_first.rs:194`)，无 rollout 也回退 `resume --last` (`tests/pr7_tests_first.rs:211`)；Antigravity 断 `--continue` (`tests/pr7_tests_first.rs:254`)；Gemini 明确 empty/deferred (`tests/pr7_tests_first.rs:264`)。
- RecoverySpawn 动态优先：Codex 动态 recovery args 先由 `compute_recovery_args` 算出再进入 `wrap_command_with_recovery` (`tests/pr7_tests_first.rs:221`)；动态 args 优先于静态 resume args (`tests/pr7_tests_first.rs:271`)；Claude 仍保留静态 `--continue` fallback (`tests/pr7_tests_first.rs:292`)；非 recovery 不追加 resume args (`tests/pr7_tests_first.rs:309`)。
- 双路径 home 保留：startup reconcile 保留 dead Codex home (`tests/pr7_tests_first.rs:326`) 且删除 dead Bash home (`tests/pr7_tests_first.rs:364`)；运行时 cleanup 保留 recoverable CRASHED home 但删除 fifo (`tests/pr7_tests_first.rs:408`)；agent_watch 崩溃路径保留 Codex home (`tests/pr7_tests_first.rs:446`)；Bash 崩溃和 Codex KILLED 仍删除 home (`tests/pr7_tests_first.rs:493`, `tests/pr7_tests_first.rs:539`)。
- Orphan GC：recovery-eligible CRASHED home/scope 视为 live ref 保留 (`tests/pr7_tests_first.rs:402`)。
- `cli.start` 复用 cutover：无既有 session 仍 create (`tests/pr7_tests_first.rs:586`)；唯一 recoverable session 改 realign 而非 create (`tests/pr7_tests_first.rs:604`)；多匹配 deterministic error (`tests/pr7_tests_first.rs:626`)；全 CRASHED ACTIVE session 仍能被 `session.list` 列出 (`tests/pr7_tests_first.rs:652`)。
- ahd systemd 自举：restartable user service command 精确断 `Restart=on-failure`、`RestartSec=1s`、`StartLimitIntervalSec=60`、`StartLimitBurst=5` (`tests/pr7_tests_first.rs:683`)；在 ahd service cgroup 内跳过递归自举 (`tests/pr7_tests_first.rs:696`)；`reset-failed` best-effort (`tests/pr7_tests_first.rs:706`)。
- Auth ladder：动态 OAuth 必须 copy 且不是 symlink (`tests/pr7_tests_first.rs:711`)；symlink 失败后 fallback copy (`tests/pr7_tests_first.rs:731`)；source missing 报 token missing (`tests/pr7_tests_first.rs:751`)；target mount/copy 失败报 mount fail (`tests/pr7_tests_first.rs:764`)。

Cutover 同步只补测试 client 路由，不削弱原断言：`tests/mvp9_acceptance.rs` 三个 start 测试 client 增加 `session.list`，分别走真实 `handle_session_list` 或返回空 sessions (`tests/mvp9_acceptance.rs:118`, `tests/mvp9_acceptance.rs:149`, `tests/mvp9_acceptance.rs:186`)；`tests/pr4c_hooks_plugins.rs` 的 recording client 增加 `session.list` 返回空 sessions (`tests/pr4c_hooks_plugins.rs:275`)。这些同步保持原 create 路径和原 payload 断言不变。

## §3 实施摘要

- Provider recovery args：`is_recovery_eligible_provider` 的集合是 `{codex, claude, antigravity}` (`src/provider/manifest.rs:27-29`)。`compute_recovery_args` 对 Claude/Antigravity 返回 `--continue`，Codex 走动态 rollout 解析，其他 provider 返回空 (`src/provider/manifest.rs:31-35`)。Codex 只扫描 `.codex/sessions` 下 `rollout-*.jsonl`，按 mtime/路径稳定排序取最新 (`src/provider/manifest.rs:61-77`, `src/provider/manifest.rs:79-100`)；只读首行 JSON，要求 `type == "session_meta"` 且 `payload.id` 为非空 UUID (`src/provider/manifest.rs:104-135`)；任何缺失/坏格式都 `tracing::warn!` 后 fallback `["resume", "--last"]` (`src/provider/manifest.rs:39-58`)。Claude 静态兜底仍是 `resume_args: &["--continue"]` (`src/provider/manifest.rs:318-325`)。
- RecoverySpawn 和 command wrapping：`RecoverySpawn { is_recovery, args }` 是动态 recovery 参数载体 (`src/sandbox/systemd.rs:7-11`)；`wrap_command_with_recovery` 接收该载体 (`src/sandbox/systemd.rs:49-58`)；最终 command 构造在 recovery 时优先追加动态 `recovery.args`，动态为空才回退 `manifest.resume_args`，非 recovery 不追加 (`src/sandbox/systemd.rs:152-175`)。生产调用点在 agent spawn：materialize home 后计算 provider recovery args，再传给 `wrap_command_with_recovery` (`src/rpc/handlers/agent.rs:80-111`)。
- 双路径 home 保留：startup reconcile 对 dead agent 先标 CRASHED，再按 provider 判断 recoverable；recoverable provider 只删 sandbox_dir、不删 cache home (`src/db/system.rs:535-561`, `src/db/system.rs:640-650`)，非 recoverable 走删除 home 的原路径 (`src/db/system.rs:619-638`)。运行时 `mark_agent_crashed_sync` 在 state 变化成功后计算 cleanup policy，并调用 policy-aware cleanup (`src/db/agents_lifecycle.rs:120-145`)；policy 由 `is_recovery_eligible_provider` 决定 (`src/db/agents_lifecycle.rs:150-155`)。registry cleanup 会 pop entry、删 fifo、kill tmux session；Default 删除 home，`PreserveRecoverableCrashedHome` 只删 sandbox_dir (`src/agent_io/registry.rs:85-124`)。`agent_watch` 后续 Default cleanup 依赖 registry 已 pop 而 no-op，这个顺序不变量已注释锁住 (`src/monitor/agent_watch.rs:82-84`)。
- Orphan GC/live-ref：recoverable CRASHED 判断集中在 `recovery_eligible_orphan_scope_should_be_preserved` (`src/db/system.rs:41-46`)；active refs SQL 把 CRASHED 且 provider in `codex/claude/antigravity` 也算作 active ref，并以注释指向 `is_recovery_eligible_provider` 为单一真相 (`src/db/system.rs:419-426`, `src/db/system.rs:439-445`)；startup crash dead pid 后也用 policy-aware cleanup (`src/db/system.rs:789-800`)。
- `cli.start` 复用 [BREAKING]：`start_project` canonicalize cwd 后先 `find_existing_start_session`，唯一匹配时调用 `session.realign` 而不是 `session.create` (`src/cli/start.rs:80-105`)。查找逻辑通过 `session.list` 过滤 `status == ACTIVE` 且 `absolute_path == canonical cwd`；0 个返回 None 走 create，1 个返回 session id，多于 1 个报 deterministic error (`src/cli/start.rs:221-249`)。realign payload 包含 master cmd/hooks/plugins 与 agent provider/env/hooks/plugins，字段形状与 `ah up` 的 realign payload 对齐 (`src/cli/start.rs:251-270`, `src/cli/up.rs:32-49`)。
- ahd systemd 自举：`ensure_daemon_running` socket 可连即短路，stale socket 则删除 (`src/bin/ah.rs:241-248`)。user systemd 可用且当前不在 ahd service cgroup 内时，先 best-effort `systemctl --user reset-failed ahd.service`，再执行 `systemd-run --user` (`src/bin/ah.rs:276-290`)；命令由 `build_ahd_systemd_run_command` 构造，包含 `--unit=ahd.service`、`Restart=on-failure`、`RestartSec=1s`、`StartLimitIntervalSec=60`、`StartLimitBurst=5` 和 `AH_STATE_DIR` (`src/cli/start.rs:29-41`)；cgroup 递归保护由 `should_skip_systemd_bootstrap_for_cgroup` 调用 systemd unit 探测 (`src/cli/start.rs:44-46`)。a3 N2 已折：`systemd-run` status 非 0 或 spawn error 会 `tracing::warn!` 并 fallback 直 spawn ahd (`src/bin/ah.rs:292-309`, `src/bin/ah.rs:326-335`)。
- Auth ladder：`AuthMaterializationErrorCode` 区分 token missing 和 sandbox mount fail，对应报告里的 `AUTH_PROVIDER_TOKEN_MISSING` / `AUTH_SANDBOX_MOUNT_FAIL` 语义 (`src/provider/home_layout.rs:37-40`)。`materialize_auth_file_with_ladder` 不绕过 `PROVIDER_AUTH_WHITELIST`，source 缺失或非 file 报 token missing，动态 OAuth 直接 copy，非动态先 symlink，symlink 失败 warn 后 copy (`src/provider/home_layout.rs:42-77`)；生产入口 `link_auth_file_into_sandbox` 调用 ladder 并仅忽略 token missing (`src/provider/home_layout.rs:381-390`)；symlink helper 与 copy helper 分别在 `src/provider/home_layout.rs:1018-1042`、`src/provider/home_layout.rs:1044-1099`，copy 后强制 `0600` 并校验 dynamic OAuth 不是 symlink。

## §4 物理断言风格

PR-7 沿用 ah 的物理断言风格，测试重点不是 mock 出“看起来对”的 JSON，而是验证真实文件、真实 cleanup、真实 argv 组成后的可观察物理结果。

Codex recovery 用真实 rollout 文件首行格式做正负断言：正向写入 `{"type":"session_meta","payload":{"id":"..."}}` 后要求 `resume <uuid>` (`tests/pr7_tests_first.rs:174`)；负向写非 `session_meta` 后要求 fallback `resume --last` (`tests/pr7_tests_first.rs:194`)。Case A home 保留用真实崩溃/cleanup 路径断 home 仍存在且 fifo 已清 (`tests/pr7_tests_first.rs:408`, `tests/pr7_tests_first.rs:446`)。Auth ladder 用 `!is_symlink()` 区分动态 OAuth copy 与普通 symlink (`tests/pr7_tests_first.rs:711`)。

负断言同样保留：Bash dead/crash 仍删 home (`tests/pr7_tests_first.rs:364`, `tests/pr7_tests_first.rs:493`)；Codex 被 KILLED 仍删 home (`tests/pr7_tests_first.rs:539`)；non-recovery command 不追加 resume args (`tests/pr7_tests_first.rs:309`)。这些断言防止 PR-7 把“CRASHED recoverable”误扩成所有退出路径。

## §5 关键设计决策

- 恢复由 operator `ah up`/revised `ah start` 触发，而不是 daemon 自动 re-spawn。核心原因是 config-blind：DB 没有完整 spawn spec，daemon 无法重建 env/hooks/plugins (`src/db/schema.rs:18-30`, `src/cli/up.rs:32-49`)。
- Session reachability 是 [BREAKING] 但必要：如果 `ah start` 永远 create，新保留的 provider home 仍不可达；因此 `ah start` 先查同 cwd ACTIVE session，唯一匹配时进入 `session.realign` (`src/cli/start.rs:88-94`, `src/cli/start.rs:221-249`)。
- Recovery 只绑定 `CRASHED`。`KILLED` / graceful shutdown 是主动管理动作，不自动 resume；测试用 `pr7_agent_watch_killed_path_deletes_codex_home` 锁住 (`tests/pr7_tests_first.rs:539`)。
- Gemini resume 显式 defer：实现返回空 (`src/provider/manifest.rs:31-35`)，测试名也写明 deferred (`tests/pr7_tests_first.rs:264`)。
- a3 audit 的 3 个 nit 已折：N1 在 active refs SQL 旁标注 provider 集合必须同步 `is_recovery_eligible_provider` (`src/db/system.rs:419-420`, `src/db/system.rs:439-440`)；N2 给 systemd 自举失败加 warn + direct spawn fallback (`src/bin/ah.rs:292-309`)；N3 在 crash policy cleanup 与 agent_watch 二次 cleanup 旁注释锁住 home 保留依赖的执行顺序 (`src/db/agents_lifecycle.rs:140-142`, `src/monitor/agent_watch.rs:82-84`)。

## §6 验收结果

- `CARGO_BUILD_JOBS=1 cargo test --test pr7_tests_first pr7_ -- --test-threads=1`：27/27 passed。
- `CCB_TEST_SKIP_REAL_PROVIDER=1 CARGO_BUILD_JOBS=1 cargo test --all-targets -- --test-threads=1`：passed，退出码 0；仅有既有 warning，例如 `src/db/system.rs` dead_code 与 `tests/pr4d_auto_provisioning.rs` unused import。
- a3 src 偏移审计结论：合格，无 must-fix；双路径 home 保留闭合，无静默失败；cutover 同步只补 RPC client 路由，原断言零削弱。
- 本 PR 是 src + 单测/集成测试层闭环；真实 OOM 端到端 availability smoke 留给 Step 3 goal-closure gate，用 ah dogfood 验证。
