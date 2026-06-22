# step-9 RE-dogfood handoff — UI-only completion recapture 兜底 (给 deploy 后的 fresh master)

> ⚠️ **最新真相 = ROUND 4 (下面这段)。ROUND 3/2/1 是历史背景。ROUND 4 = #3 两路证明全部物理闭合, 待 supervisor 复验。**

---

## ROUND 4 (2026-06-22 18:2x–18:30) — seam-fix 部署后两路证明全闭合 (PM 物理实证, 待 supervisor 复验)

部署确认: installed `ahd`/`ah` mtime = **18:15** (= 17:11 seam-fix 内容); state_dir = `/home/sevenx/.local/state/ah/29acbe42/`; session `sess_2204…` 4 agent fresh IDLE (a1/a2 codex, **a3 antigravity = chunk0 证对象**, a4 claude)。源码树有未 commit 的 seam-fix 改动 (符合边界: 不 commit)。

### 路 (b) chunks>0 lag — deterministic 真字节集成测试 PASS
部署源复跑 `cargo +stable test --lib recapture` → **12 passed / 0 failed / 0 ignored** (编译 1m49s, run 0.55s)。含决定性真字节测试:
- `ui_only_recapture_completes_busy_job_from_real_pane_when_chunks_zero` (chunk0 真字节)
- `ui_only_recapture_completes_busy_job_from_real_pane_when_chunks_prompt_only`
- `ui_only_recapture_completes_busy_job_from_real_wrapped_prompt_pane` (soft-wrap)
- `ui_only_recapture_marks_busy_agent_stuck_when_real_pane_is_prompt_only` (prompt-only → STUCK 反向)
- `test_ui_recapture_can_mark_stuck_agent_idle_without_opening_live_marker_guard`
- + viewport / anti-pattern / CAS / stable-tick-counter 共 12。

### 路 (a) chunk0 死-reader — LIVE 证 PASS
**构造**: `tmux -L ahd-9819d8d7587886a9 pipe-pane -t %3`(无命令=关掉 a3 的 pipe-pane writer)→ `pipe=0` 死 reader → `collect_reply` 拿 chunk_count=0。然后 `ah ask a3 "Reply with exactly one word and nothing else: zulu7delta"`(job `job_e3c25106-1c27-4450-84f2-01f072d67c90`)。

**物理实证 (非 self-report)**:

1. **死 reader 确认**: dispatch 全程 `%3 pipe=0`(reader 物理断开 = chunk0)。a3 pane 自身渲染出真 reply `zulu7delta` + idle prompt `>`(reader 看不到, 只有 pane 有)。

2. **journald recapture 全链** (`journalctl --user -u ahd.service`):
   ```
   18:29:37  pane_diff watcher tick busy_agents=1
   18:29:37  pane_diff UiOnly scan agent_id=a3 provider="antigravity" scan=Matched consecutive_ticks=1
   18:30:07  pane_diff watcher tick busy_agents=1
   18:30:07  pane_diff UiOnly scan agent_id=a3 provider="antigravity" scan=Matched consecutive_ticks=2
   18:30:08  pane diff UI completion recapture changed agent state agent_id=a3 disposition=MarkedIdle
   ```
   严格走满 2 tick (consecutive_ticks 1→2), tick2 fire recapture → **MarkedIdle**(非 MarkedStuck → 证明 `distill_reply` 从干净 pane 刮到**非空** pane-reply; 若 prompt-only 会是 `disposition=MarkedStuck` reason `UI_COMPLETION_RECAPTURE_PROMPT_ONLY`)。

3. **a3 状态机** (`ah logs a3` state_change 序列):
   ```
   IDLE → (dispatched) WAITING_FOR_ACK → (ACK_VISUAL_DIFF) BUSY → (UI_COMPLETION_RECAPTURE_MATCHED) IDLE
   ```
   `ah ps`: a3 `BUSY`(18:29:46)→`IDLE`(18:30:12)。**全程零 STUCK**(`ah logs a3 | grep STUCK` 空)= 不静默永 BUSY、不 <60s STUCK。转 IDLE 的 reason = `UI_COMPLETION_RECAPTURE_MATCHED`(recapture 路径, 不是 reader)。

