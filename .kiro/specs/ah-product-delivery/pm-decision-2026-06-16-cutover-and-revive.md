# PM 决策记录 — 2026-06-16 (Step-4 收尾方向)

来源: PM 直接指令 (2026-06-16 06:04 / 06:14 UTC)。

## 决策 1: 先切 (terminal self-cutover 优先于补 gap)
PM 答"先切还是先补 gap" → **先切**。即 Step-4 终极 self-cutover 是优先目标。
主控判断 (待 PM 否决): 把 PM 自己切到 ah 托管的意义 = ah 能在 PM 被 kill 时正确自愈, 而"正确自愈" = 下面决策 3 的 revive/resume 机制。故主控计划: 先落实 revive/resume 机制 → 终极 self-cutover 作为干净的最后一步 (不在 PM 给方向的对话中途 reap 自己)。PM 若要立即切, 会另行指令。

## 决策 2: GAP-2 砍掉 (ahd 项目级, 不做系统级开机自启)
PM 原话: "ahd 是项目级的, 不要做成系统级重启, 不需要你说的那些什么机器整个重启后自动启动, 这太夸张了。"
→ 主控早先 GAP-2 框定 (静态 .service / survive-reboot 持久化) **作废**。项目会话内 ahd 崩了自动重启 (现有 transient unit 的 `Restart=on-failure`, src/cli/start.rs:51) 已够, 不再扩。

## 决策 3: revive/resume 统一机制 (GAP-1 收敛, PM 定的设计方向)
PM 统一思路 (原话整理):
1. **状态是什么**: 每个 agent (**含 master**) 要有一个**即时执行状态**, 用来判断被 kill 那一刻是不是**正在执行任务**。
2. **revive 门槛**: **只有「任务正在执行中」被 kill 才自动拉起**; 空闲被 kill **不自动拉起**。
3. **resume**:
   - 拉起时**恢复被 kill 前的 session 上下文** (用进程启动字段 or `/resume`);
   - 所以 session 的保存/记录要能在 revive 时**重新注入**;
   - 恢复 session 后**输入「继续」接着干原来的活**。

> 这取代主控早先的 GAP-1 框定 (当时只说"in-flight task 无自动 resume")。PM 把它细化成 state-gated revive + session-restore + 继续 三段。

## 落实路径 (SOP-08 §1.1 新颖/架构级设计)
- 1a research (a1, 只读): 摸清 ah 现状 — agent/master 执行状态跟踪、session 上下文存储、revive 触发条件、cutover seed 机制可否复用。brief: `/tmp/a1-research-revive-resume-state.md`。job_b6d621d15430 (in flight)。
- 1c 思路 (a2): 基于 research + PM 统一思路, 第一性原理设计机制。
- 1d/1f audit (a1+a3) → 收敛 → tests-first → impl → dogfood (用 ah 自己, 不用 ccb)。
- 收敛后 + revive 机制验绿 → 终极 self-cutover。

## 仍 PM-gated
- 终极 self-cutover (不可逆: reap 当前 PM session + 起继任 master) — PM 已"先切"授权, 主控作为干净最后一步执行。
- revive/resume 机制设计收敛后, 重大设计决策 (若有真分歧) 才 escalate。
