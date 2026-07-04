# 任务：端到端验证 ah 的「ah tell master + master 可观测」功能

## 背景
`ah`（Agent Hypervisor）刚合入一个新功能（dev 仓 SevenX77/ccbd-rust 的 main，提交信息含 "master tell observability"）：
- 新命令 `ah tell master "<text>"` —— 异步把指令投递到 master 的 tmux pane，**投完立即返回、不阻塞**。
- 给 master 装了两个 hook：**UserPromptSubmit → master 状态置 BUSY**（真正的"开工"信号，不是靠"投递成功"）、**Stop → 置 IDLE**（完成）。
- 两个事件都写进 ahd 日志。`ah ps` 会显示 `master_state`。

你的任务：在这台机器（WSL2 Ubuntu）上**真跑一遍**，证明它端到端 work。**只信实测证据，不要凭猜就断言 PASS —— 每条结论必须贴出实际命令输出。**

## 前置（先逐项确认，缺哪个先补或如实报告，别硬跑）
- WSL2 Ubuntu shell；`git`、`tmux`、`cargo`(Rust toolchain) 可用。
- `claude` CLI 已安装且**已登录**（这台机器的独立 claude 账号）。确认 `claude --version` 正常、能进对话。
- 有网络。

## 步骤
1. 拉源码并确认功能在内：
   ```bash
   git clone https://github.com/SevenX77/ccbd-rust.git
   cd ccbd-rust
   git log --oneline -8 | grep -i "master tell"   # 必须能看到，否则停下报告
   ```
2. 串行编译（**必须带 `CARGO_BUILD_JOBS=1` 防 OOM**）：
   ```bash
   CARGO_BUILD_JOBS=1 cargo build --release
   ```
   产出 `./target/release/ah` 和 `./target/release/ahd`。
3. 造一个**只有 master、无 agent** 的最小配置 `e2e.toml`（只需 claude，不拉 codex/antigravity）：
   ```toml
   version = "1"
   [master]
   cmd = "claude"
   enabled = true
   [completion]
   hook_push_enabled = true
   hook_push_events = ["stop", "userpromptsubmit"]
   hook_push_providers = ["claude"]
   ```
   ⚠️ 若不确定 UserPromptSubmit 在配置里到底怎么开、或 master hook 是否自动安装：**读源码确认后再配对**——`src/provider/home_layout.rs`（hook 装配，看 master 模式怎么 push UserPromptSubmit）、`src/rpc/handlers/agent.rs`（`handle_agent_notify` 的 master/worker 分流）、`src/rpc/handlers/sessions.rs`。按代码实际行为把配置调对，别照抄。
4. 用**隔离的 state 目录**起栈（绝不污染任何现有 ah）：
   ```bash
   export AH_STATE_DIR="$PWD/.e2e-state"
   ./target/release/ah start --config e2e.toml --wait
   ./target/release/ah ps
   ```
   等到 `ah ps` 显示 master 存在且 `master_state=IDLE`（master 的 claude 已就绪）。
   - 若 master 卡在 claude 首启的主题向导/onboarding：`./target/release/ah attach master`（或 `tmux` attach 到 master pane）手动过掉，再回来。
   - 若 `ah start` 报"至少需要一个 agent"之类：先停下报告（别乱加 agent），我再给你改法。
5. **核心验证 —— 发指令 + 观测状态翻转**：
   ```bash
   ./target/release/ah tell master "严格只回复一个词：PONG，不要做别的"
   ```
   立即记录：命令是否**马上返回**（不 hang）？返回文案是什么（成功登记 / DELIVERY_FAILED_UNCONFIRMED / 其它）？
   然后**快速轮询**（每 1~2 秒跑一次）`./target/release/ah ps`，记录 `master_state` 序列。期望：`IDLE → BUSY → IDLE`（master 收到→开工→回完 PONG）。
6. **核 ahd 日志的两个事件**（结构化、可 grep）：
   ```bash
   ./target/release/ah logs | grep -iE "userpromptsubmit|stop|master_state|ignored_stale|master:"
   ```
   若 `ah logs` 里没有，去 `$AH_STATE_DIR` 下找 ahd 日志文件再 grep。期望：一条 **UserPromptSubmit**（role=master，置 BUSY）+ 一条 **Stop**（置 IDLE），带 `master:<session_id>:<generation>` sentinel 和结构化字段。
7. 确认 master pane 真收到并回复：`ah attach master` 或 tmux capture master pane，看到你发的指令 + master 回了 `PONG`。

## 判定与报告（每条都要贴实测证据，别只写"通过"）
- [ ] `ah tell master` 立即返回、不阻塞 → 贴返回内容
- [ ] `master_state` 走了 `IDLE → BUSY → IDLE` → 贴 `ah ps` 的 master_state 序列
- [ ] ahd 日志有 UserPromptSubmit + Stop 两个事件、带 master sentinel + generation → 贴日志行
- [ ] master pane 真收到指令并回复 PONG → 贴 pane 内容

任一不符 → **如实报告实际现象** + 你能拿到的错误/日志原文，别粉饰、别脑补成功。

## 收尾
```bash
./target/release/ah stop
rm -rf "$AH_STATE_DIR"
```
把上面 4 条判定各自的证据整理成一份结果发出来。
