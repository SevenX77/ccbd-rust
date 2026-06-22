# #3 hook-push — a4 (PM 替身) audit findings + disposition

audit: a4 (claude), job_b1068896, 2026-06-22. read-only。
**总结论**: #3 代码层正确性总体过关, **无阻塞 default-off WIP slice 合入的 must-fix**, 可进 step-9 (flag 仍 off)。但下列须在 flag-on / antigravity ready 前解决。

## Q&A 验证 (a4 确认正确的部分)
- a1 补的 2 测试 (`monitor.rs:200-244` pull-fallback / `router.rs:342-405` push-cancel+late-pull幂等) 断言与声称一致, 未误塞产品代码。
- push CAS 仲裁 (state_machine push transition) 正确; payload hook source 不串 log raw; flag-off 零注入 — 均核实通过。
- P3F: push 成功才 `registry::cancel` (agent.rs:564); hook 不到 → agent 留 BUSY → pull log monitor 兜底 (monitor 测试证明)。

## Findings + disposition

| # | 级别 | 问题 | 三轴 | 处置 |
|---|---|---|---|---|
| **F1** | **must-fix (flag-on 前)** | `realign.rs:322-331` 重建 spawn params 漏 `hook_push_enabled` (`agent.rs:341-346` 默认 false); 恢复快照 `AgentSpawnSpec` (agent.rs:265-273 / recovery.rs:51) 也不含该字段。worker 崩→revive/realign 后 push 通道**静默降级**只剩 pull (回 250ms 轮询)。宪法3 零容忍类。 | 证据High×影响Medium×置信A | **派 a1 修** (test-first): realign params + 恢复快照都带 `hook_push_enabled` 并回放。 |
| **F3** | **must (与 F1 耦合)** | hook 注入 (inject_claude/gemini/codex/antigravity_hook_push) 全是 append 无去重; 成功 spawn `SandboxDirGuard::release` 不擦目录, 恢复复用同 home_root → 第二次物化叠出第 2 条 ah hook。违反 design §9 "避免重复注入"。**F1 修好让恢复真带 flag 后, 这个 dup 就变 live。** | 证据Medium×影响Low×置信B | **随 F1 一起修**: 注入前先移除/覆盖同名 `ah-completion-push` named-hook (幂等注入)。 |
| **F2** | resolved (doc-sync) | impl 写 `.gemini/config/hooks.json`, design.md:163 旧写 `.gemini/antigravity-cli/settings.json` = 双真相。 | 证据High×影响Medium×置信A | **已修** (PM): design.md §5.4 + §4 表已同步到 pre-verify 验证的正确位置 `<managed_home>/.gemini/config/hooks.json`。impl 正确, 路径本体仍待 step-9 dogfood 实证。 |
| **F4** | nit (一致性) | hook 同步路径 `insert_evidence_denied_event` (state_machine.rs:526) 但 async wrapper `mark_agent_idle_hook_event` (:1201-1228) 丢 `denial_message`; marker 路径 (:1230-1265) 会 nudge denial 回 pane。即 hook 触发的证据拒绝只落 event 不 nudge agent (marker/pull 仍 nudge, 非彻底静默)。 | 证据High×影响Low×置信A | **随 F1 一起修** (同 hook 路径, 便宜): hook denial 也 nudge, 与 marker 路径一致。 |
| **F5** | nit (防御) | `home_layout.rs:543-546` 裸拼 agent_id/provider/socket 进 command。当前输入全 ah 受控无注入面, 但 agent_id 含空格/shell 元字符会破 arg 解析。 | 证据Medium×影响Low×置信B | **暂记, 后续折入** (无当前触发面)。防御性 quoting。 |

## 行动
1. ✅ F2 doc-sync (PM, design.md)。
2. ⏳ 派 a1 test-first 修 F1 + F3 + F4 (一个 round, 都在 hook/transition/恢复路径, 不需新 ahd)。
3. F5 记为后续 nit。
4. F1/F3/F4 修完 + 测试绿 → 才进 step-9 (那时 flag-on, F1 必须已修否则 revive 静默降级)。
