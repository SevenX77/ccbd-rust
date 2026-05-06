# mvp13 followup: outstanding bugs & deferred work

> **Status (2026-05-05)**: mvp12 plan 内 6 个 stage (M12.0-M12.5) 全部实施完毕 (cargo test --lib 195 passed, 0 FAILED 全 suite with `CCB_TEST_SKIP_REAL_PROVIDER=1`)。M12.6 端到端验收过程中发现真实 production gap，分两类: (a) mvp12 设计/实施层 bug 已 fix；(b) 超出 mvp12 1:1 翻译 scope 的真 sandbox e2e 配套问题，整理到本文档作为 mvp13 territory。

---

## 类别 A: mvp12 e2e debug 已修 (M12.6 round 1+2 fix)

| ID | Bug | Status | 修法位置 |
|---|---|---|---|
| **E1** | Codex update prompt 卡 startup | ✅ Fix | `src/provider/manifest.rs` codex command 加 `-c approval_policy="never"` 等 (1:1 from Python `provider_backends/codex/launcher_runtime/command_runtime/service.py:55-69`) |
| **E2** | Gemini v0.40 init_probe 不识 ready (TUI drift) | ✅ Fix | `src/provider/init_probe.rs` `GeminiInitProbe::prompt_on_last_line` 改 take(8) 倒数 N 行 (Python 原版也会 drift) |
| **E2'** | CodexInitProbe 同问题 | ✅ Fix | take(6) + 删 "OpenAI Codex" 误 banner (codex 现版 ready TUI 标题就含此字) |
| **E3** | 5 panes 而 4 panes (root pane 未复用) | ✅ Fix | `src/tmux/session.rs` first agent `respawn-pane -k` 替换 root bash pane |
| **E3'** | TMUX_COMMAND_FAILED first spawn (fresh session 无 window) | ✅ Fix | `reusable_initial_window_sync` list-windows 失败时 Ok(None) lazy |
| **E4 临时** | 191 orphan ccbd-tmux-*.scope 阻塞 systemd-run | ⚠ 手动清 | 见 `docs/upstream-ccb-bugs/tmux-scope-and-tmpdir-leak-bugs.md` Bug A 已知；当前我们的 BindsTo anchor 架构在 daemon 正常 stop 时 cascade 清；orphan 仅在 daemon 异常退出后残留。本次手动清 191 个，长期 fix 见类别 C |
| **E5 r1** | Sandbox bwrap 缺 provider binary path bind + 阻塞 net | ✅ Fix | `src/sandbox/bwrap.rs` 加 `.npm-global` `.local/bin` `.local/share/claude` `--ro-bind-try` + 默认 `--share-net` |
| **E5 r2** | Auth 文件 symlink target host 路径未 bind | ✅ Fix | `src/sandbox/bwrap.rs` 加 `.codex` `.gemini` `.claude` `.claude.json` `--ro-bind-try` |
| **E5 r3** | sandbox `/home/agent` 没 trust 注入 | ✅ Fix | `src/provider/home_layout.rs` 三 provider 各自 materialize 时写 trust 配置 (codex `[projects."/home/agent"] trust_level=trusted` / claude `projects./home/agent.hasTrustDialogAccepted=true` / gemini `/home/agent: TRUST_FOLDER`) |

---

## 类别 B: 实测仍未通过 (mvp13 territory — 超出 mvp12 1:1 翻译 scope)

### B-1: Sandbox 模式 first-run onboarding 链路完整覆盖

**现状 (E5 r4 实测)**:
即使 trust 注入 `/home/agent`、所有 binary 路径 bind、network share、CLI 加 trust-bypass flag，3 provider 在 sandbox 模式下 first-run 仍触发各自 onboarding：

| Provider | 触发的 prompt | 来源 |
|---|---|---|
| Codex | "Do you trust /home/agent?" `› 1. Yes, continue / 2. No, quit` | sandbox-fresh dir 的 trust dialog；`--dangerously-bypass-approvals-and-sandbox` 不覆盖此 prompt |
| Claude | "WARNING: Claude Code running in Bypass Permissions mode" `❯ 1. No / 2. Yes, I accept` | `--dangerously-skip-permissions` 触发的 confirm 弹窗 |
| Gemini | "No authentication method selected" auth-picker | `--yolo` 不强制 auth method；需要预先选 auth + 同意 ToS |

**根因**: Python ccb 不 sandbox（agent run 在用户 HOME），所以这些 first-run onboarding 的状态本来就在用户 host 上完成且持久化在 `~/.codex/config.toml` `~/.claude.json` `~/.gemini/...`。Rust sandbox 给 `/home/agent` 是新空间，每次都 first-run。

