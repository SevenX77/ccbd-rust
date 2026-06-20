# Research: Step-4 master 自换 ccb -> ah 现状

日期: 2026-06-15

范围: 只做现状调研。目标是给后续正式 design 提供 ground truth，不在本文件里实施 cutover。

## 1. Master 现在怎么被启动 / 怎么跑

结论:

- 当前项目规则把 Master PM 定义为 `claude --continue /remote-control` 启动、环境无 `CCB_CALLER_ACTOR`；worker 由 CCB 框架拉起且有 `CCB_CALLER_ACTOR=<agent>`。这是现存主控身份模型，不是 ah 专属模型。
- ah 代码已经能创建一个被 ah 管理的 master pane：`ah start` -> `session.create` -> `session.spawn_master_pane` -> tmux master session/window -> `systemd::master_command_with_env` -> pidfd watcher。默认命令是 `claude --dangerously-skip-permissions --continue /remote-control`。
- 现有 Step-4 PLAN 明确当时做的是 dispatch-tooling 切换，不是“master 进程被 ah 托管”。因此“真主控 Claude 本人当前是否已跑在 ah master pane 内”不能从代码推出；现有记录显示 dispatch 切换已验，但 master 进程归属仍未完成最终自换。

实证:

- `CLAUDE.md:7-10`: Master PM 通过 `claude --continue /remote-control` 启动，环境没有 `CCB_CALLER_ACTOR`; worker 有 `CCB_CALLER_ACTOR=<agent名>`。
- `src/cli/config.rs:169-170`: ah 默认 master cmd 是 `claude --dangerously-skip-permissions --continue /remote-control`。
- `src/cli/start.rs:136-165`: 新 session 时调用 `session.create`，若 `[master].enabled` 则调用 `session.spawn_master_pane`，传入 `cmd/hooks/plugins`。
- `src/rpc/handlers/sessions.rs:191-231`: `handle_session_spawn_master_pane` 解析 `session_id/cmd`，为 master 准备 claude home layout，再调用 `systemd::master_command_with_env`。
- `src/rpc/handlers/sessions.rs:232-247`: 使用 `master_session_name` 建 tmux session/window，并记录 `master_pane_id`。
- `src/rpc/handlers/sessions.rs:255-300`: 读取 tmux pane pid、打开 pidfd、记录 `master_pid/master_generation`，注册 `spawn_master_pidfd_watch_task`。
- `src/tmux/session.rs:48-102`: `ensure_session_sync` 通过 tmux `new-session` 创建/复用 session。
- `src/tmux/session.rs:113-149`: `spawn_window_sync` 通过 tmux `new-window` + `respawn-pane` 启动 pane 命令。
- `src/sandbox/systemd.rs:88-114`: master 非 unsafe 路径用 `systemd-run --user --scope --collect` 包住命令；under_systemd 时追加 workspace slice 和 daemon unit 依赖。
- `.kiro/specs/ah-master-self-switch/PLAN.md:7-10`: Step-4 目标是把调度工具从 ccb 切到 ah，并明确“不是 master 进程被 ah 托管”。

已具备:

- ah 具备 master pane 创建、隔离 home、pidfd 监控、revive 所需的底层路径。

缺口:

- 没有一个已落地的“把当前正在对话的真 Master PM 迁入/重启到 ah-managed master pane”的 cutover 入口。
- 现有 `ah start` 会新建 master pane，但不会把外部已有 `/remote-control` 主控接管进该 pane。

不确定待 design 定:

- 自换应采用“外部 helper 启动一次 ahd + ah master pane，然后用户/主控切到新 remote-control session”，还是“当前 master 自己执行迁移并退出旧 ccb 环境”。

## 2. Master 现在依赖 ccb 做什么 vs ah 已能提供什么

结论:

- ccb 当前主控工作流的公开 help 能力包括 `ccb ask`, `ccb ps`, `ccb pend`, `ccb watch`, `ccb kill`, `ccb logs`, `ccb doctor`, `ccb ping` 等。
- ah 已有对应核心命令: `ah ask`, `ah ps`, `ah pend`, `ah watch`, `ah kill`, `ah logs`, `ah doctor`, `ah ping`, 另有 `ah cancel`, `ah start`, `ah up`, `ah stop`, `ah attach`, `ah prompt resolve`。
- 语义不是完全 drop-in: 关键差异是 `ah pend` 只吃 `<job_id>`，而 ccb help 写的是 `ccb pend <agent|job_id> [N]`；Step 1 实测也记录 `ah pend <agent_name>` 会失败但 `<job_id>` 可用。

