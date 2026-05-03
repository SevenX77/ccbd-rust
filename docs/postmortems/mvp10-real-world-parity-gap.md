# Postmortem: ccbd-rust MVP 10 完成后无法替代 Python ccb

**日期**: 2026-05-03
**触发事件**: 用户首次切换到 ccb-rs 部署 → ccb-rs start 拉起 a1=codex / a2=gemini / a3=claude 后三个 provider 全部 state=KILLED → 回滚到 Python ccb
**调查方式**: 用户要求 Gemini 彻查根因；本文是 Gemini retrospective + 主控 Claude verify 后的合并备忘

---

## 1. 用户原话

> "都已经做到 mvp10 了，到现在基本功能都没有实现？正常逻辑 ccb 有什么功能，ccbd-rust 就应该像素级复刻功能，怎么会到现在还在这种程度，要彻底调查"

---

## 2. 现场事实

ccb-rs daemon 启动 OK，session 创建 OK，spawn agent RPC 也返回 pid 了：

```
session_id=sess_918b283a-54e4-413f-b9da-244069f91389 layout=grid
agent_id=a1 provider=codex pid=2834625
agent_id=a2 provider=gemini pid=2834647
agent_id=a3 provider=claude pid=2834694
```

但 `ccb-rs ps` 立刻显示三个 agent state=KILLED，daemon log:

```
agent_id=a1 spawn → pid 2834625 → 0.x秒后
WARN agent_watch: agent pidfd is ready but exit code is unavailable
```

tmux server 里 ccbd-agents 这个 session 的 a1/a2/a3 pane 全部不存在（pane 跟着进程死直接关）。

回滚到 Python ccb 后才发现 codex 启动时弹了 "Update now / Skip" 交互提示——但**这只是表面现象**，下面是真根因。

---

## 3. Gemini 找到的两个致命伤（已 verify）

### 致命伤 A — Master PID 生命周期模型从根上错了

**代码定位**: `src/bin/ccb.rs:202`

```rust
master_pid: std::process::id() as i64,  // ← ccb start 自己的 CLI 进程 PID
```

而 daemon 端 (`src/rpc/handlers.rs:73`) 起 `master_pidfd_watch_task` 监听这个 PID 死亡。**`ccb start` 是瞬时 CLI**——spawn agent 完立刻 exit，PID 立即死，daemon 立刻 `cascade_kill_session_agents` (`src/rpc/handlers.rs:89`) 把全部 agent 杀光。

**这就是 a1/a2/a3 立刻死的真根因**。codex 升级提示卡 pane 是次要的，即便没有那个提示，agent 也会被 master_watch 杀掉——只要 ccb start 退出。

设计者对 master_pid 语义的误解：以为 ccb start 是长驻进程，或没理解 master_pid 的生命周期绑定意图。

### 致命伤 B — Provider 适配层"真空化"

**代码定位**: `src/provider/manifest.rs`

ccbd-rust 整个 codex provider 配置：

```rust
{
    provider_name: "codex",
    command: &["codex", "--dangerously-bypass-approvals-and-sandbox"],
    // ... 几行 env / auth 路径
}
```

整个 manifest.rs **大约 100 行**。

Python ccb 同等功能（`/home/sevenx/.local/share/codex-dual/lib/`）合计约 **2000+ 行**：

| Python ccb 做了 | ccbd-rust 做了 |
|---|---|
| 50+ 个环境变量透传 (`ANTHROPIC_*`/`GOOGLE_*`/`OPENAI_*`/`CCB_TMUX_ENTER_DELAY=2.0`/`CCB_CLAUDE_READY_TIMEOUT_S=60`/...) | 2-3 个 env var |
| Spawn 后等 TUI 稳定 + readiness probe + 启动 Marker 检测 | 启动后只等正则匹配 |
| 模拟敲 Enter / SecondEnter 跳过启动 prompt（升级提示 / auth / yolo toggle） | 完全没有"启动后处理"概念 |
| Escape + C-u 重试清理逻辑 | 无 |
| Verification 确认送达 + Keeper 存活心跳 | 无 |
| Provider 模式调教 (`CCB_CLAUDE_MD_MODE=route` 等) | 无 |