**修法方向 (mvp13)**:
1. (优先) 完整复制 host 的 `.codex/` `.claude*` `.gemini/` 结构到 sandbox HOME（不只是 trust 字段——要 onboarding state, ToS 接受记录, auth method 选择等）。即把 host 的所有 first-run 状态完整 mirror 进 sandbox HOME。
2. (备选) 给 sandbox 加 `--chdir <host_workspace>` 让 cwd = host 路径（已 ro-bind），providers 看到的 cwd 就是 host 已 trusted 的项目目录。但 sandbox HOME 仍是 fresh，其他 onboarding state 还会触发。
3. (备选) 重新引入克制版的 `interactive_prompt_handlers`：仅针对已知的 sandbox-fresh prompt pattern auto-Skip，每个 prompt pattern 都有具体来源 (cite Python file:line if exists, or note as Rust-only)。这是 Gemini 在 mvp11 走偏批评的反模式，但 sandbox 现状下可能必要。

### B-2: NO_SANDBOX 模式 ask reply text capture (E6)

**现状**: `ccb-rs ask --wait` 提交后 codex/gemini/claude 实际处理了 prompt（pane scrollback 显示 `› echo from codex / • Ran echo from codex / • from codex`），但 CLI 返回的是 escape codes stream 不是 reply text。

**根因疑似**: M12.2 R-2 的 dispatcher 闭环在 agent BUSY→IDLE 时 mark COMPLETE + notify_job_update，但 reply text 的提取（从 PTY output 捕获 reply 部分而不是全部 escape codes）路径没完整实现。

**修法方向**: 看 Python `ccbd/services/dispatcher_runtime/finalization_runtime/persistence.py` 的 reply text 提取逻辑（怎么从 events 里 distill reply text），1:1 port 到 Rust。

### B-3: M12.6 step 4-6 未测

- step 4 `ccb-rs cancel <job_id>` 中途任务取消
- step 5 `ccb-rs kill --session <id>` 收尾 (BindsTo anchor cascade)
- step 6 跨 daemon 重启 detach 验证 (M12.5 reconcile 真实测)

NO_SANDBOX 模式下 step 1-2 可工作，所以这 3 步可以走 NO_SANDBOX 验证。等 B-2 (reply text) 修了之后再统一跑。

---

## 类别 C: upstream-ccb-bugs 文档审计的 mvp13 项 (Gemini audit 2026-05-05)

参考 `docs/upstream-ccb-bugs/tmux-scope-and-tmpdir-leak-bugs.md` 5+1 bug 审计。当前架构成熟度 8.5/10 vs Python 旧版 3/10，但仍有 3 项 mvp13 territory：

| Bug | 当前状态 | mvp13 修法 |
|---|---|---|
| **Bug E** agent-restart-loop-without-cleanup (P4) | ❌ 未规避 | orchestrator 加 agent 生命周期锁，restart 前强制 `tmux kill-pane` + 注销 DB + 清 sandbox dir |
| **Bug B** mkdtemp-leak-on-fork-failure (P1) | ⚠ 部分规避 (靠 `cleanup_sandbox_dir` 手动) | `handle_agent_spawn` 错误路径重构 RAII owning struct (Drop trait 兜底 panic) |
| **Bug F** flat-tmpdir-namespace (P5 架构债) | ⚠ 部分规避 (`<state>/sandboxes/<agent_id>` 无 session_id 前缀) | `src/sandbox/path.rs:18` 改 `<state>/sandboxes/<session_id>/<agent_id>`，1 行改动 ROI 极高 |
| **Bug A 残留 (orphan auto-clean)** | ✅ Anchor BindsTo 切断主路径，但 daemon panic + scope 残留仍有 (我们手动清 191) | 加 startup_reconcile 一段：扫 `ccbd-tmux-*.scope` 跟 DB 对账，stop 孤儿 |

---

## 推荐 mvp13 优先级 (按风险 × ROI)

1. **B-1 sandbox onboarding 完整 mirror** (P0 — sandbox 模式当前不可用)
2. **B-2 ask reply text capture** (P0 — 用户 ask 没 reply text)
3. **C Bug E restart cleanup** (P1 — 一旦 supervisor 自动 restart 会迅速漏 pane/FIFO)
4. **C Bug A startup orphan clean** (P1 — production 长寿场景)
5. **B-3 step 4-6 实测** (P2 — 验证 cancel/kill/reconcile 真路径)
6. **C Bug B/F** (P3 — 代码质量改善)

---

## 给 mvp13 master Claude 的工作流建议

1. 起 task scope 隔离 mvp13 e2e debug (会反复 spawn/kill providers)
2. **先**修 B-1 (sandbox onboarding mirror) — 这是阻塞所有 sandbox 验收的根
3. **再**修 B-2 (reply text capture)，然后跑 step 1-3 全 sandbox 验证
4. **再**修 C Bug E (restart cleanup)，跑 step 4-6 全 sandbox 验证
5. **最后**做 C Bug B/F 代码质量改善
6. mvp13 acceptance: followup-prompt 6 步全过 + sandbox 模式默认可用