实证:

- 本机 `ccb --help`: `ccb ask <agent> [from <sender>] <message>`, `ccb ping <agent|ccbd>`, `ccb pend <agent|job_id> [N]`, `ccb watch <agent|job_id>`, `ccb ps`, `ccb logs <agent>`, `ccb doctor`, `ccb kill`。
- `src/bin/ah.rs:36-106`: ah clap 子命令包括 `Ping/Version/Ps/Start/Up/Ask/Pend/Cancel/Kill/Watch/Logs/Attach/Stop/Doctor/Config/Prompt`。
- `src/bin/ah.rs:463-485`: `ah ask` 调 `job.submit`，可选 `--wait`。
- `src/bin/ah.rs:487-489`: `ah pend` 等待指定 `job_id`。
- `src/bin/ah.rs:492-504`: `ah cancel` 调 `job.cancel`。
- `src/bin/ah.rs:507-531`: `ah kill` 根据 `--session` 分流到 `session.kill` 或 `agent.kill`。
- `src/bin/ah.rs:405-425`: `ah ps` 调 `session.list` 和 `system.dump`。
- `src/bin/ah.rs:534-560`: `ah watch` 调 `agent.watch`。
- `.kiro/specs/ah-dogfooding-closure/acceptance-matrix.md:217`: 记录 `ah pend` 只吃 `<JOB_ID>`，与 ccb 的 name-or-id 语义不同。
- `.kiro/specs/ah-product-delivery/handoff-prompt.md:82`: Step-4 目标命令集合写明 `ah ask/ps/pend/kill` 替代 `ccb ...`。

逐项对照:

| 主控 ccb 能力 | ah 等价 | 对齐程度 |
| --- | --- | --- |
| `ccb ask <agent> ...` | `ah ask <agent_id> <text> [--wait] [--request-id]` | 核心对齐；sender/from 语义未见 ah 等价。 |
| `ccb ps` | `ah ps` | 对齐，ah 输出 sessions + agents。 |
| `ccb pend <agent|job_id> [N]` | `ah pend <job_id>` | 部分对齐；ah 不支持 agent 名等待。 |
| `ccb watch <agent|job_id>` | `ah watch <agent_id> --since-event-id` | 部分对齐；ah watch 按 agent/event cursor，不按 job_id。 |
| `ccb kill` | `ah kill <agent>` / `ah kill --session <session_id>` / `ah stop` | 能力覆盖更细，但命令形状不同。 |
| `ccb logs <agent>` | `ah logs <agent_id> [--since]` | 对齐。 |
| `ccb ping <agent|ccbd>` | `ah ping` | ah ping 是 daemon dump 级，不是 per-agent ping。 |
| `ccb doctor` | `ah doctor` | 对齐。 |
| `ccb ask cancel` / cancel 救场 | `ah cancel <job_id>` | ah 有显式 cancel；实测目标是减少救场式 cancel。 |

已具备:

- Master PM 日常派单、查状态、取回复、取消、kill、logs 的主要 primitives 已存在。

缺口:

- ah CLI 对 ccb 的 `pend/watch` name-or-id 便利性不完全等价。
- 未见 ah 对 `ccb ask <agent> from <sender>` 的 sender 语义兼容。

不确定待 design 定:

- 自换时是否要追求 ccb 命令形状兼容，还是接受 ah 原生命令并改 Master 操作手册。

## 3. dispatch-switch 已验证到什么程度

结论:

- 已验证两层:
  - 早期 dogfood-closure 证明 daemon 侧 completion/subscribe/stuck/slash/cancel 的机制，部分用 in-process harness + fake provider。
  - 后续 Step 1/Step 4 acceptance 记录证明真 codex/claude/antigravity 上 `ah ask --wait`、logs、cancel、kill、large payload、cleanup 可用；并且 Step-4 dispatch-tooling 已用 `ah ask` 对三 provider 完成 3 个异构真任务。