4. **reply 真交付**: `ah pend job_e3c25106…` 返回 → **`zulu7delta`**(= 我给 a3 的任务应得内容, 经 recapture 死-reader 路径完整交付给 master)。

**结论**: seam-fix 让 UiOnly + 死 reader (chunk0) 场景下, 周期 recapture 从原始 pane 快照刮出 pane-reply, `BUSY→IDLE` 且交付非空 reply, 杜绝静默永 BUSY 与误 STUCK。#3 目标(UI-only 经周期 recapture 兜底完成是 lag-safe)两路 (a)+(b) 物理证明全闭合。

### 边界遵守 + 善后
- 未 commit / 未 merge / 未动 `hook_push_providers` / 未报 done — 等 supervisor 复验。
- 善后: 已 `pipe-pane -t %3 "cat > …/a3.fifo"` 恢复 a3 pipe writer(`pipe=1`)。注: ahd 侧 reader task 在 EOF 后可能已退出, a3 当前 state=IDLE 健康但下次派单的 reader 完成路径如需完全恢复可由 supervisor 决定是否重启 a3。

### 透明说明 (证明方法调整, 供 supervisor 向用户汇报)
原计划「pipe-kill 触发 lag」对 burst provider (agy 把 reply+idle-prompt 一个 <64KB 原子 burst 吐出, reader 一次读光) 物理无法复现 reader-alive+chunks>0+matcher-lagged 的真 lag 窗口。故拆两路证明: (b) chunks>0 lag 用 deterministic 真字节集成测试(物理无法 live 诱出, 改确定性字节); (a) chunk0 死-reader 用 LIVE 证(pipe-kill 制造 chunk0 → recapture 刮 pane 兜底)。两路覆盖 reader 漏读的全部退化形态。

---

## ROUND 3 (2026-06-22 09:5x) — tracing 定位根因 + a1 修复完成, READY FOR DEPLOY

### Step 2 重跑 + tracing 一举定位 (PM 实证)
制造 reader-miss (`tmux pipe-pane -t %3` 关掉 a3 的 pipe → reader EOF), `ah ask a3`。a3 pane 完成 (干净 idle prompt) 但停 STUCK, recapture 零触发 (复现 ROUND 2 的 FAILED)。per-tick tracing 给出决定性证据:
```
09:28:37 watcher tick busy_agents=1 ; UiOnly scan a3 scan=Matched consecutive_ticks=1   ← matcher 在线上认得! 口径也对!
09:29:07 watcher tick busy_agents=0                                                       ← a3 已被标 STUCK, 离开 BUSY scan-set
(recapture 转 IDLE 日志从未出现)
```
`ah logs a3`: `BUSY→STUCK {reason:PANE_DIFF_STUCK, signal_kinds:["state_machine"], elapsed_secs:0}` — <60s 内就 STUCK。

### 根因 (已证, 跟 ROUND 2 "matcher 不是 bug" 一致, 是更深的 race + 守卫缺口)
1. **race**: recapture 要连续 2 tick (≥60s) 才 fire (`pane_diff/mod.rs:155`); 但 reader-dead 的 UiOnly agent 在 <60s 被 **completion-staleness health check** 标 STUCK (`health_check.rs:46` — 无新 output → `last_progress_ts` 看起来 stale > 300s 阈值 → 几乎即时 STUCK)。
2. **观测丢失**: watcher 只 query BUSY (`pane_diff/mod.rs:256`), a3 一 STUCK 就掉出 observation → `state_map.retain` (`:228`) 删掉 `consecutive_ticks` 计数器 → 永到不了 tick 2。
3. **守卫缺口**: `mark_agent_idle_matched` (`state_machine.rs:388`) 只允许 SPAWNING/WAITING_FOR_ACK/BUSY→IDLE, STUCK 被排除。

