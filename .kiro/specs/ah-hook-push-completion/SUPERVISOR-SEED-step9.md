# 监督方 → ah-managed Master PM:step-9 接力种子

你是 **ah 托管的 Master PM**(刚被 `ah start` 重启,全新 blank session,没有上轮上下文)。
监督方(旧 ccb master,用户替身)正在 tmux 外部驱动你的生命周期。下面是你重建上下文 + 当前任务。

## 1. 先读这两份重建完整上下文(必读,按顺序)
1. `research/ah-master-death-cutover-incident/RESTART-BRIEF.md` — 整个事故 + #3 工作全貌
2. `.kiro/specs/ah-hook-push-completion/step9-dogfood-plan.md` — step-9 执行计划(你要跑的就是它)

## 2. 当前已就绪状态(监督方已替你做完 step-9 的 step 1-3,已物理验证)
- **新 #3 二进制**:`~/.local/bin/{ah,ahd}` 已是从 HEAD(61f390f)debug build 的新版(含 agent.notify push 模型)。运行中 ahd 就是它。
- **flag 已 ON**(`ah.toml` `[completion]`,**未 commit**,dogfood 证明后再 commit):
  ```toml
  hook_push_enabled = true
  hook_push_events = ["stop"]
  hook_push_providers = ["claude", "codex", "antigravity"]
  ```
- **4 worker 全部 IDLE/Matched,且 Stop hook 已注入各自物化 home(监督方已 grep 实证)**:
  - a1 codex → `.codex/hooks.json`:`ah agent notify --agent-id a1 --event stop --provider codex`
  - a2 codex → `.codex/hooks.json`:同上 a2
  - a3 antigravity → `.gemini/config/hooks.json`:`--provider antigravity`
  - a4 claude → `.claude/settings.json`:`--provider claude`
  - 四个都指向 live socket `~/.local/state/ah/29acbe42/ahd.sock`
- **当前 session**:`ah ps` 看 sess_07170cba,4 agent。
- **新发现(记进 findings,评估是否要修)**:graceful `ah stop` → `ah start` 仍撞 `AGENT_ALREADY_EXISTS`。Bug B/C 的 KILLED-slot 回收**没覆盖优雅 stop 路径**(优雅 stop 留下非 KILLED 状态的 agent 行仍冲突)。监督方两次都用 mv state dir 兜底才起来。这是 B/C 修复的 gap。

## 3. 你的任务:跑 step-9 的 step 4-5(3 厂商真 dogfood),不要再重启/rebuild(已做完)
按 `step9-dogfood-plan.md`:
- **step 4**:对 a1(codex)/a4(claude)/a3(antigravity)各派一个真任务(`ah ask <agent> "<小任务>"`),
  **关键证据 = `ah pend <job>` 不再 hang**(对比旧 ahd completion-lag 会 hang 死等;见 dogfood-evidence.md 证据1)。
  每个厂商:agent 跑完 → Stop hook fire → `ah agent notify` → ahd 立即标 IDLE → `ah pend` 立即返回。
- **step 5**:fallback 验证(push 主、pull 兜底,无退化)。
- 三厂商都 PASS → 把证据落到 `dogfood-evidence.md`,然后走 SOP-08 step 5-8(docs 同步 + PR report)。

## 4. 边界(铁律)
- 你是 PM,**不写 src/tests 代码**;dogfood 派活儿用 `ah ask`(这是 dogfood 本身,不是 ccb)。
- **不要 `ah master cutover` / `ah up`**(Bug A:reap-after-fail 会杀你自己)。
- **不要 merge 到 main**:merge 权在监督方。你跑完 dogfood + 出 PR report 就停,报监督方,等最终拍板。
- commit footer:`Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>`;`git add` 按文件名,绝不 `-A`/`.`。
- ah.toml 的 flag-ON 改动:dogfood 证明后,作为 #3 的产物 commit 进本分支。

## 5. 完成判定
3 厂商 dogfood 全 PASS(`ah pend` 不 hang 实证)+ 证据落盘 + PR report 写好 → 报监督方。监督方查 CI 绿 + 报用户拍最终 squash。
