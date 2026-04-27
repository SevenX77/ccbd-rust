# CCB Dispatch 设计教训：slash command 必须走 keystroke 路径，禁止 bracketed paste

| 字段 | 值 |
|---|---|
| **类型** | Design lesson（给 ccbd-rust 重写者的设计输入，非 bug report） |
| **首次发现** | 2026-04-26 在 `/home/sevenx`（VPS vultr-sever-sv）调试 ccb gemini 卡死时实证 |
| **撰写人** | Claude Opus 4.7（主控）+ sevenx |
| **状态** | 已实证 PoC，建议作为 ccbd-rust dispatch 层的强制原则 |
| **关联** | 上游 bug 报告 `docs/upstream-ccb-bugs/gemini-dispatch-and-completion-bugs.md` 第 3 节 Bug X、`research/findings/synthesis-18-days-by-claude.md` 群 A 的 4-22 ah "tmux 投递成功但没自动 Enter，pane 卡 `[Pasted Text: 69 lines]`" |

---

## 1. 教训的一句话总结

**对接 TUI 应用（Gemini CLI / Claude Code / Codex / Opencode 等）时，发送 slash command（`/clear`、`/new`、`/help` 等单行 `/` 开头命令）必须用 keystroke 路径（一个一个字符送），禁止用 bracketed paste 一次性灌入。普通的多行 prompt 内容仍然走 bracketed paste（更快更原子）即可。**

简短但容易在重写时忘掉，因为表面上"两种方式都把字符送进了 pane"——区别只在 terminal 协议层的 escape 序列，对程序员是透明的，对 TUI 应用却是决定性的。

---

## 2. 物理原理：bracketed paste 跟 keystroke 在 TUI 里完全不同

### 2.1 keystroke 路径
当用户在 terminal 里**按键**时，terminal 把每个字符（或字符组合）作为独立的 input event 发给前台应用。比如按 `/`，应用收到的是单个字节 `0x2F`，**不带任何前后包装**。

TUI 应用（Gemini CLI 等）的 input loop 通常这样写：
```
on_key_press(char):
    if input_buffer.is_empty() and char == '/':
        enter_slash_command_mode()  # 弹出 /clear、/help 等候选 menu
    else:
        append_to_input_buffer(char)
```

**slash command 模式只在"输入区为空 + 第一个字符是 /"时进入**。这是绝大多数 TUI 应用的标准做法。

### 2.2 bracketed paste 路径
当用户**粘贴**文本时（terminal 协议要求 paste 来源跟 keystroke 区分开），terminal 在 paste 内容前后插入 escape 序列：
```
\x1b[200~      ← paste start marker (CSI 200 ~)
<内容>
\x1b[201~      ← paste end marker (CSI 201 ~)
```

TUI 应用的 input loop 看到 `\x1b[200~` 就**进入 paste mode**：
```
on_paste_start():
    paste_mode = true
on_byte(byte):
    if paste_mode:
        append_to_input_buffer(byte)   # 整段当作"用户粘贴的文本"
    else:
        on_key_press(byte)             # 走正常 keystroke 路径
on_paste_end():
    paste_mode = false
```

**paste mode 里所有内容直接进 input buffer**，slash command parser 完全不被触发——即使 paste 内容是 `/clear<Enter>`，应用也会把它当成"用户粘贴了一段以 / 开头的文本，然后按了回车提交"。提交结果就是把 `/clear` 当成 user message 发给 LLM 推理。

### 2.3 设计动机：为什么 terminal 要区分两者
这不是 Gemini / Claude 自己的特殊行为，是 terminal 协议（xterm 引入的标准，现代 terminal 都支持）。区分的目的：
- **vim / IDE**：粘贴大块代码时不要触发 auto-indent / abbrevs / leader keymap，避免 paste 内容被"键盘行为"破坏
- **shell**：粘贴多行命令时不要逐行 PS1 回显，整体 atomic 处理
- **TUI 应用**：区分"用户主动键入"和"程序/clipboard/script 灌入"，可以拒绝某些 paste（比如 confirmation prompts）

slash command 设计为 keystroke-only 是 TUI 应用的"防御性"选择——避免 paste 进来的恶意/误触 slash command 修改应用状态（比如 paste 一段日志里恰好包含 `/quit` 不应该让应用退出）。

---

## 3. 实证 PoC（2026-04-26 在 vultr-sever-sv 上验证）

