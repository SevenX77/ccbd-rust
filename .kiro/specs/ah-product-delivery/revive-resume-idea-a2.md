# revive/resume 机制 — a2 思路 (SOP-08 §1.1 step 1c, 2026-06-16)

> 来源: a2 (gemini) job_058777a3ba8b。基于 a1 research (`revive-resume-research.md`) + PM 统一思路 (`pm-decision-2026-06-16-cutover-and-revive.md`)。
> 主控已 fact-check 所有 file:line 锚点 (见末尾"主控校验")。这是设计 draft, 待 a1+a3 audit (1d) 收敛后 a1 主笔正式 design.md (1e)。

## PM 硬约束 (设计目标)
1. 每个 agent (含 master) 要有即时执行状态, 判断被 kill 那刻是否正在执行任务。
2. revive 门槛: 只有「任务执行中」被 kill 才自动拉起; 空闲被 kill 不拉起 (但仍清理防孤儿)。
3. resume: 拉起恢复 kill 前 session 上下文 + 输入「继续」接着干。

## 1. "正在执行任务"状态的可靠采集
**第一性原理**: 执行态不能靠上游"我发了(DISPATCHED)"假设, 必须靠下游物理世界"我动了"证实。

- **Worker**: 只有 `WAITING_FOR_ACK` 或 `BUSY` 是"正在执行"; `IDLE`/`PROMPT_PENDING` 停滞, `SPAWNING` 基建。被 kill 那刻 `agents.state` 的 DB 快照就是现场, 无需新表。
- **Master**: 摒弃强加物理执行态 (它是 tmux 黑盒交互脚本)。沿用 ActiveWork 代理推导 = 名下 session 的 `jobs` 表存在非终态 (QUEUED/DISPATCHED) 或 worker 有执行动作。有挂念→意外中断; 无→自然停歇 (IdleNoWork)。

## 2. Revive 门槛统一 (Worker 加 gate)
- 动点: `src/orchestrator/mod.rs:219` `run_recovery_once_with_respawn` (扫 CRASHED 的循环, :225/:228 现在只 gate provider-eligibility)。
- **[BREAKING]** 扫到 CRASHED 不再无条件 `spawn_realign_agent(is_recovery=true)`。反查崩前最后正常状态:
  - 崩前 `IDLE`/`PROMPT_PENDING` → 不复活, `delete_agent` 收尸。
  - 崩前有绑定活动 Job → revive。
- a2 自承认: 这需要 `agents_lifecycle.rs` 标 CRASHED 前把原 state 落到新字段/event log, **或**简化为只看是否有绑定 DISPATCHED job。

## 3. Session 上下文持久化 + Revive 注入
- **Worker (Resume)**: 复用 `--continue`。provider 状态在沙箱 `.claude` 目录, 沙箱存活就能续。
- **Master (Seed) [BREAKING]**: 现状只有 cutover seed 会话。Master revive 必须和 cutover 共享 seed 契约。改 `src/monitor/master_watch.rs:91` `revive_master_after_exit`, 在 `spawn_master_pane` 前调 `seed_claude_project_conversation` (现 master_cutover.rs:88, cutover 在 sessions.rs:570 调用)。

## 4. "输入继续"自动注入
- **时机**: recovery spawn 完成且状态机首次跃迁 `IDLE` (证明 provider 就绪)。
- **动作**: orchestrator 置 IDLE 时检查是否 recovery-spawn 后首次 idle; 是且有中断 Job (FAILED 捞出标 `RECOVERED`) → 自动消费 → 调 `send_text_to_pane_with_options` (`src/agent_io/writer.rs:15`) 注入"继续"+Enter。
- **替代关系**: 替代人工读 `AH_REDISPATCH_MARKER`。marker 降为纯监控凭证 (debug 留底)。

## 5. 缺陷定性 (三轴)
- **[重画契约/设计缺陷] Worker Revive Gate** (证据高/影响高/置信高): 无脑 revive CRASHED 违背防孤儿初衷。动点 mod.rs:219 循环。
- **[重画契约/设计缺陷] Master Revive Seed 缺失** (高/高/高): master 崩后裸启=失忆。动点 master_watch.rs:91。
- **[补全实现] Auto-Continue 注入** (中/高/高): 现半成品 (只写文件不行动)。动点 state_machine.rs IDLE 跃迁处。

