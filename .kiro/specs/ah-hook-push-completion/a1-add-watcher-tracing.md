# 任务: 给 pane_diff watcher 加永久 per-tick 可观测性日志 (logging SOP) + debug 重编

## 背景
UI-only completion recapture 兜底 (src/pane_diff/mod.rs) 在线上 dogfood 里没触发, 但纯逻辑单测是绿的
(`antigravity_real_idle_capture_matches` + `ui_only_marker_recapture_completes_real_antigravity_idle_capture_after_two_ticks`)。
现在线上 watcher 「有没有 tick 到 a3 / scan 出啥」是黑盒 (整 session 零 watcher 日志)。
按 logging SOP: recapture 把 agent 转 IDLE 是**降级恢复 side-effect**, 控制流分支**必须有可观测日志** —
这是**永久日志**, 不是一次性 debug 脚手架, 要写得规范、留在代码里。

## 你要做的 (改非测试代码 = 加日志; 不改任何判定逻辑/阈值/行为)

### 1. tick 级存活日志 (证明 watcher 真在 tick + 看到哪些 busy agent)
在 `pane_diff_watcher_tick` (src/pane_diff/mod.rs:236 起) 里, 查到 busy agents 后加一行:
`tracing::info!(busy_agents = busy_agents.len(), "pane_diff watcher tick")`
(放在 query_agents_by_state 拿到结果之后、capture 循环之前。)

### 2. UiOnly agent 的 scan 结果 + consecutive_ticks (证明 scan 在线上返回 Match/NoMatch + tick 累加)
在 `process_pane_diff_observations_with_ui_completion_stable_ticks` (src/pane_diff/mod.rs:89) 的
UiOnly 分支里 (matcher.scan 之后, line ~129), 对每个 UiOnly agent 每 tick 加一行:
`tracing::info!(agent_id = %observation.agent_id, provider = provider, scan = ?match_result, consecutive_ticks = <当前累加值>, "pane_diff UiOnly scan")`
- 注意要打出 Matched / NoMatch 两种情况 (matched 分支打 consecutive_ticks 当前值; not-matched 分支打 NoMatch + consecutive_ticks=0)。
- 这是 pure 函数, 加 tracing::info 不影响单测 (无 subscriber 时是 no-op)。

### 3. recapture 转 IDLE 的 changes==0 静默分支补日志 (logging SOP: 降级必须可观测)
src/pane_diff/mod.rs:273 现在 `Ok(_) => {}` 是**静默吞掉** (recapture 判定该转 IDLE 但 mark_agent_idle_matched 返回 0 = 没转成)。
改成:
`Ok((changes, _)) if changes == 0 => tracing::info!(agent_id = %agent_id, "pane_diff UI completion recapture matched but mark_agent_idle_matched no-op (changes=0): already idle or swallowed")`
(保留原 changes>0 分支 line 266-272 不变。)

## 不要动
- 不改任何判定逻辑 / 阈值 / consecutive_ticks 算法 / matcher。只加日志。
- 保留你刚加的 2 个真实字节单测 (matcher.rs + pane_diff.rs), 别删。
- 不 commit。

## 编译 + 自检
1. `cargo +stable build` 出 `target/debug/{ahd,ah}` (debug profile)。报编译成功。
2. `cargo +stable test --lib -- antigravity_real_idle_capture_matches ui_only_marker_recapture_completes_real_antigravity_idle_capture_after_two_ticks` 确认仍 **2 passed**。
3. `cargo +stable test --lib pane_diff:: ` 确认 pane_diff 全套绿 (没被日志改动弄坏)。

## 回复 (中文简洁)
- 加日志的 3 处 file:line。
- 三个 tracing 行的实际字符串 (我要写进 handoff 让 deploy 后 fresh master grep)。
- build 成功? target/debug/{ahd,ah} mtime?
- 单测: 2 个真实字节测试 + pane_diff:: 全套 结果 (X passed / Y failed)。
- 末尾列读了哪些文件 + 跑了哪些 cargo 命令。
