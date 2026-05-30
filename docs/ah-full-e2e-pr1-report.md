# ah Full E2E PR-1 Report

## §1 PR 目标

本 PR 落地 Grand Tour E2E 测试 PR-1，范围限定为 M1 + M2：M2 是单 Happy Path Rust 集成测试，M1 是真实 `ah` CLI 黑盒 Bash walkthrough；M3 的 DRIFT / ORPHAN / NEW / BUSY / ERROR 分支矩阵留给 PR-2/PR-3。目标来自 PM 原话：“把整个流程所有功能都测试一遍”。这里的“全流程”不是替代已有子系统测试，而是把 `start -> ask -> pend/logs -> config drift -> up -> second ask -> prompt/cancel/watch -> kill -> stop` 串成同一条状态链，验证长生命周期中 DB、tmux、FS 副作用和 CLI/RPC return shape 不互相背离。

## §2 落地清单

- M2 单 Happy Path Rust 集成测试：`tests/ah_full_e2e_main.rs`
- M1 Bash walkthrough：`scripts/ah-full-e2e/walkthrough.sh`
- Mock fixture：`tests/fixtures/mock_provider.sh`
- spec 4 件套：`.kiro/specs/ah-full-e2e/{research,design,tasks}.md`

M2 的实现方式是内部 RPC + 物理断言。`Harness::rpc` 直接调用 `ccbd::rpc::router::dispatch`，收到 JSON-RPC `error` 会 panic，不吞失败；DB helper 明确读取 `sessions.status`, `sessions.config_hash`, `agents.state`, `agents.config_hash`, `jobs.status`, `events.event_type`, `events.payload`，字段名和 schema 对齐，不使用不存在的 `payload_json` 或 `events.kind`。

M1 的实现方式是真实 CLI 黑盒。`run_ah` 调 `target/debug/ah --config <temp ah.toml>`，脚本自建 `PROJECT_DIR`, temp `HOME`, `XDG_STATE_HOME`, `XDG_CACHE_HOME`, `CCB_SOCKET`，并通过 cleanup trap 尝试 `ah stop`、kill socket owner、kill tmux server、停 systemd scope、删除 tempdir。该脚本不进入默认 `cargo test`。

Mock fixture 是 15 行 bash provider：启动时输出 `mock_provider: ready` 和 shell-like `$ ` marker；读取 stdin 后输出 `mock_provider: received=<line>`、`mock_provider: echo=<line>`、再输出 `$ `。它不读真实 token、不访问网络、不调用 Claude/Gemini/Codex CLI。

## §3 14 步主线对应表

| step | ah cmd | 真实 RPC method | DB 字段断言 | tmux 断言 | FS 断言 |
|---|---|---|---|---|---|
| 1 | `ah start` | `session.create` + `session.spawn_master_pane` + `agent.spawn` | `sessions.status=ACTIVE`; `sessions.config_hash` 非空; `agents.state -> IDLE`; `agents.config_hash` 非空 | `master_<project_id>` 存在; `agent_a1` 存在 | `state_dir/sandboxes/<session_id>/a1` exists |
| 2 | `ah ping` | `system.dump` | dump shape 包含 `projects`, `sessions`, `agents`, `evidence_pending`, `monitors` | 无新增 tmux 断言 | 无 FS 变化断言 |
| 3 | `ah ask a1 "grand tour first"` | `job.submit` | 返回 `job_id`; `jobs.status=QUEUED`; seam completion 后 `jobs.status=COMPLETED`; `events.event_type=command_received/output_chunk/state_change` | agent pane 逻辑上仍可接收任务；状态回到 `IDLE` 间接证明 | 无新增 FS 断言 |
| 4 | `ah ps` | `session.list` + `system.dump` | `session.list.sessions` 含当前 `session_id`; `system.dump.agents` 含 `a1`; DB 中 `agents.pid/state` 已可观测 | master + agent session 仍存在 | 无 FS 变化断言 |
| 5 | `ah pend <job_id>` | `job.wait` | wait result `status=COMPLETED`; `jobs.status=COMPLETED`; `agents.state=IDLE`; `events.event_type=output_chunk` | mock provider 完成输出后 session 不退出 | 无 FS 变化断言 |
| 6 | `ah logs a1` | `agent.read`（设计上 CLI logs 复用 events 读取，不存在 `agent.logs` RPC） | `agent.read.events` 非空; 至少一条 `event_type=output_chunk`; `payload` 含 mock output marker | 无 tmux 变化断言 | 无 FS 变化断言 |
| 7 | 修改 `ah.toml` | 非 RPC；写 temp project config | 修改前保存 `sessions.config_hash` 和 `agents.config_hash`; 修改后尚未 realign，DB 仍是旧 hash | 不重启 tmux | 写入 drift config 和 hook 文件 |
| 8 | `ah up` | `session.realign` | result `statuses` 为 array; `sessions.config_hash` 或 `agents.config_hash` 变化; `events.event_type=drift_realigned` 或 `agent_spawned` | agent session 经 realign 后仍可用 | sandbox / provider materialization 重新对齐 |
| 9 | 第二次 `ah ask a1 "grand tour second"` | `job.submit` | 新 `job_id != first_job_id`; seam completion 后 `jobs.status=COMPLETED`; `agents.config_hash` 非空且已沿用 realign 后配置 | agent 仍可接收任务并回到 `IDLE` | 无新增 FS 断言 |
| 10 | `ah prompt resolve a1 ...` | `agent.resolve_prompt` | manual seam 先置 `agents.state=PROMPT_PENDING`; result `status=ok`; `resolved_state` 为 `IDLE` 或 `BUSY`; `state_change` 可追踪 | action/keys 发送路径由 RPC 层验证，bash provider 不产生真实 prompt TTY | 可选 KB 文件不作为本 PR 主线强断言 |
| 11 | `ah cancel <job_id>` | `job.submit` + `job.cancel` | cancel job 初始 `jobs.status=QUEUED`; result `job_id` 匹配; result `status=CANCELLED` 或 `CANCEL_REQUESTED`; DB 终态同形 | queued cancel 不要求碰 pane | 无 FS 变化断言 |
| 12 | `ah watch a1` | `agent.watch` | result `events` 为 array; `is_truncated=false` | 无 tmux 变化断言 | 无 FS 变化断言 |
| 13 | `ah kill a1` | `agent.kill` | result `state=KILLED`; `agents.state=KILLED`; `events.event_type=agent_killed` | `agent_a1` tmux session gone | `state_dir/sandboxes/<session_id>/a1` removed |
| 14 | `ah stop` | `system.shutdown` + cleanup `session.kill` | result `status=shutting_down`; killed agent record 保留 | `master_<project_id>` tmux session gone | runtime cleanup 由 `session.kill` + tmux gone 组合覆盖 |

