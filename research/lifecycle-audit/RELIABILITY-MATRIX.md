# ah Agent 生命周期完成检测 — 可靠性矩阵 + gap 分析 + 建议

> 主控 PM 整合自：`stageC-completion.md`(a1/codex)、`stageABD.md`(a2/codex)、a4(claude) 完整性审计、a3(antigravity) 路径核实，加主控本 session dogfood 实证（codex/claude/antigravity 三 provider 全跑通）。
> 状态：定稿候选（待 a4 终审矩阵本身）。

## 0. 一句话结论

ah 的完成检测对 **codex 三重冗余(log 主 + hook + UI 兜底)、最稳**；对 **claude 双重(log 主 + hook)、次稳**；对 **antigravity 单点(pane recapture 唯一真路径)、最脆**。四阶段里 **Stage C(完成) 冗余最厚**；**Stage B(是否真开始) 最薄——没有任何"模型已开始"的正向物理信号**。两处"假冗余/盲区"必须点名：① `ah.toml` 把 antigravity 列进 `hook_push_providers` 是**永不 fire 的空冗余格**，掩盖了 antigravity 实为单点的事实；② 两份 research 都漏了一条**跨阶段的 worker 崩溃恢复 + 中断 job 重排队**路径（a4 审计抓出），它恰好是坏掉的 master_watch 的"持久化、可重启存活"正面对照。

> **⚠️ 头号缺陷（本 session live dogfood 当场抓获，详 §4 G0 + §7）**：Stage C 虽冗余最厚，却有一个**兜底层反噬主路径**的致命洞——`health_check` 把"空闲一段后被重新派单"的 codex/claude(LogAndUi) **误判 STUCK**(`dead_layers=["completion"] elapsed_secs=764`)，而 STUCK 是 LogAndUi 的死胡同：hook/log 完成 CAS 都不接受 `STUCK→IDLE`、LogAndUi 的 STUCK 又不是 recapture candidate→**job 永远完不成，只能等 3h BUSY 超时**。这不是"兜底没帮上忙"，而是**兜底误触发主动把健康 agent 推进主路径救不回的状态**。全天 chronic 复发、打两个主力 provider。这条洞推翻了"Stage C 最可靠"的乐观结论：**完成路径的真实可靠度受限于 STUCK 误判率**，必须列为 P0。

## 1. Provider 物理现实表（一切判断的依据，PM point-1）

manifest 有两个**独立字段**容易被混淆，必须分开看（a4 审计指出的术语 nit）：
- `idle_detection_mode`：怎么判 pane 回到 idle（`ObservedStability` 稳定窗口 / `LineEndRegex` 行尾正则）。
- `completion_signal`：用哪类完成信号（`LogAndUi` log主+UI备 / `UiOnly` 只能 UI）。

| provider | 输出形态 | 日志 | hook-push | idle_detection_mode | completion_signal | 完成主路径 | 最脆点 |
|---|---|---|---|---|---|---|---|
| **codex** | streaming | 有(rollout jsonl) | 可用 | `ObservedStability` | `LogAndUi` | log `task_complete` | idle anti-pattern 漏字(已修 7976fbf) |
| **claude** | streaming | 有(transcript jsonl) | 可用 | `ObservedStability` | `LogAndUi` | log `end_turn` + hook | tool_use=NotTerminal，长链等后续 end_turn/fallback；真 pane fallback 测试少 |
| **antigravity** | **burst(原子吐完<64KB)** | **无** | **不可用(SIGKILL)** | `LineEndRegex` | `UiOnly` | **pane recapture(唯一)** | 单点；硬编码 UI 正则 `? for shortcuts` 一变即瘫 |
| ~~gemini~~ | — | — | — | `LineEndRegex` | `UiOnly` | — | **已弃用，不审** |

> 实证：`src/provider/manifest.rs:29`(`CompletionSignalKind{LogAndUi,UiOnly}`)、codex `:371/:374`、claude `:409/:412`、antigravity `:427/:430`；`is_recovery_eligible_provider` = codex/claude/antigravity(`:34-36`)。

## 2. 可靠性矩阵 [生命周期阶段 × 检测方法 × provider]

图例：**主**=主路径 / **备**=兜底 / **N/A**=物理不适用 / **空冗余**=代码声明但物理假场景。可靠度 H/M/L。

### Stage A — 任务发布 (prompt 真注入到活着的对的 pane)