- 已验证的 Step-4 dispatch-switch 主要是 `ah ask` round-trip；不是整套 PM 工作流的每个命令都已经在真实 master 工作流中长链验证。

实证:

- `docs/reports/ah-dogfooding-closure-summary.md:20-30`: 7 项度量兑现，包括 cancel=0、capture=0、poll=0、push p95、stuck、slash、5 RPC/SOP-08 模拟。
- `docs/reports/ah-dogfooding-closure-summary.md:61-72`: 诚实边界说明 dogfood-8 当时仍用 in-process RPC harness 和 fake provider，不是真 CLI/真 LLM。
- `.kiro/specs/ah-dogfooding-closure/acceptance-matrix.md:181-213`: Step 1 记录真 codex/claude C/D3/E/F 全维度 PASS，包括 `ah ask --wait`, `ah logs`, `ah cancel`, ghost agent, double kill, large payload, cleanup。
- `.kiro/specs/ah-dogfooding-closure/acceptance-matrix.md:355-379`: Step-4 第一次真 `ah ask a3` 重型任务抓到并修复 claude 过早完成 bug，修后捕获真答案。
- `.kiro/specs/ah-dogfooding-closure/acceptance-matrix.md:383-396`: Step-4 dispatch-tooling 切换，master 经 `ah ask` 对 claude/codex/antigravity 三 provider 完成异构真任务；0 phantom / 0 cancel / 0 截断 / 0 desync。
- `.kiro/specs/ah-dogfooding-closure/acceptance-matrix.md:398-420`: 诚实边界包括 antigravity reply 仍走 screen 抓取、pane chrome。

已具备:

- `ah ask --wait` 在三目标 provider 上的真任务 round-trip 已有强实证。
- `ah logs`, `ah cancel`, `ah kill`, 大 payload 等单项也有真 provider 实证。

缺口:

- 未见“主控完整 SOP-08 多轮真实 PM 工作全部通过 ah 命令完成”的最终记录。
- Step-4 dispatch-switch 记录没有证明 `ah ps/pend/kill/cancel/watch/prompt resolve` 全部在同一真实 PM 长链中被主控稳定使用。

不确定待 design 定:

- 最终验收要定义 N: 是“>=3 异构 ask 任务”足够，还是必须覆盖 `ps/pend/logs/cancel/kill/prompt resolve` 的完整操作集。

## 4. master-revive 给 Step-4 提供了什么

结论:

- master-revive/纠正版 master-death 已经让“ah 管理的 master pane”具备 pidfd 监控、A/B 快照、worker 先清理、ActiveWork revive、IdleNoWork 不 revive、generation/CAS/backoff/fuse 等自救原语。
- 这说明“如果真主控是由 ah 的 `session.spawn_master_pane` 拉起并记录在 DB 中”，它已经能作为 ah 管理实体被监控和自救。
- 还差的是 cutover/bootstrap: 怎样让当前真实 Master PM 进入这个 ah-managed master pane，并确保它的规则、auth、remote-control、回退通道都正确。

实证:

- `src/monitor/master_watch.rs:24-81`: master pidfd watcher 等待 pidfd readable，按 `classify_master_death` 进入 revive 或忽略 stale/intentional。
- `src/monitor/master_watch.rs:99-130`: revive path 先拿 `master_spawn_lock`，拍 A/B 快照，清理 workers；`IdleNoWork` 直接不 revive。
- `src/monitor/master_watch.rs:131-203`: ActiveWork path 处理 backoff、CAS claim、record attempt/fuse。
- `src/monitor/master_watch.rs:204-286`: 使用原 `master_cmd` 和 existing home/env 重新 spawn master，更新 DB，注册新 watcher 和 stable confirm timer。
- `src/master_revival.rs:61-93`: `classify_master_death` 只有 ACTIVE + pid/generation 匹配才 Revive。
- `src/master_revival.rs:95-118`: `try_claim_master_transition` 用 `master_generation` CAS 防 stale/double revive。
- `src/master_revival.rs:235-290`: revive retry/backoff 和 fuse 阈值。
- `src/db/system.rs:166-222`: A/B snapshot: active worker (`SPAWNING/WAITING_FOR_ACK/BUSY/PROMPT_PENDING`) 或 `QUEUED/DISPATCHED` job => `ActiveWork`; 否则 `IdleNoWork`。
- `src/db/system.rs:224-320`: worker runtime cleanup 取消 registries、stop systemd scopes、pidfd SIGKILL fallback，不改 session status。
- `src/sandbox/systemd.rs:200-221`: 当前 master scope 内 wrapper 写 `/proc/self/oom_score_adj=500` 后 exec 真 master cmd。
- `tests/r2_master_scope_spawn.rs:389-548`: ignored 真 scope dogfood 覆盖 master spawn oom_score、ActiveWork kill 后 reap+revive、IdleNoWork kill 后 reap 不 revive。
- `.kiro/specs/ah-oom-restart-resume/design-master-death-corrected.md:11-17`: 设计目标是 master 死亡后先清 worker，再按 A/B revive 或不 revive。
- `.kiro/specs/ah-oom-restart-resume/design-master-death-corrected.md:48-60`: 新流水线定义 detect -> classify -> lock -> snapshot -> reap -> A/B 分叉。

