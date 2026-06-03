# Dogfood Pass 1 — antigravity 全生命周期实测 (2026-05-30)

真 ahd (target/release) + 真 agy (antigravity CLI) + 真 OAuth, 隔离 state (CCB_ENV=dev)。
配置: /tmp/agy-dogfood/ws/ah.toml — `[master] cmd=bash` + `[agents.ag1] provider=antigravity`。

## 生命周期实测结果

| 阶段 | 结果 | 物理证据 |
|---|---|---|
| spawn → IDLE | ✅ (修后) | `ah ps`: `ag1 antigravity IDLE Matched`; pane line12 = `? for shortcuts ... Gemini 3.5 Flash (High)` |
| ask 完成检测 | ✅ 检测对 / ⚠️ 内容脏 | `collect_reply complete reply_len=1066`; BUSY→IDLE 转换正确; **但 reply 含 TUI chrome (gap #2)** |
| cancel (ESC) | ✅ | `ah cancel` → pane 由 `esc to cancel` 回到 `? for shortcuts`; ag1 回 IDLE/Matched |
| kill 进程清理 | ✅ | `ah kill ag1` → **0 个 agy 孤儿进程**; ag1=KILLED |
| kill scope 清理 | ❌ | systemd scope `run-r65937....scope` 卡 **failed** 状态, `ah stop` 后仍残留 (gap #4) |
| tmux 清理 | ✅ | `ah stop` 后 tmux socket `ahd-*` 消失; 0 孤儿进程 |

## 缺陷清单

### Gap #1 [已修 by a1] — idle 检测 model 后缀精确匹配失败
- **证据 High / 影响 High / 已修**
- 真实 idle 状态行 `? for shortcuts` 同行尾带 model 名 (`Gemini 3.5 Flash (High)`)。
- `init_probe.rs:135` 用 `line.trim() == "? for shortcuts"` 精确相等 → 永 false。
- `matcher.rs:103` 正则 `(?m)^\s*\? for shortcuts\s*$` 的 `\s*$` 锚定行尾 → 同样 fail。
- 后果: agent 真到 IDLE (pane 可见), 但 ah 永远检测不到 → SPAWNING → 60s 超时 UNKNOWN。
- 修复: `trim_start().starts_with("? for shortcuts")` + 正则 `\b` 取代 `\s*$`; 测试 fixture 改成真实带 model 后缀行 (防假绿)。

### Gap #2 更新 (pass 1.5 真 ahd 复测) — chrome 行已修, 残留 prompt-echo + Thought 行
- **第一轮修复 (a1) 已生效**: box-drawing / `? for shortcuts` / `esc to cancel` 状态行已被 `is_reply_chrome_line` 过滤掉。
- **真实 ask 复测** (问 "what is a systemd transient scope" 两句话), 蒸馏 reply 残留 2 处噪音 (答案本身干净, 占主体):
  1. `> ` 残渣 —— prompt-echo 行 `> <prompt>` 被 distill 只删了 prompt 子串, 留下裸 `> `。
  2. `▸ Thought for 1s, 306 tokens` —— antigravity reasoning-summary 行, 现有 filter (只认 `Thinking...`/`Working(`) 没覆盖。
- **首次 ask 额外噪音 (一次性)**: 全屏未滚走时 reply 含 systemd `Running scope as unit:` 行 + antigravity ASCII banner (CLI 版本/账号/model/cwd)。滚屏后消失。screen-scrape 蒸馏天然受 scroll state 影响。
- **真实布局** (cat -A 实测): `> <prompt>` 行 → `  <answer>` (缩进) → box separator → `>` (空输入) → 状态行。答案永远在 prompt-echo 行和下一个 separator 之间, 可恢复。
- **Phase 1 修法 (targeted, 低风险)**: (a) prompt-echo 整行删 (不只删子串, 避免 `> ` 残渣); (b) filter `▸ Thought for`/`▸ Thinking` reasoning 行。
- **pass 1.6 复测 (distill v2 后)**: 稳态 reply (banner 滚走后) **完全干净** —— 真实问题答案无 `>` / `▸ Thought` / banner / 状态行。✅
- **残留已知 minor (一次性)**: **首次 ask** (banner 还占屏时) reply 仍含 banner overlay —— prompt-echo 行被 banner 的 model badge `(Google AI Ultra)` 右对齐覆盖在同一视觉行 (`> Reply...PONG and (Google AI Ultra)`), 整行删因带 badge 后缀匹配不上。答案仍可恢复。banner 滚走 (任何后续 ask) 即干净。每个 agent 生命周期一次性。
- **principled 修法 (留给 unified-capture 设计, a2 域)**: 答案区域提取 (取最后一个 prompt-echo 行到下一个 separator 之间) —— 一次性解决 banner/scope-noise/prompt-echo/Thought 全部噪音, 不受 scroll state 影响。这是 completion-capture 的一部分, 应折进学习式抽取规则, 不再硬编码堆 filter。**不继续硬编码 filter 追 first-ask banner (whack-a-mole, 脆弱)**。
- **已验证 5/5 机械生命周期 PASS**: spawn→IDLE / cancel(ESC) / kill→0 孤儿 / **无 failed scope (--collect 生效)** / stop 干净。

### Gap #4 更新 — 已修 (--collect)
- a1 在 agent + master 的 `systemd-run --scope` 后加 `--collect` (CollectMode=inactive-or-failed)。
- 真 ahd 复测: kill 后 failed scope 被 systemd 自动 GC, `systemctl --user list-units` 不再残留。✅

### Gap #2 [原始记录] — antigravity reply 蒸馏返回 TUI chrome
- **证据 High / 影响 Medium / 置信度 B**
- `distill_reply` (src/db/jobs.rs:517) 只 strip ANSI + "Thinking.../Working(" 行 + prompt 回显。
- 不 strip antigravity 的 TUI chrome: box-drawing (`────`), 纯 `>` 提示行, `? for shortcuts ... model` / `esc to cancel ... model` 状态行。
- 后果: 问 "reply PONG" 返回 1066 字符 (screen_text 1118), 真答案 `PONG` 埋在框线+状态行里。master/调用方拿到脏 reply。
- 修法方向: distill_reply 对 antigravity 增加 chrome 行过滤 (box-drawing 行 / 纯提示符行 / 状态行)。注意别过度 strip 真内容。需对照 claude/gemini 现有蒸馏行为校准。

### Gap #3 [待修] — busy anti_pattern 同 model 后缀 bug + 假绿测试
- **证据 High / 影响 Medium / 置信度 A**
- `manifest.rs:236` busy anti_pattern = `r"(?m)^\s*esc to cancel\s*$"`, `\s*$` 锚定行尾。
- 真实 busy 行 `esc to cancel ... Gemini 3.5 Flash (High)` 带 model 后缀 → anti_pattern 永不匹配。
- 跟 gap #1 同根 (a1 修了 idle 侧, 漏了对称的 busy 侧)。
- 假绿: `test_marker_matcher_antigravity_suppresses_idle_when_cancel_status_present` fixture 用干净 `esc to cancel\n` (无 model 后缀) → 测试过但真实行 fail。
- 后果 (潜在): 生成中若 scrollback 残留 `? for shortcuts`, anti_pattern 失效 → 可能误判完成。本次单测试没触发 (in-place redraw 覆盖了旧状态行), 但是真实 fragility。
- 修法: anti_pattern 去掉 `$` 锚定 (如 `(?m)^\s*esc to cancel\b`); 测试 fixture 改真实带 model 后缀行。

### Gap #4 [待修] — ah kill 泄漏 failed systemd scope (通用缺陷, 非 antigravity 专属)
- **证据 High / 影响 Medium / 置信度 B**
- `handle_agent_kill` (src/rpc/handlers.rs:1150) 只 `libc::kill(pid, SIGKILL)` 杀 agent 进程, 不 stop / reset-failed 包裹它的 systemd scope。
- 进程被 SIGKILL → scope 退出非 0 → 进 `failed` 状态滞留。代码里无任何 `reset-failed` 调用。
- `ah stop` 也不清理 → failed scope 在 user systemd session 累积。
- 这正是 ah 立项要避开的上游 ccb scope-leak bug (docs/upstream-ccb-bugs/tmux-scope-and-tmpdir-leak-bugs.md)。
- 影响所有 provider (codex/claude/gemini agent kill 同样泄漏), 不只 antigravity。
- 用户验收明确要求 "0 memory leak" → 失败 scope unit 累积属泄漏, 在 scope 内。
- 修法方向: handle_agent_kill 杀进程后 (或改为) `systemctl --user stop <scope>` 干净移除 unit; 或 SIGKILL 后 `systemctl --user reset-failed <scope>`。需 ah 知道 agent 的 scope 名 (spawn 时 `--description=ccbd-agent-<id>@<server>`, 但 systemd 实际 unit 名是 `run-r<hash>.scope`, 需映射)。

## Gap #5 [待修, 预存在 — 非 antigravity 引入] — claude CLI v2.1.158 到 idle 但 ClaudeInitProbe 检测不到
- **证据 High / 影响 High / 置信度 B**
- 全量测试发现 `tests/mvp11_real_claude.rs::test_claude_spawn_ask_flow` FAILED: "timed out waiting for agent ... state IDLE" (90s)。
- **git stash 干净树复测坐实预存在**: 把本会话所有 antigravity 改动 stash 掉, last-commit 代码跑同一测试**同样 FAILED** (90.36s, 同行 161) → **100% 非 antigravity 回归**, 是预存在 bug。
- **真 pane 实证**: fresh claude agent **真到了 idle 提示符** (`❯ Try "fix lint errors"` + "Opus 4.8 (1M context)" + bypass permissions 行, 无 onboarding 向导), 但 ah `ClaudeInitProbe` 没标 IDLE → 60s readiness 超时 → agent 卡 UNKNOWN → 测试 90s 超时。
- `ClaudeInitProbe.detect` (src/provider/init_probe.rs) 要 3 条全真: `banner_gone` (无 CLAUDE_INIT_BANNERS) + `prompt_present` (`❯`) + `steady_marker_present` ("Opus"/"Sonnet"/"Haiku")。40s 实测 pane 三条都满足却没检测到 → 检测路径有问题 (疑: readiness 窗口内 0-60s 的瞬时屏幕态 vs 稳态不一致 / 或 scan_startup_prompt 对 claude 新版 box 渲染误判)。
- ClaudeInitProbe 最后改动 commit 47b2779 (PR1), 早于本次 antigravity 工作。claude CLI 是 v2.1.158 (近期更新)。
- **这正是用户核心担忧的活样本**: "单凡程序有点更新有点调整就抓瞎" —— claude CLI 更新后 ah 硬编码的 ClaudeInitProbe 失效 → 检测不到 idle。**是 unified auto-capture 机制最强的立项动机**。
- **不在 antigravity Phase 1 scope** (预存在, 独立 provider)。优先级高, 留作下一步 (可能直接用来验证 unified-capture 机制)。

## 已验证 OK (无需动)
- 配置隔离: HOME=/home/sevenx/.cache/ah/sandboxes/<hash>, CCB_ENV=dev → target/dev_state 独立 state。
- OAuth: antigravity-oauth-token copy (非 symlink), trust + onboarding.json 种子生效, fresh sandbox 直 boot 到 IDLE 无向导。
- master cascade: `[master] cmd=bash` 在场, agent 不被误杀。
- 进程清理: kill / stop 后 0 孤儿 agy 进程; tmux server 干净。