| # | 机制 | file:line | 测试 | codex | claude | agy | 可靠度 | 洞 |
|---|---|---|---|---|---|---|---|---|
| A1 | job.submit 入队(拒 missing/terminal agent，request_id 幂等) | `src/rpc/handlers/jobs.rs:14-45` | `test_handle_job_submit_*` | 主 | 主 | 主 | **H** | 只证入队，不证送达 |
| A2 | 事务认领 + dispatch seq 边界(reply 只读 seq>dispatched) | `src/db/jobs.rs:249-319` | `dispatch_atomicity_preserves_seq_id_through_completion` | 主 | 主 | 主 | **H** | 不证 tmux send 成功 |
| A3 | pane 绑定 pid revalidate 防错 pane(Bug B/C 修) | `src/orchestrator/mod.rs:777-875` | `stale_dispatch_pane_refresh_rejects_single_pane_with_wrong_pid` | 主 | 主 | 主 | **M-H** | 多 pane 歧义时保守用旧 pane |
| A4 | dispatch guard 防 prompt/trust/启动交互前误送 | `src/orchestrator/mod.rs:744-775,877-910` | `dispatch_guard_*` + `prompt_handler_e2e.rs` | 主 | 主 | 备(测试不足) | **M** | agy 无真物理 prompt 测试 |
| A5 | tmux paste-buffer + send Enter 注入 | `src/agent_io/writer.rs:15-60` / `src/tmux/session.rs:341-372` | `dispatch_io_failure_compensates_agent_to_stuck` | 主 | 主 | 主 | **M** | **无 post-send 屏幕确认**：粘贴了但 Enter 被吞，只能靠 B/D 后验 |

### Stage B — 是否开始 (IDLE→BUSY，真进入工作态) ← 最薄

| # | 机制 | file:line | 测试 | codex | claude | agy | 可靠度 | 洞 |
|---|---|---|---|---|---|---|---|---|
| B1 | dispatch ACK stability 兜底 WAITING_FOR_ACK→BUSY | `src/orchestrator/mod.rs:1110-1128` | **无直接测试**(间接) | 主 | 主 | 主 | **M-L** | 把"send 未失败"当 BUSY，Enter 被吞会误判 BUSY |
| B2 | marker matcher 防假 idle(底部6行+anti-pattern) | `src/marker/matcher.rs:42-77` | codex/claude synthetic + agy `REAL-a3` fixture | 备 | 备 | 备 | **M** | **反向证据**，不能正向证明真开始 |
| B3 | pane diff watcher(无实质变化→STUCK) | `src/pane_diff/mod.rs:447-485` | `test_pane_diff_watcher_*` | 备 | 备 | 备 | **M** | agy burst 可能两 tick(~30s/tick)间已完成，B 观察不到 busy |

### Stage C — 任务完成 (BUSY→IDLE) ← 冗余最厚

BUSY→IDLE 成功完成路径全仓**只有 4 个 `mark_agent_idle_*` 家族**(`src/db/state_machine.rs:1416-1522`：matched / recaptured / log_event / hook_event)，下面 5 机制完整覆盖、无隐藏第 5 条（a4 审计背书）。

