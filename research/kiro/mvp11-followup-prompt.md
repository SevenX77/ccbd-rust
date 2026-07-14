# MVP 12 — Python ccb 1:1 翻译续作 prompt

> 复制粘贴整段作为新 master Claude 的开场 prompt 即可。
> 本 prompt 替代旧版 "mvp11 hot fix 续作"——上一轮 Gemini 自评诚认 mvp11 设计走偏（"打着复刻旗号的拙劣发明"），整体方向纠偏为"架构降级到 Python ccb 稳定性"。

---

## 任务

让 `ccbd-rust` 在 `/home/sevenx/coding/ccbd-rust` **真正能替代 Python ccb**：默认 sandbox 模式下 `ccb-rs start` 拉起 a1=codex / a2=gemini / a3=claude 三个真 binary（订阅认证），全部到 IDLE，能 `ccb-rs ask` 发任务 + 收回 reply，能 cancel，能 kill --session 干净收尾。**这就是终点。**

跑通之前不要找用户。设计选型问 Gemini，实施修复测试找 Codex，自己实证。

## 核心方向：1:1 翻译，不发明

**这是本轮跟之前所有 mvp 的根本区别**。

Python ccb 在 `~/.local/share/codex-dual/lib/` 是**已经稳定运行的生产级实现**——用户每天用它跟 codex/gemini/claude 协作。理论上 Rust 重写**应该 1:1 复刻 Python 行为**，跑通的功能就该跑通。

但 mvp11 R/D（Gemini 起草的）实际做的是 **架构发明**：发明了 `StartupSequenceEngine + SendKeysVerified + InteractivePromptInterceptor + retry_fallback_keys + max_triggers` 这一套"模拟用户敲键盘"的机制，**没去看 Python ccb 实际怎么做的**。结果实测发现 6 个基础功能 bug——全部源于 mvp11 设计走偏。

Gemini 自评原话（commit 里有完整 Q1-Q5 自评，参考 git log 上下文 + 本 prompt 同目录的 mvp11-postmortem-design-deviation.md 如果存在）：

> "我不应该试图在 Rust 里'做得更好'，而应该先在 Rust 里'做得一样'。MVP12 的目标不应是功能叠加，而应该是**架构降级（向 Python 稳定性降级）**"

**本轮所有派活铁律**：

1. 任何 Codex 实施任务，**brief 必须包含**："先 read Python ccb 对应路径的源码（提供 file 路径），grep 出关键函数 + 数据结构 + 控制流。然后**逐行翻译到 Rust**，不允许凭直觉发明新机制。"
2. 任何 Gemini 设计任务，**brief 必须包含**："给出方案前必须先 read Python ccb 对应实现，方案中每个字段 / 状态 / 流程必须能映射回 Python 某个 file:line。"
3. master Claude 自己 review Codex 提交的 diff 时，**必须 grep verify**：所有新增 Rust 字段在 Python ccb 里能找到来源；不能 grep 到来源的 = 发明，要求重做。

## 当前状态

git HEAD = `fa6712b` (含 mvp11 R/D/T 立项 + G11.-1~G11.3 实施 + bwrap auth mount fix)。

**保留的 mvp11 资产**：
- master_pid 拔除路径（G11.-1）✅ 跟 Python ccb 一致（Python 也无 master_pid 概念）
- systemd anchor + agent.scope BindsTo session anchor（G11.0 + G11.1 R-6.4）✅ 这是 Rust 特有的内核级 detach，Python 没有但是好东西
- ProviderManifest 升级（env_passthrough / injected_env_vars 字段 + 50+ env 白名单）✅ 跟 Python control_plane.py 一致
- bwrap auth mount path translation（fa6712b）✅ 跟 Python home.py 思路一致
- DB CAS cascade idempotency（G11.0 T0.2.1）✅
- 8 维 rubrics（G11.3 T3.4）✅

