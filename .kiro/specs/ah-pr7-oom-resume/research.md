# Research: ah PR-7 — OOM Self-Restart + Codex/Agy Resume Completion

> 主笔: a1 (codex) 调研, Master PM grep-verified 全部 wiring file:line (2026-06-11).
> 衔接: 本 PR 在 [ah-pr6-recovery-resume](../ah-pr6-recovery-resume/design.md) 基础上推进。
> PR-6 已做: `resume_args` 静态字段 + claude `--continue` + CRASHED recovery realign。
> PR-6 显式排除: codex/gemini resume、ahd 自身 OOM 重启、auth fallback。本 PR 补这三块。

## §1 立项目标 (PM 定义, Step 2)

ah 替代 ccb 的根本诉求之一: **OOM 后能有意识重启 + resume 续断点**。
拆三块:
1. **ahd 自身 OOM 后自动重启** (daemon 级) — 现在 ahd 被 OOM-killer 杀掉后不会自起。
2. **重启后 agent 续断点 resume** — codex/agy 的"同一会话精确恢复"(PR-6 只做了 claude)。
3. **auth fallback ladder** — OAuth 凭据进沙箱失败时的降级阶梯 (symlink → copy → 沙箱外诊断)。

## §2 Provider 原生 resume 机制 (a1 实测)

### codex (codex-cli 0.135.0, 本机实测)

- 原生命令: `codex resume [SESSION_ID] [PROMPT]`
- 精确恢复应传: `codex <现有全局参数> resume <session_uuid>`
- 兜底: `codex resume --last` — 仅适合"隔离 CODEX_HOME 下最近会话明确等于该 agent"的情况。
- **session-id 来源**: `$CODEX_HOME/sessions/YYYY/MM/DD/rollout-*.jsonl` 首行 `session_meta.payload.id`;
  文件名 UUID 后缀也一致。
- 本机证据样本 id: `019e85d0-8f7a-73c2-9169-1ab5eebf5c08`。

### agy / antigravity (agy 1.0.7, 本机实测)

- 原生恢复: `agy --continue` / `agy -c` 继续最近 conversation。
- 精确恢复 flag: `agy --conversation <conversation_id>` (help: "Resume a previous conversation by ID")。
- ah 应传: `agy --dangerously-skip-permissions --conversation <id>`; 无 id 且每-agent HOME 隔离可靠时退化 `--continue`。
- **未确认**: conversation-id 落盘格式。`~/.gemini/antigravity-cli/conversations` 当前为空 (因未登录/未形成真实会话)。
  → 设计需把"agy conversation-id 落盘位置/格式确认"列为实施期 spike, 或先用 `--continue` 兜底。

### claude (PR-6 已做)

- `--continue` 在隔离 `CLAUDE_CONFIG_DIR` 下续接最近会话, 已 wired (`resume_args: &["--continue"]`)。

## §3 ah 现有 wiring (Master PM grep-verified 2026-06-11)

| 位置 | 现状 |
|---|---|
| `src/provider/manifest.rs:11` | `pub resume_args: &'static [&'static str]` ← **静态数组** |
| `src/provider/manifest.rs:175` | codex `resume_args: &[]` (空) |
| `src/provider/manifest.rs:211` | claude `resume_args: &["--continue"]` |
| `src/provider/manifest.rs:228` | antigravity `resume_args: &[]` (空) |
| `src/sandbox/systemd.rs:132-133` | `if is_recovery { cmd.extend(manifest.resume_args...) }` |
| `src/sandbox/systemd.rs:555` | `test_wrap_command_with_recovery_appends_resume_args` (已测) |
| `src/rpc/handlers/realign.rs:156` | `if running.state == "CRASHED"` → recovery spawn |
| `src/rpc/handlers/agent.rs:91-96` | spawn 把 `is_recovery` 传给 `wrap_command` |

## §4 核心风险 / 设计约束 (中心议题)

**`resume_args: &'static [&'static str]` 是静态数组, 物理上无法承载动态 session/conversation id。**

- claude `--continue` 是静态字符串 → 现机制能表达。
- codex `resume <session_uuid>` / agy `--conversation <id>` → uuid/id 是**每-agent-运行时**才知道的值,
  须在 recovery 时**读沙箱内 session 文件**算出 → 现有静态 manifest 字段表达不了。

设计必须解决: 如何让 resume 参数**动态化**到能携带 codex session_uuid / agy conversation_id。
候选方向 (留给 a2 第一性原理发散, 不预设):
- provider 级 `compute_resume_args(sandbox_home) -> Vec<String>` hook (recovery 时探测沙箱算出)
- 或 per-agent 持久化 `resume_command` 到 DB / manifest 实例
- 兜底: 探测失败时退 `--last` / `--continue` (静态兜底, 但语义弱)

## §5 a1 读过/跑过 (审计可复核)

- 读: `/tmp/a1_resume_research.md`, `src/provider/manifest.rs`, `src/sandbox/systemd.rs`,
  `src/rpc/handlers/realign.rs`, `src/rpc/handlers/agent.rs`
- 跑: `codex --help`, `codex resume --help`, `agy --help`, `agy --conversation --help`, `agy help`,
  `which/version`, `find ~/.codex/sessions`, `sed` 读 codex jsonl 首行, `find/rg ~/.gemini`,
  `rg resume_args|--continue|CRASHED src`, `nl -ba`. **未改任何文件** (read-only).
