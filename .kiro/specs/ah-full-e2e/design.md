# Design: ah 全流程 E2E Grand Tour 测试设计

## §1 第一性原理 + 目标

- **为什么要进行 Grand Tour**: 现有的单元测试和子系统 E2E 测试虽然达到了 95%+ 的模块覆盖率，但它们都是"模块对"（Component-level verification），无法保证产品作为一个整体能顺利交付给最终用户。Grand Tour 是"产品对"（Product-level verification），它代表了从最终用户视角出发的完整、连续、不间断的宏观业务流程。
- **与现有测试的互补性**: 子系统测试验证了状态机的细节逻辑（如某个特定 RPC 的原子性或某个钩子的物理拦截），而 Grand Tour 验证的是多个连续的状态演变在持久化、进程生命周期 and 物理副作用（文件系统）中累积时的宏观正确性，确保不发生由于长链路累积产生的非预期脑裂或资源泄漏。

## §2 核心机制思路

- **单 Happy Path (Rust Integration)**: 在 Rust 集成测试中实现一个长生命周期的 `#[test]`。该测试从完全干净的临时环境启动，依次通过多次状态调谐和 RPC 驱动，串联起从环境初始化、服务拉起、任务下发、配置变更、指纹对齐到最终安全关闭的完整生命周期，中途不重置数据库或状态目录。
- **多场景 Path (Rust Integration)**: 针对主线旅程中发生的分支行为（如指纹漂移、孤儿进程清除、并发阻塞、状态崩溃等），利用相同的集成测试脚手架构建独立的、次要的 Happy Path，模拟用户在特定产品阶段遭遇的典型异常并验证系统的鲁棒性。
- **Bash Walkthrough**: 从纯黑盒 and 真实用户的命令行视角出发。不直接通过 Rust 的 `dispatch` 模块或 RPC 客户端调用，而是通过调用编译出的 `ah` CLI 二进制执行命令。该脚本旨在提供给人类进行黑盒审查或在 CI 独立流水线中快速验证 CLI 入口的一致性。

## §3 关键决策

- **真实 LLM 调用决策 (Mock vs Real)**:
  - *决策*: 在 Grand Tour 全流程测试中**不引入真实 LLM 调用**（如不使用真实的 Claude/Gemini API），全部采用 **mock provider**（如自定义的短命命令、返回固定 Echo 的 Bash 进程）。
  - *理由*: 真实 LLM 会带来高昂的 Token 成本、显著的网络 Flakiniss，且无法在没有预设环境变量凭证的干净 CI 环境中稳定运行。真实的 LLM 校验和接口连通性应严格限制在 `mvp11_real_*` 系列等专项测试中，Grand Tour 聚焦于产品“功能链路”与“状态机制”的连通。真 LLM 链路由 mvp7/mvp9/mvp11_real_* 系列独立覆盖，本 Grand Tour 用 mock 聚焦状态机制链路与功能闭环。
- **入口选择 (ah CLI vs RPC)**:
  - *决策*: 核心的 Rust 串联测试（Happy Path & 多场景 Path）采用 **RPC/内部核心组件入口**（复用现有测试的进程管理与数据层结构）；而 Bash Walkthrough 采用 **ah CLI 入口**。
  - *理由*: 如果在 Rust 长测试中全部使用 `Command::new("ah")`，会导致测试代码充斥着大量的进程等待、标准输出解析，不仅难以捕获精细的 DB 状态进行断言，还会由于大量的 TTY 嵌套导致测试脆弱。用内部核心入口保证断言的深度和物理透明度，用 Bash 脚本保证最外层 CLI 的交付质量。

## §4 用户旅程矩阵 (主线 + 分支)

### 主线 Happy Path 流程 (13 个 ah 命令 + 1 个配置改动)