**走偏待重写的 mvp11 部分**：
- `src/marker/startup_engine.rs` (654 行 SendKeysVerified + ClearLine + InteractivePromptInterceptor) — Gemini 凭直觉发明的"模拟键盘交互"，Python 不这么做。**整个文件可能要废弃或退化为 fallback**。
- ProviderManifest 里 `startup_sequence: &[StartupStep]` + `interactive_prompt_handlers: &[PromptHandler]` 字段 — Gemini 发明的字段，Python 没有等价物。
- `src/provider/manifest.rs` 里 marker_pattern 字面值（gemini/claude）— 凭"应该长这样"猜的，**没去 Python init_probe.py 实证**。
- ccb-rs ask 的 send 路径里 Enter 触发判断错误（Python 总是补 Enter）。
- ccb-rs ask 的 reply 收回路径（reader.rs 缺静默超时触发器）。
- grid layout 下 agent_id ↔ pane_id binding 完全没实装（mvp11 只调 `select-layout tiled` 没做 Grid2x2 序号映射）。
- SessionWatch reconcile-on-restart TODO（G11.0 留的，没做）。

## Gemini Q1 对照表（Python ccb 真做法 vs mvp11 走偏）

| Bug 编号 | Python ccb 实现 (file:line) | mvp11 走偏点 | 翻译方向 |
|---|---|---|---|
| 1 | `terminal_runtime/tmux_send.py:137` Python 总是补 Enter | mvp11 `writer.rs` 凭 `text.ends_with('\n')` 按需补错了 | Rust 翻译为"无条件补 Enter" |
| 2 | `ccbd/services/dispatcher.py` 用明确 Job 完成信号 + `message_bureau` 异步推送 | mvp11 `reader.rs` 自创稳定性检测无静默超时 | Rust 翻译 dispatcher 完成信号机制 |
| 3 | `launcher/sandbox_home.py` + `__init__.py` Grid2x2 等布局明确序号绑定 | mvp11 `apply_layout` 只 `select-layout tiled` 没做序号映射 | Rust 翻译 Grid 映射到 Agent→Pane 确定性绑定 |
| 4 | `init_probe.py` 用 S1-S3 分步状态机检测 readiness（不是单一正则） | mvp11 D §2.1 硬编码 `(?m)^❯\s*$` / `Type your message...` 凭空猜 | Rust 翻译 S1-S3 状态机替代单一 regex |
| 5 | `gemini/launcher_runtime/home.py:109` `_materialize_trusted_folders` 把 host 路径翻译注入 sandbox `trustedFolders.json` | mvp11 完全漏 "Provider 内部配置文件路径翻译"层 | Rust 翻译 `_materialize_*` 完整 home_layout 模块 |
| 6 | `workspace/reconcile.py:358` 明确重装 SessionWatch | mvp11 D §4.1 写了但 src/db/system.rs:200 留 TODO | Rust 补回 reconcile 路径 |

## 工作流（铁律）

1. **设计/方案选型** → `command ccb ask --wait --timeout 600 a2 ...`（Gemini 必须先 read Python 源码再设计）
2. **代码实施 + 测试** → `command ccb ask --wait --timeout 600 a1 ...`（Codex 必须先 read Python 对照源码再写 Rust）
3. **filesystem 实证 + Python 对照** → master Claude 自己跑：
   - `git diff` `cargo test --lib` `cargo test --tests` 验证代码没问题
   - **grep Python ccb verify Rust 新增的每个字段/函数能映射回 Python 某个 file:line**——找不到 = 发明 = 重做
   - `ccb-rs start` + `tmux capture-pane` 真 spawn 实测
4. **不找用户** — 设计选型 / 卡点 / 方向疑问全部先问 Gemini。Gemini 给不出 Python ccb 来源的方案 = 拒绝接受。

## 工作分阶段（建议）

### Stage M12.0 — Python ccb 行为映射文档（设计先行）

派 Gemini：read Python ccb 关键路径，列出**完整逐行行为映射表**：

- `terminal_runtime/tmux_send.py` (321 行) — send_keys + verify_send + Enter 补发逻辑
- `ccbd/keeper_runtime/loop.py` (265 行) — keeper main loop
- `ccbd/services/dispatcher.py` — Job 调度 + 完成信号 + message_bureau
- `provider_core/init_gate.py` (357 行) — readiness probe + S1-S3 状态机
- `provider_core/subcgroup.py` (333 行) — agent 子 cgroup 隔离
- `provider_backends/{codex,claude,gemini}/launcher_runtime/home.py` — home_layout materialize
- `provider_backends/{codex,claude,gemini}/launcher_runtime/command_runtime/` — 启动命令构造
- `provider_backends/{codex,claude,gemini}/execution_runtime/` — provider 执行循环
- `launcher/sandbox_home.py` + `launcher/__init__.py` — Grid2x2 等布局
- `workspace/reconcile.py` (358 行) — daemon 重启 reconcile

