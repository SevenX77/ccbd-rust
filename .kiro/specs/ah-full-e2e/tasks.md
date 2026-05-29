# Tasks: ah 全流程 E2E Grand Tour PR-1

## Scope Guard

PR-1 只落地 M1 + M2:

- M1: `scripts/ah-full-e2e/walkthrough.sh`，黑盒 Bash walkthrough，不进入默认 `cargo test`。
- M2: `tests/ah_full_e2e_main.rs`，单 Happy Path Rust 主线集成测试，`#[ignore]` 标记，默认 `cargo test` 跳过。
- M3 分支矩阵 DRIFT/ORPHAN/NEW/BUSY/ERROR 的专项文件留 PR-2/PR-3，不在 PR-1 新建。
- 本任务清单按 test-first 顺序执行；先立红灯和 harness，再补实现/脚本，不改 `src/` 主逻辑，除非红灯证明当前产品入口不可用且另开修复任务。

## 主线 14 步覆盖清单

PR-1 M2 + M1 必须串联同一条状态链，中途不重置 DB / state dir:

1. `ah start`
2. `ah ping`
3. `ah ask`
4. `ah ps`
5. `ah pend`
6. `ah logs`
7. 修改 `ah.toml`
8. `ah up`
9. `ah ask`
10. `ah prompt resolve`
11. `ah cancel`
12. `ah watch`
13. `ah kill`
14. `ah stop`

## Step A: M2 Rust Harness 骨架

### T1: Add ignored Rust Grand Tour test skeleton

- 文件: Add `tests/ah_full_e2e_main.rs`
- 依赖: 无
- 内容:
  - 新增 `mod common;`
  - 新增单个 `#[tokio::test(flavor = "multi_thread")] #[ignore] async fn grand_tour_mainline_13_ah_commands_plus_config_drift()`
  - 先只 `panic!("red: grand tour mainline not implemented")` 或 `assert!(false, ...)`，确认 ignored test 能被显式执行并红灯。
- 验收标准:
  - 默认跳过: `CARGO_BUILD_JOBS=1 cargo test --test ah_full_e2e_main -- --test-threads=1`
  - 显式红灯: `CARGO_BUILD_JOBS=1 cargo test --test ah_full_e2e_main -- --include-ignored --test-threads=1`
  - 红灯信号: test 名出现，失败信息包含 `grand tour mainline not implemented`。
  - 绿灯目标: T2-T7 完成后同一命令通过。
- audit 视角:
  - a1 检查 `#[ignore]` 存在，避免默认 lane 变慢。
  - a3 检查文件没有复制 PR4e 大段无用逻辑，只有最小红灯骨架。

### T2: Implement M2 harness helpers

- 文件: Modify `tests/ah_full_e2e_main.rs`
- 依赖: T1
- 内容:
  - 复用 `tests/common/mod.rs` 的 `TmuxServerGuard`。
  - 按 `tests/pr4e_up_fingerprint.rs:18-50` 形态建立 `Harness { ctx, _tmux_guard, _db_file, _state_dir, _project_dir }`。
  - helper: `rpc(method, params)` 调 `ccbd::rpc::router::dispatch`，解析 JSON-RPC result/error。
  - helper: DB query helpers 覆盖 `agents.state`, `agents.config_hash`, `sessions.config_hash`, `events.event_type`, `events.payload`, `jobs.status`。
  - helper: tmux assertion 覆盖 session 存在/不存在，参考 `tests/r1_session_lifecycle.rs:26-39`。
- 验收标准:
  - 命令: `CARGO_BUILD_JOBS=1 cargo test --test ah_full_e2e_main -- --include-ignored --test-threads=1`
  - 红灯信号: harness 编译通过，但主线仍在第一个未实现 step 失败。
  - 绿灯信号: helper 自测或主 test 可调用 `system.dump` 并拿到 JSON result。
  - 不允许访问真实 HOME；需要 temp `HOME` / `XDG_CACHE_HOME` fixture。
- audit 视角:
  - a1 检查 dispatch helper 对 JSON-RPC `error` 不静默吞掉。
  - a3 检查 DB helper 查询字段来自 `src/db/schema.rs`，尤其 `events.event_type` / `events.payload`，不是 `payload_json`。