1. **ah start**: CLI `Cmd::Start` / match at `src/bin/ah.rs:42` and `src/bin/ah.rs:143`。
   真实 RPC 链为 `session.create`、`session.spawn_master_pane`、`agent.spawn`，router 注册在 `src/rpc/router.rs:14`, `src/rpc/router.rs:16`, `src/rpc/router.rs:19`。
   CLI 调用在 `src/cli/start.rs:54-62`, `src/cli/start.rs:73-84`, `src/cli/start.rs:104-117`。
   四维断言: OS 侧 tmux master/agent session 存在；SQLite 触 `sessions.status`, `sessions.config_hash`, `agents.state`, `agents.pid`, `agents.config_hash`, `events.event_type/payload`；FS 触 `$state_dir/sandboxes/<session>/<agent>` 与 provider HOME；外层返回 `session_id`, `pane_id`, `state=SPAWNING`, `pid`，shape 见 `src/rpc/handlers.rs:71-99`, `src/rpc/handlers.rs:309`, `src/rpc/handlers.rs:899`。
2. **ah ping**: CLI `Cmd::Ping` / match at `src/bin/ah.rs:36` and `src/bin/ah.rs:137`。
   RPC 为 `system.dump`，router 注册 `src/rpc/router.rs:34`，CLI 调用 `src/bin/ah.rs:341-348`。
   四维断言: OS 不新增进程；SQLite dump 覆盖 `projects/sessions/agents/evidence_pending`；FS 不变；外层 stdout `ok=true`, `sessions=N agents=N`，返回 shape 见 `src/db/system.rs:119-125`。
3. **ah ask**: CLI `Cmd::Ask` / match at `src/bin/ah.rs:52` and `src/bin/ah.rs:161-166`。
   RPC 为 `job.submit`，router 注册 `src/rpc/router.rs:31`，CLI 调用 `src/bin/ah.rs:409-430`。
   四维断言: OS pane 收到输入并保持活跃；SQLite 触 `jobs.status=QUEUED` 后 orchestrator 派发，`events.event_type=command_received/state_change`；FS 暂无新增证据文件；外层返回 `job_id`, `status=QUEUED`，shape 见 `src/rpc/handlers.rs:902-933`。
4. **ah ps**: CLI `Cmd::Ps` / match at `src/bin/ah.rs:40` and `src/bin/ah.rs:142`。
   RPC 为 `session.list` + `system.dump`，router 注册 `src/rpc/router.rs:18`, `src/rpc/router.rs:34`，CLI 调用 `src/bin/ah.rs:351-370`。
   四维断言: OS 状态与 DB `agents.pid` 对齐；SQLite 读 `sessions.status/active_agents` 与 `agents.state/sub_state/state_version/pid`；FS 不变；外层返回 session table + agent table，JSON shape 见 `src/rpc/handlers.rs:312-328` 与 `src/db/system.rs:80-125`。
5. **ah pend**: CLI `Cmd::Pend` / match at `src/bin/ah.rs:61` and `src/bin/ah.rs:167`。
   RPC 为 `job.wait`，router 注册 `src/rpc/router.rs:32`，CLI 调用 `src/bin/ah.rs:433-435`。
   四维断言: OS pane 完成 mock 输出；SQLite 触 `jobs.status=COMPLETED|FAILED|CANCELLED`, `agents.state=IDLE`, `events.event_type=output_chunk/state_change/evidence_denied`；FS 视 evidence 要求检查落盘；外层 terminal job shape 来自 `src/rpc/handlers.rs:936-974`。
6. **ah logs**: CLI `Cmd::Logs` / match at `src/bin/ah.rs:79` and `src/bin/ah.rs:178`。
   无独立 `agent.logs` RPC，grep miss: `agent.logs` 不在 `src/rpc/router.rs:13-35`，实现应复用 `agent.watch` 或 `agent.read` 读取 `events`。
   四维断言: OS 不变；SQLite 读 `events.event_type=output_chunk` 与 `events.payload`；FS 不变；外层输出 text，`output_chunk` 采集见 `src/agent_io/reader.rs:136-144`，job reply 查询见 `src/db/jobs.rs:466-485`。