输出：写到 `research/kiro/mvp12-R.md` + `mvp12-D.md` —— 每个 Rust 模块都明确"对应 Python 哪几个 file:line + 关键函数签名 + 控制流"。

### Stage M12.1 — home_layout materialize 模块（Bug 5 修复）

派 Codex 实施 `src/provider/home_layout.rs`（新建）：

参考 Python `provider_backends/{codex,claude,gemini}/launcher_runtime/home.py` 的 `_materialize_trust` / `_materialize_trusted_folders` / `prepare_*_home_overrides` / `resolve_*_home_layout`，**逐行翻译**到 Rust。

关键功能：
- 启动 agent 时，根据 sandbox 内的 cwd 路径，**重写**复制到 sandbox 的 trust 配置文件（codex `config.toml` 的 `[projects."<sandbox_path>"] trust_level="trusted"` / gemini `trustedFolders.json` 加 sandbox 路径 / claude `.claude.json` 的 trust 字段）
- bwrap mount 时把 sandbox HOME 内对应路径 mount overlay（rw 临时层 + ro 底层）

### Stage M12.2 — Send/Reply 路径翻译（Bug 1+2 修复）

派 Codex：**整体重写** `src/agent_io/writer.rs` 的 send 路径 + `src/agent_io/reader.rs` 的 reply 路径，对照 Python `terminal_runtime/tmux_send.py` + `ccbd/services/dispatcher.py` + `ccbd/keeper_runtime/loop.py`。

关键功能：
- send: prompt 文本走 paste-buffer + 必补 Enter（无条件，跟 Python 一致）+ verify_send_succeeded
- reply: dispatcher 模式（明确 Job 完成信号），不是猜稳定性

### Stage M12.3 — Grid layout 序号绑定（Bug 3 修复）

派 Codex：对照 Python `launcher/__init__.py` 的 Grid2x2 实现，把 `src/tmux/layout.rs::apply_layout` 改造成 Agent→Pane 确定性映射（按 ccb.toml 配的 a1/a2/a3 顺序绑到固定 pane 序号）。

### Stage M12.4 — init_probe S1-S3 状态机（Bug 4 修复）

派 Codex：对照 Python `provider_core/init_gate.py` 的状态机，改造或废弃 mvp11 G11.2 的 startup_engine.rs。**优先选废弃**，写新模块 `src/provider/init_probe.rs` 跟 Python 对应。manifest 里 marker_pattern 字面值废弃，改用 init_probe 的状态机判定。

### Stage M12.5 — reconcile 重装 SessionWatch（Bug 6 修复）

派 Codex：对照 Python `workspace/reconcile.py:358`，补回 mvp11 G11.0 留的 TODO 路径——daemon 重启时扫 DB ACTIVE session 重 attach SessionWatch。

### Stage M12.6 — 端到端真实测验收

(同下文 "实测验收清单")

## 实测验收清单（mvp12 完成判据）

```bash
cd /home/sevenx/coding/ccbd-rust
pkill -KILL -f 'target/release/ccbd' 2>/dev/null || true
rm -rf .ccb-rs
cargo build --release --bins

# Step 1: 默认 sandbox 模式 start (不带 NO_SANDBOX)
ccb-rs start --wait
# 期望: 三 agent 全 IDLE, 不 timeout

# Step 2: ps 验状态
ccb-rs ps
# 期望: a1=codex IDLE, a2=gemini IDLE, a3=claude IDLE
# 关键: agent_id ↔ provider 对得上 ccb.toml

# Step 3: 三 agent 各发真 ask
ccb-rs ask --wait --timeout 60 a1 "echo from codex"
ccb-rs ask --wait --timeout 60 a2 "echo from gemini"
ccb-rs ask --wait --timeout 60 a3 "echo from claude"
# 期望: 三个都返回 COMPLETED + reply 含 "echo from XXX"
# 必须自动化 (不手动 tmux send-keys)

# Step 4: cancel 中途任务
ccb-rs ask a1 "sleep 30 then echo done"  # 异步发，不 wait
JOB_ID=$(... 抓 job_id)
ccb-rs cancel $JOB_ID
# 期望: job CANCELLED + agent 恢复 IDLE

# Step 5: kill --session 收尾
SESSION_ID=$(ccb-rs ps | awk '/sess_/{print $2}' | head -1)
ccb-rs kill $SESSION_ID --session
# 期望: anchor unit stopped, agent 内核级被杀, ps 显示 0 active_agents

# Step 6: 跨 daemon 重启 detach 验证 (Bug 6 验收)
ccb-rs start
DAEMON_PID=$(pgrep -f 'target/release/ccbd')
kill -9 $DAEMON_PID
sleep 3
ccb-rs ps  # 触发 daemon 自动重起
# 期望: anchor unit 仍 active, agent 仍存活, ps 列出原 session
```