| # | 机制(→家族) | file:line | 测试 | codex | claude | agy | 可靠度 | 洞 |
|---|---|---|---|---|---|---|---|---|
| C1 | Log event pull monitor →log_event(codex `task_complete`/claude `end_turn`) | `src/completion/parser.rs:30-99` / `monitor.rs:20-64` | parser+reader+state 全套 | **主** | **主** | N/A | **codex H / claude M-H** | claude `tool_use`=NotTerminal(`parser.rs:85`)，长链等 end_turn/fallback |
| C2 | Hook push / agent.notify →hook_event(event==stop, CAS) | `src/rpc/handlers/agent.rs:520-580` | `tests/pr4c_hooks_plugins.rs` + state 全套 | 备 | 备 | **空冗余** | **codex/claude M-H** | CLI 不传 hook reply→真 reply 靠 screen fallback；**agy 注入了但 SIGKILL 物理假** |
| C3 | Live PTY marker / prompt match →matched | `src/agent_io/reader.rs:151-192` | matcher 套 | 备(log active 时 defer) | 备(同) | 备 | **M** | 依赖 UI 字符串；有 anti-pattern/stability/job-id guard |
| C4 | ACK capture seed direct idle →matched(仅 LineEndRegex) | `src/rpc/handlers/ack.rs:207-291` | `test_observed_stability_capture_seed_never_marks_direct_idle` | N/A(禁) | N/A(禁) | 备 | **M-L** | 只 5s 轮询窗口；agy 真场景测试不足 |
| C5 | **PaneDiff UiOnly recapture** →recaptured | `src/pane_diff/mod.rs:125-167,286-315` | `ui_only_*` + `REAL-a3-*` fixtures | N/A | N/A | **主(唯一)** | **agy M(脆)** | 见 §4 G3 四失败模式；2 tick×30s=**下限~60s 检测延迟**(实测~90s 含首观测 tick) |
| — | evidence gate(完成前置拦截) | `src/db/state_machine.rs:1001-1020` | `pr1a_evidence_statemachine.rs` | 复用 | 复用 | 复用 | — | 拦截，非独立检测 |
| — | **负向完成出口**(crash/kill/startup-timeout→dispatched job FAILED) | reconcile `src/db/system.rs:1010` / kill `src/db/agents_lifecycle.rs:81` / startup-timeout `src/db/state_machine.rs:1226-1230` | (a4 补) | 出 | 出 | 出 | **M** | pend 会拿到 error_reason；研究原稿没枚举 |
| — | STUCK 检测(pane hash/mtime/thinking 静默 / BUSY marker timeout→STUCK / **health_check completion 层超时→STUCK**) | `src/pane_diff/mod.rs:185-228` / `src/marker/timer.rs:113` / `src/provider/health_check.rs:48-54` | `test_three_signals_static_marks_stuck` | — | — | — | — | **不是完成，是 hang 兜底**；但对 LogAndUi 是**死胡同**：误判进 STUCK 后 hook(`state_machine.rs:743`)/log(`:863`) 完成 CAS 都拒 STUCK→IDLE，且 LogAndUi STUCK 非 recapture candidate→救不回(见 §4 G0) |

### Stage D — 拿到结果 (reply 真实/完整/交付)

| # | 机制 | file:line | 测试 | codex | claude | agy | 可靠度 | 洞 |
|---|---|---|---|---|---|---|---|---|
| D1 | pipe chunks + collect_reply(seq边界+vt100+distill) | `src/db/jobs.rs:592-688` | `test_collect_reply_*` | 主 | 主 | 备(可能空) | **M-H** | chunk缺失/prompt-only/UI chrome 未覆盖 |
| D2 | log/hook reply 优先(log reply 权威) | `src/db/state_machine.rs:871-895` | `log_event_missing_reply_uses_screen_collection` | **主** | **主** | N/A | **H** | log 格式变更/log root 不可用→退 UI/pipe |
| D3 | UI-only pane recapture / distill_reply(prompt-only→STUCK) | `src/db/state_machine.rs:393-463` | `ui_only_recapture_completes_*_from_real_pane_*` | N/A | N/A | **主** | **agy M** | 无 hook/log 二次确认 |
| D4 | job.wait / CLI pend 交付(无独立 mailbox 模块=jobs+pubsub+job.wait) | `src/rpc/handlers/jobs.rs:48-102` | `test_handle_job_wait_fast_path_completed` | 主 | 主 | 主 | **H** | reply_text 写对就不易丢；风险在前置 D 提取 |

### Stage R — 崩溃恢复 (跨阶段，a4 审计补；两份 research 都漏)

一条完整的 **worker 崩溃 → 捕获恢复意图 → realign 重生 → 中断 job 重排队重放** 路径，直接影响"发布→开始→完成"判定：一个已 DISPATCHED/BUSY 的 job 在 agent 崩溃后不是简单终结，而是被捕获 + requeue 重走一遍。

| # | 机制 | file:line | 测试 | 适用 | 可靠度 | 要点 |
|---|---|---|---|---|---|---|
| R1 | 崩溃捕获恢复意图 + 持久化表(重启存活) | `src/db/agents_lifecycle.rs:178-180` / 表 `src/db/recovery.rs:20-41` | (a4 grep 实证) | codex/claude/agy | **M-H** | `agent_recovery_intents` 存 interrupted_job_id/prompt/request_id，**DB 持久化** |
| R2 | 每 tick 扫 CRASHED 候选 + realign 重生 | `src/orchestrator/mod.rs:280-305` | — | 同上 | **M-H** | **每 tick 轮询**，ahd 重启后仍继续 |
| R3 | 中断 job 重排队重放 | `src/orchestrator/mod.rs:458` / `src/db/recovery.rs:284` | — | 同上 | **M** | 重走 dispatch→completion |