### T3: Add mock provider fixture

- 文件: Add `tests/fixtures/mock_provider.sh`; Modify `tests/ah_full_e2e_main.rs`
- 依赖: T2
- 内容:
  - 新增短命 echo provider，风格参考 `tests/fixtures/mock_prompt_provider.sh`。
  - 支持输出 ready marker、回显 ask 文本、输出 shell-like idle marker，保证 marker timer 可把 agent 回到 `IDLE`。
  - 在 Rust harness 中通过 temp `ah.toml` 或 RPC spawn params 指向 fixture。
- 验收标准:
  - 命令: `bash tests/fixtures/mock_provider.sh <<< 'grand-tour-smoke'`
  - 命令: `CARGO_BUILD_JOBS=1 cargo test --test ah_full_e2e_main -- --include-ignored --test-threads=1`
  - 红灯信号: fixture 可执行后，主线失败点从 provider spawn/ready 前移到未实现的业务 step。
  - 绿灯信号: `agent.spawn` 后 `agents.state` 能稳定到 `IDLE`，`events.event_type` 至少出现 `output_chunk` 或 startup `state_change`。
- audit 视角:
  - a1 检查 fixture 不调用网络、不读真实 token、不依赖真实 LLM CLI。
  - a3 检查 fixture 权限和 shebang，避免 CI 上 `permission denied`。

## Step B: M2 主线 14 步落地

### T4: Implement steps 1-3: start / ping / ask

- 文件: Modify `tests/ah_full_e2e_main.rs`
- 依赖: T3
- 覆盖 step: 1 `ah start`, 2 `ah ping`, 3 `ah ask`
- 内容:
  - 通过 RPC 模拟 `ah start` 链: `session.create` -> `session.spawn_master_pane` -> `agent.spawn`。
  - 通过 `system.dump` 模拟 `ah ping`。
  - 通过 `job.submit` 模拟 `ah ask`。
- 验收标准:
  - 命令: `CARGO_BUILD_JOBS=1 cargo test --test ah_full_e2e_main -- --include-ignored --test-threads=1`
  - 红灯信号: 未实现前在 step 1-3 断言失败。
  - 绿灯信号: `sessions.status=ACTIVE`, `agents.state in (SPAWNING, IDLE, WAITING_FOR_ACK, BUSY)`, `jobs.status=QUEUED`, `system.dump` 返回 `projects/sessions/agents/evidence_pending/monitors`。
  - OS 断言: master/agent tmux session 存在。
  - FS 断言: `$state_dir/sandboxes/<session>/<agent>` 或 provider HOME 已物化。
- audit 视角:
  - a1 检查 method 名必须来自 `src/rpc/router.rs:13-35`。
  - a3 检查 start 链没有用不存在的 `session.start`。

### T5: Implement steps 4-6: ps / pend / logs

- 文件: Modify `tests/ah_full_e2e_main.rs`
- 依赖: T4
- 覆盖 step: 4 `ah ps`, 5 `ah pend`, 6 `ah logs`
- 内容:
  - `session.list` + `system.dump` 模拟 `ah ps`。
  - `job.wait` 模拟 `ah pend`。
  - `agent.watch` 或 `agent.read` 模拟 `ah logs`，因为 router 没有 `agent.logs`。
- 验收标准:
  - 命令: `CARGO_BUILD_JOBS=1 cargo test --test ah_full_e2e_main -- --include-ignored --test-threads=1`
  - 红灯信号: 未实现前停在 `job.wait` timeout 或 logs 无 `output_chunk`。
  - 绿灯信号: `jobs.status=COMPLETED`, `agents.state=IDLE`, `events.event_type` 包含 `command_received`, `output_chunk`, `state_change`。
  - 外层 shape: `agent.watch/read` 返回 `events` 数组与 `is_truncated=false` 或等价读取结果。
- audit 视角:
  - a1 检查 `ah logs` 设计明确映射到 `agent.watch`/`agent.read`，不硬造 `agent.logs`。
  - a3 检查 `job.wait` timeout 有明确上限，避免 CI 卡死。

