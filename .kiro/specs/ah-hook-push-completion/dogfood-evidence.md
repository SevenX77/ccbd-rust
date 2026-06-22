# #3 hook-push completion — dogfood / 活证据 (live evidence)

记录: 2026-06-22, ah-managed Master PM (clean-restart 继任)。

## 证据 1: completion-lag 活证据 (#3 要解决的核心问题, 现场抓到)

**现象**: 2026-06-22 派 a1 (codex) 跑 root-cause job (`job_2842c084`)。a1 pane 实际显示 "Worked 8m 36s" 已回到 idle (任务完成), 但 `ah ps` 仍报 a1 `BUSY`; master 的 `ah pend <job_id>` 因此**永久阻塞**, 直到人工 `ah cancel`。

**根因 (监督者诊断 + 实证)**: 当前**运行中的 ahd 是旧二进制** `/home/sevenx/.local/bin/ahd`, 文件时间戳 `2026-06-21 15:49`, 进程启动 `Jun 21 22:16` — 只编译到 `0fd2ec6` (#54), **不含 `eabd987` (#3)**。它带的是 completion-v2 **pull** (日志轮询) 模型, 却仍然没检测到 a1 这个已完成的 codex job → 这正是 #3 的 **push 模型**要消灭的 completion-lag。

**对 #3 的意义**: 这是 pull-only 完成检测漏判的真实生产现场。它佐证 design §1 / §3 的立项动机 ("消除 pull completion-lag")。**注意**: 这条证据**不是** #3 push 路径的忠实测试 (#3 代码不在运行二进制里, 且默认 flag-off), 只是证明问题真实存在。#3 push 的忠实端到端 dogfood 必须等 slice-4/step-9 用 HEAD rebuild ahd + 重启 session 后再做。

## 证据 2: slice-3b "master_watch revive 测试失败" = 测试隔离 flake, 非 #3 回归

**a1 root-cause 结论** (job_2842c084, `ah logs a1` 可复核):
- 实际失败测试: `monitor::master_watch::tests::master_revive_recovered_job_survives_stale_pane_dispatch_and_retries_new_pane`
- panic: `master_watch.rs:1713` `assertion left == right failed: dispatch must refresh the stale runtime pane binding before sending; left: Some("%1"), right: Some("%2")`
- 失败发生在 **revive 已完成、worker 已 IDLE 之后**的 recovered-job dispatch stale-pane 刷新断言 — **不是 revive 没触发**。
- `eabd987` 未改 `master_watch.rs` / `master_revival.rs` / `db/jobs.rs` / `db/recovery.rs`。
- 复跑: 单测单独 3/3 过, 全量 master_watch 过滤 3/3 过, **只首次当前源码全量跑失败 1 次** → 测试隔离 flake (同进程内前置测试遗留 tmux pane 编号/registry 全局状态污染), **非 debug 下 3/3 稳定失败**。
- 置信度: 中等 (panic 物理证据明确, 但无稳定复现)。

**结论**: 与事故 Bug A (master 死后 revive 不触发) **无直接关联**, 不证明也不反证 Bug A。slice-3b 解锁 — antigravity 自身 3 测试已绿, 这个 flake 单独记录 (见 task#4 reliability fix: pid-validate dispatch pane), **不阻塞 #3**。

## 证据 3: 旧 ahd 下的派单/完成不可靠 (两个现场, 均 #3/可靠性相关)

本 session 跑在旧 ahd (pre-#3) 上, dogfood `ah ask` 派单累计观察到:
- **codex 完成漏判 (completion-lag)**: a1 完成后 `ah ps` 仍 BUSY/STUCK, `ah pend` 永久阻塞 (证据1)。复现 ≥3 次 (root-cause job / slice-4 gap / F1F3F4 fix)。绕过法: 不信 ah 状态, 轮询 pane 真实内容 (spinner 消失) 判完成, 完后 `ah cancel` 释放。**这正是 #3 push 要消灭的。**
- **claude worker 派单 stale dispatch**: 给 a4 (claude) 派第二轮 audit 时, prompt **没真正提交到 a4 TUI**, a4 input box 残留无关文本 ("F1 修了再进 dogfood"), 状态 `IDLE→PROMPT_PENDING reason=unknown_prompt`, 派的 job 没运行。`ah cancel` + tmux send-keys (Escape/C-u/C-a/C-k/BSpace/C-c) 均**无法清除** a4 input box。绕过法: 改派 a2 (codex, 本 session 派单可靠) 承接该 audit。
  - **可靠性 finding (记 task#3 family)**: 旧 ahd 对 claude worker 的 dispatch 可能落不进 TUI 且不可恢复; 需在新 ahd 验证 (#3 含 dispatch 相关改动) 是否改善, 或单列 dispatch 可靠性修复。

## 当前策略 (监督者建议 + PM 采纳)

1. **现在**: 把 #3 实现 + 测试做完 (这些**不需要新 ahd**, 都是 unit/integration 测试)。
2. **不中途重启** ahd / session (会丢 master 上下文 + ~40min release rebuild; #3 在 git 安全)。
3. **slice-4/step-9 忠实端到端 dogfood 时**: 才 rebuild ahd from HEAD + 重启 session, 用真 Stop hook → 运行的新 ahd → push transition 证明三厂商不降级。
