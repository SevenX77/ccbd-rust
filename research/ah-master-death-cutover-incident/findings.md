# ah master-death cutover incident — Findings (task#4 dogfood)

记录日期: 2026-06-22
记录人: ah-managed Master PM (clean-restart 后的继任 master)
来源: 前任 master 在执行 "重启完整版 ah" 时崩溃的事故复盘 (transcript `dead-master-transcript-f3808d36.jsonl`) + `RESTART-BRIEF.md`。

## 事故时间线 (what happened)

1. 前任 master 跑 `ah master cutover` → 报 `AGENT_ALREADY_EXISTS`。cutover 要在新 session 建 a1, 但旧 a1 还活着 → 撞名。
2. 前任随后按监督者要求验证 `ah up` 代码路径是否会动 master (读 `src/cli/up.rs`), 读到一半被外部 **SIGKILL** (a1 pane signal 9 @ 01:29:24, master tmux server 同时被拆)。前任**未**主动跑破坏性命令 — 是被外部杀的。
3. 系统进入**无 master 的孤儿态**: 旧 master 死, 新 master 没起来, revive 没补上。
4. 监督者最终 clean-restart: `ah stop` → 旧 state dir 挪到 `…/29acbe42.incident-bak-*` (保留证据) → `ah start` → 全新 4-agent + master (a1 codex / a2 codex / a3 antigravity / a4 claude)。

## 三个真实 ah 缺陷 (defects)

### Bug A — cutover 失败仍 reap 旧 master (孤儿态)

| 轴 | 评级 | 依据 |
|---|---|---|
| 证据度 | **High** | 事故现场: cutover provisioning 失败 (AGENT_ALREADY_EXISTS) 后, reap-old-master 路径仍在约 7 分钟后触发, SIGKILL 旧 a1 + 拆 master tmux。 |
| 影响度 | **High (Critical)** | 系统失去 master, 进入孤儿态, 只能人工 clean-restart 恢复。生产级可靠性红线。 |
| 方案置信度 | **C → 待设计** | 这是 reap 语义的设计缺陷 (cutover 失败 vs 成功的 reap 决策没有正确门控), 不是一行补丁。需独立设计。 |

**根因 (initial)**: #54 (`0fd2ec6` master cutover readiness gate + scoped rollback) 的 scoped rollback **没有覆盖**这条 "cutover 失败后反向 reap 旧 master" 路径。即 rollback 回滚了新 master 的 provisioning, 却没阻止 reap-old-master 定时器继续杀旧 master。
**待 a1 root-cause 关联**: 这条 revive/reap 失效, 极可能与 slice-3b 遗留的 `master_watch` `master_revive` 单测在 debug 下稳定失败**同源**。a1 正在查 (job_2842c084)。若坐实, Bug A 既是事故根因, 也是 #3 红灯根因。
**修复方向 (待设计, 非定稿)**: cutover 失败路径必须把 reap-old-master 决策门控在 "新 master 已确认 ready (in-band ack 通过)" 之后; 失败/rollback 时必须取消 reap timer 并保留旧 master。

### Bug B — `ah stop` 不清死 agent 行

| 轴 | 评级 | 依据 |
|---|---|---|
| 证据度 | **High** | `ah stop` 只关进程, SQLite 里 session + `KILLED` agent 行残留; `ah start` 建新 session 撞旧 `KILLED` a1 → `AGENT_ALREADY_EXISTS`。 |
| 影响度 | **Medium** | 不直接杀 master, 但导致 stop→start 循环无法干净重起, 必须人工挪 state dir。 |
| 方案置信度 | **B → 改动可能小** | DB 清理逻辑, 顺路可一并修 (评估中)。 |

**根因 (initial)**: `ah stop` 的关停逻辑只 terminate 进程, 没有在 stop 时清理或归档同 session 的 agent 行 (尤其 `KILLED` 状态行)。
**修复方向 (待评估)**: `ah stop` 应在关进程后, 把该 session 的 agent 行清理/归档 (或 start 时对 `KILLED` 行做 upsert 而非撞名 reject)。

### Bug C — `KILLED` slot 无回收

| 轴 | 评级 | 依据 |
|---|---|---|
| 证据度 | **High** | `ah kill --session --force` 只软标 `KILLED` 不删行; 无 CLI purge/prune/gc; 崩过的 `agent_id` 永久占位, 只能整库 wipe 才能重起。 |
| 影响度 | **Medium** | 同 Bug B, 阻碍干净恢复; 长期看每次崩溃都污染 agent_id 命名空间。 |
| 方案置信度 | **B → 改动可能小** | 加 CLI purge/prune + slot 回收语义, 顺路可一并修 (评估中)。 |

**根因 (initial)**: `KILLED` 是软标状态, 没有任何回收路径 (无 prune CLI, start 不复用 KILLED slot)。
**修复方向 (待评估)**: 加 `ah agent prune`/`ah gc` CLI 回收 `KILLED` 行; 或 `ah start` 复用同名 KILLED slot。

## 修复策略 (PM 评估, 待 a1/a2 确认)

- **Bug A** = 设计级 (reap 语义), 单列设计 PR, 不混进 #3。但 a1 root-cause 若证明它跟 #3 master_watch 红灯同源, 则 #3 的修复会顺带定位 A 的机制。
- **Bug B / Bug C** = DB 清理/回收, 改动可能小, 评估是否顺路一并修 (作为 #3 之后的小 PR, 或独立 reliability PR)。
- **当前优先级**: 先不中断 #3 (hook-push completion), 但把 A/B/C 落盘 (本文件)。a1 root-cause 回来后再定 A 是否前置。

## 派单纪律提醒 (dogfood)

- 用 `ah ask <agent>` 派单 (不用 ccb) = step-9 dogfood。
- **绝不**再跑 `ah master cutover` / `ah up` (含 Bug A/B/C, 会再次杀 master)。就用当前 session `sess_9bc03782` 工作。
