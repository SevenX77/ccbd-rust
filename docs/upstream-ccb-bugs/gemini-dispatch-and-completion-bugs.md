# 上游 CCB Bug：Gemini provider 投递 + completion 检测三个独立 bug（合并报告）

| 字段 | 值 |
|---|---|
| **状态** | Bug X + Y + Z 全部已修（fork `personal` 分支，commits `c786ec9` + `214eef7` + `ae6d9a7` + `9b90e2e`，已 push origin/personal）。upstream PR 待提。 |
| **首次集中复现** | 2026-04-26 在 `/home/sevenx`（VPS vultr-sever-sv）+ `/home/sevenx/coding/agent-harness` 两个项目 |
| **影响范围** | 所有用 ccb 调度 Gemini provider (`a2:gemini`) 的项目，长跑都会撞上 |
| **所属仓库** | 上游 [bfly123/claude_code_bridge](https://github.com/bfly123/claude_code_bridge) 及本地 fork `~/coding/claude_code_bridge/`（branch `personal`），**不是** ccbd-rust 仓库 |
| **优先级建议** | 高 — Gemini 在 sevenx 工作流里承担 analyst / 思考 / 设计审阅角色（见 `~/.claude/rules/ccb-collaboration.md`），这三个 bug 让 Gemini 实际不可用 |
| **撰写人** | Claude Opus 4.7（主控）+ sevenx（识别 + 决策方向） |
| **关联文档** | `~/coding/ccbd-rust/research/findings/synthesis-18-days-by-claude.md` 群 A "CCB 投递 / completion 检测 bug（最高频，13/18 天有）" — 本文是群 A 的三个具体子 bug 的可操作版报告 |
| **建议交付者** | 任何对 `claude_code_bridge` Python 仓有写权限的开发者；可走 upstream PR 也可只在本地 fork 修。三个 bug 应该分别 PR，不要合并 |

---

## 1. 上下文：CCB 是怎么把消息派到 Gemini pane 的

`claude_code_bridge`（CCB）调度多个 LLM CLI（codex / gemini / claude / opencode）作为 sub-agent。当 master Claude 跑 `ccb ask a2 /clear` 这种命令时，整条投递链是：

1. **master Claude shell** — 调 `ccb ask` 命令
2. **ccb CLI**（`/home/sevenx/.local/bin/ccb`）— 解析参数，连接到当前项目的 ccbd（daemon）
3. **ccbd**（`/home/sevenx/.local/share/codex-dual/lib/ccbd/main.py`）— 把消息写入 a2 的 mailbox（`<project>/.ccb/ccbd/mailboxes/a2/inbox.jsonl`），创建 job 记录
4. **gemini provider 的 communicator**（`provider_backends/gemini/comm_runtime/communicator_facade.py`）— 从 mailbox 取消息，准备投递到 a2 pane
5. **tmux backend 的 send_text**（`terminal_runtime/tmux_send.py:33`）— 用 `tmux load-buffer + paste-buffer -p + send-keys Enter` 把内容塞进 pane
6. **Gemini CLI**（运行在 a2 tmux pane 里）— 接收输入，处理，回输出
7. **completion detector**（`provider_backends/gemini/execution_runtime/polling_runtime/reader.py`）— polling 读 pane 输出，判定 job 是否完成
8. **ccbd 把 job 标 completed → ccb ask 那一端 wait 返回**

本文要报告的三个 bug 分别发生在第 5 步（投递机制）、第 7 步（completion 检测）、和一条平行小路径（autonew 工具的 session 查找）。

---

## 2. 三个 bug 概要

| # | 短名 | 严重性 | 一句话现象 | 一句话根因 |
|---|---|---|---|---|
| **X** | slash-via-paste-not-recognized | High | `ccb ask a2 /clear` 被 Gemini 当成普通 user prompt 处理（不清屏，反而进 LLM 推理） | tmux 投递走 `paste-buffer -p` = bracketed paste，Gemini CLI 的 slash command parser 只在 keystroke 流上触发，paste 内容直接进 prompt buffer |
| **Y** | completion-detector-misses-thinking-and-clear | High | Gemini 长 thinking 时 ccbd 误判 idle；`/clear` 后 pane fresh 但 ccbd 也不标 job 完成，state 卡 `busy queue=1` | completion detector 依赖某种 anchor / reply marker 信号，Thinking spinner 状态没被识别，且 `/clear` 后 anchor 被清屏抹掉，detector 拿不到 reply_stable 标记 |
| **Z** | autonew-cannot-find-gemini-session | Medium | `autonew gemini` 直接报 "No active gemini session found"，明明 ccbd 里 a2:gemini 在跑 | autonew 走 `pane_registry`（`pane_registry_runtime/`），但 ccbd 启动 agent 时只写 lease.json / agents/<name>/ 不写 pane_registry — 两套独立的 session tracking 互不同步 |

---

## 3. Bug X — slash command 通过 ccb ask 投递时不被 Gemini 识别

### 3.1 现象（具体观察 — 2026-04-26 12:15 复现）

主控 Claude 在 `/home/sevenx` 跑：
```bash
cd /home/sevenx && ccb ask --wait --timeout 30 a2 /clear
```

`ccb ask` 端：
```
command_status: failed
error: wait timed out for job_f0759f8a53e8
event: ... a2 job_accepted ...
event: ... a2 job_started ...
event: ... a2 completion_item ...   # 出现一次 anchor_seen
event: ... a2 completion_state_updated ...   # terminal=false
```

a2 pane 实际显示的内容（`tmux capture-pane -t %9 -p` 抓的，节选）：
```
✦ I will check the diff for TECH_DEBT.md and TASK-PLAN-2026-04-24-handoff.md to see the changes
  made by Codex.

  ✓  Shell git -C .claude diff TECH_DEBT.md TASK-PLAN-2026-04-24-handoff.md
    ... [git diff 输出 一大段] ...

✦ I will now delegate the implementation of the context_bridge validator to the Codex agent (a1),
  as outlined in the docs/superpowers/plans/2026-04-25-pr7-context-bridge-validator.md plan,
  starting with Tasks 1 through 13.
```

**Gemini 没有清屏，也没有进 slash command 模式**，反而把 `/clear` 当成了一段普通的用户输入，进入 LLM 推理：调 `git diff` 工具、生成 "I will delegate the implementation to the Codex agent" 这种回复。换句话说，`/clear` 被当成了"用户用自然语言要求我清理什么东西"的 prompt。

### 3.2 PoC 验证：直接 keystroke 路径就能正确触发 slash command

绕开 ccb 的 mailbox + paste-buffer 投递链，直接用 tmux 的 keystroke 模式发同样字符串：

```bash
tmux -S /home/sevenx/.ccb/ccbd/tmux.sock send-keys -t %9 -l "/clear"
sleep 0.5
tmux -S /home/sevenx/.ccb/ccbd/tmux.sock send-keys -t %9 Enter
```

a2 pane 立刻反应（同样 `capture-pane` 抓的）：
```
 > /clear

 ▝▜▄     Gemini CLI v0.39.1
   ▝▜▄
  ▗▟▀    Signed in with Google /auth
 ▝▀      Plan: Gemini Code Assist in Google One AI Pro /upgrade

 *   Type your message or @path/to/file
```

**Gemini 真的执行了 `/clear` slash command** — 进入清屏 + 重置 conversation，回到 Gemini CLI 启动画面。这证明了：
- 同样的字符串 `/clear<Enter>`，**走 keystroke 路径会被 Gemini 识别为 slash command**
- **走 ccb 现在的 paste-buffer -p 路径不会被识别**

### 3.3 根因（指向具体代码）

源码：`/home/sevenx/.local/share/codex-dual/lib/terminal_runtime/tmux_send.py`，`_paste_via_buffer_legacy` 函数（第 88-122 行）：

```python
def _paste_via_buffer_legacy(self, *, target: str, text: str, pane_target: bool) -> None:
    buffer_name = ...
    self.tmux_run_fn(['load-buffer', '-b', buffer_name, '-'], ..., input_bytes=text.encode('utf-8'))
    ...
    if pane_target:
        self.tmux_run_fn(['paste-buffer', '-p', '-t', target, '-b', buffer_name], check=True)  # ← 关键：-p
    ...
    self.tmux_run_fn(['send-keys', '-t', target, 'Enter'], check=True)
```

**`tmux paste-buffer -p` 的语义**：
- `-p` 表示 "use bracketed paste mode if the application has it enabled"
- 实际行为：在 paste 内容前后插入 escape 序列 `\x1b[200~ ... \x1b[201~`，告诉接收端"以下是用户粘贴的内容，不是逐字符键入"
- 这是 terminal 标准协议（让 vim、IDE 等区分 keystroke 和 paste，避免 auto-indent 和 keymap 干扰）

**Gemini CLI 怎么处理这两种输入**：
- 当用户在 prompt 区**键入** `/` 字符时（terminal 报告为 keystroke event）→ Gemini CLI 进入 slash command 模式（弹出 `/clear`、`/help`、`/auth` 等候选 menu）
- 当 terminal 收到 bracketed paste（`\x1b[200~/clear\x1b[201~`）→ Gemini CLI 把整段内容当作"用户粘贴的文本"塞进 prompt buffer，**不走 slash command parser**
- 当用户随后按 `Enter` → Gemini 把 prompt buffer 的内容（`/clear`）作为 user message 提交给 LLM 推理

这就是 `/clear` 被当成 prompt 处理的物理原因。

`_paste_via_buffer_reception_driven`（第 124-213 行）也是同样的 `paste-buffer -p`，所以 reception-driven 路径同样有 bug。

`autonew` 调 `backend.send_text` 也走这条 `_paste_via_buffer` 路径（见 `/home/sevenx/.local/share/codex-dual/bin/autonew` 第 107 行 `backend.send_text(pane_id, reset_cmd)`），所以即便 autonew 的 Bug Z 修好了，它发的 `/clear` 也会被同样的 paste 机制拦截。

### 3.4 修复方向

**方案 A（推荐 — 最小改动且只影响 slash command）**：

在 `_paste_via_buffer` 入口加判断：如果 `text.startswith('/')` 且不含换行，走 keystroke 路径；其他情况保留 paste-buffer 路径。

伪代码：
```python
def send_text(self, pane_id, text, ...):
    sanitized = self.sanitize_text_fn(text)
    if not sanitized:
        return
    target_is_tmux = self.looks_like_tmux_target_fn(pane_id)
    
    # NEW: slash command 检测，绕过 bracketed paste
    if target_is_tmux and self._looks_like_slash_command(sanitized):
        self.ensure_not_in_copy_mode_fn(pane_id)
        self.tmux_run_fn(['send-keys', '-t', pane_id, '-l', sanitized], check=True)
        time.sleep(0.3)
        self.tmux_run_fn(['send-keys', '-t', pane_id, 'Enter'], check=True)
        return
    
    # 原有路径不变
    if not target_is_tmux:
        ...
    self.ensure_not_in_copy_mode_fn(pane_id)
    self._paste_via_buffer(target=pane_id, text=sanitized, pane_target=True, ...)


def _looks_like_slash_command(self, text: str) -> bool:
    """Slash command = 单行，以 / 开头，全字符无换行。"""
    return bool(text) and text[0] == '/' and '\n' not in text and '\r' not in text
```

**优点**：
- 只对 `/` 开头的单行命令变路径，普通 prompt（多行 plan / context）继续走 paste（更快）
- 跨 provider 通用 — codex / claude / opencode 的 slash command 都能受益（虽然它们的 slash 命令名不同，但都是 keystroke 触发）

**缺点**：
- 假设 "以 `/` 开头的单行 = slash command"。如果用户真的想问"`/etc/passwd` 在哪"这种以 `/` 开头但是 prompt 的内容，会被错路由（但这种 prompt 一般会带空格或问号，`_looks_like_slash_command` 加严一点就能区分；或者保留显式 escape 选项）

**方案 B（更保守 — 只改 autonew 的快速路径）**：

只在 `autonew` 这种"明确发 slash command"的工具里用 keystroke，主流的 `ccb ask` 不动。
- 缺点：`ccb ask a2 /clear` 仍然不能用，违反"两条路径效果应一致"的最小惊讶原则

**方案 C（最重 — 完全弃用 paste-buffer）**：

`send_text` 改全部用 `send-keys -l`，每个字符 keystroke 模式。
- 缺点：长文本（几 KB 的 prompt）会很慢；keystroke 没有 atomic paste 的"一次性插入"语义，pane 处于半接收状态时容易撞上 prompt 区状态变化

---

## 4. Bug Y — completion detector 在 Gemini 长 thinking / clear 后漏判 job 完成

### 4.1 现象 1：long-thinking 期间投新消息会永久 stuck

复现：
1. 让 a2 (gemini) 处于"上一轮还在 Thinking" 状态（pane 上有 `Thinking... (esc to cancel, 48s)` spinner）
2. ccbd 此时通过 polling reader 判定 a2 = `idle`（这本身就是误判，但 supervisor 还触发了 `recover` action，标 `prior_health=pane-missing` 然后又 `recover_succeeded` —— 状态机的两个判定互相矛盾）
3. master 跑 `ccb ask --wait a2 <任何内容>` → ccbd 派 job → tmux 投递 → 进入 Gemini input queue（但 Gemini 在 Thinking 不消费）
4. ccbd events 看到 `completion_item: anchor_seen` 一次（因为 anchor 文本碰巧出现），然后 `completion_state_updated: terminal=false, reply_stable=false`，**永远不会再有新事件**
5. `wait` timeout，agent state 卡 `busy queue=1`

具体例子（job_5fa6f4eca5af 在 11:56 复现）：
- 11:56:15 job_accepted
- 11:56:15.102 job_started
- 11:56:16.812 completion_item: anchor_seen，completion_state_updated terminal=false
- (no more events for 3 minutes until manual esc)
- 11:59:17 (after manual esc) completion_item: assistant_final，completion_terminal completed，job_completed

### 4.2 现象 2：`/clear` 之后 pane 完全干净，但 ccbd 仍然不标 job 完成

复现（同次会话，job_f0759f8a53e8 在 12:15 复现）：
1. a2 pane 处于 stuck 状态
2. master 用 keystroke 路径手动 `/clear`，pane 真的清屏，回到 Gemini CLI 启动画面
3. ccbd 看到 `anchor_seen` 后再也没新事件
4. 8 分钟后 a2 state 仍 `busy queue=1`

这意味着 completion detector 不只在"long thinking"时漏判，在 "fresh state after /clear" 时也漏判。说明 detector 依赖的 anchor / reply marker 不能 cover Gemini 的所有完成路径。

### 4.3 supervisor 还会"瞎 recover"

`supervision.jsonl` 里看到这种事件序列：
```
recover_started prior_health=pane-missing → recover_succeeded result_health=healthy restart_count=N+1
```

但 a2 pane 实际**根本没死** —— Gemini 进程一直在跑（pid 没变，内存数字在涨）。supervisor 的 health check 把 long-thinking pane 误判为 missing，触发 recover，但 recover 行为**只是把状态机标 recovered**，没真的杀进程重起。结果就是 restart_count 持续累加（已经看到 restart_count: 4），但底层啥也没变。

### 4.4 根因（指向具体代码）

嫌疑代码（需要进一步阅读）：
- `/home/sevenx/.local/share/codex-dual/lib/provider_backends/gemini/execution_runtime/polling_runtime/reader.py` — polling 读 pane 输出做 idle / completion 判定
- `/home/sevenx/.local/share/codex-dual/lib/provider_backends/gemini/comm_runtime/communicator_facade.py` — anchor / reply marker 投递与监听
- ccbd supervisor 部分 — health check 判定 `pane-missing` 的具体逻辑

具体根因猜测（待源码确认）：
- detector 依赖某种 anchor 文本（reply 后的特定字符串 / 文件 marker）
- Gemini 的 reply 路径有多种：assistant_final、tool_call、partial（streaming）、cancelled、cleared
- detector 只 cover 了部分路径（比如 assistant_final），对 cancelled / cleared / 长 thinking 中无 reply 这些状态没有 fallback

### 4.5 修复方向

**方案 A（推荐 — 多信号融合判定）**：

参考 `synthesis-18-days-by-claude.md` 第 168 行的设计输入"provider-aware completion detection（不只看 pane 内容增长，要 hook 信号 + multi-signal + deadline 三层）"，在 detector 加：
- **多信号融合**：anchor seen / reply file / pane stable / Gemini CLI internal state（如果有可读的） — 任一确定信号就标 done，不依赖单 anchor
- **死线**：每个 job 设硬 deadline（比如 60s 无任何信号 → 标 `terminal=true status=stuck`，让 master 端能感知）
- **状态对齐 invariant**：supervisor 的 health 判定与 detector 的 idle 判定必须用同一套数据，禁止"detector 说 idle 但 supervisor 说 pane-missing"这种矛盾

**方案 B（窄修 — 只补 cleared / cancelled 信号）**：

在 detector 里加：监听 pane 内容是否包含 Gemini CLI 启动横幅（`Gemini CLI v...` / `Signed in with Google` / `Plan: Gemini Code Assist`）—— 任一出现说明刚 `/clear`，立刻把当前 in-flight job 标 terminal completed reason=cleared。

类似地为 `Request cancelled.` / `Esc to cancel` 等 marker 加 fallback 路径。

**缺点**：太脆 — Gemini 升级可能换 marker 文本；且没解决 long-thinking 的 stuck 问题。

---

## 5. Bug Z — `autonew gemini` 找不到 gemini session  ✅ 已修（fork commit `9b90e2e`，2026-04-26）

> **修复摘要**：采纳本节方案 A。新增 `lib/agents/runtime_lookup.py:find_agent_runtime_by_provider()`，扫 `<project>/.ccb/agents/<name>/runtime.json` 按 `provider` 字段过滤，挑 `last_seen_at` 最新且 `pane_state != "dead"` 的 agent。`bin/autonew` 不再 import `pane_registry_runtime`，直接用新 lookup 函数。
>
> **测试**：`test/test_agent_runtime_lookup.py` 7 个 case 全过（match / no-match / no-anchor / multi-prefer-recent / dead-skip / cwd 上行 walk / 坏 JSON 跳过）。
>
> **端到端验证**：`/home/sevenx` 下 `autonew gemini` → `Sent /clear to gemini (pane: %3)` exit=0；`tmux capture-pane -t %3` 看到 Gemini CLI 启动横幅，确认 `/clear` 真生效（依赖 Bug X 修过的 keystroke 路径）。
>
> **install path 同步**：`~/.local/share/codex-dual/bin/autonew` 和 `~/.local/share/codex-dual/lib/agents/runtime_lookup.py` 已 cp 到位，日常 `autonew gemini` 立即可用，不需重跑 install.sh。

### 5.1 现象（已修复前）

```bash
cd /home/sevenx && autonew gemini
[ERROR] No active gemini session found for this project.
```

但 `ccb ps` 同时显示 a2:gemini state=busy/idle，pane 实际活着。

### 5.2 根因

源码 `/home/sevenx/.local/share/codex-dual/bin/autonew`：
```python
project_id = compute_ccb_project_id(work_dir)
record = load_registry_by_project_id(project_id, provider)
if not record:
    print(f"[ERROR] No active {provider} session found for this project.", file=sys.stderr)
    return EXIT_ERROR
```

`load_registry_by_project_id` 来自 `pane_registry_runtime`，查的是某个 pane registry 文件（具体路径需要 grep 确认，可能是 `~/.cache/ccb/pane-registry/` 之类）。

但 ccbd 启动 agent 时（`provider_backends/gemini/...`）写的 session 状态在另外两个地方：
- `<project>/.ccb/ccbd/lease.json`（ccbd 自己的 lease 信息）
- `<project>/.ccb/agents/a2/runtime.json`（agent 的 binding 信息，包含 pane_id=%9）

**两套 session tracking 系统互相不同步**：
- autonew 期待 pane_registry 有数据
- ccbd 不写 pane_registry，只写自己的 lease + agent binding

### 5.3 修复方向

**方案 A（推荐 — autonew 改用 ccbd 的真实数据源）**：

autonew 不查 pane_registry，改用：
1. 找当前 cwd 的 `.ccb/ccbd/lease.json` 拿 ccbd_pid + socket_path
2. 通过 ccbd socket 查 a2 (or 任意 provider 名) 的 binding，拿 pane_id
3. 直接 send 到 pane

**优点**：跟 ccb 命令同源，不会再出现"ccb ps 看得到，autonew 看不到"的不一致

**方案 B（较松 — ccbd 启动时同步写 pane_registry）**：

ccbd 起 agent 时把 pane_id 也写一份到 pane_registry 文件，让 autonew 能查到。

**缺点**：维护两份数据源，仍然可能不一致；不解决根本问题

---

## 6. 三个 bug 的修复优先级 + 关联性

**修复优先级**（从最该先修）：
1. **Bug X** — 修了之后 `ccb ask a2 /clear` 立刻能用，是用户日常清 Gemini session 的核心入口 ✅
2. **Bug Y** — 修了之后 long-thinking + cleared 的 stuck job 不再无限挂起，state 一致性回归 ✅
3. **Bug Z** — autonew 是 fallback 工具，使用频率较低；但因为 X 修好后 Bug X 的 PoC workaround 也算解决了 autonew 的需求场景，Z 可以延后 ✅

**关联性**：
- X 是投递层 bug，Y 是检测层 bug，Z 是工具链 bug
- 三个互相独立，应该分三个 PR
- 但 X 和 Y 的测试用例会大量重叠（都需要走"发 slash command → 看 Gemini 反应 → 看 ccbd state 收敛"的完整链路），可以共享 test fixture

---

## 7. 不要做的事

1. **不要在 ccbd-rust 仓库里直接修 X / Y / Z 的实现** — ccbd-rust 是 Rust 重写项目，修复要在 Python 的 `claude_code_bridge` fork 里做。本文档放在 ccbd-rust 是因为它属于 ccbd-rust 的"上游 bug 跟踪"目录（跟 installer-default-config-mismatch.md 同处）
2. **不要把 X 和 Y 的修复合到同一个 PR** — 投递机制和 completion 检测是两层，独立修便于回归排查
3. **不要修了 X 之后忘记测试 codex / claude provider 的 slash command** — 同样的 paste-buffer -p 也用在 codex/claude 投递上，修复方案 A 自动覆盖它们，但需要回归测试
4. **不要试图通过"写 conversation log"绕过这些 bug** — 用户已经反复观察到 ccb 投递不可靠，再加 log 解决不了根因

---

## 8. 验证清单（修完后请按此项验证）

- [ ] `ccb ask --wait --timeout 30 a2 /clear` 在 30 秒内返回 success，pane 真的清屏到 Gemini CLI 启动画面
- [ ] 在 a2 处于 long thinking（>1 分钟）期间发 `ccb ask a2 <test>`，job 在合理时间内（<2 分钟）要么真完成、要么标 terminal=true reason=stuck，state 不能永远 busy
- [ ] `autonew gemini` 不再报 "No active gemini session found"，能找到正确的 pane 并发 `/clear`
- [ ] 跨 provider 回归测试：codex 的 `/new`、claude 的 `/new`、opencode 的 `/new` 都通过 `ccb ask` 能正常触发对应 provider 的 slash command
- [ ] `<project>/.ccb/agents/a2/events.jsonl` 不再出现"completion_item anchor_seen 之后永久无 terminal 事件"的孤立 job
- [ ] `<project>/.ccb/ccbd/supervision.jsonl` 不再出现"prior_health=pane-missing → recover_succeeded restart_count=N+1"反复累加但 pane 实际没死的 noise

---

## 9. 相关文件路径（修复时 grep 起点）

**ccb 投递路径**（Bug X 修复入口）：
- `/home/sevenx/.local/share/codex-dual/lib/terminal_runtime/tmux_send.py:33` — `TmuxTextSender.send_text`
- `/home/sevenx/.local/share/codex-dual/lib/terminal_runtime/tmux_send.py:88` — `_paste_via_buffer_legacy`（核心 bug 行：第 101 行 `paste-buffer -p`）
- `/home/sevenx/.local/share/codex-dual/lib/terminal_runtime/tmux_send.py:124` — `_paste_via_buffer_reception_driven`（同样的 paste 机制）
- `/home/sevenx/.local/share/codex-dual/lib/terminal_runtime/tmux_backend_control.py:27` — backend 的 `send_text` 入口

**Gemini provider 与 communicator**：
- `/home/sevenx/.local/share/codex-dual/lib/provider_backends/gemini/comm_runtime/communicator_facade.py` — 投递前的 init-gate 检查
- `/home/sevenx/.local/share/codex-dual/lib/provider_backends/gemini/execution_runtime/polling_runtime/reader.py` — Bug Y 主嫌疑代码
- `/home/sevenx/.local/share/codex-dual/lib/provider_backends/gemini/` — 整个 gemini provider 目录

**autonew 工具**（Bug Z 修复入口）：
- `/home/sevenx/.local/share/codex-dual/bin/autonew` — 入口脚本，第 75 行 `load_registry_by_project_id` 是 bug 触发点
- `/home/sevenx/.local/share/codex-dual/lib/pane_registry_runtime/` — autonew 期望的 registry，但 ccbd 不写

**ccbd 真实数据源**（Bug Z 方案 A 的修复参考）：
- `<project>/.ccb/ccbd/lease.json` — ccbd 启动时写的 lease 信息（含 socket_path）
- `<project>/.ccb/agents/<name>/runtime.json` — agent binding 信息（含 pane_id）
- `<project>/.ccb/ccbd/state.json` — ccbd namespace state

**fork 路径**（修复时改这里再同步到 install 路径）：
- `~/coding/claude_code_bridge/lib/terminal_runtime/tmux_send.py`
- `~/coding/claude_code_bridge/lib/provider_backends/gemini/`
- `~/coding/claude_code_bridge/bin/autonew`

---

## 10. 提交者注意事项

- 修复后请在本机用真实的 `ccb ask a2 /clear` + `autonew gemini` + Gemini long-thinking 场景跑通验证清单第 8 节，每条都有截图 / events 日志
- 如果走 upstream PR：附本文档作为问题描述，并附 PoC（第 3.2 节）的 reproducible 命令
- 如果只在本地 fork 改：提醒 sevenx 在 install 脚本流程里同步改后的文件到 `~/.local/share/codex-dual/`
- 三个 bug 单独 PR；如果 maintainer 想合并讨论，至少 commit 拆开（一个 commit 一个 bug）