> **关键对照**：Stage R 是 **DB 持久化 + 每 tick 轮询 → ahd 重启后存活**；master_watch(§6) 是 **进程内一次性 pidfd → 重启即失效**。同样是"恢复/复活"，worker 做对了，master 做错了。**master_watch 的修复应照 Stage R 的模式**（持久化判定 + 周期巡检）。

## 3. 综合可靠度（每 provider × 每阶段）

| 阶段 | codex | claude | antigravity |
|---|---|---|---|
| A 发布 | **M-H**(DB原子H，A5送达无确认) | M-H | M-H(A4测试弱) |
| B 开始 | **M-L**(无正向信号) | M-L | **L**(burst 可能观察不到 busy) |
| C 完成 | **H**(三重冗余+真anti-pattern测试) | **M-H**(双重，真pane fallback测试少) | **M**(单点C5，真fixture但脆，~60s延迟) |
| D 结果 | **H**(log权威+pipe兜底) | **H** | **M**(只pane刮) |
| R 恢复 | **M-H** | **M-H** | **M-H**(agy 也 recovery-eligible) |

## 4. Gap 分析

**G0 — [P0，头号] health_check 重派误判 STUCK + STUCK 卡死 LogAndUi（codex/claude；live 实证）**。两层叠加：
- **误判源** `src/provider/health_check.rs:48-54`：`last_progress_ts = last_marker_ts.or(last_output_ts)` 取该 agent **历史最后一次** progress，跟 `now - stuck_threshold`(默认 ~300s) 比，**没 floor 到当前 job 派单时刻**。所以"空闲 > 阈值后再被派单"的 agent 一进 BUSY 立即被判 completion 层死→STUCK。只打 LogAndUi(`completion_signal != UiOnly`)→**codex + claude 都中招，antigravity 不中**（主力 provider 系统性漏洞）。
- **死胡同** `STUCK` 对 LogAndUi 无完成出口：hook(`state_machine.rs:743`)、log(`:863`) 完成 CAS 都只接受 `WAITING_FOR_ACK|BUSY→IDLE`，STUCK 被 swallow；LogAndUi 的 STUCK 不是 pane_diff recapture candidate；LogAndUi 的 live UI marker 在 log monitor 还活着时被 defer。→ 误判后**无任何路径救回**，只能等 3h `BUSY_TIMEOUT`(`marker/timer.rs:15`) 或人工。
- **危害**：① job 永不 terminal，reply 拿不到；② STUCK 触发 Stage R `action=Revive`，会 respawn 一个本来健康的 codex。
- **证据**：本 session 21:10:46 journald `health check marked agent STUCK agent_id=a1 provider=codex from=BUSY dead_layers=["completion"] elapsed_secs=764`（764s = a1 上个任务完成到本次派单的纯空闲）；a1 STUCK 7.5min+ 不自愈，期间 codex 已干完活回 idle composer。全天 chronic 复发(01:57/04:13/... a1/a2/a4)。详 memory `project_ah_health_check_redispatch_false_stuck`。

**G1 — Stage B 没有正向"已开始"信号（全 provider）**。B1 把"send 未失败"当 BUSY、B2 只防假idle的反证、B3 卡死兜底。没有"首个 dispatch 后有意义 pane diff = 确认开始"的正向确认。后果：Enter 被吞但 send 未报错时误判 BUSY→一路走到 STUCK，浪费一个 stuck timeout 周期。

**G2 — A5 送达无 post-send 确认（全 provider，单一失败模式）**。"粘贴进 TUI 但未提交"只能靠 B/D 后验。

