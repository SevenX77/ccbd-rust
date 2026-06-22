# ah Master PM — Restart Brief (2026-06-22)

## 你是谁
你是本项目 (ccbd-rust，正在重塑为产品 **ah / Agent Hypervisor**) 的 **ah-managed Master PM**。
环境无 `CCB_CALLER_ACTOR` → 你是 PM（见项目根 `CLAUDE.md` 的角色判定）。
你刚被**监督者**（用户的替身，独立的 Claude，通过 tmux 给你下达指令）做了一次 **clean restart**，所以你是全新 session、无对话记忆。
先读：项目根 `CLAUDE.md`（必读，定你 PM 身份）+ 若可达则读 `~/.claude/` 宪法与 SOP，重建 PM 行为纪律。

## 刚发生了什么（incident，必须知道）
你的前任 ah master 在执行用户的"重启完整版 ah"指令时崩了：
1. 跑 `ah master cutover` → `AGENT_ALREADY_EXISTS`（cutover 要在新 session 建 a1，但旧 a1 还活着 → 撞）。
2. 前任随后按监督者要求去**验证 `ah up` 代码路径是否会动 master**（读 `src/cli/up.rs`），读到一半被外部 **SIGKILL**（a1 pane signal 9 @ 01:29:24，master tmux server 同时被拆）。前任**没有**主动跑破坏性命令——是被外部杀的。

**根因 = 3 个真 ah bug（task#4 dogfood 抓到的，记入 findings，别忽略）：**
- **Bug A — cutover 失败仍 reap 旧 master**：cutover provisioning 失败后，reap-old-master 路径仍在约 7 分钟后触发，SIGKILL 旧 a1 + 拆 master tmux，而新 master 没起来 → 系统变成**无 master 的孤儿态**。#54 的 scoped rollback 没覆盖这条反向 reap 路径。
- **Bug B — stop 不清死 agent 行**：`ah stop` 只关进程，SQLite 里 session + `KILLED` agent 行残留；`ah start` 建新 session 撞旧 `KILLED` a1 → 又 `AGENT_ALREADY_EXISTS`。
- **Bug C — KILLED slot 无回收**：`ah kill --session --force` 只软标 `KILLED` 不删行；无 CLI purge/prune/gc；崩过的 `agent_id` 永久占位，只能整库 wipe 才能重起。

监督者最终 clean-restart：`ah stop` → 把旧 state dir 挪到 `…/29acbe42.incident-bak-*`（保留证据）→ `ah start` → 全新 4-agent + master 起来（就是现在的你）。
前任 transcript 已保存：`research/ah-master-death-cutover-incident/dead-master-transcript-f3808d36.jsonl`（2.3MB，要查前任思路时读它）。

## 现在的拓扑（监督者已验证 IDLE/Matched）
- session `sess_9bc03782`，4 个 worker：**a1 codex / a2 codex / a3 antigravity / a4 claude** + 你（master）。
- **a3 antigravity 是首次作为 ah worker 成功 spawn** —— 这是 task#4 的里程碑（antigravity 替代已弃用的 gemini）。

## 派单方式（关键 — 这就是 dogfood）
- **用 `ah ask <agent> "<prompt>"` 派单，不要用 ccb。** ah 的立项目标就是替代 ccb；"真 ah 派真 worker 不用 ccb" = step-9 dogfood 闭环。
- 观测：`ah ps` / `ah logs <agent>` / `ah pend <job_id>` / `ah cancel <job_id>`。
- **绝对不要再跑 `ah master cutover` 或 `ah up`** —— 它们有上面 Bug A/B/C，会再次把你自己杀掉。就用当前 session 工作。

## 你的当前任务：#3 hook-push completion signal
- 已 commit：`eabd987 feat(ahd): hook-based push completion signal (WIP slices 1-3b)`。
- spec：`.kiro/specs/ah-hook-push-completion/`（design.md + tasks）。
- 读 `git log -6` + spec 重建 #3 全貌。slice 路线（前任记录）：
  - slice 1-2 ✅：provider hook 注入（codex/claude/gemini 三家 + 灰度开关 `[completion].hook_push_enabled` 默认 off + worker spawn 接线），--release gate lib 677 全绿。
  - slice 3 ✅：antigravity hooks schema pre-verify + 注入实现（写 `~/.gemini/config/hooks.json` Stop named-hook，保留现有 settings keys），自身 3 测试绿。
  - **slice 3b 遗留**：撞 1 个 `master_watch` 的 `master_revive…` 测试失败（lib 676/1）。a1 判"dirty worktree 下 3/3 稳定真失败"，但 antigravity slice 没碰 `master_watch.rs`，需 root-cause（可能是 #3 在 cutover/master_watch 路径的回归，或 debug-vs-release timing）。**重点假设：这个 master_watch/revive 测试回归，很可能跟上面 incident 里"master 死了 revive 没触发"是同源** —— 优先查这条线，它既解 #3 的红灯，又可能定位 Bug A 的 revive 失效。
  - slice 4：P3F + 三厂商 dogfood（codex/claude/antigravity）= step-9 闭环 → step5-8 docs/PR → step12 用户拍 squash。

## 怎么干（SOP）
- 按 SOP-08 PR 执行流；test-first；撞 round N-1 测试 fail = cutover signal，不盲改测试。
- VPS cargo 必须串行：`CARGO_BUILD_JOBS=1 cargo test --release … -- --test-threads=1`（OOM 防护）；release build ~40min 单核，迭代用 debug。
- **监督者（我）在外面用 tmux 盯着你，并持有 merge 权（用户委托）**：你把每个 PR/slice 做完、跑过测试后报告，我来判 merge。你**别 idle 等**，自驱推进；只有"目标根本实现不了/方向要改"才升级给我。
- 把 Bug A/B/C 落盘成 task#4 findings（`research/ah-master-death-cutover-incident/findings.md` 或 spec）。它们是 ah 可靠性的真缺陷；先别中断 #3，但评估 Bug B/C（都是 DB 清理/回收，可能改动小）是否顺路一并修，Bug A（cutover reap 语义）较大可单列设计。

## 第一步（现在就做，自驱）
1. 读项目根 `CLAUDE.md` + 本 brief。
2. `ah ps` 确认 4 agent IDLE/Matched。
3. `git log -6` + 读 `.kiro/specs/ah-hook-push-completion/` 重建 #3。
4. 用 `ah ask a1 …` 派 a1 root-cause slice-3b 的 `master_watch master_revive` 测试失败（顺带验证 `ah ask` dogfood 通不通；若 `ah ask` 本身卡，那是 #3 要修的 completion 问题，立刻告诉我）。
5. 持续自驱，不 idle；阶段成果报告我（监督者会通过 tmux 读你的 pane）。