已具备:

- ah-managed master 的监控和自救核心机制已经成立。
- ActiveWork/IdleNoWork 语义已经从设计落实到代码和 dogfood 测试。

缺口:

- revive 复用 `master_cmd`，但自定义 master cmd 是否真正能 resume 取决于用户配置；设计文档也标注 custom command 不强制改写。
- 真主控 Claude 的 remote-control 会话是否能在 revive 后自动回到用户可控状态，需要单独 dogfood。

不确定待 design 定:

- Master 自换后的“续断点”验收是 provider conversation 继续、还是 PM 工作流 job-level 状态继续，还是两者都要。

## 5. 递归 bootstrap / 鸡生蛋问题

结论:

- ah CLI 已能在 socket 不存在时启动 ahd: `ensure_daemon_running` 查 socket，找不到则定位旁边的 `ahd`，优先 `systemd-run --user --unit=ahd.service`，失败则 direct spawn。
- `ah start` 会在 ahd 就绪后创建 session/master/agents。因此从冷启动角度，“先起 ahd，再 spawn master pane”这条链存在。
- 鸡生蛋仍在操作层: 当前 Master PM 是外部 ccb/手工 remote-control 进程；它可以启动一套 ah，但启动后的 ah-managed master 是另一个 Claude remote-control。如何把“主控身份/会话入口”切到这个新 master，代码没有自动步骤。
- “master 起来后怎么知道用 ah 而非 ccb”当前依赖规则/文档，不是运行期 env: ah src/tests 中没有 `CCB_CALLER_ACTOR` 机制；ah 的身份隔离主要靠 provider home layout 的 Master/Worker rules。

实证:

- `src/bin/ah.rs:223-238`: 无子命令默认动作会检查嵌套、确保 daemon、然后 `start_from_options(wait=true)`。
- `src/bin/ah.rs:241-324`: `ensure_daemon_running` 检查/移除 stale socket，定位 `ahd`，创建 state dir，优先 systemd bootstrap，等待 socket ready。
- `src/bin/ah.rs:326-339`: direct spawn fallback 设置 `AH_STATE_DIR` 后启动 `ahd`。
- `src/cli/start.rs:30-67`: systemd bootstrap command 包含 `--unit=ahd.service`, Restart 策略和 `OOMScoreAdjust=-900`。
- `src/state_layout.rs:16-23`: `AH_STATE_DIR` / `CCBD_STATE_DIR` 可显式指定 state dir。
- `src/cli/rpc_client.rs:102-113`: `CCB_SOCKET` 可覆盖 socket，否则从 state layout 得到 `ahd.sock`。
- `src/cli/start.rs:136-172`: ahd ready 后创建 session 并 spawn master pane。
- `.kiro/specs/ah-dogfooding-closure/acceptance-matrix.md:119-123`: 正确姿势是通过 `ah --config "$TEST_CONFIG" start --wait` 调 `session.spawn_master_pane`，默认 master cmd 是 `claude --dangerously-skip-permissions --continue /remote-control`。
- `.kiro/specs/ah-dogfooding-closure/acceptance-matrix.md:109-115`: 真跑 ah 后发现 master 与 agents 都没有 `CCB_CALLER_ACTOR`; ah 的 master-vs-worker 区分走 config-home layout 的 MASTER_RULES/WORKER_RULES。
- `.kiro/specs/ah-pr4b-builtin-layer/design.md:57-66`: 规则层计划把 `ah ask/ping/pend` 通信原语写进内建 Section，并保留异步护栏。