7. **修改配置**: 非 ah 命令，动态改写项目 `ah.toml`，制造 hooks/plugins/cmd/env drift。
   四维断言: OS 尚未重启；SQLite `sessions.config_hash` / `agents.config_hash` 仍为旧值；FS 只改变项目配置和 fixture hook；外层无 RPC return。
   指纹测试复用 `ConfigFingerprintInput` pattern at `tests/pr4e_up_fingerprint.rs:54-85`。
8. **ah up**: CLI `Cmd::Up` / match at `src/bin/ah.rs:47` and `src/bin/ah.rs:144-156`。
   RPC 为 `session.realign`，router 注册 `src/rpc/router.rs:17`，CLI 调用 `src/cli/up.rs:12-42`。
   四维断言: OS 对 drift agent 执行 spawn/kill，busy 且非 force 时不杀；SQLite 触 `sessions.config_hash`, `agents.config_hash`, `agents.state`, `events.event_type=drift_skipped/drift_realigned/agent_spawned/agent_killed`；FS 重新物化 provider HOME/hooks/plugins；外层返回 `{ "statuses": [...] }`，shape 见 `src/rpc/handlers.rs:450-558`。
9. **ah ask**: 同第 3 步，第二次提交任务用于验证新配置/新 hook 已生效。
   四维断言额外要求 `events.payload` 中命令与 request_id 可追踪，`agents.config_hash` 已不同于第 3 步前快照；外层仍为 `job_id/status`。
10. **ah prompt resolve**: CLI `Cmd::Prompt::Resolve` / match at `src/bin/ah.rs:98-127` and `src/bin/ah.rs:186-203`。
    RPC 为 `agent.resolve_prompt`，router 注册 `src/rpc/router.rs:25`，CLI 调用 `src/cli/prompt.rs:14-30`。
    四维断言: OS pane 收到 action/keys；SQLite 触 `agents.state=PROMPT_PENDING -> IDLE|BUSY`, `events.event_type=state_change`，可选 `prompt_experience`；FS 可选 `prompt-cases.json`；外层返回 `status`, `resolved_state`, `saved_to_kb`, `case_id`, `action_sent`，shape 见 `src/rpc/handlers.rs:1305-1350`。
11. **ah cancel**: CLI `Cmd::Cancel` / match at `src/bin/ah.rs:63` and `src/bin/ah.rs:168`。
    RPC 为 `job.cancel`，router 注册 `src/rpc/router.rs:33`，CLI 调用 `src/bin/ah.rs:438-450`。
    四维断言: OS queued 取消不碰 pane，dispatched 取消发 ctrl-c；SQLite 触 `jobs.cancel_requested`, `jobs.status=CANCELLED|CANCEL_REQUESTED`, `events.event_type=state_change/output_chunk`；FS 不变；外层返回 `job_id/status`，shape 见 `src/rpc/handlers.rs:976-1010`。
12. **ah watch**: CLI `Cmd::Watch` / match at `src/bin/ah.rs:73` and `src/bin/ah.rs:174-177`。
    RPC 为 `agent.watch`，router 注册 `src/rpc/router.rs:23`，CLI 调用 `src/bin/ah.rs:480-520`。
    四维断言: OS 不变；SQLite 读 `events.seq_id/event_type/payload`；FS 不变；外层返回 `{ "events": [...], "is_truncated": false }`，shape 见 `src/rpc/handlers.rs:1368-1424`。
13. **ah kill**: CLI `Cmd::Kill` / match at `src/bin/ah.rs:65` and `src/bin/ah.rs:169-173`。
    RPC 为 `agent.kill` 或 `session.kill`，router 注册 `src/rpc/router.rs:24`, `src/rpc/router.rs:15`，CLI 调用 `src/bin/ah.rs:453-477`。
    四维断言: OS agent tmux session/pid 消失；SQLite 触 `agents.state=KILLED`, `events.event_type=state_change` payload reason；FS 删除 `$state_dir/sandboxes/<session>/<agent>`；外层返回 `state=KILLED`，shape 见 `src/rpc/handlers.rs:101-165` and `src/rpc/handlers.rs:1092-1121`。