字段级补充：

- `session_id` 来自 step 1 的 `session.create` result，后续作为 sandbox path、master tmux session、`session.kill` cleanup 的主键。
- `project_id` 固定为 `ah_full_e2e_project`，master session name 通过 `master_session_name(PROJECT_ID)` 生成。
- `AGENT_ID` 固定为 `a1`，agent session name 通过 `agent_session_name(AGENT_ID)` 生成。
- `sessions.status=ACTIVE` 是 start 链最终会话可用性的 DB SoT，不只依赖 stdout。
- `sessions.config_hash` 在 start 后必须非空，drift 前后用于验证配置指纹被持久化。
- `agents.state` 在 spawn 后等待到 `IDLE`，在 prompt seam 中变为 `PROMPT_PENDING`，kill 后变为 `KILLED`。
- `agents.config_hash` 在 start 后必须非空，`ah up` 后和 session hash 一起验证 realign 是否落库。
- `jobs.status` 覆盖 `QUEUED`, `COMPLETED`, `CANCELLED|CANCEL_REQUESTED` 三类主线状态。
- `events.event_type` 覆盖 `state_change`, `command_received`, `output_chunk`, `drift_realigned|agent_spawned`, `agent_killed`。
- `events.payload` 在 logs/read 路径中作为字符串 JSON 读取，step 06 检查 output marker。
- tmux 层只通过隔离 `TmuxServerGuard` 的 socket 观察，不碰用户真实 tmux server。
- FS 层重点检查 sandbox dir create/remove；provider HOME 物化由 spawn/realign 链和 sandbox 断言间接覆盖。
- `system.dump` 的 shape 是 ping/ps 的共享基线，必须包含 `projects/sessions/agents/evidence_pending/monitors`。
- `agent.logs` 不存在于 router，Rust M2 用 `agent.read` 验证 logs 语义，设计文档明确该映射。
- `agent.watch` 使用 `events/is_truncated` shape，Rust M2 断 `is_truncated=false`，Bash M1 用短时后台采样避免无限阻塞。
- `system.shutdown` 的 return shape 是 `{ "status": "shutting_down" }`，这是 step 14 的真实断言对象。

## §4 Test Seam 诚实声明

- step_06 的 `mock_provider` 字符串来自 `dispatch_and_complete_job` 手插 `output_chunk`，这是 test seam，不是真实 spawn 的 stdout 逐字捕获。真实 spawn 的闭环由 `agent.spawn` 后 `agents.state -> IDLE`、tmux session 存在、sandbox dir exists 隐式验证。
- step_10 的 `mark_agent_prompt_pending` 是 manual seam。`tests/fixtures/mock_provider.sh` 是普通 bash echo provider，不会发真实 prompt，因此测试手动制造 `PROMPT_PENDING` 后调用 `agent.resolve_prompt`，覆盖 resolve RPC 和状态机路径。
- step_14 已修掉“自己 mock `shutting_down` 再 assert”的 tautology，当前调用真实 `h.rpc("system.shutdown", json!({}))` 并断言返回 shape。特殊性是 Rust in-process `Ctx { daemon_unit: None }` 上下文不会等同生产 daemon 进程自杀；测试随后保留 `session.kill` 和 master tmux gone 断言来验证 cleanup。
- Bash walkthrough 的 step 10 对 `ah prompt resolve` 做真实 CLI 调用；如果 bash mock 不产生真实 pending prompt，会显式 `SKIP step 10: bash mock does not emit a real prompt`，不伪造成功。