### 3.1 失败路径：bracketed paste（CCB 当前 v6.0.7 用的）

```bash
# CCB 内部 _paste_via_buffer_legacy 等价：
buf=ccb-test-$(date +%s)
echo -n "/clear" | tmux -S /home/sevenx/.ccb/ccbd/tmux.sock load-buffer -b $buf -
tmux -S /home/sevenx/.ccb/ccbd/tmux.sock paste-buffer -p -t %9 -b $buf
tmux -S /home/sevenx/.ccb/ccbd/tmux.sock send-keys -t %9 Enter
tmux -S /home/sevenx/.ccb/ccbd/tmux.sock delete-buffer -b $buf
```

**关键 flag**：`paste-buffer -p`，`-p` = use bracketed paste mode if the application has it enabled。

实际行为：Gemini pane 显示 `> /clear` 然后**进入 LLM 推理**，输出回复"由于你发出了 /new 指示且当前环境处于 YOLO 模式，如果你没有其他特定应用要开发，我将默认你希望我启动一个新的子任务"——把 `/clear`（实际还有先前的 `/new`）当成了用户 prompt。

### 3.2 成功路径：keystroke

```bash
tmux -S /home/sevenx/.ccb/ccbd/tmux.sock send-keys -t %9 -l "/clear"
sleep 0.5
tmux -S /home/sevenx/.ccb/ccbd/tmux.sock send-keys -t %9 Enter
```

**关键 flag**：`send-keys -l`，`-l` = literal mode，每个字符作为独立 keystroke event 发出去（不走 bracketed paste）。

实际行为：Gemini 立刻执行 `/clear`，清屏 + 重置 conversation，回到 Gemini CLI 启动画面：
```
 ▝▜▄     Gemini CLI v0.39.1
   ▝▜▄
  ▗▟▀    Signed in with Google /auth
 ▝▀      Plan: Gemini Code Assist in Google One AI Pro /upgrade

 *   Type your message or @path/to/file
```

---

## 4. 抽象出的设计原则（给 ccbd-rust 强制采纳）

### 原则 1：投递层必须区分两种内容

dispatch 实现里，`send_to_pane(pane, text)` 不能一刀切。要在投递前判定 text 的类型：

| 内容类型 | 判定条件 | 投递方式 | 示例 |
|---|---|---|---|
| **slash command** | 单行 + 第一字符是 `/` + 不含 `\n` `\r` | keystroke：逐字符 PTY write，最后送 Enter | `/clear`、`/new`、`/help`、`/agents` |
| **普通 prompt** | 其他所有 | bracketed paste：原子 paste + Enter | "请帮我审一下 PR #123 的 design"、长 plan、context |

### 原则 2：keystroke 投递的细节

- **逐字符或小批量**：`tmux send-keys -l "<text>"` 是 tmux 的 keystroke 模拟，背后还是逐字符发 keypress event。Rust 实现应该用 PTY write 逐字符 / 整字符串字面 write（不要包 `\x1b[200~` `\x1b[201~`）
- **Enter 单独发**：keystroke 路径的 `<Enter>` 必须独立 keypress event，不能跟 `/clear` 拼一起 `"/clear\n"`——某些 TUI 把内嵌 `\n` 当 multi-line input 而不是提交
- **slash 字符前确保 input buffer 干净**：投递前可选发 `Escape` + `Ctrl-U`（清当前 prompt 区残留），保证 `/` 是 input buffer 的第一字符——否则可能进入"非空 buffer 收 / "的分支，slash mode 同样不触发

### 原则 3：bracketed paste 投递保留

普通 prompt 内容（多行 plan / context / 长上下文）继续用 bracketed paste 路径——理由：
- atomic 一次性插入，pane 状态清晰
- 内嵌 `\n` 不会被误判为 multi-line input 提交
- `\t` `\r` 等控制字符被 paste 包装保护，不被 TUI keymap 拦截

### 原则 4：投递层不依赖外部 mapping 表

CCB 现有的 `bin/autonew` 维护一个 hardcoded `PROVIDER_COMMANDS = {"gemini": "/clear", "claude": "/new", ...}` 表（位于 `/home/sevenx/.local/share/codex-dual/bin/autonew` 第 32-38 行），这个表已知至少有两条错的（"claude": "/new" 实际应该是 "/clear"，sevenx 日常用 /clear 验证），且跟主投递层（`ccb ask`）维护的状态完全独立 — 一改动一边另一边永远不知道。