**G3 — antigravity Stage C 是单点（C5 唯一），四个具体失败模式（a3 核实）**：
- **F1 [永不完成] prompt 被挤出底部 6 行视窗**：大量输出/空行把 `? for shortcuts` 推到倒数 7 行以上→`viewport_bottom_region`(`matcher.rs` VIEWPORT_BOTTOM_LINES=6) 匹配不到→卡 BUSY→STUCK。(test `ui_only_marker_recapture_ignores_historical_marker_outside_bottom_viewport`)
- **F2 [误判完成] 输出文本恰含提示符**：reply 内容里出现 `? for shortcuts` 且落在底部 6 行且 2 tick 稳定→假完成。
- **F3 [误判完成] anti-pattern 遗漏**：busy 状态没被 anti_regex 抓住→2 tick 稳定→误判→拉到残缺 reply。
- **F4 [永不完成，最致命] agy CLI 升级/文案变更/折行**：硬编码正则 `r"(?m)^\s*\? for shortcuts\b"`(`src/marker/matcher.rs:92` `prompt_regex_for_provider`，viewport 常量 `:6` `VIEWPORT_BOTTOM_LINES=6`) 一旦 agy 改文案(如→`? for help`)或终端列数变化折行即彻底失效→该 provider 完成判定全面瘫痪。**这是单点里的单点：一个硬编码 UI 字符串绑死整个 provider 的可用性。** 注：`src/db/learned_rules.rs:373,404,444` 已携带同一模式但未接进 live `MarkerMatcher`(所以"硬编码"定性成立)——G3 修复①可复用该 learned_rules 底座而非从零造。
- 外加 **~60s 检测延迟**(2 stable tick × 30s/tick)。

**G4 — `ah.toml` 把 antigravity 列进 `hook_push_providers`（误导/空冗余）**。`ah.toml:10-13`=`["claude","codex","antigravity"]`，但 agy hook 物理不可用(SIGKILL)。矩阵里多一个永不 fire 的格子，掩盖 antigravity 实为单点。（antigravity dogfood 实验里故意让 agy 走 hook→失败→兜底以测兜底链；常态配置应澄清。）

**G5 — claude 真实 pane UI fallback 覆盖偏少**。log 路径测试充分，但 log root 不可用时 claude 的 UI fallback 缺真 pane fixture（codex 有 idle anti-pattern 真测试、agy 有 REAL-a3、claude 偏 synthetic）。

**G6 — B1 dispatch ACK stability 无直接测试**(`spawn_dispatch_ack_stability_busy` 只间接覆盖)。

**G7 — 两份 research 漏了 Stage R(worker 崩溃恢复)整条路径**（a4 补；已纳入 §2）。盲区本身说明这套体系缺一份"全 lifecycle 出入口"总图。

**G8 — master_watch 重启不重装探针**（master 生命周期，与 Stage R 同类问题但做反了）。详 §6。

## 5. 建议

**承重必须硬化（不能砍）**：codex C1(log) / claude C1(log)+C2(hook) / antigravity C5(recapture) / D1+D2(reply) / Stage R(R1-R3 崩溃恢复)。这些是真主路径。

**补**：
- **[P0，先于一切] G0 health_check 重派误判 STUCK**：① 主修——`health_check.rs:48-54` 的 `last_progress_ts` 对"当前 job 派单时刻"取 floor(或派单时重置 progress 基线)，只衡量"本次任务开始后"的停滞，不把空闲计入；② 纵深防御——让 hook/log 完成 CAS 也接受 `STUCK→IDLE`(`state_machine.rs:743,863`)，即便误判也能在 agent 真完成时救回。tests-first 先红灯："空闲>阈值后再派单的 codex 不应被判 STUCK" + "STUCK 的 codex 收到 log/hook 完成应能回 IDLE"。
- **B1→正向化**：用"dispatch 后首个有意义 pane diff"作正向"已开始"确认，替代纯 ACK-stability 推断（解 G1），顺带给 A5 一个 post-send 送达确认（解 G2）。
- **antigravity C5 去脆**：① 把硬编码 `? for shortcuts` 正则**提到 manifest/配置可调**并加"匹配不到时降级告警"，避免 agy 升级即瘫（解 F4）；② 加第二完成确认（burst 后内容指纹二次稳定校验），破单点（解 G3）；③ 评估 ~60s 延迟是否可接受。
- **补测试**：claude 真 pane fixture(解 G5)、B1 直接测试(解 G6)。

**澄清/relabel（避免假冗余误导）**：
- `ah.toml` 把 antigravity 从 `hook_push_providers` 移除，或代码/文档显式标注"agy hook = 故意 no-op，仅兜底链测试用"（解 G4）。
- 落一份"全 lifecycle 出入口总图"(本矩阵即雏形)，把 Stage R + 负向出口纳入常驻文档（解 G7）。

**修**：master_watch 重启重装探针 + 周期巡检，**照 Stage R 模式**（§6）。

## 6. master_watch 重启不重装探针 bug（顺带修，PM "合完再修"）

**结论：当前代码仍成立（a2 核实 + a4 独立对照背书）**。

