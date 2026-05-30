# antigravity (agy) 真实行为观察记录 — Phase 1 manifest 种子数据

> 主控 dogfooding 实测 2026-05-30, 独立 tmux socket 观察, 物理实证 (非猜测)。
> 这是写 Phase 1 manifest 种子的依据 + 验证 round-2 设计假设。

## 基本信息
- **binary**: `~/.local/bin/agy` (182MB Go binary, v1.0.3)
- **安装**: `curl -fsSL https://antigravity.google/cli/install.sh | bash` (装到 ~/.local/bin, 加 PATH 到 .bashrc/.profile)
- **配置目录**: `~/.gemini/antigravity-cli/` (settings.json / oauth-token / keybindings.json / brain/ / knowledge/)
- **OAuth**: `~/.gemini/antigravity-cli/antigravity-oauth-token` (已登录: xingqiqi77@gmail.com, Google AI Ultra)
- **settings.json**: `{colorScheme: "tokyo night", model: "Gemini 3.5 Flash (High)", trustedWorkspaces: ["/home/sevenx"]}`

## CLI 接口 (几乎照搬 Claude Code CLI)
```
--print / -p / --prompt        非交互单 prompt 打印 (实测干净输出, 适合 ah --print 类)
--prompt-interactive / -i      初始 prompt 后继续交互
--continue / -c                继续最近对话
--conversation                 按 ID 恢复
--add-dir                      加 workspace 目录 (repeatable)
--dangerously-skip-permissions 自动批准工具权限 (= codex YOLO)
--sandbox                      沙箱终端限制
子命令: changelog/install/plugin/update
```

## 关键状态标记 (完成检测的真值来源 — 比 gemini 简单太多!)

**底部状态行是干净的 busy/idle 判别信号**:

| 状态 | 底部状态行 | 屏幕特征 |
|---|---|---|
| **IDLE / DONE** | `? for shortcuts` | 横线间一行 `>` 提示符 |
| **BUSY / thinking** | `esc to cancel` | spinner `⣯ Generating...` / `▸ Thought for Ns, Nk tokens` |

→ **antigravity 完成检测应该用状态行 marker (LineEndRegex 类), 不用 gemini 的 ObservedStability (屏幕稳定模糊判定)**。这直接解了 a3 §1.1 命门: 对 antigravity 不存在"稳定=完成 还是 还在算"的歧义, 因为状态行显式区分。
→ **这回答用户 Q7**: 换 antigravity **会**解决 gemini 完成检测慢的问题 (有显式 idle marker, 不用等屏幕稳定 ms)。

## Trust 对话框 (cwd 不在 trustedWorkspaces 时弹)
```
Antigravity CLI requires permission to read, edit, and execute files here.
> Yes, I trust this folder
  No, exit
  ↑/↓ Navigate · enter Confirm
```
- 自动处理: 光标默认在 "Yes, I trust this folder" → 直接 Enter。
- 注意: settings.json trustedWorkspaces 含 `/home/sevenx`, 所以在 `/home/sevenx/coding/ccbd-rust` 下启动**可能不弹** (待 ah 实际在项目目录验证); 在 `/tmp` 下必弹。

## Cancel 键 = ESC (单次, 实测干净)
- 实测: busy 时发单次 ESC → 显示 `⎿ Interrupted · What should Antigravity CLI do instead?` → 回到 idle (`? for shortcuts`)。
- **印证用户 Q8 + a2 设计**: cancel 必须发 ESC, 不是 Ctrl-C。UI 自己写 `esc to cancel`。

## 派发方式 (paste-buffer 多行, 实测正常)
- `tmux load-buffer` + `paste-buffer` + Enter → antigravity 正确接收多行 prompt (实测 LINE_ONE_OK/LINE_TWO_OK 两行都进了一个 prompt)。
- → ah 现有 writer.rs 的 paste-buffer 派发方式对 antigravity 直接可用, 无需改。

## Phase 1 manifest 种子结论
antigravity 接入比 gemini 更简单:
1. **idle marker**: 状态行 `? for shortcuts` (LineEndRegex 或状态行匹配) — 比 gemini ObservedStability 快且无歧义。
2. **busy marker**: `esc to cancel`。
3. **cancel 键**: ESC (per-provider cancel_sequence = ["Escape"])。
4. **trust 框**: prompt KB seed case (anchor "Yes, I trust this folder", action 选第一项 + Enter)。
5. **派发**: 复用现有 paste-buffer, 无需改。
6. **登录态**: 已登录, Phase 1 不需处理冷启动登录 (MF5 scope-out 成立)。

## 追加实测发现 (Phase 1 关键缺口, 2026-05-30 主控 dogfooding)

