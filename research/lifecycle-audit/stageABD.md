# ah agent 生命周期 Stage A/B/D 检测机制盘点

范围：只盘点 A 任务发布、B 是否开始、D 拿结果；不分析 gemini。结论均来自当前代码 grep/read。

## 物理现实表

| provider | 物理现实 | 当前 manifest |
| --- | --- | --- |
| codex | streaming，有 log/hook，reply 主路径走 log 或 pipe chunks | `src/provider/manifest.rs:347-375`：`ObservedStability`，anti-pattern 包含 `esc to interrupt`/hook trust modal，`CompletionSignalKind::LogAndUi` |
| claude | streaming，有 transcript/hook，reply 主路径走 log 或 pipe chunks | `src/provider/manifest.rs:397-413`：`ObservedStability`，anti-pattern 包含 `esc to interrupt`/`Architecting`/`Reading N files`，`CompletionSignalKind::LogAndUi` |
| antigravity | burst，一口气吐完；无 log/hook；reply 只能从 pane 刮 | `src/provider/manifest.rs:416-431`：`LineEndRegex`，anti-pattern `esc to cancel`，`CompletionSignalKind::UiOnly` |

## Stage A 发布：job.submit 入队

- 逻辑落点：CLI `ah ask` 调 `cmd_ask`，RPC 方法 `job.submit` 写入 `jobs` 并唤醒 orchestrator。入口在 `src/bin/ah.rs:57-65`, `src/bin/ah.rs:218-224`, `src/bin/ah.rs:812-833`；RPC handler 在 `src/rpc/handlers/jobs.rs:14-45`；router 注册在 `src/rpc/router.rs:38-39`。
- 检测/保证：只保证 prompt 进入 SQLite job queue，且拒绝 missing/terminal agent；不保证已注入 pane。`request_id` 幂等由 insert/query 语义覆盖。
- 测试覆盖：`src/rpc/handlers.rs::test_handle_job_submit_queues_job` (`src/rpc/handlers.rs:710-737`)，`test_handle_job_submit_is_idempotent_by_request_id` (`src/rpc/handlers.rs:740-774`)，`test_handle_job_submit_rejects_missing_or_terminal_agent` (`src/rpc/handlers.rs:777-807`)。
- 适用 provider：codex/claude/antigravity，入队逻辑 provider 无关。
- 是否 provider 物理真场景：否。这里是 DB/RPC 层，不接触 provider TUI。
- 可靠度初判：高，但它只是 Stage A 的“发布到 ahd 队列”，不是“送达 provider”。

## Stage A 发布：事务认领 + dispatch seq 边界

- 逻辑落点：orchestrator 主路径 `run_once` 发现 IDLE agent 有 QUEUED job 后调用 `dispatch_queued_job`，落到 `db::jobs::dispatch_job_to_agent`。call site 在 `src/orchestrator/mod.rs:71-95`, `src/orchestrator/mod.rs:124-128`, `src/orchestrator/mod.rs:724-737`；事务内更新 job DISPATCHED、agent WAITING_FOR_ACK、插入 `command_received` event 并写 `dispatched_at_seq_id` 在 `src/db/jobs.rs:249-319`。
- 检测/保证：保证一个 QUEUED job 被原子认领，且 reply collection 只读 `seq_id > dispatched_at_seq_id` 的 output chunks，避免读到本次 dispatch 前的 UI 噪音。
- 测试覆盖：`tests/dispatch_atomicity.rs::dispatch_atomicity_preserves_seq_id_through_completion` (`tests/dispatch_atomicity.rs:106-149`)；`src/db/jobs.rs::test_collect_reply_for_dispatched_job_uses_seq_id_boundary` (`src/db/jobs.rs:1732-1751`)。
- 适用 provider：codex/claude/antigravity。
- 是否 provider 物理真场景：否。测试是 DB/event 层，但覆盖关键事务边界。
- 可靠度初判：高。关键限制：不能证明 tmux send 成功，只证明 DB 状态和后续 reply 边界一致。

## Stage A 发布：pane 绑定存在性 + pid revalidate 防错 pane

