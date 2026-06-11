# Dispatch Interstitial Guard — 设计 (resume 续断点 race 修复)

> 状态: 设计已收敛 (a2 思路 + a1/a3 audit, round-1 收敛, 2026-06-11)。
> 关联: Step 3 dogfooding closure (OOM 后 resume 续断点); PR-7 OOM-resume 机制 (已 merged)。

## 1. 问题 (witnessed + 代码实证)

OOM 杀 codex worker → ahd 检测 CRASHED → `resume <rollout-uuid>` 重启 codex。codex 重放 transcript 比全新启动慢:
1. init probe 在重放期间于 ~400ms 窗口 (`STEADY_COUNT=2 × POLL_FAST=200ms`, `init_probe_task.rs`) 命中 ready marker → 置 IDLE。
2. IDLE 之后, codex 的"检查更新"逻辑才把 `Update available! ... npm install -g @openai/codex` 弹窗渲染出来 (在 IDLE 之后、下一次派单之前)。
3. dispatcher 看 IDLE 就派单 → paste + Enter, Enter 落在弹窗默认项 "Update now" → codex 触发失败路径崩溃 → tmux server 死 → resume 续断点失败。

**根因**: polling 架构里 "IDLE = 可安全派单" 是隐含契约, 但 ah 在 pane 真正静默前就宣布 IDLE, 且派单路径对"发送瞬间 pane 顶着已知 interstitial"零防御。非 codex 更新弹窗独有: 任何 IDLE 之后才渲染的可识别 interstitial (后台更新检查 / OAuth 提醒) 都会被下一次派单 Enter 误触。

## 2. 契约 (收敛结论)

**契约 B — 派单器防御纵深 (dispatcher self-doubt)**: 派单器不盲信 IDLE 标记; 在真正发送前确认 pane 上没有可识别 interstitial。

### 关键放置 (a1 audit must-fix #2, 纠正 a2 的 :88 放置)
预扫 **必须发生在 `dispatch_job_to_agent` 把 job 从 QUEUED 改 DISPATCHED 之前** (`orchestrator/mod.rs` run_once, 现 :49-63 先 pull)。

理由:
- 若在 :88 (job 已 pull, agent 已 WAITING_FOR_ACK) 后预扫并跳过发送: job 已被消费成 DISPATCHED, 无 DISPATCHED→QUEUED requeue helper, 用现有失败路径会把用户 job 标 FAILED (丢任务)。
- 且 `scan_prompt_and_apply_outcome` 对 active-dispatch state 会 **defer** unknown-prompt demote (`integration.rs:100-117`) — agent 已 WAITING_FOR_ACK 时扫未知弹窗不会进 PROMPT_PENDING。
- 在 pull 之前预扫: agent 仍 IDLE, 命中则 job 原样留队、下一轮重试; 非 active-dispatch state 下 unknown prompt 能正确 demote PROMPT_PENDING。

### 机制 (a1 #1 + a3 复用纪律)
- 复用现有 async `scan_prompt_and_apply_outcome` (`integration.rs:56`, 内部 `spawn_blocking` 跑同步 `handle_prompt_chain`)。**不要**新写并行 `fast_scan_known_interstitials` scanner (会和 canonical KB chain 分叉, KB 加 case 两边不同步)。
- 新增 `PromptScanPurpose::DispatchGuard` 变体 (现有 `Direct`/`AckVisualDiff`/Startup 同模式)。
- 不能在 async run_once 里直接调 sync `handle_prompt_chain` (阻塞 runtime worker)。

### 排序 (a1 #3)
确认 pane/parser 可用 → **预扫** → 仅当决定真发单时才 `set_idle_scan_enabled(false)` → baseline → log monitor → send。预扫跳过发送的轮次绝不能留下"未派单但 idle scan 已关"的 agent。

### fail-closed (a3 must-address, 黄金原则零容忍静默失败)
预扫 / capture-pane **本身失败** 时必须 **fail-closed**: "扫不动 = 无法证明 pane 干净 = 本轮拒派 + `tracing::warn!`"。**绝不 fail-open** 继续 paste+Enter (否则崩溃悄悄回归)。

### 命中处置
- 命中已知弹窗 (如 `codex_update_01`) → scan 自动 dismiss (keysym 2+Enter) → 本轮 continue, 下一轮重新确认稳定 IDLE 再派。
- 命中 unknown prompt → scan demote agent 进 PROMPT_PENDING → 本轮不派, job 留队。
- 干净 idle → 继续正常派单路径。

## 3. Scope 边界

### 本轮做 (§2.1)
派单前预扫 guard (上述契约 B)。

### Follow-up (§2.2, 不在本 goal scope)
a2 的"两阶段 Enter (文本/Enter 分离)": a3 实证现已有 paste→Enter 0.5s 间隔 (`CCB_TMUX_ENTER_DELAY`, `writer.rs:40-50`)。§2.1 已闭合 witnessed bug; §2.2 只闭合更窄残留竞态 (弹窗恰在 paste 后 0.5s 内才冒) 且引入新风险 (半输入 prompt / slash command 特例 / antigravity 不按 Enter)。标 follow-up, 不撑大 Step-3 scope。

### 已知残留 (文档化, 非缺陷)
完全无法识别成 prompt 形态的全新弹窗仍无防御 — 检测的物理下限, 不是设计漏洞。

## 4. 可测试性 (确定性, 不依赖真实时序)

红测试在 **扫描决策函数层** (pane-text 入 → decision 出), 喂固定 pane 快照:
1. 含 `codex_update_01` 模式 (`Update available!.*@openai/codex`) → 决策 = **拒派** (job 不 pull / agent 退 PROMPT_PENDING 或发 dismiss keysym), **负向断言: prompt_text 未被 paste, Enter 未送进弹窗** (契约核心)。
2. 干净 idle 提示符快照 → 决策 = ok-to-dispatch。
3. **fail-closed 用例**: capture-pane 返回 Err → 决策 = 拒派 (非放行)。
4. (可选) 若 `TmuxServer.capture_pane` 可注入, 补一条 run_once 集成测试。
不测真实 OOM 时序 (不确定性, 留 dogfood 验证)。

## 5. 物理实证路径
退 PROMPT_PENDING emit `state_change`; dismiss emit prompt 事件 — 都是可观测物理痕迹。配合红测试 + 事件断言确定性证明"Enter 没进弹窗"。dogfood: OOM→resume→派单, 看 daemon log 有无 DispatchGuard 命中 + agent 不崩 + resume 会话记得断点前内容。