已具备:

- 冷启动 ahd + spawn ah master pane 的技术路径存在。
- state/socket 可用 env 隔离，能与 helper ccb 并存。

缺口:

- 没有正式 bootstrap script/command 定义“一键启动 ah-managed PM master 并交接当前控制权”。
- Master 使用 ah 而非 ccb 依赖规则文字，缺少运行期 guard 或路径隔离来阻止误用 ccb。

不确定待 design 定:

- 是否应保留 helper ccb 作为 rollback 到某个里程碑，还是切换后禁止 master 访问 ccb socket/命令。

## 6. 回退 / 安全

结论:

- 已有 Step-4 PLAN 把 helper ccb 作为 rollback 通道: ah dispatch 卡住就回退 `command ccb ask`，并保留 helper ccbd 在线。
- ah 与 ccb 可通过 `AH_STATE_DIR` / tmux socket hash / `CCB_SOCKET` 形成并存隔离；Step 1 记录也明确各 provider 使用独立 `AH_STATE_DIR` 和 ahd socket，与 master 的 ccb Python socket 隔离。
- 风险是双调度/双脑: 如果 master 同时能调用 ccb 和 ah，就可能把同一任务派到两套系统，状态和证据分裂。现有代码没有“master-lock 防止同时连 ccb 和 ah”的产品级策略；只有 ah 自身的 socket/state 隔离、session window lock、master spawn lock。

实证:

- `.kiro/specs/ah-master-self-switch/PLAN.md:17-28`: cutover 序列明确保留 helper ccb 作 rollback，ah dispatch 失败则用 `command ccb ask` 重派并记录现场。
- `.kiro/specs/ah-dogfooding-closure/acceptance-matrix.md:185`: Step 1 dogfood 使用独立 `AH_STATE_DIR` + `ahd-<hash>` tmux socket，与 master 的 ccb Python + 独立 socket 完全隔离。
- `src/state_layout.rs:16-23`: `AH_STATE_DIR` / `CCBD_STATE_DIR` 可隔离 state dir。
- `src/cli/rpc_client.rs:102-113`: `CCB_SOCKET` 可指定 ah CLI 连接的 socket。
- `src/rpc/mod.rs:98-102`: 另一个 ccbd/ahd socket 正在运行时，RPC bind 会警告已有进程或移除 stale socket。
- `src/rpc/handlers/sessions.rs:31-42`: session window lock 防同 session tmux window 并发创建。
- `src/master_revival.rs:377-385`: master spawn lock 防同 session master revive/spawn 并发。
- `src/bin/ah.rs:362-371`: ah CLI 有 nested environment detection，发现 tmux env/cgroup 嵌套时可提示；但这是防嵌套启动，不是 ccb/ah 双派单锁。

已具备:

- 可并存、可回滚的物理隔离基础。
- ah 内部有 session/master 级并发锁。

缺口:

- 没有产品化的“cutover mode”标志，无法声明当前 master 只能用 ah 或正在回退到 ccb。
- 没有自动防止同一 PM 任务被 ccb/ah 双派的机制。

不确定待 design 定:

- 回退策略是“helper ccb 长期保留”，还是“短窗口保留，达到 N 个真任务后禁用/退出 helper ccb”。
- 是否需要给 Master 规则注入硬性提示/alias，使 `ccb ask` 在 cutover 后显式失败或要求确认。

## 汇总结论

已具备:

- ah 已有主要 PM 调度 CLI: ask/ps/pend/cancel/kill/logs/watch/start/stop/up/prompt resolve。
- ah 已能创建、隔离、监控、revive 自己管理的 master pane。
- `ah ask` dispatch-tooling 已在 codex/claude/antigravity 三 provider 真任务上通过，且修过一个真实 Step-4 blocker。

关键缺口:

