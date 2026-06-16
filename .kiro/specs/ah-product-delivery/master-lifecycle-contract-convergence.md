# Master 生命周期契约 — a2+a1 收敛结论 (2026-06-16)

> 起因: Step-4 (master 自换 ccb→ah) 推进中, 发现 a2 早先的 OOM-restart-resume 设计 (`a2-oom-restart-resume-design.md`) 的结论 "ahd 不应 revive master" 跟 Step-4 已验绿的 "ahd 自愈 revive ah-托管 master" 有张力。本文件是 a2 (架构师) 重新收敛 + a1 (工程审计) 对照代码实证后的统一结论。
>
> **净结论: 张力是表面的 — 系统有两种 master 模型, 两者不冲突, Step-4 架构正确。**

## 1. 两种 Master 生命周期模型 (a2 收敛)

| | **Model A: External Master (冷切前)** | **Model B: AH-Managed Master (冷切后)** |
|---|---|---|
| 特征 | 用户 SSH 里裸跑的 `claude`, 用 `ah ask` 派单但自身存活由外部 (SSH/用户 tmux) 管 | `ah master cutover` / `spawn_master_pane` 由 ahd 主动 spawn 的 master pane |
| ahd 视角 | 看不见, 无 spawn_spec | 编排图谱一等公民: 有完整 cmd/env/hooks/plugins + pidfd + systemd scope |
| 谁 revive | **用户 / 外部 harness** | **ahd** (master_watch self-revive) |

a2 原设计 "ahd 不该 revive master" **只针对 Model A** (ahd 擅自拉起后台 claude → 用户 SSH 看不到输出 → 幽灵进程)。Step-4 的 revive 是 **Model B**, 由 ahd spawn 故由 ahd 自愈是天经地义的闭环责任。**两者不冲突。**

## 2. 统一契约表 ("谁管生育, 谁管复活") + a1 代码实证

| 实体 | 谁 Spawn | 死亡检测 | 清理 | 谁 Revive | a1 审计 | 关键 file:line |
|---|---|---|---|---|---|---|
| **Worker** | ahd (`agent.spawn`) | ahd monitor (pidfd→CRASHED) | ahd Reap | **ahd** orchestrator recovery | ✅ 实现 | agent.rs:78/252, agent_watch.rs:47, agents_lifecycle.rs:85, orchestrator/mod.rs:219/296, recovery.rs:158 |
| **Managed Master** | ahd (`spawn_master_pane`) | ahd master_watch | ahd Reap 级联 Worker | **ahd** self-revive | ✅ 实现 (resume 是 GAP-1) | sessions.rs:355/408, master_watch.rs:49/293/327/340/370, master_revival.rs:61 |
| **级联 reap (唯一真决策点)** | — | — | master 死→snapshot worker→clean→stop scope+SIGKILL+标 KILLED | — | ✅ 实现, KILLED/CRASHED 分离正确 | master_watch.rs:104, system.rs:166/224/252/294/327, agents_lifecycle.rs:28/85, orchestrator/mod.rs:224 |
| **External Master** | 用户/外部 | ahd 无法主动检测 | ahd (外部发 session.kill) | **用户/外部** | ✅ 实现 (revive 只认 ahd-spawned + session ACTIVE + pid/gen 匹配) | sessions.rs:408/88/99/105, master_revival.rs:85 |
| **ahd 自身** | systemd | systemd | systemd / startup_reconcile 验尸 | **systemd** Restart=on-failure | ⚠️ 部分 (见 GAP-2) | start.rs:47/51, ahd.rs:56, system.rs:513/579, ah.rs:348 |

**唯一真决策点 (master-OOM vs 反孤儿级联杀) 已在代码中正确实现**: master 死时先连坐清 workers (标 KILLED 防僵尸/孤儿), recovery loop 只扫 CRASHED 不会误复活被连坐的 victim; revive 时重新预建 declared workers + 恢复 sandbox_overrides snapshot。跟 2026-06-14 修正语义 ([[project-master-death-corrected-semantics]]) 一致。

## 3. 两个 GAP (下一轮实施 input)

- **GAP-1 (in-flight task 无自动 resume)**: worker crash/KILLED 把 DISPATCHED job 标 FAILED (agents_lifecycle.rs:34, jobs.rs:425); master revive 只写 redispatch marker 提示人工 re-dispatch (master_watch.rs:207/515, master_cutover.rs:65)。即 "OOM-restart-**resume**" 的 resume 半部分未自动化。⚠️ 触及 2026-06-14 刚修正的 master-death/resume 语义, 实施前需 PM 知情。
- **GAP-2 (ahd 无静态 .service unit)**: 只有 transient `systemd-run --user --unit=ahd.service` bootstrap (失败 fallback direct spawn, ah.rs:348)。若契约要求"安装态 unit 文件存在"(survive-reboot 持久化) 则未兑现。

## 4. a2 事实修正

a2 撤回早先 "`reconcile_orphan_scopes` 漏接" 的判断: 经查已正确接入 `src/db/system.rs:523` (`reconcile_orphan_scopes_sync` in `reconcile_startup_sync_with_state_dir`)。孤儿回收逻辑完整活跃。

## 5. 对 Step-4 终极验收的影响

- Step-4 核心 (冷切 + ah-托管-master 自愈 + 级联 reap + worker 重建) **已 dogfood 验绿 (committed 257bea4) 且契约审计确认实现正确**。
- 两个 GAP **都不阻塞 cutover 机制本身** (GAP-1 是 resume 鲁棒性, GAP-2 是 survive-reboot 持久化)。
- **终极 self-cutover (切主控自己) 仍须 PM 授权** (不可逆: reap 当前 PM session + 起继任 master)。