ccbd-rust 必须**只有一处** "如何让 provider 重置 session"的定义，不要 PROVIDER_COMMANDS 表 + send_text 逻辑两处分立。建议做法：每个 provider 的 trait/struct 暴露一个 `reset_session_keystroke() -> &str` 方法，投递层统一调用，永远不会失同步。

### 原则 5：测试时用真 TUI 应用，不要 mock terminal

slash command 的识别完全发生在 TUI 应用内部（Gemini CLI / Claude Code 自己的 input parser），mock 一个"假 terminal" 测试不出来。集成测试必须：
- 真起 Gemini CLI / Claude Code 进程（在 PTY 里）
- 真发 keystroke 或 paste
- 真观察 pane 输出 / 内部状态变化（slash mode 进入 vs LLM 推理触发）

ccbd-rust 的测试 fixture 可以直接 spawn `gemini --yolo` / `claude` 等 CLI 进程在 PTY 里，跑 PoC 命令，capture pane 校验。

---

## 5. ccbd-rust 实施建议（按这个顺序考虑）

### 5.1 Dispatch 模块结构（建议）
```rust
trait PaneDispatcher {
    fn send_text(&self, pane: PaneId, text: &str) -> Result<()>;
}

struct PtyDispatcher { /* PTY handle */ }

impl PaneDispatcher for PtyDispatcher {
    fn send_text(&self, pane: PaneId, text: &str) -> Result<()> {
        if is_slash_command(text) {
            self.send_keystroke(pane, text)?;
            self.send_keystroke(pane, "\r")?;  // Enter
        } else {
            self.send_bracketed_paste(pane, text)?;
            self.send_keystroke(pane, "\r")?;  // Enter
        }
        Ok(())
    }
}

fn is_slash_command(text: &str) -> bool {
    text.starts_with('/') 
        && !text.contains('\n') 
        && !text.contains('\r')
        && !text.contains(' ')   // optional: pure command, no spaces
}
```

### 5.2 PTY 层 keystroke vs paste 实现
- **keystroke**：直接 `pty_write(text.as_bytes())`，不加任何 escape 包装
- **bracketed paste**：`pty_write(b"\x1b[200~"); pty_write(text.as_bytes()); pty_write(b"\x1b[201~"); `

### 5.3 Provider trait 暴露 reset 命令（替代 PROVIDER_COMMANDS 表）
```rust
trait Provider {
    fn name(&self) -> &str;
    fn reset_session_command(&self) -> &str;  // "/clear" 或 "/new" 等
    // ...
}

struct GeminiProvider;
impl Provider for GeminiProvider {
    fn name(&self) -> &str { "gemini" }
    fn reset_session_command(&self) -> &str { "/clear" }
}

struct ClaudeProvider;
impl Provider for ClaudeProvider {
    fn name(&self) -> &str { "claude" }
    fn reset_session_command(&self) -> &str { "/clear" }  // ← 注意是 /clear，不是 /new；CCB 的 autonew 表写错了
}
```

集中在一处定义，dispatch 调 `provider.reset_session_command()` 拿命令，再走 dispatch 的 keystroke 路径送出去——表 + 投递逻辑永远同步。

### 5.4 命令名校验（开发期）
开机时（或测试 fixture）真实 spawn 各 provider，发其 `reset_session_command()`，capture pane 校验是否真清屏（不是被当 prompt）。任何一个 provider 校验失败就 panic — 开发期立刻发现 PROVIDER_COMMANDS 表和实际行为不一致的问题。

---

## 6. 测试用例建议（这些场景 ccbd-rust 必须测）

