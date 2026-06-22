# 任务: 修 UI-only completion recapture 兜底输给"快速 STUCK"的运行期 race (含 STUCK→IDLE 守卫缺口)

你是 a1 (codex), 主力编程。这是一个**已被 tracing 实证定位**的运行期 (operational) 缺陷修复。请按 tests-first 走: 先写复现这个 gap 的失败测试, 再改 src 让它变绿, 然后 `cargo +stable build` (debug) 确认重编 OK。**不要 deploy (不 cp 到 ~/.local/bin)、不 commit、不 merge、不动 hook_push_providers**。改完报告我 (PM)。

## 背景 (完整自包含)

ah 给 UI-only completion provider (antigravity / gemini, `ProviderManifest.completion_signal == UiOnly`) 设计了一个**周期 recapture 兜底**: 当 live FIFO reader 漏判了 agent 的完成 (final idle prompt 没被 reader event 接住), `pane_diff_watcher_tick` 每 30s 复用 capture-pane, 对 BUSY 的 UiOnly agent 跑 provider MarkerMatcher, 连续 `AH_UI_COMPLETION_STABLE_TICKS` (默认 2) tick 稳定 Matched 后, 调 `mark_agent_idle_matched` 把它转 IDLE。

**这个兜底的设计假设**: agent 完成后会一直停在 BUSY, 直到很久 (原以为 ~300s) 后才 STUCK, 给 recapture 留足 ≥10 个 tick。

## 实证定位 (我刚跑的 dogfood Step 2, 监督方硬门槛)

制造手段: `tmux -L <label> pipe-pane -t %3` 关掉 a3 (antigravity) 的 pipe → 喂 reader 的 `cat > fifo` 进程死 → live reader EOF 失效 → a3 完成但 reader 看不到。然后 `ah ask a3 "reply bravo"`。

a3 pane 实际完成 (显示 `> bravo` + 干净 idle prompt `>` + `? for shortcuts`), 但**整 session recapture 零触发, a3 停 STUCK**。

journald tracing (这轮新加的 per-tick 日志) 的决定性证据:
```
09:28:37 pane_diff watcher tick busy_agents=1
09:28:37 pane_diff UiOnly scan agent_id=a3 provider="antigravity" scan=Matched consecutive_ticks=1
09:29:07 pane_diff watcher tick busy_agents=0      ← a3 已不在 BUSY (被标 STUCK 了)
(recapture 转 IDLE 日志从未出现)
```
`ah logs a3` 的 state 轨迹:
```
IDLE → WAITING_FOR_ACK (dispatched)
WAITING_FOR_ACK → BUSY (ACK_VISUAL_DIFF)
BUSY → STUCK {reason:"PANE_DIFF_STUCK", signal_kinds:["state_machine"], elapsed_secs:0}   ← 约 50s 内就 STUCK 了
```

## 根因 (已证, 两层缺口)

1. **race**: recapture 要连续 2 个 tick (≥60s @ 30s interval) 才 fire (`src/pane_diff/mod.rs:155` `consecutive_ticks >= ui_completion_stable_ticks`)。但 a3 的 reader-miss 完成在 **<60s** 内就被某个"快速 STUCK 路径"标 STUCK (elapsed_secs:0, 即接近即时)。tick1 拿到 ticks=1, 还没等到 tick2, agent 已离开 BUSY。
2. **观测丢失**: `pane_diff_watcher_tick` 只 query `state="BUSY"` 的 agent (`src/pane_diff/mod.rs:256-257`)。a3 一旦 STUCK 就不在 busy_agents 里 → 不构造 observation → `process_pane_diff_observations` 末尾 `state_map.retain(|id,_| active_agent_ids.contains(id))` (`src/pane_diff/mod.rs:228`) 把 a3 的 `ui_marker_match` 计数器**直接删掉** → recapture 永远到不了 ticks=2。
3. **状态守卫缺口** (即使修了 1+2 也会撞): `mark_agent_idle_matched` 的 SQL guard (`src/db/state_machine.rs:388`) 是 `WHERE ... AND state IN ('SPAWNING','WAITING_FOR_ACK','BUSY')` — **STUCK 不在内**。且更上层 `is_active_state` (`src/db/state_machine.rs:61` = SPAWNING|WAITING_FOR_ACK|BUSY) 在 `mark_agent_idle_matched_outcome_sync` (`:317`) 会先 swallow 掉非 active 的 agent。所以哪怕 recapture 在 STUCK 状态下 fire, 转 IDLE 也会 no-op (changes=0)。

**结论**: 兜底的设计假设 (agent 停 BUSY 很久) 被一个 elapsed_secs:0 的快速 STUCK 路径打破; recapture 既追不上 (2 tick 太慢), STUCK 后又看不到也转不动它。所以 reader-missed 的 UiOnly 完成永远兜不住。

## 你要做的

