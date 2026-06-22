# SUPERVISOR → ah Master：恢复 #3 最后一步（chunk0 LIVE 证）

你是 **ah Master PM**。我（外部 supervisor）刚为部署 seam 修复重启了 ahd，期间你被换进了一个**全新 session**——你上一个 session 的 in-context 记忆没了。**别慌**：所有关键产物都在磁盘上，按下面重建上下文即可。完整旧 transcript（仅备查，通常不需要）在 `~/.cache/ah/sandboxes/d86ad60e7b7c/.claude/projects/-home-sevenx-coding-ccbd-rust/2f3a3588-995c-4fd7-acf6-f6bcf8eff5fb.jsonl`。

## 第一步：重建上下文（必读）
1. 读 `.kiro/specs/ah-hook-push-completion/step9-redogfood-backstop-handoff.md`（#3 完整 handoff，含 ROUND 1/2/3）。
2. 这是 #3「step-9 UI-only 完成兜底（backstop）」dogfood 的**最后一步**。背景：antigravity 是 **UI-only**（无 log-monitor，hook-push 物理不可能，因为 agy 生成中途会被 SIGKILL）。完成检测靠 `pane_diff_watcher_tick` 复用它的 30s capture：对 `UiOnly + BUSY` agent 在 pane 底部视窗跑 MarkerMatcher，连续 `AH_UI_COMPLETION_STABLE_TICKS`（默认 2）次稳定匹配 → 标 idle。

## 现状：seam 修复已部署
- 我已 `ah stop` → cp `target/debug/{ahd,ah}` → `ah start`。**installed ahd = 18:15:25**（= 你 17:11 那版 seam-fix 内容）。
- 这版包含：(a) UI-only 完成兜底；(b) STUCK-rescue 修复；(c) **seam 修复**——`UiCompletionRecapture` 把 recapture 的**原始 pane 快照**（`observation.text`，非 sanitized）带到状态机；UiOnly recapture 入口在 chunk reply 为 prompt-only 时用 `distill_reply(pane_snapshot)` 刮 pane-reply → `BUSY→IDLE+交付`，仍 prompt-only 则同 transaction 内 `BUSY→STUCK`（新 reason `UI_COMPLETION_RECAPTURE_PROMPT_ONLY`），杜绝静默永 BUSY；(d) a4 audit 抓的 2 个 must-fix 已修：soft-wrap 误判 STUCK（prompt 软换行 → 归一化空白再匹配，gate 与 distill 共用）+ 抽共享 `mark_agent_idle_matched_conn_inner` 消死代码。
- 新 session，a1/a2/a3/a4 已 fresh 拉起。a3 = fresh，可作 chunk0 证的干净对象。

## #3 目标 + 两路证明
**目标**：物理证明 UI-only（antigravity）经周期 recapture 兜底完成是**lag-safe**（reader 漏读时不会卡死/丢 reply）。
- **(b) chunks>0 lag 路**（真 ROUND1 目标）：burst provider live 诱不出（agy 把 reply+idle-prompt 一个 <64KB 原子 burst 吐出，reader 一次读光 → 只能产 chunk0 或完整完成，物理无法复现 reader-alive+chunks>0+matcher-lagged 的真 lag 窗口）。**正确方法 = deterministic 真实字节集成测试**，上一 session 已 PASS。**请在部署源复跑确认**：`cargo +stable test --lib recapture`（应 12/0，含断言 `reply=="charlie"` / `reply=="delta"` 的真字节测试）。
- **(a) chunk0 死-reader 路**：**现在 LIVE 证**（见下）。

## 你的任务（自驱，按 ah SOP，开 60s loop 盯自己的 a3 dogfood）
1. **复跑集成测试**确认 lag 路（上面 (b)）。
2. **chunk0 LIVE 证**：构造 a3 的 chunk0（kill a3 的 stdout pipe / reader，使 collect_reply 得 chunk_count=0）→ `ah ask a3` 一个短任务（让 a3 真生成一句可辨识 reply）→ 预期：seam 修复让 recapture 从干净 pane 刮出 pane-reply → **a3 `BUSY→IDLE` 且交付非空 reply**（**不**静默永 BUSY、**不** <60s STUCK）；若 pane 本身确实 prompt-only 才允许 STUCK（reason `UI_COMPLETION_RECAPTURE_PROMPT_ONLY`）。
3. **铁证采集**（物理实证，非 self-report）：
   - `journalctl --user -u ahd.service` 找 recapture 标 IDLE 的行 + `reply_len>0`；
   - `ah ps` 看 a3 从 BUSY 转 IDLE；
   - 交付的 reply 内容 = 你给 a3 的任务应得的内容。
4. **聚合证据** → 写进 handoff 的 ROUND 4 段（journald 行号/内容 + 测试名与结果 + ah ps 截状）+ 在你的 pane 打印简明汇总。我（supervisor）会 capture-pane 读你的汇总并复验。

## 边界铁律（绝对守住，等我复验）
- **不 commit、不 merge、不动 `hook_push_providers`、不报 "done"。**
- 证据齐了 → 报我（写 ROUND 4 + pane 汇总）→ 我复验 → 由我统一向用户汇报（含证明方法从 pipe-kill 调整为「chunk0 LIVE + lag 集成测试」的透明说明）+ 拍最终 squash merge。
- 你不是被授权 merge 的那个；merge 权在我（supervisor）手里，我对用户负责。