**复杂度差距 10-20 倍**。这不叫重写 ccb，叫**功能阉割版 tmux 包壳**。

---

## 4. 工程纪律根因（Gemini Q4）

### a) AC 抽象化陷阱

mvp R 文档第 0 节口口声声"达到并超越旧版"，但 R-* 矩阵和 AC 落地为"功能点"而不是"语义完整"：

- mvp7 AC4: "静态配置表"（仅要求 manifest 解析正确）
- mvp9 AC1: "spawn 多 agent" （仅要求 RPC 返回 pid，不要求 agent 真活到能干活）
- mvp10 AC1-7: cgroup binding / graceful shutdown / 测试 harness ——全部跟 provider parity 无关

**没有任何一条 AC** 写了"用真实 codex CLI 在干净沙盒里完成 spawn → 跳过 update prompt → ask → reply 全流程"。

### b) Dogfooding 缺失

开发 ccbd-rust 整个期间，用户自己**仍然在用 Python ccb 干活**（agent-harness 那个 keeper_main.py 跑了 1+ 天没切换）。ccbd-rust 从来没真在生产环境替代过 Python——所以"现实根本起不来"这个事实直到 mvp10 收尾才被发现。

### c) 自验证循环陷阱

Codex 实施 + Codex Round 1/2/3 review + Codex 写的 acceptance test。逻辑自洽但跟外部真实 provider 行为完全脱节。

### d) 测试清一色 fake provider

| MVP | acceptance test 用的 provider |
|---|---|
| mvp6/7/8/9 | bash provider |
| mvp10 真实环境测试 | `ccbd_test_helper`（自己 spawn TmuxServer + sleep forever 的 mock） |
| 跨 mvp1-10 全部 acceptance | **没有一处**真用 codex / gemini / claude 跑通端到端 |

唯一"真 codex"的脚本是 `scripts/ac_mvp8_real_codex.sh`，但它在 `CCBD_UNSAFE_NO_SANDBOX=1` 模式下跑 + 测试机的 codex 恰好那次没触发 update prompt → 假绿。

### e) Plan review rubrics 缺 parity 维度

mvp1-10 用的 rubrics 7 维度（spec_fidelity / carve_out_clarity / architecture_consistency / pseudocode_rigor / task_atomicity / ac_traceability / risk_coverage）**没有任何一个**评估"是否真覆盖 Python ccb 的等价行为"。所以 Codex review 自己也注意不到——rubric 没要求他注意。

---

## 5. 修复路径（必须立项 mvp11，不是补丁）

按 Gemini Q5 建议：

### R-1 重构 Launcher 生命周期

废弃 `master_pid: std::process::id()` 这条设计。可选方案：

- **A**: `session.detach` 语义——ccb start 提交 spawn 请求后 master_pid 字段填一个**长驻锚**（如 systemd user service unit name），daemon 改用 systemctl 状态查询而非 pidfd_watch
- **B**: ccb start 内部 fork 一个长驻 sentinel 进程（detach 父，fork 子），sentinel PID 作 master_pid——ccb start 退出但 sentinel 长活
- **C**: 让 ccb start 默认 attach 到 tmux session（用户在 tmux client 内），ccb start 进程跟 tmux client 生命周期绑定

哪条都需要重新评估 `cascade_kill_session_agents` 的触发条件。

### R-2 升级 ProviderManifest 协议

新格式应包含：

```rust
struct ProviderManifest {
    name: &str,
    command: &[&str],
    env_passthrough: &[&str],         // 全量透传 ANTHROPIC_* / OPENAI_* / GOOGLE_* / CCB_*
    startup_sequence: &[StartupStep], // wait_ms / send_keys / probe_marker
    readiness_timeout_s: u32,
    interactive_prompt_handlers: &[(regex, response)],  // codex update prompt / yolo toggle / auth 等
}
```