14. **ah stop**: CLI `Cmd::Stop` / match at `src/bin/ah.rs:90` and `src/bin/ah.rs:180`。
    RPC 为 `system.shutdown`，router 注册 `src/rpc/router.rs:35`，CLI 调用 `src/bin/ah.rs:335-338`。
    四维断言: OS daemon、master、agent tmux session 被收割；SQLite active agent 不再活跃，已杀 agent 保留 KILLED 记录；FS 清理运行期 socket/pipes/sandbox；外层返回 `{ "status": "shutting_down" }`，shape 见 `src/rpc/handlers.rs:1458-1464`。

*注: 主线显式排除以下命令: (a) `ah attach`，TTY 交互式 attach 到 agent pane，自动化断言价值低，在 §6 M1 Bash walkthrough 中做 smoke；(b) Doctor/Config validate/Migrate/Version 非 Runtime 命令，在 §6 M1 Bash walkthrough 中覆盖。*

*grep miss: `session.start` / `session.up` / `agent.ask` / `daemon.ping` / `agent.logs` / `session.stop` / `session.ps` 均不在 router method 表内，真实 method 以 `src/rpc/router.rs:13-35` 为准。*

### 分支 Path 矩阵

- **DRIFT**: 变更物理环境中的 hooks / plugins / cmd，调用调谐流验证指纹变动并正确触发自动 realign。
  断言 `events.event_type=drift_realigned`, `agents.config_hash` 更新。
  handler 分支见 `src/rpc/handlers.rs:475-520`。
- **ORPHAN**: 在 `ah.toml` 中显式删除某 agent 块，执行调谐，验证系统在审计模式或强制模式下能安全收割被遗弃的孤儿 Agent。
  断言 audit-only 返回 `status=ORPHAN`，force 返回 `action=KILLED` 并写 `agent_killed`。
  handler 分支见 `src/rpc/handlers.rs:523-555`。
- **NEW**: 在 `ah.toml` 中追加未声明的 agent 块，执行调谐流，验证能无缝 spawn 并物化该新 Agent 而不干扰现有活跃 Session。
  断言 `status=NEW`, `action=spawned`, `events.event_type=agent_spawned`。
  handler 分支见 `src/rpc/handlers.rs:457-465`, `src/rpc/handlers.rs:618-624`。
- **BUSY**: 当 Agent 处于 `BUSY` 状态（任务正在执行）时触发配置更新，验证系统能根据策略选择跳过 (skip) 或在加了强制参数 (`--force`) 时能安全收割重来。
  断言非 force `status=SKIPPED_BUSY` + `drift_skipped`。
  force 路径经 `DRIFT_FORCE_REALIGN`。
- **ERROR 恢复**: 制造 provider 启动失败（如返回非零退出码），验证 Agent 状态机转为 `CRASHED`；随后清理物理环境并重新进行调谐，验证系统能从崩溃状态平滑恢复到就绪。
  断言 `agents.error_code/exit_code` 与 `state_change` payload。
  crash 写入见 `src/db/agents_lifecycle.rs:65-120`。

## §5 四维断言细化

- **OS 进程树结构**: 复用 `TmuxServerGuard` 建立隔离 tmux socket，定义 `has_session(server, name)` / `list-panes` 辅助。
  现有 pattern: `tmux has-session` 在 `tests/r1_session_lifecycle.rs:26-39`, `tests/r1_session_lifecycle.rs:140-153`, `tests/r1_shutdown_cleanup.rs:31-43`。
  pane 枚举在 `tests/mvp6_acceptance.rs:189`, `tests/mvp6_acceptance.rs:338`；`/proc/<pid>` 存活/comm/cgroup 断言在 `tests/mvp2_acceptance.rs:152-170`, `tests/mvp10_acceptance.rs:77`。