- 逻辑落点：`run_once` 先查 agent_io registered pane，缺失则失败补偿；存在时调用 `resolve_current_dispatch_pane`。主路径在 `src/orchestrator/mod.rs:79-95`；pane refresh/pid 校验在 `src/orchestrator/mod.rs:777-875`，其中单 live pane 时查询 DB agent pid (`:787-818`)，再 `tmux get_pane_pid` 比对 (`:819-842`)，匹配才 `agent_io::update_pane_id` (`:843-852`)。
- 检测/保证：防 KILLED slot recycle/stale pane id 导致 prompt 注入死 pane 或错 pane。只有“当前 tmux session 只有一个 live pane 且 pane pid == DB agent pid”才更新绑定。
- 测试覆盖：`tests/dispatch_atomicity.rs::dispatch_io_failure_compensates_agent_to_stuck` (`tests/dispatch_atomicity.rs:71-104`)；`src/orchestrator/mod.rs::stale_dispatch_pane_refresh_rejects_single_pane_with_wrong_pid` (`src/orchestrator/mod.rs:1354-1397`)。
- 适用 provider：codex/claude/antigravity，tmux pane 层 provider 无关。
- 是否 provider 物理真场景：部分。使用 tmux/pid 概念，但单元测试偏模拟，不是真 provider TUI。
- 可靠度初判：中高。能防错 pane；如果 list panes 失败或多 pane 歧义，会保守使用旧 pane，随后 send 失败路径兜底。

## Stage A 发布：dispatch guard 防 prompt/interstitial 前误送

- 逻辑落点：`run_once` 在 job 认领前调用 `run_dispatch_guard` (`src/orchestrator/mod.rs:113-121`)；guard 只对 prompt-handling provider 生效，调用 `scan_prompt_and_apply_outcome`，失败/Handled/Pending/Deferred 都拒绝 dispatch (`src/orchestrator/mod.rs:744-775`, `src/orchestrator/mod.rs:877-910`)。
- 检测/保证：保证 provider 当前不是 update/trust/启动交互 prompt；若能自动处理则保留 queued job 等下一轮，若无法处理则 fail-closed，不提前把 prompt 塞进错误 UI。
- 测试覆盖：`src/orchestrator/mod.rs::dispatch_guard_handled_or_error_refuses_before_job_claim` (`src/orchestrator/mod.rs:1443-1456`)；`src/orchestrator/mod.rs::dispatch_guard_capture_error_keeps_job_queued_before_log_monitor` (`src/orchestrator/mod.rs:1497-1553`)；prompt 细节在 `prompt_handler_e2e.rs` 多个 codex/claude prompt 测试覆盖。
- 适用 provider：codex/claude/antigravity 中，取决于 `is_prompt_handling_provider`；从 provider prompt handler 命名看主要覆盖 codex/claude，antigravity coverage 未见明确真物理测试。
- 是否 provider 物理真场景：部分。codex/claude 有 fixture/e2e prompt；antigravity 不足。
- 可靠度初判：中。它偏“发前阻断”，不是送达确认。

## Stage A 发布：tmux paste-buffer/send Enter 注入

- 逻辑落点：`run_once` 认领 job 后禁用 idle scan、抓 baseline、启动 log monitor，再调用 `send_text_to_pane_with_options` (`src/orchestrator/mod.rs:130-143`)。发送实现是 load-buffer/paste-buffer/可选 Enter，在 `src/agent_io/writer.rs:15-60`；底层 `tmux send-keys` 在 `src/tmux/session.rs:341-372`, async wrapper 在 `src/tmux/session.rs:602-620`。antigravity 若 prompt 以 `\n` 结尾则不额外按 Enter：`src/orchestrator/mod.rs:134-136`。
- 检测/保证：保证 prompt 经 tmux buffer 粘贴到指定 pane，并根据 provider 策略提交；发送错误会取消 log monitor、标记 dispatch IO failed、job FAILED (`src/orchestrator/mod.rs:145-197`)。但没有 post-send screen verification。
- 测试覆盖：`tests/dispatch_atomicity.rs::dispatch_io_failure_compensates_agent_to_stuck` 覆盖缺 pane补偿；writer 自身只覆盖 slash command/buffer name 判断 (`src/agent_io/writer.rs:99-127`)；tmux 发送 smoke 在 `src/tmux/mod.rs`。
- 适用 provider：codex/claude/antigravity。
- 是否 provider 物理真场景：部分。真 provider ask/pend 测试存在：`tests/mvp8_real_codex.rs::test_true_codex_ask_pend_roundtrip`、`tests/mvp11_real_codex.rs::test_codex_spawn_ask_flow`、`tests/mvp11_real_claude.rs::test_claude_spawn_ask_flow`、`tests/mvp9_real_codex_claude.rs`；未见 antigravity 真 ask/pend。
- 可靠度初判：中。tmux 命令失败可见；“粘贴进 TUI 但 Enter 被吞/未提交”没有直接送达确认，只靠 Stage B/D 后验发现。