### 第 0 步: 先把那个"快速 STUCK"路径查清楚 (它是 silent 的, 没 info/warn 日志, 也是个缺口)
- `mark_agent_stuck` 的 reason 永远是 `PANE_DIFF_STUCK` (`src/db/state_machine.rs:879`), 它的默认 event 永远带 `signal_kinds:["state_machine"]` (`:881/:1105`), 所以 event 的 signal_kinds 区分不出 caller。
- `mark_agent_stuck` 的 3 个 caller: `pane_diff/mod.rs:300` (会另发 warn 日志 line 320, 本次**没出现**), `provider/health_check.rs:73` (会另发 `["health:completion"]` event), `marker/timer.rs:113` (BUSY_TIMEOUT=10_800s=3h, **不可能 60s 内 fire**, 且会发 info line 117, 本次也没出现)。
- 这三个都对不上 (silent + elapsed 0 + <60s)。请你用 grep / 读代码找出**真正在 <60s 内把 BUSY 的 UiOnly agent 标 STUCK 的那条路径** (可能是 orchestrator 的某个 reconcile / dispatcher 路径, 或 ack/event 摄入路径 `src/rpc/handlers/events.rs:255-274`, 或 `transit_agent_state` BUSY→STUCK 直转)。把它定位 + 顺手补上 observability 日志 (logging SOP: STUCK 转换不该 silent)。
- 这条路径的**确切身份直接决定最优修法** (见下面), 所以先查清。

### 第 1 步: 选最小正确的修法 (你做工程判断 + 给我理由, 不要列 ABC 让我选)
候选方向 (你 root-cause 完那条快速 STUCK 路径后, 评估哪个最小且最正确, 也可提更好的):
- (A) **让 recapture 能兜 STUCK 的 UiOnly agent**: 把 watcher 的 query 从 BUSY-only 扩到 BUSY+STUCK (仅对 UiOnly provider 有意义), 让 STUCK 后计数器不丢; 并加一条 recapture 专用的 STUCK→IDLE 转换 (或扩 `mark_agent_idle_matched` 的 allowed states 含 STUCK, 但要小心**只**走 recapture 这条路, 别放开 live reader 的守卫)。
- (B) **从源头不让 UiOnly agent 过早 STUCK**: 如果第 0 步发现那条快速 STUCK 路径对"无 log 信号的 UiOnly agent"判定 STUCK 本身就过激 (UiOnly 本来就没 log 完成信号, 用 log/completion 维度判 STUCK 是误判), 在那条路径里对 UiOnly 延后/豁免, 给 recapture 留窗口。
- (C) 组合 (A)+(B) 的最小子集。

**硬约束**:
- 必须 reconcile 跟历史 must-fix #1 "只 BUSY" 的关系: 当初 "只 BUSY" 是为了**不让 recapture 去碰 IDLE / 非 active agent**。STUCK 是非 active 但**非 IDLE** 的失败态, 一个 pane 显示干净 Matched idle prompt 的 STUCK UiOnly agent 正是兜底该救的对象。你的修法要保证 (a) 不会误转 IDLE 的 agent, (b) anti-pattern 守卫 (生成期 "esc to cancel" 抑制, must-fix #3, `mod.rs:119-129` 复用 MarkerMatcher) 在扫 STUCK agent 时**仍然生效** (防 scrollback / 生成期假匹配)。
- 不要削弱正常路径 (live reader 即时完成不能退化)。
- 不放开 STUCK→IDLE 给任意 caller — 只给 UiOnly recapture 这条受 matcher + anti-pattern 守卫的路径。

### 第 2 步: tests-first
- 先加**失败**的回归测试复现这个 gap。最直接的是 `process_pane_diff_observations` 这层的纯逻辑单测: 构造一个 UiOnly agent 的 observation 序列, 模拟它在拿到 ticks=1 后离开 BUSY scan-set (即下一 tick 不在 observations 里), 断言计数器丢失 / recapture 不 fire = 当前 bug; 修完后断言它能兜住 (按你选的修法)。如果修法涉及 STUCK→IDLE 转换, 在 `src/db/state_machine.rs` 加状态机单测 (仿 `:1524 test_mark_agent_stuck_from_busy_succeeds` 风格) 断言 recapture 路径能 STUCK→IDLE 且 live-reader 守卫不被放开。
- 保留这轮已加的 2 个真实字节单测 (`antigravity_real_idle_capture_matches` + `ui_only_marker_recapture_completes_real_antigravity_idle_capture_after_two_ticks`) 和 3 处 tracing::info (它们是永久可观测性, 不是脚手架, 别删)。

### 第 3 步: 验证 + 报告
- `cargo +stable test --lib pane_diff:: state_machine:: marker::` 相关全绿 + 你新加的回归测试绿。
- `cargo +stable build` (debug) 确认 `target/debug/{ahd,ah}` 重编 OK。
- 报告给我: (1) 第 0 步查到的真正快速 STUCK 路径是哪条 (file:line + 触发条件); (2) 你选的修法 + 理由 + 怎么 reconcile must-fix #1/#3; (3) 改了哪些文件; (4) 测试结果; (5) 明确 "ready for deploy" (但你自己不 deploy)。

## 边界 (再强调)
- 不 cp 到 ~/.local/bin (deploy 是监督方做)。不 commit。不 merge。不动 `hook_push_providers`。不跑 `ah master cutover` / `ah up`。
- 只改必要的 src + tests。改完回复结果即可。