这些 seam 的边界是有意收窄的：

- 不把 mock provider 当成真实 LLM 能力验证；真实 provider 链路仍由 `mvp7/mvp9/mvp11_real_*` 系列覆盖。
- 不把 Bash walkthrough 当成深 DB 断言来源；它负责 CLI 参数、stdout/stderr、daemon auto-start、cleanup trap 和 smoke 命令。
- 不把 M2 Rust 测试包装成 CLI 黑盒；它负责内部 RPC shape、DB SoT、tmux、FS 的精确断言。
- 不在 PR-1 里实现 M3 分支矩阵；PR-1 只做单主线 Happy Path + drift mainline。
- 不隐藏 skipped prompt：Bash M1 的 prompt resolve 在无真实 prompt 时显式 SKIP，Rust M2 才是 resolve path 的强验证。
- 不隐藏 in-process shutdown 特殊性：step 14 验证真实 RPC return shape 和 session cleanup，不声称等价生产 daemon 进程退出全过程。

## §5 验证结果

- `CARGO_BUILD_JOBS=1 cargo test --test ah_full_e2e_main -- --include-ignored --test-threads=1`: 4 passed (0.77s)
- `bash -n scripts/ah-full-e2e/walkthrough.sh`: syntax PASS
- `CARGO_BUILD_JOBS=1 cargo build --bin ah --bin ccbd`: PASS

验证口径：

- Rust 命令包含 1 个 ignored grand tour + 3 个 `common` tests，因此总数是 4 passed。
- Bash walkthrough 只跑 `bash -n`，没有在本轮直接执行脚本本体，避免启动 ccbd 和当前开发实例冲突。
- Build 命令只确认 `ah` / `ccbd` 二进制可编译，Bash walkthrough 依赖这两个产物但不在默认 lane 执行。
- 默认 lane 安全性来自 `#[ignore]`，开发者需要显式加 `--include-ignored` 才会跑 Grand Tour。

## §6 已修 must-fix

- step 3 round 2：canonical `mod common` 复用，修正 a3 catch B。`tests/ah_full_e2e_main.rs` 直接复用 `tests/common/mod.rs` 的 `TmuxServerGuard`，避免复制漂移版 common harness。
- step 5 round 2：修正 step 14 tautology，改为真 `system.shutdown` RPC；补 N1/N2 注释，明确 step_06 `dispatch_and_complete_job` output seam 和 step_10 manual `PROMPT_PENDING` seam。a3 catch M1/N1/N2，a2 同意 M1。

## §7 Commit History

- `2ec26a2` spec 1a-1d research + design 思路 lock
- `3337994` spec 1e-1g formal design.md
- `6508811` spec tasks.md
- `6a88cda` step 3 T1+T2+T3 红灯
- `30af859` step 3 round 2 canonical common
- `f9c2f8f` step 4 主线 14 步落地
- `6e4f29a` step 5 round 2 step 14 + N1/N2
- `2d9b856` step 4 wave 2 M1 walkthrough

## §8 后续 PR

- PR-2 M3 DRIFT + NEW 分支：新增 `tests/ah_full_e2e_drift.rs`，覆盖 config drift 以外的新增 agent、realign 分支和物化状态。
- PR-3 M3 ORPHAN + BUSY + ERROR 分支：新增 `tests/ah_full_e2e_lifecycle.rs` 与 `tests/ah_full_e2e_evidence.rs`，覆盖 orphan 收割、busy skip/force、provider crash/recovery 和 evidence/prompt 扩展链路。
- CI Nightly workflow yaml：按 design §7.1 决议，把 ignored Grand Tour 和 Bash walkthrough 放到 nightly/人工 lane，不进入默认 `cargo test` 快速反馈路径。

PR-2/PR-3 的验收应继续沿用本 PR 的物理断言风格：

- 每个分支都要有真实 RPC method 或真实 CLI command，不写自造 result 后 assert。
- 每个分支都要查询 DB SoT 字段，而不是只看 stdout 文本。
- 涉及 tmux 生命周期时必须使用隔离 socket，不误杀测试外 session。
- 涉及 FS 物化时必须断实际 path exists/removed/symlink/copy 状态。
- 涉及 seam 时必须在测试或报告中明示 seam 的原因和未覆盖范围。