### T6: Implement steps 7-9: toml drift / up / second ask

- 文件: Modify `tests/ah_full_e2e_main.rs`
- 依赖: T5
- 覆盖 step: 7 修改 `ah.toml`, 8 `ah up`, 9 第二次 `ah ask`
- 内容:
  - 修改 temp project 的 `ah.toml` 或等价 config source，制造 hook/plugin/cmd/env drift。
  - 调 `session.realign` 模拟 `ah up`。
  - 再次 `job.submit`，验证 realign 后新配置生效。
- 验收标准:
  - 命令: `CARGO_BUILD_JOBS=1 cargo test --test ah_full_e2e_main -- --include-ignored --test-threads=1`
  - 红灯信号: 未实现前 `sessions.config_hash` / `agents.config_hash` 不变，或没有 `drift_realigned` / `agent_spawned`。
  - 绿灯信号: `sessions.config_hash` 或 `agents.config_hash` 与 drift 前快照不同；`session.realign` 返回 `statuses`；`events.event_type` 包含 `drift_realigned` 或 `agent_spawned`。
  - 第二次 ask 生成新的 `job_id`，不能复用第一次 job。
- audit 视角:
  - a1 检查 config_hash 断言直接查询 DB 字段，不只看 RPC 文本。
  - a3 检查 drift 事件名真实存在，grep 自 handler，不写 `events.kind`。

### T7: Implement steps 10-14: prompt resolve / cancel / watch / kill / stop

- 文件: Modify `tests/ah_full_e2e_main.rs`
- 依赖: T6
- 覆盖 step: 10 `ah prompt resolve`, 11 `ah cancel`, 12 `ah watch`, 13 `ah kill`, 14 `ah stop`
- 内容:
  - 用 `tests/fixtures/mock_prompt_provider.sh` 或 T3 fixture 的 prompt mode 触发 `PROMPT_PENDING`，调用 `agent.resolve_prompt`。
  - 创建 queued 或 dispatched job，调用 `job.cancel`。
  - 调 `agent.watch` 读取事件流。
  - 调 `agent.kill`，最后调 `system.shutdown` 或 session-level cleanup helper 完成 stop 断言。
- 验收标准:
  - 命令: `CARGO_BUILD_JOBS=1 cargo test --test ah_full_e2e_main -- --include-ignored --test-threads=1`
  - 红灯信号: 未实现前缺 `resolved_state`, `CANCELLED|CANCEL_REQUESTED`, `events`, 或 agent 仍存活。
  - 绿灯信号: `agent.resolve_prompt` 返回 `status=ok` 与 `resolved_state`；`job.cancel` 返回 `job_id/status`；`agent.watch` 返回非空 `events` 或 timeout 空数组可解释；`agent.kill` 后 `agents.state=KILLED` 且 sandbox dir 删除；stop 后 daemon/session runtime 被收割。
  - 关键 event_type: `state_change`, `command_received`, `output_chunk`, `drift_realigned|agent_spawned` 至少出现在同一条主线 DB 中。
- audit 视角:
  - a1 检查 stop 断言不误杀测试外 tmux server。
  - a3 检查 prompt resolve 没有依赖真实 provider TTY。

## Step C: M1 Bash Walkthrough

### T8: Add Bash walkthrough scaffold

- 文件: Add `scripts/ah-full-e2e/walkthrough.sh`
- 依赖: T3
- 内容:
  - 脚本自建 temp project、temp HOME、temp XDG state/cache。
  - 构造 `ah.toml` 指向 `tests/fixtures/mock_provider.sh` 和必要 hook fixture。
  - 提供 helpers: `run_ah`, `wait_for`, `assert_contains`, `sqlite_query`。
  - 默认不进入 `cargo test`，只作为人工/Nightly smoke 脚本。
- 验收标准:
  - 命令: `bash -n scripts/ah-full-e2e/walkthrough.sh`
  - 命令: `CARGO_BUILD_JOBS=1 cargo build --bins`
  - 红灯信号: 脚本 scaffold 存在但未串命令时输出 `TODO mainline` 或明确失败。
  - 绿灯信号: shellcheck 如本地有则通过；无 shellcheck 时 `bash -n` 通过。