- “dispatch-tooling 切换”不等于“真 Master PM 进程跑在 ah 上”。后者还缺正式 bootstrap/cutover/rollback 设计。
- ah CLI 与 ccb CLI 非 drop-in，尤其 `pend/watch` 语义差异需要规则或兼容层解决。
- 完整 PM 工作流长链的最终 dogfood尚未闭合，特别是 ps/pend/logs/cancel/kill/prompt resolve 在真实多轮 SOP-08 中的组合使用。

## 读了哪些文件

- `/tmp/a1-step4-research.md`
- `CLAUDE.md`
- `.kiro/specs/ah-master-self-switch/PLAN.md`
- `.kiro/specs/ah-product-delivery/handoff-prompt.md`
- `.kiro/specs/ah-dogfooding-closure/acceptance-matrix.md`
- `.kiro/specs/ah-dogfooding-closure/design.md`
- `.kiro/specs/ah-dogfooding-closure/research.md`
- `.kiro/specs/ah-real-dogfooding-acceptance/test-matrix.md`
- `.kiro/specs/ah-real-dogfooding-acceptance/findings.md`
- `.kiro/specs/ah-oom-restart-resume/design-master-death-corrected.md`
- `.kiro/specs/ah-pr4b-builtin-layer/design.md`
- `docs/reports/ah-dogfooding-closure-summary.md`
- `docs/reports/pr-dogfood-final.md`
- `docs/reports/pr-dogfood-m1.md`
- `docs/DESIGN.md`
- `docs/upstream-ccb-bugs/gemini-dispatch-and-completion-bugs.md`
- `src/bin/ah.rs`
- `src/bin/ahd.rs`
- `src/cli/start.rs`
- `src/cli/config.rs`
- `src/cli/rpc_client.rs`
- `src/state_layout.rs`
- `src/rpc/handlers/sessions.rs`
- `src/rpc/handlers/realign.rs`
- `src/rpc/router.rs`
- `src/tmux/session.rs`
- `src/sandbox/systemd.rs`
- `src/master_revival.rs`
- `src/monitor/master_watch.rs`
- `src/db/system.rs`

## 跑了哪些 grep / 命令

- `sed -n '1,260p' /tmp/a1-step4-research.md`
- `git status --short`
- `ls -la .kiro/specs/ah-master-self-switch`
- `rg -n "spawn_master|spawn_master_pane|master_command|master_pane|master_pid|master_generation|claude --continue|/remote-control|CCB_CALLER_ACTOR|CCBD_UNSAFE_NO_SANDBOX|ah ask|ccb ask|ask wait|ask cancel" src .kiro docs CLAUDE.md README.md`
- `rg -n "enum .*Command|Subcommand|Ask|Ps|Pend|Kill|Cancel|Wait|clap|derive\\(.*Parser|struct .*Args" src/cli src/bin src/main.rs`
- `rg -n "dispatch-tooling|dogfood|phantom|cancel|truncate|desync|codex|agy|claude|PASS|acceptance" .kiro/specs/ah-dogfooding-closure .kiro/specs`
- `rg -n "master death|master-death|revive|IdleNoWork|ActiveWork|resume|generation|master scope|oom_score_adj|systemd-run" src/master_revival.rs src/monitor/master_watch.rs src/sandbox/systemd.rs .kiro/specs/ah-oom-restart-resume/design-master-death-corrected.md`
- `rg -n "ensure_daemon_running|spawn.*ahd|AH_STATE_DIR|CCB_CONFIG_PATH|resolve_socket|DaemonNotRunning|systemd-run" src/bin/ah.rs src/cli src/state_layout.rs src/bin/ahd.rs`
- `rg -n "master-lock|lock|owner|detach|no_detach|feedback_ccb_dispatch_no_detach|CCB_SOCKET|CCB_CALLER_ACTOR|ccb" src .kiro docs CLAUDE.md`
- `rg -n "ccb (ask|ps|pend|kill|cancel|wait)|ah (ask|ps|pend|kill|cancel|wait|logs|watch)|ask --wait|pend <job_id>|ask wait|ask cancel" .kiro docs CLAUDE.md README.md`
- `command -v ccb && ccb --help`
- 尝试 `cargo run --quiet --bin ah -- --help` 只为 help 输出；因触发长编译且非必要，已终止，未用作结论依据。