## Stage B 开始：dispatch ACK stability 兜底 WAITING_FOR_ACK→BUSY

- 逻辑落点：发送成功后 `spawn_dispatch_ack_stability_busy` (`src/orchestrator/mod.rs:218`)；延迟 `CAPTURE_SEED_STABILITY_MS` 后从 WAITING_FOR_ACK 转 BUSY (`src/orchestrator/mod.rs:1110-1128`)。
- 检测/保证：它不是物理 busy 检测，而是 ACK 稳定窗口后把“发送命令未失败”解释为 BUSY，避免一直停在 WAITING_FOR_ACK。
- 测试覆盖：无直接测试；间接在 ask/pend 和 dispatcher lifecycle 测试中经过。
- 适用 provider：codex/claude/antigravity。
- 是否 provider 物理真场景：否。纯状态机兜底。
- 可靠度初判：中低。能防漏判开始，但可能把“prompt 粘贴了但未提交/被 TUI 吞 Enter”误判为 BUSY。

## Stage B 开始/未结束：marker matcher 防假 idle

- 逻辑落点：agent spawn 后创建 provider matcher 和 FIFO reader (`src/rpc/handlers/agent.rs:290-305`)；reader 每个 pipe chunk 更新 vt100 parser、保存 output_chunk、再 `matcher.scan` (`src/agent_io/reader.rs:125-164`)；match 后经稳定窗口调用 `mark_agent_idle_matched` (`src/agent_io/reader.rs:166-191`)。matcher 实现在 `src/marker/matcher.rs:42-77`：只看底部 6 行，先接受 `<<ah-idle:job-id=...>>`，再按 provider prompt regex 和 anti-pattern 判断。
- 检测/保证：B 阶段核心作用是“不把 working/busy UI 当 idle”，从而间接确认仍在 BUSY；codex/claude 用 ready composer + busy anti-pattern，antigravity 用 `? for shortcuts` idle 和 `esc to cancel` busy。
- 测试覆盖：codex `test_marker_matcher_codex_suppresses_idle_when_working_spinner_present`, `codex_ready_composer_with_esc_to_interrupt_is_busy`, `codex_hook_trust_modal_is_not_idle` (`src/marker/matcher.rs:224-287`)；claude `claude_working_status_with_ready_composer_is_busy`, `claude_idle_marker_accepts_try_placeholder_without_matching_non_input_prompt` (`src/marker/matcher.rs:383-433`)；antigravity `test_marker_matcher_antigravity_marks_idle_from_status_line`, `test_marker_matcher_antigravity_suppresses_idle_when_cancel_status_present`, `antigravity_real_idle_capture_matches`, `antigravity_bottom_generating_or_esc_to_cancel_is_busy` (`src/marker/matcher.rs:299-367`)；manifest calibration `test_provider_commands_and_probe_kinds_match_calibration` (`src/provider/manifest.rs:558-617`)。
- 适用 provider：codex/claude/antigravity。
- 是否 provider 物理真场景：codex/claude 多为 synthetic UI strings；antigravity 有 `REAL-a3-idle-capture.txt` fixture (`src/marker/matcher.rs:323-334`)。
- 可靠度初判：中。对“是否已开始”是反向证据：防假 idle 强，但不能单独证明模型真的开始处理。

## Stage B 开始/卡住：pane diff watcher

- 逻辑落点：`pane_diff_watcher_loop` 周期 tick (`src/pane_diff/mod.rs:246-258`)；查询 UI recapture candidates、capture pane、调用 `process_pane_diff_observations` (`src/pane_diff/mod.rs:260-287`)；清洗 spinner/status 噪音和比较实质 diff 在 `src/pane_diff/mod.rs:447-485`；无实质变化超过阈值标 STUCK (`src/pane_diff/mod.rs:317-340`)。
- 检测/保证：BUSY 后如果 pane 有实质变化，刷新 stuck timer；如果长期只有 spinner/无变化，标 STUCK。对 B 来说用于识别“看似 BUSY 但实际没进展/未开始”。
- 测试覆盖：`test_is_meaningful_diff_detects_length_growth`, `test_is_meaningful_diff_ignores_spinner_only_change`, `test_pane_diff_watcher_marks_stuck_after_threshold`, `test_pane_diff_watcher_resets_timer_on_meaningful_change` (`src/pane_diff/mod.rs:565-625` 等)；`tests/ah_dogfooding.rs` 也覆盖 stuck path。
- 适用 provider：codex/claude/antigravity；但主要承担 UI-only/provider fallback。
- 是否 provider 物理真场景：部分。核心 diff 是 synthetic；antigravity recapture 使用真实 pane fixture。
- 可靠度初判：中。能兜底“无进展”，但 antigravity burst 物理现实下可能在两次 tick 之间已完成，B 阶段不一定观察到中间 busy。

