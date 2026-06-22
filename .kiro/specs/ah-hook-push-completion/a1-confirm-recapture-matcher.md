# 任务: 确认 UI-only completion recapture 的 matcher 是否能匹配「真实 capture-pane 的 antigravity idle 画面」

## 背景 (只读, 不改 src)
`src/pane_diff/mod.rs` 的周期 recapture 兜底, 对 UiOnly provider (antigravity) 用
`MarkerMatcher::from_manifest(&manifest).scan(&parser)` 判定 idle, 其中 parser 是
`vt100::Parser::new(200, 200, 0)` 处理 `tmux capture-pane -p -t <pane> -S -200` 的输出
(见 `src/tmux/session.rs:428` capture_pane_sync 用 `-p -S -200`)。

`MarkerMatcher::scan` (`src/marker/matcher.rs:60`) 取 `screen.contents()`, 再
`viewport_bottom_region` 只保留**最后 6 行** (`VIEWPORT_BOTTOM_LINES=6`, matcher.rs:80-87),
对 antigravity 用正则 `(?m)^\s*\? for shortcuts\b` 匹配 + 反 pattern `(?m)^\s*esc to cancel\b`。

现网实测怀疑: 真实 capture-pane 输出里 `? for shortcuts` 状态行不在画面底部, 后面跟着一堆
空行 (填充 pane 高度), 被挤出 bottom-6 viewport → scan 返回 NoMatch → recapture 永不触发。

## 真实数据 (已抓取, 就在仓库里)
`.kiro/specs/ah-hook-push-completion/REAL-a3-idle-capture.txt`
= 一个 antigravity (Gemini) agent 完成后 idle 时, ahd 用同样 `capture-pane -p -S -200` 抓到的
**原样字节** (60 行: `? for shortcuts` 在第 27 行, 后面 33 行空行)。

## 你要做的 (单一任务)
1. 在 `src/marker/matcher.rs` 的 `#[cfg(test)]` 里加一个**新测试** (只加测试, 不改任何非测试代码),
   读取上面那个 fixture 文件的原样字节 (用 `include_str!` 或 `std::fs::read` 相对仓库根),
   走真实路径: `let mut p = vt100::Parser::new(200,200,0); p.process(bytes);`
   `let m = MarkerMatcher::from_manifest(&get_manifest("antigravity"));`
   断言 `m.scan(&p)` 的**实际结果**。
2. `cargo test` 跑这一个测试, 看 `m.scan` 到底返回 Matched 还是 NoMatch。
3. 如果是 NoMatch (证实 bug): 在测试里顺便打印 `screen.contents()` 的总行数 +
   `viewport_bottom_region` 实际拿到的 6 行内容 (临时 eprintln 即可, 跑完可留可删),
   定位 `? for shortcuts` 是不是被挤出了 bottom-6。

## 回复 (中文, 简洁)
- `m.scan` 实际返回: Matched / NoMatch
- 如果 NoMatch: contents() 总行数 + bottom-6 viewport 实际内容 + 一句话根因
  (是否 = 状态行被尾部空行挤出 bottom-6)
- **只确认根因, 先不要改 matcher / 不要修复**。修复方案下一轮再派。
- 末尾列: 你读了哪些文件 + 跑了哪个 cargo test 命令。