- **SQLite SoT**: schema 以 `src/db/schema.rs` 为准。
  `sessions` 字段必须覆盖 `id/project_id/master_pid/master_pane_id/status/config_hash/created_at` (`src/db/schema.rs:8-16`)。
  `agents` 字段必须覆盖 `id/session_id/provider/state/state_version/pid/exit_code/error_code/sub_state/config_hash/updated_at` (`src/db/schema.rs:18-31`)。
  `events` 实际字段为 `event_type` + `payload`，不是 `payload_json`，schema 在 `src/db/schema.rs:35-42`。
- **events.kind 期望值**: 设计里统一写作 `events.event_type`。主线至少覆盖 `state_change` (`src/db/state_machine.rs:151-155`), `command_received` (`src/rpc/handlers.rs:1191-1197`), `output_chunk` (`src/agent_io/reader.rs:136-144`), `evidence_denied` (`src/db/state_machine.rs:418-422`), `evidence_recorded` (`src/db/evidence.rs:42-46`), `drift_skipped` / `drift_realigned` (`src/rpc/handlers.rs:477-512`), `agent_spawned` (`src/rpc/handlers.rs:618-624`), `agent_killed` (`src/rpc/handlers.rs:534-540`)。
- **Filesystem 物理副作用**: sandbox 根由 `state_dir/sandboxes/<session_id>/<agent_id>` 创建，见 `src/sandbox/path.rs:6-20`。provider HOME 映射为 `$XDG_CACHE_HOME/ah/sandboxes/<hash>`，见 `src/provider/home_layout.rs:603-618`；`HOME` + `CLAUDE_CONFIG_DIR` / `GEMINI_CLI_HOME` / `CODEX_HOME` 注入见 `src/provider/home_layout.rs:116-119`, `src/provider/home_layout.rs:142-145`, `src/provider/home_layout.rs:163-165`。
- **Auth symlink + rules + extension 物化**: OAuth/凭据白名单为 `.claude/.credentials.json`, `.codex/auth.json`, `.gemini/oauth_creds.json` 等，见 `src/provider/home_layout.rs:12-20`；auth 文件以 symlink 进入 sandbox，见 `src/provider/home_layout.rs:204-210`, `src/provider/home_layout.rs:248-255`, `src/provider/home_layout.rs:847-870`；per-provider rules 路径为 `.claude/CLAUDE.md`, `.gemini/GEMINI.md`, `.codex/AGENTS.md`，见 `src/provider/home_layout.rs:187-201`；hook 和 plugin symlink 物化见 `src/provider/home_layout.rs:323-356`, `src/provider/home_layout.rs:497-519`, `src/provider/home_layout.rs:561-579`。
- **证据链文件路径**: DB evidence 表字段为 `id/agent_id/event_seq_id/pane_bytes/failed_rules/status/l3_asserted_state/job_id/evidence_type/subject_path/payload` (`src/db/schema.rs:47-60`)；pane death 文件路径为 `$state_dir/evidence/<agent_id>/pane_at_death.txt` (`src/agent_io/registry.rs:131-142`)；PR1a 的证据门控断言 pattern 覆盖 `evidence_denied` 和 `diff_generated/test_passed`，见 `tests/pr1a_evidence_statemachine.rs:48-67`, `tests/pr1a_evidence_statemachine.rs:88-118`, `tests/pr1a_evidence_statemachine.rs:145-166`。
- **外层 RPC return**: `dispatch` 返回 JSON-RPC envelope，已在 router 测试覆盖未知 method 和已注册 method (`src/rpc/router.rs:182-205`)；Grand Tour 内部测试断言 result payload，Bash walkthrough 断言 CLI stdout/stderr。关键 shape: `system.dump` returns `projects/sessions/agents/evidence_pending/monitors` (`src/db/system.rs:119-125`)；`agent.watch` returns `events/is_truncated` (`src/rpc/handlers.rs:1390-1424`)；`session.realign` returns `statuses` (`src/rpc/handlers.rs:557`)。