## Stage D 拿结果：pipe chunks + collect_reply

- 逻辑落点：spawn agent 时对 pane `pipe-pane -O cat > fifo` (`src/rpc/handlers/agent.rs:154-231`, `src/tmux/session.rs:319-339`)；reader 持久化 `output_chunk` (`src/agent_io/reader.rs:136-149`)；完成时 `collect_reply_for_dispatched_job_sync` 按 dispatch seq 读取 chunks、vt100 重放、`distill_reply` 去 prompt echo/UI chrome (`src/db/jobs.rs:592-688`)；async wrapper 在 `src/db/jobs.rs:1084-1094`。
- 检测/保证：保证只聚合本 job dispatch 之后的 pipe 输出；用 vt100 处理 cursor/status overwrite；记录 `chunk_count/raw_bytes_total/reply_len` 日志 (`src/db/jobs.rs:645-652`)；vt100 空屏时用 strip-ANSI raw fallback (`src/db/jobs.rs:635-643`)。
- 测试覆盖：`test_collect_reply_for_dispatched_job_uses_seq_id_boundary`, `test_collect_reply_uses_vt100_to_handle_cursor_reposition`, `test_collect_reply_handles_status_bar_overwrite`, `test_collect_reply_handles_long_output_beyond_50_lines`, `test_collect_reply_fallback_when_vt100_screen_empty` (`src/db/jobs.rs:1732-1871`)；distill 单测 `test_distill_reply_*` (`src/db/jobs.rs:1146-1215`)。
- 适用 provider：codex/claude 主路径；antigravity 只作为 UI recapture 的辅助/可能为空，因为其 D 物理来源是 pane scrape。
- 是否 provider 物理真场景：部分。chunk/vt100 测试是 synthetic；真实 codex/claude ask flow 覆盖端到端，但不是专门验证所有 ANSI/长输出边界。
- 可靠度初判：中高。对 streaming provider 合适；失败模式是 chunk 缺失、prompt-only、UI chrome 未覆盖。

## Stage D 拿结果：log/hook completion 优先

- 逻辑落点：dispatch 前为 LogAndUi provider 建立 log cursor baseline 并 spawn monitor (`src/orchestrator/mod.rs:912-970`)；log reader 只收 codex rollout jsonl 和 claude jsonl (`src/completion/reader.rs:39-49`, `src/completion/reader.rs:154-185`)；parser 提取 codex `task_complete.last_agent_message` 和 claude assistant end_turn text (`src/completion/parser.rs:18-51`, `src/completion/parser.rs:53-130`)；monitor 调 `mark_agent_idle_log_event` (`src/completion/monitor.rs:20-64`)。
- 检测/保证：codex/claude 有 log 时，log completion 是权威完成信号；状态机在 log monitor active 时拒绝 UI marker 抢先完成 (`src/db/state_machine.rs:528-543`, `src/db/state_machine.rs:359-371`)。log reply 存在则直接用 log reply；缺 reply 才 fallback 到 screen/chunks (`src/db/state_machine.rs:871-895`)。
- 测试覆盖：`src/completion/monitor.rs::monitor_wakes_orchestrator_and_notifies_job_update_on_complete` (`src/completion/monitor.rs:172-198`)，`pull_fallback_completes_when_hook_push_never_transitions` (`src/completion/monitor.rs:200-243`)；`src/db/state_machine.rs::log_event_completes_busy_job_with_log_reply`, `ui_marker_does_not_complete_dispatched_job_while_log_monitor_active`, `log_event_missing_reply_uses_screen_collection` (`src/db/state_machine.rs:1963-2148`)。
- 适用 provider：codex/claude；不适用 antigravity。
- 是否 provider 物理真场景：部分。parser 格式贴近真实 log，但多数测试是构造 jsonl；真实 codex/claude e2e 存在。
- 可靠度初判：高于 UI scrape。最大风险是 provider log 格式变更或 log root 不可用，此时退回 UI/pipe。

## Stage D 拿结果：UI-only pane recapture / distill_reply