**全部 6 步真跑通 = mvp12 完成 = 用户问题真解决**。每步失败回去修，Gemini 设计 Codex 实施 master 实证 闭环，不达标不要找用户。

## 调试套路速查

- **filesystem 实证**: `git diff` / `cargo test --lib` / `cargo test --tests` / `wc -l file.rs`
- **Python 对照证据**: `grep -rn '<keyword>' ~/.local/share/codex-dual/lib/<path>/`——所有 Rust 新增字段必须能 grep 到 Python 来源
- **真 spawn 看 pane**: `tmux -L ccbd-<short> list-panes -a` 然后 `capture-pane -t %N -p -S -50`
- **daemon log**: `tail -50 .ccb-rs/ccbd-rs.log`
- **NO_SANDBOX bypass** (debug 用，不是验收): `CCBD_UNSAFE_NO_SANDBOX=1 ccb-rs start`

## 关键文件路径

- 设计/任务: `research/kiro/mvp11-{R,D,T}.md`（mvp11 已 commit，作为基础）
- mvp12 R/D/T: 还要写（Stage M12.0 时让 Gemini 起草到 `research/kiro/mvp12-{R,D,T}.md`）
- postmortem: `docs/postmortems/mvp10-real-world-parity-gap.md`
- 8 维 rubrics: `docs/rubrics.md`
- ccb.toml 配置（已就绪）: `./ccb.toml`

## 调用 Gemini/Codex 模板

```bash
# Gemini 设计 — brief 必须强制看 Python
command ccb ask --wait --timeout 600 a2 <<'EOF'
请用中文回答。

# 任务背景
[简明描述]

# 强制要求 (mvp12 铁律)
你给出任何方案前，**必须先 read Python ccb 对应路径**:
- ~/.local/share/codex-dual/lib/<path>

方案中每个字段 / 状态 / 流程必须**能映射回 Python 某个 file:line**。
不允许"凭直觉"或"应该长这样"——这是 mvp11 走偏的根因。

# 你要决断的具体问题
[具体问题]

# 边界
- ✅ 允许 read_file / grep
- ❌ 禁止改任何文件 / 派任务给其他 agent / commit

直接 reply 文本回我。
EOF

# Codex 实施 — brief 必须强制对照 Python 源码
command ccb ask --wait --timeout 600 a1 <<'EOF'
You are implementing fix for [Bug N], following mvp12 1:1 translation policy.

# Mandatory pre-implementation step (mvp12 rule)
Before writing any Rust code, **read Python ccb source**:
- ~/.local/share/codex-dual/lib/<path>

Your Rust implementation must be a line-by-line translation of Python behavior.
**No invention.** If a Rust struct/function has no Python counterpart, that's a red flag — flag it back to me.

# Specific change
[Gemini 选型 + 具体改动方案 + Python file:line 引用]

# Files allowed
- src/...

# Acceptance
1. cargo build clean
2. cargo test --lib (≥ baseline)
3. specific manual test: ...
4. **Python parity check**: 报告里列出每个新增 Rust 字段对应的 Python file:line

# Boundaries
- ❌ DO NOT touch other modules
- ❌ DO NOT commit
- ❌ DO NOT dispatch ccb ask

Report: files changed + test result + Python parity table + any blocker. Stop after reporting.
EOF
```

## 完成定义

在 master Claude 自己的 shell 里跑完上面"实测验收清单"6 步全过 → ccb-rs 默认 sandbox 模式真能跑通三 agent + ask/cancel/kill 全流程 → mvp12 真完成 → 整理 + 给个完成提交（建议 `feat(mvp12): finalize Python ccb 1:1 parity, all live AC verified`）→ 然后才向用户报告。

不达标不要找用户。每个 stage 都先 Gemini 设计（强制 Python 对照）→ Codex 实施（强制翻译不发明）→ 自己实测 → 不过就再 round → 直到通过。
