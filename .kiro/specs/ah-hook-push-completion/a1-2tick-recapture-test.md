# 任务: 用真实字节跑 2-tick `process_pane_diff_observations`, 看 recapture 逻辑是否真触发

## 背景 (只读非测试代码)
周期 recapture 兜底的**核心逻辑**是 `src/pane_diff/mod.rs` 的
`process_pane_diff_observations(state_map, observations, now, stuck_threshold)`。
它对 UiOnly provider (antigravity) 需要**连续 2 个 tick** 同一 `content_hash` 匹配,
才把 agent 放进 `ui_completed_agent_ids` (默认 `DEFAULT_UI_COMPLETION_STABLE_TICKS=2`)。

已证实: 单次 `MarkerMatcher::from_manifest(antigravity).scan()` 对真实抓取的 idle 画面
返回 `Matched` (见你刚加的 `antigravity_real_idle_capture_matches` 测试, 已绿)。

但现网 live dogfood 里: 把 a3 (antigravity) 的 reader 弄漏判后, a3 完成却停 BUSY→STUCK,
**recapture 从没触发** (零日志)。需要判断是「逻辑 bug」还是「运行期 watcher 没 tick 到」。

## 真实数据 (仓库内)
`.kiro/specs/ah-hook-push-completion/REAL-a3-idle-capture.txt`
= ahd `capture-pane -p -S -200` 抓到的 antigravity idle 原样字节。

## 你要做的 (单一任务, 只加测试)
在 `src/pane_diff/mod.rs` 的 `#[cfg(test)]` 加一个新测试, 模拟 watcher 连续 2 个 tick:
1. `let bytes = include_str!(".../REAL-a3-idle-capture.txt").to_string();`
2. 构造 `PaneDiffObservation { agent_id:"a3", text: bytes.clone(), log_mtime:None, provider:Some("antigravity") }`
3. `let mut state_map = HashMap::new();`
4. 第 1 次: `let r1 = process_pane_diff_observations(&mut state_map, vec![obs.clone()], Instant::now(), Duration::from_secs(300));`
   断言 `r1.ui_completed_agent_ids` **为空** (第 1 tick 还没到 stable 阈值)。
5. 第 2 次 (同样 bytes, 模拟 30s 后又抓到同一 idle 画面):
   `let r2 = process_pane_diff_observations(&mut state_map, vec![obs.clone()], Instant::now(), Duration::from_secs(300));`
   断言 `r2.ui_completed_agent_ids` **== ["a3"]** (第 2 tick 应触发 recapture)。
6. `cargo +stable test` 跑这一个测试, 报实际结果。

## 回复 (中文, 简洁)
- 第 2 tick `ui_completed_agent_ids` 实际值 = ?  (是 `["a3"]` 还是空?)
- 测试 PASS 还是 FAIL?
- 如果 FAIL (逻辑没触发): 用临时 eprintln 打出每个 tick 后 `state_map["a3"].ui_marker_match`
  (consecutive_ticks / content_hash) 帮定位是 matcher 没进 / 还是 ticks 没累加。
- **只加测试 + 报结果, 先不要改 src 逻辑 / 不要修复。**
- 末尾列: 读了哪些文件 + 跑了哪个 cargo 命令。