- 逻辑落点：pane diff 对 `CompletionSignalKind::UiOnly` provider 用 provider matcher 连续稳定 tick 判断完成 (`src/pane_diff/mod.rs:125-166`)；tick 后调用 `mark_agent_idle_recaptured_with_pane` (`src/pane_diff/mod.rs:288-356`)；状态机若 chunks 空或 prompt-only，且 pane 包含 prompt，则 `distill_reply(pane_snapshot, prompt)` 作为 reply (`src/db/state_machine.rs:393-463`)。
- 检测/保证：antigravity 没 log/hook，完成和 reply 都来自最终 pane；连续稳定 tick 降低把 burst 中间态误判为完成；prompt-only 则标 STUCK，避免空交付。
- 测试覆盖：`ui_only_marker_recapture_completes_after_stable_antigravity_ticks`, `ui_only_marker_recapture_completes_real_antigravity_idle_capture_after_two_ticks`, `ui_only_marker_recapture_respects_antigravity_anti_pattern`, `ui_only_marker_recapture_ignores_historical_marker_outside_bottom_viewport`, `ui_only_marker_recapture_uses_stable_tick_counter` (`src/pane_diff/mod.rs:749-925`)；真实 pane reply fixture 覆盖 `ui_only_recapture_completes_busy_job_from_real_pane_when_chunks_prompt_only`, `ui_only_recapture_completes_busy_job_from_real_pane_when_chunks_zero`, `ui_only_recapture_completes_busy_job_from_real_wrapped_prompt_pane`, `ui_only_recapture_marks_busy_agent_stuck_when_real_pane_is_prompt_only` (`src/pane_diff/mod.rs:927-1125`)。
- 适用 provider：antigravity 主路径；理论上 gemini 也 UiOnly 但本任务排除；codex/claude log monitor active 时 UI recapture 被 defer。
- 是否 provider 物理真场景：是，antigravity 使用 `REAL-a3-*` pane capture fixture；但仍不是 live agy 进程。
- 可靠度初判：中。它是 antigravity 唯一路径，也是最脆弱路径；对 burst <64KB 一次读完的物理现实，最终 pane scrape 可行，但没有 hook/log 二次确认。

## Stage D 交付：job.wait / CLI pend

- 逻辑落点：`ah pend` 调 `job.wait` (`src/bin/ah.rs:224`, `src/bin/ah.rs:836-838`)；`handle_job_wait` 先查 terminal fast path，再订阅 job updates，终态返回 `reply_text/error_reason` (`src/rpc/handlers/jobs.rs:48-102`)；CLI 对 COMPLETED 直接打印 reply_text (`src/cli/output.rs:85-92`)。
- 检测/保证：保证用户只在 job terminal 后拿结果；pubsub lag 时会重新查 DB (`src/rpc/handlers/jobs.rs:66-69`)。当前 `src/`/`tests/` 没有独立 `mailbox/inbox/outbox` 模块；旧 mailbox 语义在当前实现中对应 SQLite `jobs` + pubsub + `job.wait`。
- 测试覆盖：`src/rpc/handlers.rs::test_handle_job_wait_fast_path_completed` (`src/rpc/handlers.rs:809-836`)；router 注册 `test_dispatch_job_wait_method_registered` (`src/rpc/router.rs:503` 附近)。
- 适用 provider：codex/claude/antigravity。
- 是否 provider 物理真场景：否。交付层 provider 无关；真 e2e ask/pend 间接覆盖 codex/claude。
- 可靠度初判：高。只要 job.reply_text 已正确写入，交付不易丢；风险在前置 D 提取。

## master_watch 重启不重装探针 bug 核实

结论：当前代码状态下，OPEN bug 仍成立。