### a1 修复 (防御纵深 A+B, 已 build + test 全绿; 未 commit / 未 deploy)
- **源头 (防过早 STUCK)**: `health_check.rs:46` 对 UiOnly provider 跳过 completion-staleness 判定 ("leave stale-output judgment to pane_diff recapture")。
- **兜底 (STUCK 也能救)**: `pane_diff/mod.rs` 新 `query_ui_completion_recapture_agents` = query BUSY + STUCK(仅 UiOnly); 新 `state_machine::mark_agent_idle_recaptured` (`allow_stuck_recapture` gated) 允许 **仅 recapture 路径** STUCK→IDLE, live reader `mark_agent_idle_matched` 守卫 byte-identical 不变。
- **可观测性**: 补 `health check marked agent STUCK` 日志 (堵 silent STUCK 缺口)。
- must-fix #1/#2/#3 全 reconcile + 有测试: `test_ui_recapture_can_mark_stuck_agent_idle_without_opening_live_marker_guard` (live_changes==0 / recapture_changes==1) / anti-pattern / viewport / CAS / 计数器存活 / 全完成路径 — 共 12 新测试。

### PM verify (cargo +stable, 物理实证)
- `cargo build` 绿 (target/debug/{ahd,ah} mtime 09:50)。
- 全 lib: **696 passed / 1 failed / 3 ignored**。唯一 fail = `monitor::master_watch::...master_revive_stale_inflight_dispatch_failure_does_not_overwrite_requeued_job` — **stash a1 改动后在 clean HEAD 同样 fail (同 panic 同 10.72s)** = 预存在 committed 失败 (master-revival WIP), 跟本修复无关, 不阻塞本 deploy (另案 tech debt)。
- 集成: mvp7_acceptance 6/0, pr4c_hooks_plugins 21/0。recapture+守卫 9 测试显式全 ok。

### DEPLOY 后 fresh master 要验什么 (Step 2 真过的硬门槛)
重跑 Step 2 (同上制造 reader-miss + `ah ask a3`)。这次**应该**看到:
| 期望 | = 兜底成立 |
|---|---|
| a3 **不再** 在 <60s 被 health_check 标 STUCK (UiOnly 跳过 completion-staleness) | 源头修生效 |
| `pane_diff UiOnly scan a3 scan=Matched consecutive_ticks=1` 然后 `=2` | 计数器存活到 tick 2 |
| `pane diff UI completion recapture marked agent IDLE` + a3 BUSY→IDLE + job COMPLETED + master 收到 reply | **Step 2 PASS, #3 闭合** |
- 若万一仍走到 STUCK (别的快速路径), 应看到 STUCK→IDLE via recapture (`reason=UI_COMPLETION_RECAPTURE_MATCHED`)。
- 顺带验 Step 1 (正常即时完成不退化) + Step 3 (生成期不误转)。

---

## ROUND 2 (2026-06-22 09:1x) — tracing build, 给运行期 watcher 加可观测性

### 上一轮 re-dogfood 结果 (完整: `REDOGFOOD-RESULT-step9.md`)
- **Step 1 正常路径 PASS** (reader ~15s 转 IDLE, recapture 不抢活)。
- **Step 2 硬门槛 FAILED**: 杀 a3 pane→FIFO 管道模拟 reader-miss 后, a3 在 pane 里完成了 (reply "bravo" + idle prompt) 却停 BUSY→STUCK, **recapture 整 session 零触发**。
- 根因隔离: recapture **逻辑全对** (2 个真实字节单测绿: `antigravity_real_idle_capture_matches` + `ui_only_marker_recapture_completes_real_antigravity_idle_capture_after_two_ticks`), hash 稳定, agent BUSY/active, watcher 无条件 spawn, 无 panic。→ **运行期(operational) gap: 线上 pane_diff watcher 没有效处理到 a3**。`matcher viewport` 不是 bug (我最初的猜测被真实字节单测推翻)。

### 这轮加了什么 (src/pane_diff/mod.rs, 仅加日志 + 测试, 零行为改动, 未 commit)
按 logging SOP 加 3 处**永久** `tracing::info` (不是一次性脚手架):
1. tick 级存活: `pane_diff_watcher_tick` 查到 busy agents 后 → `pane_diff watcher tick busy_agents=N`
2. 每个 UiOnly agent 每 tick 的 scan 结果: `pane_diff UiOnly scan agent_id="a3" provider="antigravity" scan=Matched|NoMatch consecutive_ticks=K`
3. recapture 判定转 IDLE 但没转成 (原 line 273 静默吞): `pane_diff UI completion recapture matched but mark_agent_idle_matched no-op (changes=0): already idle or swallowed`
- (已有的成功日志不变: `pane diff UI completion recapture marked agent IDLE`)
- 保留上一轮 a1 加的 2 个真实字节单测。

