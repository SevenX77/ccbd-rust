# step-9 RE-dogfood handoff (修复一轮后, 给重启后的 fresh master)

记录: 2026-06-22, ah-managed Master PM (step-9 fix round 后)。
**前置**: 首轮 dogfood 未闭合 (#3 hook-push 三厂商零 RPC 到达 ahd)。已修一轮 + a4 audit APPROVE + debug build。本文件给监督方重启后的 fresh master 跑 RE-dogfood。

## 首轮为何没闭合 (一句话)
#3 卖点 = Stop hook → `ah agent notify` RPC → 运行 ahd push transition。首轮三厂商完成全走 fallback (codex/claude log-signal, antigravity UI-pull), **hook-push RPC 全程零到达 ahd** (监督方 journald 实证)。详 `dogfood-evidence.md` 证据4 + root-cause `step9-fix-research.md`。

## 这轮修了什么 (7 项, a1 实施 + a4 audit APPROVE; commit 见 git log)
1. `ah agent notify --hook-json` → 成功输出 `{}` (默认仍三行 human 输出不变); 新增 `--hook-debug-log <path>` 落 argv/stdout/stderr/exit。(`src/bin/ah.rs`)
2. `build_ah_hook_command` 所有 provider command 加 `--hook-json` + `--hook-debug-log <state_dir>/hooks-debug/<agent>.log`。(`home_layout.rs:540`)
3. **codex stdout 契约**: codex v0.135.0 要求 Stop hook exit0 stdout 空/合法 JSON; 之前纯文本被判 `invalid stop hook JSON output`。现 `--hook-json` 输出 `{}` 解决。
4. **codex feature flag**: `enable_codex_hooks` 改写 `[features].hooks=true` (删弃用 `codex_hooks`)。(`home_layout.rs:864`)
5. **codex trust 弹窗**: spawn 加 `--dangerously-bypass-hook-trust` (首轮 a2 dead-on-arrival 卡死)。(`manifest.rs`)
6. **codex idle 误判**: anti-pattern 加 `Hooks need review|Trust all and continue|Continue without trusting`, 弹窗不再被判 IDLE。(`manifest.rs`)
7. **antigravity timeout 单位**: gemini/antigravity hook `timeout` 单位是**毫秒** (确认: gemini docs default 60000 + bundle timeoutMillis); 之前写 `5`=5ms 在 `ah agent notify` 连 socket 前杀掉 = antigravity 零 RPC 根因。现 provider-aware: gemini/antigravity=5000ms, codex/claude=5s。(`home_layout.rs:540`)

测试: ah bin 12 + lib 685 + pr4c 21 全绿 (debug 单核串行)。

## claude 零 RPC 根因仍未定 (这轮没盲修, 留 RE-dogfood debug 定位)
- claude **live** sandbox `.claude/settings.json` 确含注入的 Stop hook (主控实证), 不是"没配置"。
- `ah agent notify` 先发 RPC 再 println, 所以 claude 零 RPC **不是** stdout 契约 (`--hook-json` 修不了 claude)。
- 候选: PATH (裸 `ah` 在 claude hook 环境 PATH 失配 → command not found 静默) 或 hook 没 fire。
- **RE-dogfood 时**: 派 a4 真任务后, `cat <state_dir>/hooks-debug/a4.log` 看有没有 argv/exit — 有 argv 但 RPC 失败 = PATH/连接问题; 一个字没有 = hook 没 fire。再决定修法。可加 `claude --debug-file` 复测 (manifest claude spawn)。

## RE-dogfood 怎么跑 (fresh master 按序)
0. `ah ps` 确认新 session 4 agent IDLE/Matched + 新 debug 二进制时间戳是刚 build 的 (`ls -la ~/.local/bin/ahd`)。确认 `ah.toml [completion]` flag 仍 ON。
1. 对 a1(codex)/a4(claude)/a3(antigravity) 各 `ah ask <agent> "reply pong"`。
2. **关键证据 (这次要的是真 hook-push, 不只是不 hang)**:
   - `ah logs <agent>` 看 `state_change ... "source":"hook" ... ` 是不是真来自 hook-push RPC (而非 log-signal) — 交叉看下一条。
   - **监督方 journald** (`journalctl --user -u ahd.service`, 沙箱内主控看不到, 须监督方 grep): 这次应**有** `ah agent notify` RPC 到达记录 (首轮是零)。这是 #3 闭合的最强 proof。
   - `cat <state_dir>/hooks-debug/<agent>.log`: 三厂商各应有 argv + exit=0 一手证据 (主控自己能读, 不依赖 journald)。state_dir = `ah.sock` 所在目录, 现为 `~/.local/state/ah/29acbe42/`。
3. **闭合判定**: 三厂商 hooks-debug log 都有 `exit=0` 的 notify argv + (监督方 journald) RPC 到达 + agent push 转 IDLE。三厂商全过 = #3 step-9 真闭合。
4. 若 claude/antigravity 仍有问题: 读 hooks-debug log 定位 (argv 有无 / exit 几), 自驱回修 (不抛监督方拍工程细节, 仅"目标实现不了"才升级)。
5. 闭合后: 走 SOP-08 step5-8 (docs 同步 + a1 主笔 PR report + a4 honesty audit), 报监督方拍最终 squash。**不 merge** (merge 权监督方)。
6. follow-up (非阻塞, 已记 task): GA 前 gate `--hook-debug-log` (现无条件开 + 无 rotation) + 写失败加 eprintln + error 分支 exit 码用真值。

## 边界 (铁律)
- PM 不写 src/tests; dogfood 用 `ah ask` (这是 dogfood 本身)。
- **不要 `ah master cutover` / `ah up`** (Bug A reap 杀自己)。
- 重启 (stop→cp target/debug 到 ~/.local/bin→start) 是**监督方**执行 (会杀当前 master)。
- commit footer `Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>`; `git add` 按文件名不 `-A`。