| # | 场景 | 期望 |
|---|---|---|
| 1 | 给 Gemini pane 发 `/clear` | pane 清屏 + 显示 `Gemini CLI v...` 启动画面，**不能**进入 LLM 推理 |
| 2 | 给 Claude pane 发 `/clear` | pane 清屏 + 显示 Claude Code 欢迎画面，**不能**进入 LLM 推理 |
| 3 | 给 Codex pane 发 `/new` | pane 重置 session（具体行为待 codex 文档），**不能**当 prompt 处理 |
| 4 | 给 Gemini pane 发 `/help` | 显示 slash command 帮助，**不能**进入 LLM 推理 |
| 5 | 给任意 pane 发 "请审一下 PR #123 的 design"（普通 prompt） | LLM 推理 + 输出 review 内容（这个就是要 paste 路径） |
| 6 | 给任意 pane 发以 `/` 开头但**多行**的内容（`/some/path/and\nthen more text`） | 走 paste 路径（不是 slash command） |
| 7 | input buffer 已有半截输入（`hello`）时再发 `/clear` | slash command **不该** 触发（input buffer 非空），应该被当成 paste 处理；或者 dispatch 应该先发 `Ctrl-U` 清 buffer 再发 `/clear` keystroke |
| 8 | 长 thinking 时发 slash command | 应该 queue 到当前 turn 结束 + ccbd 状态机正确反映"当前 turn 还在跑、新 turn pending" |

测试 1-4 直接覆盖 Bug X 的回归。测试 5-6 防止"slash 检测过严"误把普通 prompt 路由错。测试 7 触及 provider 实现细节。测试 8 跟 completion detection (Bug Y) 联合测，防止两个 bug 复合。

---

## 7. 跟其他设计输入的关联

### 7.1 跟 `synthesis-18-days-by-claude.md` 群 A
synthesis 里第 33 行记录 "tmux 投递成功但没自动 Enter，pane 卡 `[Pasted Text: 69 lines]`" — 这是 Gemini 早期版本看到 bracketed paste 后**显示了 `[Pasted Text: ...]` 提示但没自动提交**（要按 Enter）。当时的 fix 是 send Enter 后再发一次 Enter（`CCB_TMUX_SECOND_ENTER_DELAY`），但**根因（paste 路径不能传 slash command）从来没被理解或修过**。本文档算是对那次教训的延续 + 真正定位根因。

### 7.2 跟 synthesis 第 168 行 "provider-aware completion detection"
那条建议是 detection 端的（多信号融合 + deadline）；本条是 dispatch 端的（投递路径区分 slash vs paste）。两者都属于"dispatch + completion 状态机重新设计"的工作范畴，但应该分开实现（一个是发送方向，一个是接收方向）。

### 7.3 跟 ccbd 自己的 reception-driven retry
现 CCB 在 `_paste_via_buffer_reception_driven` 里有"看 reception 文件 + pane agent activity 双信号"的 D3 invariant 设计——这是检测投递是否真到达。但 D3 不能解决"到达了但被当 prompt 处理"这种语义错误（投递成功 ≠ 命令被识别）。ccbd-rust 必须做更细：**投递路径选错（paste 给了 slash）本身就是 dispatch 层的语义 bug**，不是后面 reception detect 能补救的。

---

## 8. 不要做的事

1. **不要试图通过"教 Gemini 识别 paste 来的 slash"绕过这个原则**——上游应用的 slash detection 逻辑不会因为 ccbd-rust 改而改。强制 dispatch 层适配上游协议是**设计约束**，不是 dispatch 层的"缺陷"
2. **不要把"普通 prompt 也走 keystroke"作为 lazy 简化**——长 prompt（10KB+ context）keystroke 投递会非常慢（每字符一次 syscall），还会撞上 PTY buffer 满 / 应用 input loop 跟不上等问题。bracketed paste 是普通内容的正确路径
3. **不要在 dispatch 层之上再加一层"command alias 表"**——重蹈 CCB autonew PROVIDER_COMMANDS 表的覆辙。reset 命令应该是每个 Provider trait 的 owned 属性，dispatch 只调 `provider.reset_session_command()`
4. **不要让"测试用 mock terminal"成为常态**——slash detection 是 TUI 应用 input loop 内部行为，mock terminal 永远测不出 keystroke vs paste 的差异。集成测试必须用真 PTY + 真 spawn 的 provider 进程

---

## 9. 跟 Bug 报告的边界

本文档是 ccbd-rust 的**前置设计输入**。跟 `docs/upstream-ccb-bugs/gemini-dispatch-and-completion-bugs.md` 的边界：
- bug 报告：CCB（Python，v6.0.7）当前实现的具体 bug 现场 + 修复方向（在 CCB 源码里改）
- 本文档：ccbd-rust 重写时**从一开始**就要避开这个陷阱，不要再写一遍同样的错

两份文档相互引用，重写者读 bug 报告知道"CCB 现在错在哪、为什么"，读本文档知道"ccbd-rust 应该怎么做"。