- ahd startup 只 `reconcile_startup_with_tmux_socket`(`src/bin/ahd.rs:56-75`)，reconcile 只处理 **agents**(`reconcile_active_agents_to_crashed_sync` 重建 agent pidfd watch `src/db/system.rs:757-787,1031-1060`)，**没有对 session master ACTIVE 做 pidfd_open/arm**(`src/db/system.rs:521-532`)。
- master watch arm 只在 spawn(`arm_revival_watch=true` `src/rpc/handlers/sessions.rs:359-410`) / cutover VERIFYING→ACTIVE(`:890-900`) / revive 后(`src/monitor/master_watch.rs:315-325`)。
- `master_process_is_alive` 只用于 cutover readiness(`sessions.rs:455-460,631-637`)，**无周期巡检 / startup reconcile 调用**。
- monitor 是 pidfd 一次性 async wait(`master_watch.rs:30-88`)；ahd 重启后旧 async task 消失，继承的 ACTIVE master 没重 arm→master 再死则零检测/零复活/零 reap。无相关测试。

**修向**（沿用 memory `project_ah_master_watch_not_rearmed_on_restart`，照 Stage R 持久化+周期巡检模式）：
1. **startup reconcile** 对每个 ACTIVE session 重 `pidfd_open`+重 arm master 探针；那刻已死则立即走死亡处理（连坐清 worker / 视语义 revive，须与 `project_master_death_corrected_semantics` + `project_ah_session_watch_cascade_defeats_revive` 对齐，**别重新引入级联杀**）。
2. **orchestrator tick 周期巡检**兜底(`master_process_is_alive` 周期跑)，防 startup 那刻没死、之后才死又漏掉。

## 7. Live dogfood 实证（本 session 真跑 ah，三 provider 全跑通，pane 实证非自报）

| provider | dispatch 任务 | 完成 sub_state / 路径 | 检测延迟 | 结论 |
|---|---|---|---|---|
| codex(a1/a2) | 两份 research | IDLE/**HookEvent**(20:58) | 即时 | hook-push 完成路径 live 命中 ✓ |
| claude(a4) | research 审计 | IDLE/**HookEvent**(21:05) | 即时 | hook-push 完成路径 live 命中 ✓ |
| antigravity(a3) | 路径核实 | IDLE/**Matched**(recapture) | ~90s(含 ~60s recapture 延迟) | UiOnly recapture 完成 ✓，pane 见真实 Read/Search 工具调用 |

> 三 provider 本 session 全部完成检测无漏判/无早判，reply 完整正确交付。codex/claude 走 hook_event(主动通知，即时)，antigravity 走 recaptured(被动稳定 tick，~60s 延迟) —— 与矩阵预测的 provider 物理路径完全吻合。
> 注(解 a4-N1 主/备张力)：§1/§2 标 log=主、hook=备，是**可靠性-权威兜底序**(log 是落盘可重读的权威，hook 是主动 push 的快信号)，**非运行时频率**；happy path 里 hook 因主动通知且 fire 时 cancel pull monitor(C2)，常态下 hook 抢先成为首个完成者——故本 session codex/claude 都显示 HookEvent 完成，与"log 是主"不矛盾。

### 7.1 [反例] 头号缺陷当场抓获：health_check 重派误判 STUCK（同 session live）

本 session 第二轮派单（给已空闲 ~12.7min 的 a1/codex 派 master_watch 调研）**当场复现 G0**：
- `21:10:46` journald：`health check marked agent STUCK agent_id=a1 provider=codex from=BUSY dead_layers=["completion"] elapsed_secs=764`。派单后仅 ~6s 即被误判。
- pane 实证：a1 实为**正常工作**——"Working (3m 46s)"→真实 Read/Search/写文件，最终 `21:14` 落盘完整 14KB 设计文档(`masterwatch-fix-design.md`)，pane 显示 "Worked for 4m 39s" 回 idle composer。
- **但 ah 侧 a1 STUCK 持续 7.5min+ 不自愈**：codex 干完活、job 却永不 terminal——`ah pend` 拿不到 reply（我是从 filesystem 直接取的设计文档，印证 STUCK 卡死的是 reply 交付链，不是 work 本身）。
- 与 §7 happy-path 对照：**完成主信号(hook/log)本身没问题，但被 STUCK 态拦在门外**——这正是 G0"兜底反噬主路径"的活样本，也是 PM"多重兜底为何还不可靠"的直接答案。