## 主控校验 (2026-06-16, fact-check 全部通过)
| a2 锚点 | grep 实证 | 结论 |
|---|---|---|
| run_recovery_once_with_respawn | src/orchestrator/mod.rs:219 | ✅ (:200 是入口 run_recovery_once) |
| send_text_to_pane_with_options | src/agent_io/writer.rs:15 | ✅ |
| seed_claude_project_conversation | src/master_cutover.rs:88, cutover 调用在 sessions.rs:570 | ✅ |
| revive_master_after_exit | src/monitor/master_watch.rs:91 | ✅ |
| CRASHED 无条件 revive | mod.rs:225 query_agents_by_state("CRASHED"), :228 仅 gate is_recovery_eligible_provider | ✅ 确认无 in-flight gate |

## 主控 flag 给 audit 的工程疑点 (不替 a2 解, 留 1d 审)
crash-mark 序列把 DISPATCHED job 翻 FAILED (agents_lifecycle.rs:34, jobs.rs:425) 发生在标 CRASHED 时。若 worker gate 想"反查崩前有无绑定活动 Job", 而 job 已被翻 FAILED, 这个信号在 recovery 扫描时还在不在? a2 的"新字段 vs 只看 DISPATCHED job"两条路哪条可落地, 是 1d audit 必须钉死的命门。

---

## 1d AUDIT 收敛 (round 1-2, 2026-06-16)

a1 (codex, 工程) + a3 (claude, PM 替身) audit → round-2 a2 收敛。**round-2 无新 must-fix, 收敛 (round-cap 停 round-2)。**

### 3 个工程 must-fix (a1 给落地路线, a2 round-2 确认契合第一性原理) — 进 1e 正式 design
1. **worker gate 信号 [命门, a1+a3 强收敛]**: a2 原"agents.state 快照就是现场无需新表" **事实错** — crash-mark 同事务里 (agents_lifecycle.rs:75 读 prev → :83 UPDATE state='CRASHED' → jobs.rs:425 把 DISPATCHED 翻 FAILED → :105 插 event) 把崩前态和 DISPATCHED job 都销毁。落地: **crash-mark 事务内持久化 `previous_state + interrupted_job_id` 到可查字段 (新列/新表)**, recovery gate 读这个 recovery intent, 不读 DISPATCHED。
2. **master seed 复用 [a1 精确化]**: cutover seed 是 old_home→master_home **复制** (master_cutover.rs:88, 需 request.old_home, sessions.rs:570)。revive 时 master 已死无 old_home → **不能复用 cutover 签名**; revive 改复用现有 master_home + 写 handoff/continue marker (种子意图对, copy 机制不适用)。
3. **auto-continue 落点 [a1]**: state_machine IDLE 跃迁是同步 DB 事务无 pane context (state_machine.rs:292)。落地: **不在 DB 层调 writer (writer.rs:15), 放 orchestrator post-ready 路径**。RECOVERED job 不必新增状态, 简化为把 interrupted 的 FAILED job 翻回 QUEUED 走现有 dispatch (新增 RECOVERED 会牵动 API/wait/cancel/dispatch 语义)。

### master 执行态 [a3 flag 偷换概念 → a2 round-2 决断 → 呈 PM 确认]
- **a2 单一推荐**: 拒绝 master 端 heartbeat/instrumentation, 坚持 ActiveWork (session jobs 非终态 OR worker 活跃) 作为 master 执行态 **ground truth**。产品契约: **"对 Master PM, 执行态由且仅由托管资源活跃度定义; 未进入派发阶段的纯端侧规划视为不可靠瞬态, 死即灭。" 不改一行代码** (现 master_watch.rs:104/125 的 ActiveWork/IdleNoWork gate 即正确)。
- **第一性原理**: ① 薛定谔的执行 (黑盒无法区分沉思 vs 卡死, 心跳污染业务语义+破坏透明底座); ② 纯思考未落盘 → 复活无断点可续 (面对跟死前一样的世界), 只有派单/等 worker 才与外部锚定; ③ 对齐 PM #2 (无在途工作=空闲=不复活) + 防僵尸底线 (破损 master 死循环无限复活)。
- **状态**: 呈 PM 确认这个对 #1 "含 master 即时执行状态" 的诠释 (ActiveWork=master ground truth, 不加 instrumentation), 或 PM 坚持要 master 端真信号。**主控带推荐呈, 不 raw ABC。** 此点若 PM 确认 = master 侧零代码改动; 1e 按推荐写, PM 若 redirect 仅 ADD instrumentation work。