### 1. trust 框: `--dangerously-skip-permissions` 不能跳过
- 实测: 在 `/home/sevenx/coding/ccbd-rust` (不在 trustedWorkspaces 精确列表) 用 `agy --dangerously-skip-permissions` 启动, **仍弹 trust 框**:
  ```
  Accessing workspace: /home/sevenx/coding/ccbd-rust
  Do you trust the contents of this project?
  > Yes, I trust this folder
    No, exit
  ```
- → `--dangerously-skip-permissions` 只跳工具权限, **不跳 workspace 信任框**。
- **修复**: 种子化 — 把 workspace 路径写进 `~/.gemini/antigravity-cli/settings.json` 的 `trustedWorkspaces` 数组。实测: 路径在数组里时, 启动**直接到 idle, 无 trust 框**。

### 2. paste 带换行尾 (`\n`) 自动提交
- 实测: paste-buffer 内容以 `\n` 结尾, **不补 Enter 也会自动提交** (antigravity 把尾部 `\n` 当 Enter)。
- → 若 ah 派的 prompt 带换行尾又补 Enter = **双提交** (第二次 Enter 提交空 prompt)。
- a1 的 `press_enter_after_paste(antigravity, text) = !text.ends_with('\n')` 修复**有据, 应保留**。

### 3. cancel = 单次 ESC (不是 ESC+Enter)
- 实测: busy 时单次 ESC → `⎿ Interrupted...` → 回 idle。单 ESC 足够。
- → a1 的 `["Escape","Enter"]` 错, 应为 `["Escape"]` (多发 Enter 会在空 prompt 回车 + 测试断言单字节 [0x1b])。

### 4. OAuth token 位置
- `~/.gemini/antigravity-cli/antigravity-oauth-token` (498 bytes)。sandbox materialization 需 copy 它 (类似 BUG-4 对 gemini creds 的 copy-not-symlink)。

## 追加实测发现 2 (真 ahd dogfooding 跑通 Phase 1, 2026-05-30)

### 关键缺口: fresh sandbox 有多步 onboarding 向导, trust 种子不够
真 ahd 起 antigravity agent 实测: sandbox HOME 是全新的, antigravity 跑**首次启动 onboarding 多步向导**, 挡住 readiness:
1. **theme 选择向导** (配色: tokyo night / dark / ... + 代码预览 + [Next])
2. **Terms of Service & Data Use 同意框** ([x] checkbox + [Previous]/[Done])
3. 完成后才到 IDLE

后果: agent spawn 后卡在向导, 60s readiness 窗口 (CCB_GEMINI_READY_TIMEOUT_S) 超时 → ah DB 里 agent state = **UNKNOWN**, 从没到 IDLE, `ah ask` 无法 dispatch。

### 修复 (验证有效): 种子 cache/onboarding.json
onboarding 完成后 antigravity 写 `~/.gemini/antigravity-cli/cache/onboarding.json`:
```json
{"consumerOnboardingComplete": true, "enterpriseOnboardingComplete": false, "onboardingComplete": true}
```
**主控实测**: fresh HOME 同时种子化 3 个文件 → antigravity **直接 boot 到 IDLE, 无任何向导**:
1. `.gemini/antigravity-cli/settings.json` 含 trustedWorkspaces (已做)
2. `.gemini/antigravity-cli/cache/onboarding.json` 含 onboardingComplete=true (**Phase 1 还缺这个**)
3. `.gemini/antigravity-cli/antigravity-oauth-token` copy 0600 (已做)

### init probe
GeminiInitProbe (init_probe.rs:65) = banner 消失 + 最后 8 行有 `> ` 提示符。它测试 fixture 已含 `? for shortcuts` + `> `, 所以 seed onboarding 后大概率能识别 antigravity idle。需 re-dogfood 确认 SPAWNING→IDLE。

## 追加实测发现 3: init probe 复用 Gemini 失败 (裸 `>` 提示符)
- 种子 onboarding.json 后 antigravity **真到 IDLE** (无向导), 但 ah 仍卡 **SPAWNING**。
- 根因: a1 复用 InitProbeKind::Gemini, GeminiInitProbe 要求最后 8 行某行 lstrip 后以 `> ` (**带空格**) 开头 (init_probe.rs GEMINI_PROMPT_PREFIXES)。但 antigravity idle 提示符是**裸 `>`**, tmux capture-pane strip 行尾空格 → `cat -A` 实测提示符行 = `>$` (无空格) → 不匹配 → 永不就绪。
- **修复**: 加 AntigravityInitProbe (InitProbeKind::Antigravity), 用 `? for shortcuts` 状态行判就绪 (该行可靠存在, 实测在第 12 行), 跟 matcher idle 检测一致。antigravity manifest init_probe 从 Gemini 改 Antigravity。
