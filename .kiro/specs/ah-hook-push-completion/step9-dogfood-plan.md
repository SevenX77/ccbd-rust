# #3 step-9 dogfood 执行计划 + 重启 handoff

记录: 2026-06-22, ah-managed Master PM。
**前置**: 仅在 a4 audit 通过 (代码层无 must-fix) + 监督者同意做 disruptive 重启后执行。

## 为什么 step-9 必须重启 (不可避免)
真厂商端到端 dogfood = "真 Stop hook → **运行中的** ahd 收 `agent.notify` → push transition"。
当前运行 ahd 是旧二进制 (`~/.local/bin/ahd` @ 2026-06-21 15:49, 只到 #54, 不含 #3 的 `agent.notify`/push transition/hook 注入)。
所以必须 rebuild ahd from HEAD (含 #3) + 重启 session 让新 ahd 接管。**重启会杀掉当前 master (我) + 所有 worker, 丢 master 对话上下文** —— 但 #3 全在 git (commit eabd987 + 5ce70e2 + 1cb969a), spec/findings/本计划全落盘, fresh master 可据此 resume。

## 重启前置 (current master 在被杀前必须确认已落盘)
- ✅ #3 代码: git (eabd987 slices1-3b + 5ce70e2 P3F tests)
- ✅ 设计/证据/findings/本计划: `.kiro/specs/ah-hook-push-completion/` + `research/ah-master-death-cutover-incident/`
- ✅ task 状态: 见本文件末"任务快照"
- ⬜ a4 audit 结论 + 任何 audit 修复 (audit 后补)

## 执行步骤 (fresh master 或监督者按序执行)

### 0. 确认环境
- `git -C /home/sevenx/coding/ccbd-rust log --oneline -4` 应见 1cb969a/5ce70e2/eabd987。
- `git status` 干净 (或仅无关 untracked)。

### 1. rebuild ahd from HEAD (release, ~40min 单核)
派 a1: `CARGO_BUILD_JOBS=1 cargo build --release --bin ahd --bin ah`
(VPS OOM 防护: 串行单核。release 必须, 因 dogfood 是端到端真二进制。)
产物: `target/release/ahd` + `target/release/ah`。

### 2. 干净重启 session (避开事故 Bug A/B/C)
**严禁** `ah master cutover` / `ah up` (Bug A 会 reap 旧 master 致孤儿态)。
按监督者上次 clean-restart 成功流程:
1. `ah stop` (关进程)
2. **挪旧 state dir 保留证据** (避开 Bug B/C 的 KILLED 行残留撞 `AGENT_ALREADY_EXISTS`):
   `mv ~/.local/state/ah/<id> ~/.local/state/ah/<id>.pre-step9-bak-$(date +%s)`
   (上次 incident 用的是 `29acbe42` → `.incident-bak-*`; 本次确认当前 active id 再挪)
3. 安装新二进制: `cp target/release/ahd target/release/ah ~/.local/bin/`
4. `ah start` → 起全新 session + 4 worker (a1 codex/a2 codex/a3 antigravity/a4 claude) + 新 master。
5. `ah ps` 确认 4 agent IDLE/Matched + 新 ahd 二进制时间戳是刚 build 的。

### 3. 开灰度开关 (push enabled)
编辑 `ah.toml` 或对应 config:
```toml
[completion]
hook_push_enabled = true
hook_push_events = ["stop"]
hook_push_providers = ["claude", "codex", "antigravity"]
```
(确认 worker spawn 时会注入 hook —— 可能需重起 worker 让新 home materialization 生效。)

### 4. 三厂商 dogfood (逐家, design §12 末 3 项)
对 codex / claude / antigravity 各跑一遍:
1. `ah ask <agent> "ping, reply pong"` (真派单 → worker 真干活)。
2. worker 完成 → provider 触发 Stop hook → hook command `ah agent notify --agent-id <id> --event stop ...` → 运行 ahd 收 `agent.notify`。
3. 观测 (物理实证, 不信 ah 状态信号也要交叉看):
   - `ah logs <agent>` 看 `--- state_change ... "source":"hook" ... ---` (push 赢) 而非 LogEvent。
   - agent 从 WAITING_FOR_ACK/BUSY → IDLE, job COMPLETED, 延迟明显低于旧 pull。
   - 验证 pull monitor 没造成二次 transition。
4. **关键对照**: 这次 `ah pend <job_id>` 应该**不再卡死** (push 让 ahd 及时检测完成) —— 直接对照本 session 旧 ahd 下 `ah pend` 卡死的 completion-lag 证据 (dogfood-evidence.md 证据1)。这是 #3 价值的最强 proof。

### 5. fallback 验证 (flag 仍保 pull)
临时让一家 hook 失败 (或 flag-off 单家), 确认 pull monitor 仍兜底完成 → 证明 push-primary/pull-fallback 不降级。

### 6. 闭环判定 (step-10)
三厂商 push 都通 + fallback 通 → #3 step-9 闭合。否则找 gap → 回设计/实施 (不抛监督者拍工程细节, 仅"目标实现不了"才升级)。

### 7. step5-8 docs/PR (闭合后)
- mvp/logic-explained 文档同步 + PR report (a1 主笔) + a4 honesty audit。
- 报告监督者 (持 merge 权) 拍最终 squash (step-12)。

## 任务快照 (供 fresh master resume)
- #1 ✅ root-cause flake (测试隔离, 非回归)
- #2 in_progress: slice-4 = (4a 非dogfood ✅ 代码完整+测试全绿, commit 5ce70e2) + (4b dogfood = 本计划, 待执行)
- #3 pending: incident Bug B/C (DB 清理, 小改动, 可顺路) + Bug A (reap 语义, 设计级, 单列)
- #4 pending: reliability fix `resolve_current_dispatch_pane` pid 校验 (flaky test 根因, 非阻塞 #3)
- a4 audit job_b1068896: 结论待收 (见 `ah logs a4`)