跟 Python `terminal_runtime/control_plane.py` + `provider_backends/` 1:1 对照。

### R-3 强制性 Parity AC

mvp11 起每条 AC 必须包含：

> "用真实的 [codex / gemini / claude] CLI 在 fresh sandbox 下完成 spawn → 跳过启动 prompt → 处理 N 个真实 ask → reply 完整 → idle 转回，**不得人为干预交互**"

且每个 provider 都要单独有这条 AC——不允许"用 bash 代表"。

### R-4 R 文档强制章节 "Python ccb behavior mapping"

每个 mvp-R.md 必须有专节，逐行对照 Python ccb 在该范围内做了什么 + Rust 侧怎么对应。验收时 reviewer 必须 grep Python 源码确认对照表完整。

### R-5 plan review rubrics 增加 parity 维度

rubrics 7 维度扩展到 8 维度，新增：

- **`real_provider_parity`**: 是否覆盖 Python ccb 在该 MVP 范围内的所有等价行为？测试是否真用真实 provider 跑通？

这维度 score ≤ 5 自动 verdict=FAIL，无论其他维度多高。

### R-6 mvp1-10 部分返工

不是全部重做，但下面 stage 的核心交付物要重新审视：

- mvp7 G7.1 manifest 设计 → 重做协议（按 R-2）
- mvp9 G9.0 launcher master_pid 模型 → 重做（按 R-1）
- mvp7-10 acceptance test → 至少加一个真 codex/gemini/claude 端到端测试覆盖

mvp1-6 的 cgroup / DB / state machine / sandbox 等基础设施层不动（这块 ccbd-rust 的 cgroup 治理设计还是好的）。

---

## 6. 流程教训

### 不能依赖单一 reviewer 视角

mvp1-10 全程 Codex 实施 + Codex review。即便 Gemini 在 mvp10 做了一轮 plan review 也漏了"测试用 bash 不是真 provider"这条——因为没人主动检查 acceptance test 用的是不是真 provider。

未来 plan review 至少要：
- 一轮 Codex rubrics（pseudocode rigor / 实施细节）
- 一轮 Gemini 架构纪律 retrospective（是不是补丁式 / 是不是覆盖业务语义）
- **一轮强制 dogfooding check**：跑一遍真实部署看是否能替代旧实现

### 不能在 fake provider 测试上签 PASS

测试用 bash / mock helper 是单元测试合理，但**acceptance test 不能用 mock** —— acceptance 的语义就是"用户真用得上"。mvp7-10 的 mvpN_acceptance.rs 全部用 mock 是 anti-pattern。

### 不能用 Codex 自审

Codex review Codex 实施 = self-validating loop。没有外部视角的 Codex review 充其量是 lint / type check，不是真的 review。

### "像素级复刻"必须落地为 AC

文档第 0 节的口号 ≠ 验收。如果某个语义重要，它必须以 AC 形式出现在 R-* 矩阵里，且必须有对应的真实测试覆盖。

---

## 7. Evidence 留档

- `.ccb-rs.aborted-1777824413/` — ccb-rs daemon debug log + sandbox 残留 + sqlite state（state=KILLED 记录现场）
- `~/.local/share/codex-dual/lib/` — Python ccb 完整实现，作为 mvp11 R-2 manifest 协议返工的对照源
- `src/bin/ccb.rs:202` — master_pid 误用代码定位
- `src/provider/manifest.rs` — provider 真空层代码定位
- `src/rpc/handlers.rs:55-89` — master_pid → master_watch → cascade_kill 这条致命链路

---

## 8. 给后续 mvp11 立项的 prompt

见 commit message 末尾或本仓 `docs/postmortems/mvp10-real-world-parity-gap.md` 的 §8。下次开 master Claude 用以下 prompt 进入 mvp11 R 立项：

> 见单独 prompt（用户保管）。