### PM 已 verify (cargo +stable)
- `target/debug/{ahd,ah}` 重编 OK (mtime 09:15:07), 3 条 tracing 串都进了二进制 (strings 确认)。
- `cargo test --lib pane_diff:: + antigravity_real_idle_capture_matches` = **24 passed / 0 failed**。

### DEPLOY (监督方执行)
`ah stop` → cp `target/debug/{ahd,ah}` → `~/.local/bin/` → **state dir 挪开** (防 AGENT_ALREADY_EXISTS) → `ah start` → 重新 seed fresh master。a3 STUCK 随 redeploy 自然清。

### Deploy 后 fresh master 怎么用日志拿决定性证据
重跑下面 ROUND 1 的 **Step 2** (杀 a3 pane→FIFO 管道 = `tmux -L <label> pipe-pane -t %3` 关掉, 它会让 reader EOF 退出; 然后 `ah ask a3` 派个短任务)。a3 完成后 (pane 显示 idle prompt), **盯 ahd 日志** (`journalctl --user -u ahd.service -f` 或 `--since`):

| grep 到什么 | 说明 |
|---|---|
| `pane_diff watcher tick busy_agents=` 周期出现 (每 ~30s) | watcher **真在 tick** (排除 loop 死/hung) |
| `pane_diff UiOnly scan agent_id="a3" ... scan=Matched consecutive_ticks=1` 然后 `=2` | watcher **抓到 a3 且 matcher 在线上也 Matched** → 应触发 recapture |
| `pane_diff UiOnly scan ... scan=NoMatch` | **线上 capture 出来的画面 matcher 不认** → 问题在 capture/画面 (跟单测不一致, 要对比 capture 字节) |
| `busy_agents` 日志里**根本没 a3** / scan 日志根本不出 | watcher **没把 a3 当 busy / 没 tick 到它** → 问题在 query/pane_id/loop |
| `recapture matched but ... no-op (changes=0)` | matcher 认了、判定转 IDLE 了, 但 `mark_agent_idle_matched` 没转成 → 问题在状态机守卫/defer |
| `pane diff UI completion recapture marked agent IDLE` + a3 BUSY→IDLE | **recapture 真 fired = Step 2 PASS** (硬门槛过, 补上证据) |

→ 一个 deploy 周期就能定位: PASS=兜底成立 (补硬门槛证据); 没过=上表一眼定位线上掉链子在哪一环, 再针对性修。

---

## ROUND 1 (历史背景)

记录: 2026-06-22, ah-managed Master PM。**前置**: #3 收口卡在 antigravity hook-push 不可达 (agy mid-gen + sub-ms ctx-cancel, 已 4 路 lock)。结论 = antigravity/gemini 走 marker fallback, 但实证 marker fallback **不 lag-safe** (单通道, 漏 final prompt → 停 BUSY → 300s STUCK 不是 IDLE)。已实施 **UI-only completion recapture 兜底**给它补 pull 通道。本文件给 deploy 后的 fresh master 跑真 re-dogfood。

## 这轮做了什么 (已完成, 全绿 + a4 APPROVE)
- 新 capability `ProviderManifest.completion_signal: {LogAndUi, UiOnly}`; antigravity/gemini=UiOnly, codex/claude=LogAndUi (`src/provider/manifest.rs`)。
- `pane_diff_watcher_tick` 复用 30s capture, 对 UiOnly+BUSY agent 在判 STUCK 前内联跑 provider MarkerMatcher (同 live reader 的 bottom-viewport scan), 连续 `AH_UI_COMPLETION_STABLE_TICKS` (默认 2) tick 稳定匹配 → `mark_agent_idle_matched` 转 IDLE (`src/pane_diff/mod.rs`)。
- 4 must-fix 全折进: #1 只 BUSY / #2 viewport 复用 matcher 防 scrollback 假匹配 / #3 anti-pattern (生成期 "esc to cancel" 抑制) / #4 计数器自洽。
- CAS 去重: reader 先转 IDLE 后 recapture 返回 0 不重复发事件。
- 测试: lib 全套绿 (含 6 个 recapture 测试) + mvp7_acceptance 6 + pr4c 21 全绿。a4 src audit = 有条件 APPROVE (2 个测试假覆盖 must-fix 已折修)。
- 二进制已 build: `target/debug/{ahd,ah}` (含本兜底)。

