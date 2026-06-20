# Research — revive/resume 现状 vs PM 统一思路 (a1, 2026-06-16, 只读)

对齐 PM 决策 (`pm-decision-2026-06-16-cutover-and-revive.md`) 的 revive/resume 统一思路, a1 grep 实证 ah 现状, 找 gap。这是设计 input。

## 1. Worker 执行状态
- 现状: `agents.state` / `sub_state` / `state_version` 存 SQLite (src/db/schema.rs:50)。状态含 IDLE / SPAWNING / WAITING_FOR_ACK / BUSY / PROMPT_PENDING / CRASHED / KILLED。
- job 状态: QUEUED / DISPATCHED / COMPLETED / FAILED / CANCELLED — **无 RUNNING**。dispatch 把 job 置 DISPATCHED + agent IDLE→WAITING_FOR_ACK (src/db/state_machine.rs)。
- "正在执行"底座: WAITING_FOR_ACK / BUSY 最接近; is_active_state 还含 SPAWNING (state_machine.rs:56)。**可靠性不足: DISPATCHED 只表示已派发, 不等价 provider 真开始执行。**
- **GAP**: 缺独立精确的"当前 task 正在执行中"信号。

## 2. Master 执行状态
- 现状: `sessions` 只有 master_pid / master_pane_id / status / master_retry_count / master_generation / master_last_exit_reason (src/db/schema.rs:8)。**没有 master 自身"正在执行某个 PM task"状态。**
- master 死时的 "ActiveWork" 是**推导**: worker 为 SPAWNING/WAITING_FOR_ACK/BUSY/PROMPT_PENDING, 或 session 内有 QUEUED/DISPATCHED job。
- **GAP**: master 无一等"正在执行 PM task"状态, 当前用 worker/job 活跃度代理。

## 3. revive 触发条件
- Worker: 所有 eligible CRASHED **无条件** revive (orchestrator recovery)。
- Master: **已有 active/idle gate** — IdleNoWork 只 reap 不 revive, ActiveWork 才 revive (src/monitor/master_watch.rs:104 / :125)。
- **GAP**: worker 不满足 PM "只有执行中 kill 才拉起"; master 大体满足, 但 active 判定是**代理信号**, 不是 master 自身 task 状态。

## 4. session 上下文恢复
- Worker recovery: 全新 spawn **但追加 provider resume 参数** — Claude `--continue`; Codex 从 `.codex/sessions/rollout-*.jsonl` 取 session id 或 `resume --last` (src/master_cutover.rs:146 一带)。
- cutover seed: Claude 专用文件复制 (seed_claude_project_conversation); cutover RPC 调 seed (src/rpc/handlers/sessions.rs:560)。**revive master 路径未调 seed**, 只复用 sandbox home + env。
- **GAP**: 有 provider resume 参数底座, 但没把"死前正在执行的 job/prompt + provider session id"作为持久状态绑定进 revive (尤其 master)。

## 5. "输入继续"
- 现状: 输入 pane 通用机制存在; job dispatch 用 send_text_to_pane_with_options 发原 job prompt (src/orchestrator/mod.rs:133)。
- Master revive 只写 `AH_REDISPATCH_MARKER` (提示人工 inspect/re-dispatch, src/monitor/master_watch.rs:207 / :515)。**rg 未发现 ah 端消费该 marker 或自动发"继续"。**
- **GAP**: 缺 revive 后自动注入"继续"的机制; 可复用底座 = pane send + job submit/dispatch。

## 净 gap 总结 (对齐 PM 三点)
| PM 思路 | 现状 | gap |
|---|---|---|
| #1 状态 (含 master 即时执行态) | worker 有 state 但 DISPATCHED≠执行; master 无一等执行态 (用代理) | 需精确"正在执行 task"信号 (worker + master) |
| #2 只有执行中被 kill 才拉起 | master 已 gate (ActiveWork/IdleNoWork); worker 无条件 | worker 加 gate; master gate 改用真 task 态 |
| #3a 恢复 session 上下文 | worker 有 provider resume 参数; master revive 不 seed | 死前 session 上下文绑定进 revive (尤其 master) |
| #3b 输入继续 | 只有 redispatch marker, 无 auto-继续 | revive 后自动注入"继续" |

读了: schema.rs, state_machine.rs, orchestrator/mod.rs, recovery.rs, monitor/{master_watch,agent_watch}.rs, master_revival.rs, db/{system,agents_lifecycle,jobs}.rs, rpc/handlers/{agent,jobs,realign,sessions}.rs, master_cutover.rs, sandbox/systemd.rs, agent_io/writer.rs, bin/{ah,ahd}.rs, cli/start.rs。