- ahd startup 只调用 `reconcile_startup_with_tmux_socket` 后启动 orchestrator/server：`src/bin/ahd.rs:56-75`。
- startup reconcile 实现只处理 active agents：`reconcile_startup_sync_with_state_dir` 调 `reconcile_active_agents_to_crashed_sync` 和 orphan scopes (`src/db/system.rs:521-532`)；`reconcile_active_agents_to_crashed_sync` 明确选择 agent candidates、probe agent pid、重建 agent pidfd watch (`src/db/system.rs:757-787`, `src/db/system.rs:1031-1060`)。没有 session master ACTIVE 的 pidfd_open/arm。
- master watch arm 只在 RPC/spawn/cutover ACTIVE 路径：spawn master 时 `arm_revival_watch` 为 true 才 arm (`src/rpc/handlers/sessions.rs:359-410`)；cutover VERIFYING→ACTIVE 后 arm (`src/rpc/handlers/sessions.rs:890-900`)。revive 后也 arm 新 master (`src/monitor/master_watch.rs:315-325`)。
- `master_process_is_alive` 只用于 cutover readiness 检查新 master 是否提前退出 (`src/rpc/handlers/sessions.rs:455-460`, `src/rpc/handlers/sessions.rs:631-637`)；grep 未发现周期巡检或 startup reconcile 调用。
- monitor 机制本身是 pidfd readiness 一次性 async wait：`src/monitor/master_watch.rs:30-88`。如果 ahd 重启，旧 async task 消失；继承 ACTIVE master 没有重 arm，就不会 revive。
- 测试覆盖：有 `src/monitor/master_watch.rs::test_master_watch_revives_active_session_on_master_exit` (`src/monitor/master_watch.rs:1750-1800`) 和 `src/rpc/handlers/sessions.rs::spawn_master_pane_does_not_arm_revival_watch_before_active` (`src/rpc/handlers/sessions.rs:1310-1344`)；未见“ahd restart 后为 ACTIVE master 重 arm”的测试。
- 可靠度初判：bug 高可信。现有代码能处理“本进程内 arm 后 master 死亡”，不能处理“ahd 重启继承 ACTIVE master 后 master 再死”。

## 读过的关键文件

- `/tmp/research-stageABD.md`
- `src/bin/ah.rs`
- `src/rpc/handlers/jobs.rs`
- `src/rpc/handlers/agent.rs`
- `src/rpc/handlers/sessions.rs`
- `src/rpc/router.rs`
- `src/orchestrator/mod.rs`
- `src/agent_io/reader.rs`
- `src/agent_io/writer.rs`
- `src/tmux/session.rs`
- `src/db/jobs.rs`
- `src/db/state_machine.rs`
- `src/pane_diff/mod.rs`
- `src/marker/matcher.rs`
- `src/provider/manifest.rs`
- `src/completion/reader.rs`
- `src/completion/parser.rs`
- `src/completion/monitor.rs`
- `src/db/system.rs`
- `src/monitor/master_watch.rs`
- `tests/dispatch_atomicity.rs`
- 相关测试 grep 命中的 `tests/mvp*_real_*`, `tests/ah_dogfooding.rs`, `prompt_handler_e2e.rs`

## 跑过的 grep/命令原文

```sh
pwd && ls
sed -n '1,240p' /tmp/research-stageABD.md
rg --files
rg -n "resolve_current_dispatch_pane|pane pid|pane_pid|send-keys|send_keys|dispatch" src/orchestrator/mod.rs src/rpc/handlers src/bin src/tmux tests/dispatch_atomicity.rs
rg -n "busy|working|idle|marker|pane_diff|diff" src/marker src/pane_diff src/provider/manifest.rs tests
rg -n "collect_reply|chunk_count|raw_bytes_total|reply_len|distill_reply|mailbox|inbox|outbox|pipe|reply" src tests
rg -n "arm|pidfd|reconcile|master_process_is_alive|master_watch|MasterWatch" src/rpc/handlers/sessions.rs src/db/system.rs src/monitor/master_watch.rs src
rg -n "mailbox|inbox|outbox" src tests
rg -n "spawn_master_pidfd_watch_task|arm_master_revival_watch|master_process_is_alive|master_monitor_key" src tests
rg -n "test_provider_commands_and_probe_kinds_match_calibration|test_marker_matcher|antigravity_real|claude_|codex_|ui_only_recapture|collect_reply|log_event|monitor_wakes|pull_fallback|test_handle_job_submit|dispatch_" src tests
nl -ba src/orchestrator/mod.rs | sed -n '60,230p'
nl -ba src/orchestrator/mod.rs | sed -n '724,920p'
nl -ba src/tmux/session.rs | sed -n '319,380p'
nl -ba tests/dispatch_atomicity.rs | sed -n '1,170p'
nl -ba src/db/jobs.rs | sed -n '592,706p'
nl -ba src/db/state_machine.rs | sed -n '340,485p'
nl -ba src/pane_diff/mod.rs | sed -n '749,1128p'
nl -ba src/marker/matcher.rs | sed -n '224,435p'
nl -ba src/provider/manifest.rs | sed -n '320,460p'
nl -ba src/bin/ahd.rs | sed -n '45,85p'
nl -ba src/db/system.rs | sed -n '517,535p'
nl -ba src/rpc/handlers/sessions.rs | sed -n '340,465p'
```