## deploy (监督方执行, 会杀当前 master)
`ah stop` → cp `target/debug/ahd` + `target/debug/ah` 到 `~/.local/bin/` → `ah start`。起新 4-agent session。

## RE-dogfood 怎么跑 (deploy 后 fresh master 按序)

state_dir = `ah.sock` 所在目录 (deploy 后是新的, `ls ~/.local/state/ah/*/ahd.sock`)。

### 0. 确认新二进制 + flag
- `ls -la ~/.local/bin/ahd` mtime = 刚 deploy 的。
- `ah ps` 4 agent IDLE。
- antigravity/gemini = UiOnly (这条兜底覆盖它们)。

### 1. 正常路径实证 (recapture 不抢 reader 的活)
- `ah ask --wait a3 "reply pong"` → a3 应**即时**经 primary FIFO reader 转 IDLE (sub_state=Matched), 不等 30s recapture。证明兜底不破坏正常即时完成。

### 2. **关键: 故意制造 primary reader 漏判, 让周期 recapture 真兜底接住** (监督方硬门槛)
目标: 让 a3 完成但 final idle prompt **不被 FIFO reader event 触发到**, 只能靠 30s recapture 兜。制造手段 (择一实测):
- (a) 派 a3 任务后, 在它快完成时, 让 reader 错过 final chunk — 难精确控制。
- (b) **更可控**: 直接观察一次自然 recapture 兜底 — 派 a3 任务, 完成瞬间 `ah logs a3` 看 state_change 的 reason: 若是 `MARKER_MATCHED` 来自 **pane_diff recapture 路径** (不是 reader) = recapture 真接住。或在 ahd 日志 grep recapture 转 IDLE 的 `tracing::info "pane diff UI completion recapture marked agent IDLE"`。
- (c) **构造法**: 临时把 `AH_UI_COMPLETION_STABLE_TICKS` 调低 + 用一个 reader 容易漏的场景 (e.g. agent 输出 final prompt 后无新 output)。
关键证据: ahd 日志出现 `pane diff UI completion recapture marked agent IDLE` (recapture 路径) + a3 BUSY→IDLE + 对应 job COMPLETED + master 收到完成 (job reply delivered)。

### 3. 防早转实证 (生成期不误判)
- 派 a3 一个多秒生成任务 (如 "count 1 to 30 slowly"), 生成期间 `ah logs a3` / `ah ps` 确认它**停在 BUSY 不被 recapture 误转 IDLE** (因为生成期 pane 底是 "esc to cancel" anti-pattern, 主控本轮已 live 实证)。完成后才转 IDLE。

### 4. gemini 顺带覆盖 (如 session 有 gemini agent)
- 本 session agent lineup 是 a1/a2 codex + a3 antigravity + a4 claude, **没有 gemini agent**。gemini=UiOnly 已单测覆盖 (manifest 断言)。如要 live 验 gemini 需额外起 gemini agent (可选, 非阻塞)。

### 5. 汇总物理证据给监督方
- `ah ps` sub_state (a3 经 recapture 转 IDLE)
- 完成事件 (job COMPLETED + reply delivered, ahd events)
- recapture 路径日志 (`pane diff UI completion recapture marked agent IDLE`)
- 时间线 (BUSY→recapture tick→IDLE)
- 监督方验完 = #3 闭合 → consolidated 呈用户拍 final squash。

## 硬门槛 (不变)
- 单测绿 ≠ 闭合。必须真 re-dogfood 见上。
- re-dogfood 过前: 不 merge / 不动 hook_push_providers / 不报 done。
- 边界: PM 不写 src; dogfood 用 `ah ask`; 不跑 `ah master cutover`/`ah up`。

## 已知 live 派发 gap (老二进制, 别钻)
ahd 有时把 agent 误标 PROMPT_PENDING (reason unknown_prompt) 但 pane 实际 idle, 新 prompt 不投递。workaround: tmux send-keys 直发 pane (SOP §7.5) + 直接 capture-pane 监控 (不信 ahd state)。本兜底**不修**这个 gap (是另一条 live dispatch 路径问题)。
