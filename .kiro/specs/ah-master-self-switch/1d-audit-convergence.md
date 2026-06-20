# Step-4 宽版 master 自换 — 1d 思路 audit 收敛 (PM 综合)

> 2026-06-15 Master PM 综合 a1 (工程) + a3 (PM 替身) 对 `idea-a2.md` 的 1d audit。
> 结论: **思路方向成立 (蓝绿自举 + flat-peer + resume=对话级), 可进 1e 正式 design; 但有 7 条 must-fix 必须在 design 里收敛。** 1d round-1 即收敛 (a1/a3 无矛盾结论, 唯一分歧 MF1 已被已合代码事实裁定)。

## 思路 CORE (保留, 不动)
- 自托管 bootstrap: 旧 master 自起 ahd + 拉新 master + 同步 + 自我放逐 (self-hosted control plane, 拒外部硬切脚本)。
- flat-peer scope: master 用 `systemd-run --user --scope` workspace_slice, worker 用 agent_slice, 都 `BindsTo ahd.service` — **已合代码实证一致** (systemd.rs:41/107/121)。master OOM 在 systemd 层不 cascade-kill worker。
- resume 语义 = **provider 对话记忆级**, 不续 ah-job 在途 — 这正面回答 PM 反复追问的"续断点对 master 意味着什么": **续对话, 不续在途命令**。

## 7 条 must-fix (1e design 必须解)

### MF1 [证据 High × 影响 High × 置信 A] worker-reap 矛盾 (a3 抓, a1 漏)
- a2 叙事"master 死时 worker 还在干活, 新 master 用 ah ps/logs 查 worker 进度" **与已合 corrected master-death 正面矛盾**: master 死 → `clean_worker_runtime_resources` **无条件 reap** 该 session 所有 worker (db/system.rs:224-320); a3 上轮 r2 dogfood 实测 case A `db_killed=1`。这是 PM 2026-06-14 明确拍定的语义 ("必连坐清 worker 防僵尸/孤儿"), 不可逆。
- **a1/a3 分歧裁定**: a1 #5 接受了 a2 的 worker-progress 叙事 (a1 没读 reap 路径); a3 用已合代码 + dogfood 推翻。**a3 对** — 已合代码 + PM-locked 决策为准, 无需三轮辩论。
- **design 必须采用 re-dispatch 模型**: master OOM → worker 被 reap → 复活 master 续对话记忆 + **重新派发**丢失的在途任务 (不是"查正在跑的 worker")。这是合法 PM 工作流, 留在已合语义内。
- **不走反转 reap (option b)**: 反转会推翻 PM-locked corrected master-death, 须 PM 拍 — **本设计不追求**, 除非后续 e2e 暴露 re-dispatch 真不可行才 escalate。

### MF2 [证据 High × 影响 High × 置信 A] --continue 跨沙箱 home 失效 (a3 抓)
- a2 "新 master 带 `--continue` 同步当前对话" 在跨沙箱 home 下不成立: `--continue` 恢复的是**本地** HOME/CLAUDE_CONFIG_DIR + cwd 下 `.claude/projects/<cwd-hash>/` 最近会话。
- 已合: ah master 跑在独立沙箱 home (`<ah-state>/sandboxes/<session>/master/.claude`, master_watch.rs:213-221 / sessions.rs:206-215) ≠ 旧真 master 的 ccb 沙箱 home。新 ah master `--continue` 在自己空沙箱里找不到旧对话 → 开全新对话。蓝绿 State Handover 命门按描述是断的。
- **design 必须给显式对话迁移方案** (把旧会话文件 seed 进 ah master 沙箱 / 让 ah master 指向同一 .claude store / 或其他 handover), 不能靠 `--continue` 自动魔法。

### MF3 [证据 High × 置信 High] spawn_master_pane 不是完整 cutover 入口 (a1 抓)
- 现 `session.spawn_master_pane` 是 RPC (router.rs:14-18,79), 只在 `ah start` 内部调 (start.rs:152-165); ah.rs:36-106 **无对外 cutover/spawn-master CLI**。
- design 必须给产品化 cutover 命令 (旧 master 触发"起绿 master + handoff + 返回 attach 信息")。
- 注意: cutover **不能**用 `ah kill --session` 让旧 master 退出 — `session.kill` 会杀 master pane/session (sessions.rs:74-140)。

### MF4 [证据 High × 置信 High] TTY 黑洞 (a1+a2 都标 Critical)
- 新 master 起在 ahd 托管后台 tmux pane; `ah attach` 现只接 agent_id (ah.rs:378-386), master/agent session 命名分离 (tmux/mod.rs:13-32)。
- design 必须扩 `ah attach` 支持 master pane + 设计交接盲区 (handoff prompt 步骤) 让用户能看到/输入新 master。

### MF5 [证据 High × 置信 High] cutover fencing 须 DB/CAS 不是工作区文件 (a1 改进 a2)
- a2 提 `.ah_cutover_active` 工作区锁文件 = advisory 软锁, 挡不住旧终端误派。
- a1: 现只有进程内锁 (session_window_lock / master_spawn_lock), 不能跨进程 fencing。**建议放 DB/state_dir, 带 session/generation/pid/CAS** 做强 fencing 防双调度脑裂。

### MF6 [证据 中高 × 置信 中高] 新 master 连同一 ahd 的 socket/env 须显式 (a1 抓)
- CLI socket 优先 `CCB_SOCKET` 否则按 `AH_STATE_DIR`/cwd config 算 (rpc_client.rs:102-113, state_layout.rs:16-47); master spawn 只注入 home/auth env (sessions.rs:205-230)。
- design 必须显式保证新 master 连对同一 ahd (cutover 用非默认 state_dir/socket 时会连错)。

### MF7 [证据 中高 × 置信 中高] --continue ≠ PM handoff (a1+a3 收敛)
- `--continue` 够 provider 会话恢复, 但旧 master 的当前计划/未发完命令/交接确认需**显式 handoff prompt 或文件**。
- 跟 MF1 resume 语义一致: 续对话记忆 + 显式 handoff 状态, 不续在途 CLI 命令。

## should-fix (design 一并解)
- **S1 (a3 #5)**: 验收重写 — 用 re-dispatch 模型 (复活 master 续对话 + 识别旧 worker 已 reap + 重新派发该任务直至成功), 删 a2 "查到正在跑的任务最终成功" 矛盾表述。
- **S2 (a3 #5)**: 补 **case B** 验收 (待命态 master 被杀 → reap 但不 revive)。
- **S3 (a3 #6 + a1)**: 补两风险 — cutover 期 worker 归属 (旧+新 master 都可能派, 软锁挡不住) + ahd 自身重启 (非 master OOM) 时 master 怎么办 (startup_reconcile 兜底?)。

## 已有低成本原语 (a1 确认, design 直接复用)
- `ah start → session.create → session.spawn_master_pane` 已有 (start.rs:136-165)。
- master pane 已进 tmux + systemd scope + pidfd watcher (sessions.rs:225-300)。
- master revive 已有 CAS/generation/lock/backoff (master_watch.rs:99-170, master_revival.rs:95-185)。
- `ah ask/ps/logs` **不走 nesting guard** (guard 只在默认入口, ah.rs:223-224,351-371) → master pane 内部 `ah ask` 派 worker 这条递归路径通。

## 下一步
1e: a1 主笔正式 `design.md`, 基于 research + 本收敛 (CORE 保留 + 7 must-fix 全解 + 3 should-fix), grep 实证 file:line / schema 字段全准。1f: a3 audit 文档。