- audit 视角:
  - a1 检查脚本没有写真实 `~/.claude`, `~/.codex`, `~/.gemini`。
  - a3 检查 cleanup trap 覆盖 daemon、tmux、temp dir。

### T9: Implement walkthrough mainline 14 steps and extra smoke

- 文件: Modify `scripts/ah-full-e2e/walkthrough.sh`
- 依赖: T8
- 覆盖 step: Bash 黑盒执行同一主线 14 步
- 内容:
  - 串联 `ah start`, `ah ping`, `ah ask`, `ah ps`, `ah pend`, `ah logs`, 修改 `ah.toml`, `ah up`, 第二次 `ah ask`, `ah prompt resolve`, `ah cancel`, `ah watch`, `ah kill`, `ah stop`。
  - 附加 smoke: `ah attach --help` 或安全替代、`ah doctor`, `ah config validate`, `ah config migrate`, `ah version`。
  - 对 stdout/stderr、SQLite、FS 关键点做轻量断言。
- 验收标准:
  - 命令: `CARGO_BUILD_JOBS=1 cargo build --bins`
  - 命令: `bash scripts/ah-full-e2e/walkthrough.sh`
  - 红灯信号: 每个未落地命令必须在脚本中以 step name 标出失败，例如 `FAIL step 8 ah up`。
  - 绿灯信号: 输出 `PASS grand tour walkthrough`；SQLite 中可查到 `state_change`, `command_received`, `output_chunk`，drift 后可查到 `drift_realigned` 或 `agent_spawned`。
- audit 视角:
  - a1 检查 walkthrough 使用真实 `ah` CLI，而不是直接 RPC。
  - a3 检查 extra smoke 不阻塞交互式 attach，不打开长期 TTY。

## Step D: PR-1 Documentation Notes

### T10: Document local run commands inside tasks follow-up notes

- 文件: Modify `.kiro/specs/ah-full-e2e/tasks.md`
- 依赖: T7, T9
- 内容:
  - 在任务完成记录中保留最终本地运行命令。
  - 不修改 `CLAUDE.md` / `docs/`，除非 PR review 明确要求；PR-1 scope 以 M1+M2 为准。
- 验收标准:
  - Rust 主线命令: `CARGO_BUILD_JOBS=1 cargo test --test ah_full_e2e_main -- --include-ignored --test-threads=1`
  - Bash walkthrough 命令: `bash scripts/ah-full-e2e/walkthrough.sh`
  - 红灯信号: 缺命令或命令与实际文件名不一致。
  - 绿灯信号: 两条命令可复制执行，且默认 `cargo test` 不运行 ignored grand tour。
- audit 视角:
  - a1 检查 tasks.md 不承诺 PR-2/PR-3 分支已完成。
  - a3 检查 CI Nightly workflow 未在 PR-1 scope 中被偷偷加入。

## PR-1 Final Verification

- Rust mainline: `CARGO_BUILD_JOBS=1 cargo test --test ah_full_e2e_main -- --include-ignored --test-threads=1`
- Bash walkthrough: `bash scripts/ah-full-e2e/walkthrough.sh`
- Default lane safety: `CARGO_BUILD_JOBS=1 cargo test --test ah_full_e2e_main -- --test-threads=1` 应显示 ignored test skipped。
- Grep checks:
  - `rg -n "ah_full_e2e_main|mock_provider.sh|walkthrough.sh" .kiro/specs/ah-full-e2e/tasks.md`
  - `rg -n "ah start|ah ping|ah ask|ah ps|ah pend|ah logs|修改 ah.toml|ah up|ah prompt resolve|ah cancel|ah watch|ah kill|ah stop" .kiro/specs/ah-full-e2e/tasks.md`
  - `rg -n "DRIFT|ORPHAN|NEW|BUSY|ERROR" tests/ah_full_e2e_main.rs scripts/ah-full-e2e/walkthrough.sh` 应只出现主线 drift，不出现 M3 专项分支实现。
