# 结论: requirement #2 "OOM 后有意识重启 + resume 续断点" — 范畴重定 (research+design+双审 后)

> 2026-06-12 主控 PM 综合 a1(工程) + a3(PM 替身) 审计后定。配 `research.md` (地基) + `a2 design` (/tmp/ah-oom-dogfood/a2-design-oom-restart-resume.md, 已被双审判 NEEDS-FIX)。

## TL;DR

req#2 的 OOM 韧性分三个 facet, 实证后归类:

| Facet | 状态 | 证据 |
|---|---|---|
| **A. worker OOM (ahd 活着) → 自动 recovery + provider-resume** | ✅ **已闭环, dogfood-proven** | Case B/C/D (codex/agy/claude), 矩阵 45fcf8a, seed token 召回 |
| **B. ahd 自身 OOM → systemd 自动重启 (守护进程层)** | ✅ **已具备** | `Restart=on-failure` (src/cli/start.rs:51), 仅 direct-spawn fallback 无监督 (边角) |
| **C. ah-job 级在途任务续接 + ahd-OOM-后整套 resume** | ⛔ **未闭, 且 gated on master 重生 (SF1)** | 见下 |

**核心结论: facet C 不是一个独立的 Step-2 可交付物 —— 它的端到端闭合 (以及能否 dogfood 验证) 都被 "master 自身重生 (SF1)" 卡住。master 重生是已升级给用户的 goal-level 决策, 本质属 Step-4 (master 自换 ccb→ah) 范畴。facet C 应与 master 重生捆绑进 Step-4, 不在 Step-2 单独实施。**

## 双审实证 (为什么 facet C 现在做不了 / 不该现在做)

### a1 工程审计: a2 "DISPATCHED job 留着会自然接上" = 不成立 (must-fix)

- recovery 成功前 **DELETE FROM agents** (src/orchestrator/mod.rs:323), 而 `jobs.agent_id REFERENCES agents(id) ON DELETE CASCADE` (src/db/schema.rs:78-80, FK 已开 src/db/mod.rs:76-84) → 留着的在途 job 在 respawn 时**被级联删除**。
- 即使不删: 完成只在 agent 处于 SPAWNING/WAITING_FOR_ACK/BUSY 时触发 (state_machine.rs:315-321 / 462-468), respawn 强制置 IDLE (realign.rs:311-318), dispatcher 只扫 QUEUED (jobs.rs:82-90) → job **孤悬永不完成**。
- 真修需 (A) 显式重新入队, 或 (B) 完整 ownership-restore (避开 cascade-delete + 恢复 BUSY/WAITING + 重建 marker/log baseline + 触发 evidence 重判)。现有测试锁死当前 "fail dispatched job" 语义 (system.rs:1156-1180, tests/mvp8_acceptance.rs:441-461, tests/mvp9_acceptance.rs:471-492/535-558) → cutover discipline。

### a3 PM 替身审计: 端到端闭合依赖一个不存在的 master 重生 (must-fix)

- 旧 grep 实证曾显示 master 只由 `ah start` 经 `session.spawn_master_pane` 起、master 退出后不 respawn。PR #52 曾把 `ACTIVE` master raw exit cut over 为 `master_watch` revive **且 worker 不动** —— **该语义已被 PM 2026-06-14 推翻并重做** (见 `design-master-death-corrected.md` + commit 295508c)。
- **纠正后语义** (现行): master 死 → **无条件真 Reap 名下所有 worker** (防僵尸/孤儿) → 再按死时 A/B 决定是否 revive。**A/B 不按怎么死 (OOM vs clean-exit) 区分** —— PM 明确否决了 OOM-vs-clean-exit 路线 (`waitid(P_PIDFD)` 对非子进程 master 不可靠已实证); A/B 按 master 死时**有没有在跑任务** (任一 worker active / PROMPT_PENDING / QUEUED·DISPATCHED job → A 拉起+resume; 否则 B 不拉起 + ahd 常驻)。
- a3 定性 (准): master 重生 scope-out 给上层**架构上合理** (master 非 CCB worker, 符合 handoff §8 + SF1), **不是回避**; 但设计不能把不存在的东西画成已完成步骤。

## 关键洞察: master 重生 (SF1) 是剩余 OOM 目标的 lynchpin

facet C **和** "ahd-OOM→restart→整套 resume" (Step 3 的难半) **都**撞同一堵墙: ahd OOM 时 BindsTo 会连 master 一起杀, 而没有任何东西重生 master → 没有 realign 触发者 → worker 恢复链起不来。

所以: **master 自身重生 (SF1, 已升级用户) 是 req#2 剩余部分 + Step-3 ahd-OOM 半 + Step-4 的共同关键路径。** 它不是可选的延后项, 是 critical path。Step-4 (master 自换 ccb→ah) 本就是 "让 master 可恢复" 的载体 —— SF1 与 Step-4 是同一件事。

## 推荐 (主控自驱方向, 待 PM 在 SF1 上拍方向)

1. **facet A (worker-resume) 维持已闭** (Case B/C/D 不动)。
2. **facet C (ah-job 续接) + master 重生 → 捆绑进 Step-4**, 不在 Step-2 单独建 (现在建也无法 dogfood 验证, 违反"dogfood 才算闭")。
3. **现在可自驱**: Step 3 的 worker-OOM 并发峰值 smoke (ahd 活着, 多 worker 同时 OOM → 验 recovery+resume 在峰值负载下成立 + 无孤儿) —— 不依赖 master 重生, 验证 facet A 在并发下的韧性。
4. **ahd-OOM 半 + facet C** 等 SF1/Step-4 方向定后一起做。

## 待 PM 确认的唯一 goal 点 (其余全自驱)

"resume 续断点" 的本意 = **provider 上下文恢复** (worker OOM 后记得之前的任务上下文, 已闭) 即可? 还是要求 **ah-job 级在途任务作为被追踪 job 续到完成** (深改 + 须先有 master 重生)? 若是后者, 它与 SF1/Step-4 是同一捆绑。