## §6 实施切片大方向

- **M1: Bash Walkthrough 验证脚本** (工作量: 约 100-200 行 Shell)
  - 新增 `scripts/ah-full-e2e/walkthrough.sh`，完全依赖 `target/debug/ah` 和 `target/debug/ccbd`，串联 `start/ping/ask/ps/pend/logs/up/prompt resolve/cancel/watch/kill/stop`，附带 `attach --help`、`doctor`、`config validate`、`config migrate`、`version` smoke。该 lane 不进默认 `cargo test`，用于 Nightly 或人工验收。
  - 共用 fixture: `tests/fixtures/mock_prompt_provider.sh` 作为 mock provider 基础；脚本自建临时 `ah.toml`、hook 文件、host HOME auth 文件，断言 `$XDG_STATE_HOME/ah/state/<project>/ccbd.sqlite` 与 `$XDG_CACHE_HOME/ah/sandboxes/<hash>`。
- **M2: Rust 宏观单 Happy Path 集成测试** (工作量: 约 500-700 LOC)
  - 新增 `tests/ah_full_e2e_main.rs`，复用 `tests/common/mod.rs:56-88` 的 `TmuxServerGuard`、`ccbd::rpc::Ctx`、`ccbd::rpc::router::dispatch`。PR4e harness 可直接作为骨架: `tests/pr4e_up_fingerprint.rs:18-50` 初始化 `Ctx/db/state_dir/project_dir/tmux_guard`，`tests/pr4e_up_fingerprint.rs:70-85` 复用 fingerprint input pattern。
  - 主线每一步都通过 helper `rpc(method, params)` + DB query + tmux assertion + filesystem assertion 实现；`#[ignore]` 标记，默认不阻塞开发循环。
- **M3: Rust 多分支扩展路径测试** (工作量: 约 150-250 LOC / 每个主要分支)
  - 新增 `tests/ah_full_e2e_drift.rs` 覆盖 DRIFT/NEW，复用 `tests/pr4e_up_fingerprint.rs` 的 realign 语义。
  - 新增 `tests/ah_full_e2e_lifecycle.rs` 覆盖 ORPHAN/BUSY/ERROR，复用 R1 session cleanup pattern: `tests/r1_session_lifecycle.rs:98-163`, `tests/r1_shutdown_cleanup.rs:74-147`。
  - 新增 `tests/ah_full_e2e_evidence.rs` 覆盖 evidence gate 与 prompt resolve，复用 PR1a pattern `tests/pr1a_evidence_statemachine.rs:28-67` 和 prompt fixture `tests/fixtures/mock_prompt_provider.sh`。

## §7 决议汇总

- **§7.1 CI lane**: Grand Tour 测试用 `#[ignore]` 标记，默认 `cargo test` 跳过，CI Nightly Lane + 本地 `cargo test --include-ignored` 执行。决议: 主控+a1 自决，不抛 PM。理由: 主控 vibecoding 反馈循环不被 Grand Tour 拖累。
- **§7.2 分支覆盖范围**: 按 PM "所有功能都测试一遍" 诉求，5 分支 (DRIFT/ORPHAN/NEW/BUSY/ERROR) 必须在 PR 系列内全覆盖 (M3 分批落实)。决议: 主控自驱分批策略 (建议 PR-1 M1+M2 主线; PR-2 M3 DRIFT+NEW; PR-3 M3 ORPHAN+BUSY+ERROR)，不抛 PM。
- **§7.3 工作量估算**: 按 §4 主线扩充到 14 步后，M2 单 Happy Path 实际 500-700 LOC (含 Harness ~100 + RPC 调用 ~200 + 四维断言 ~300)。M3 每分支 150-250 LOC。M1 Bash walkthrough 100-200 行。决议: 主控自决估算，不抛 PM。
