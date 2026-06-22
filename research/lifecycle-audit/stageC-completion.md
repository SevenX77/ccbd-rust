# Stage C: BUSY->IDLE 完成检测盘点

本报告只盘点代码中能让 agent 从 BUSY/WAITING_FOR_ACK/STUCK 进入 IDLE 的完成路径。pane hash/mtime/thinking 静默主要用于 STUCK，不是完成检测，见末尾“相关但非完成”。

## 1. Log event pull monitor

- 活路径 call site:
  - dispatch 前注册 log monitor: `src/orchestrator/mod.rs:130-132`, `src/orchestrator/mod.rs:912-969`
  - monitor 轮询日志并调用状态机: `src/completion/monitor.rs:20-45`, 完成后唤醒 orchestrator: `src/completion/monitor.rs:48-58`
  - 状态机落点: `src/db/state_machine.rs:835-951`, wrapper: `src/db/state_machine.rs:1462-1482`, `src/db/state_machine.rs:1585-1620`
- key 信号:
  - codex: JSONL `event_msg.payload.type == "task_complete"`，取 `turn_id` 和 `last_agent_message`，见 `src/completion/parser.rs:30-50`。
  - claude: JSONL assistant message `stop_reason in end_turn|stop_sequence|max_tokens` 且含 text reply，见 `src/completion/parser.rs:53-99`。注意代码把 `tool_use` 判为 `NotTerminal`，不是完成信号，和任务背景里的 “stop_reason(end_turn/tool_use)” 不一致。
  - reader 还要求 claude 完成行前曾看到 user entry，防止 stale assistant 行误完成，见 `src/completion/reader.rs:101-123`。
  - 文件范围: codex 只读 `rollout-*.jsonl`，claude 读任意 `.jsonl`，见 `src/completion/reader.rs:177-185`。
- 测试覆盖:
  - parser: `src/completion/parser.rs` 的 `codex_task_complete_emits_turn_complete`, `claude_end_turn_emits_turn_complete`, `claude_tool_use_is_busy_not_terminal`, `claude_unknown_stop_reason_warns_and_falls_back`。
  - reader: `src/completion/reader.rs` 的 `codex_task_complete_after_cursor_completes_without_user_entry`, `claude_user_entry_then_end_turn_after_cursor_completes`, `claude_user_entry_then_end_turn_across_separate_reads_completes`, `claude_stale_end_turn_across_ticks_without_user_entry_is_rejected`。
  - monitor/state: `src/completion/monitor.rs` 的 `monitor_wakes_orchestrator_and_notifies_job_update_on_complete`, `pull_fallback_completes_when_hook_push_never_transitions`; `src/db/state_machine.rs` 的 `log_event_completes_busy_job_with_log_reply`, `log_event_does_not_complete_spawning_agent`, `log_event_state_change_reason_is_log_event_task_complete`, `log_event_missing_reply_uses_screen_collection`。
- provider 适用性:
  - codex: 适用，`CompletionSignalKind::LogAndUi`，见 `src/provider/manifest.rs:347-375`。测试基本符合物理真场景。
  - claude: 适用，`CompletionSignalKind::LogAndUi`，见 `src/provider/manifest.rs:397-413`。end_turn 测试符合；tool_use 测试明确证明代码不把 tool_use 当完成。
  - antigravity: 不适用。manifest 是 `UiOnly`，见 `src/provider/manifest.rs:416-430`；reader 对非 codex/claude 日志直接返回 `NotTerminal`，见 `src/completion/parser.rs:23-27`。
- 可靠度初判: codex 高，claude 中高。依据是 log 是 provider 自有完成事件，且有 cursor、user-entry guard、schema degrade；但 claude tool_use 不完成，长工具链只能等后续 end_turn 或 fallback。

## 2. Hook push / `agent.notify`

- 活路径 call site:
  - hook 命令生成: `src/provider/home_layout.rs:552-564`
  - 只在 ctx enabled 且 provider 匹配时注入: `src/provider/home_layout.rs:582-587`
  - CLI 发 RPC: `src/bin/ah.rs:248-270`, `src/bin/ah.rs:393-412`
  - RPC route: `src/rpc/router.rs:15-30`, `src/rpc/router.rs:80-95`
  - handler 调状态机并取消 pull monitor: `src/rpc/handlers/agent.rs:520-580`
  - 状态机落点: `src/db/state_machine.rs:715-833`, wrapper: `src/db/state_machine.rs:1484-1522`
- key 信号:
  - provider hook 执行 `ah agent notify --event stop --provider ... --socket ... --hook-json`。
  - 状态机只接受 `event == "stop"`，见 `src/rpc/handlers/agent.rs:520-527`。
  - CAS 只允许 `WAITING_FOR_ACK` 或 `BUSY` -> `IDLE`，见 `src/db/state_machine.rs:743-789`。
  - CLI 当前没有把 hook stdin reply 传入 RPC params；状态机支持 `reply`，但真实 hook 命令通常会走 screen reply fallback，见 `src/db/state_machine.rs:765-777`。
- 测试覆盖:
  - 注入/命令: `tests/pr4c_hooks_plugins.rs` 的 `hook_push_context_extends_prepare_home_layout_signature`, `hook_push_disabled_context_does_not_inject_ah_hook`, `hook_push_command_bakes_agent_id_socket_and_ccb_socket`, `hook_push_command_uses_provider_timeout_units_and_hook_json`, `claude_hook_push_injection_preserves_user_hooks`, `codex_hook_push_injection_writes_hooks_json_and_feature_flag`, `antigravity_hook_push_injection_writes_global_named_stop_hook_and_preserves_settings`。
  - RPC: `src/rpc/router.rs` 的 `test_agent_notify_rejects_non_stop_event_first_release`, `test_agent_notify_success_cancels_pull_monitor_and_late_log_is_idempotent`。
  - 状态机: `src/db/state_machine.rs` 的 `hook_push_waiting_for_ack_to_idle_uses_hook_source_not_log_payload`, `hook_push_busy_to_idle_completes_dispatched_job`, `hook_push_duplicate_idle_returns_zero`, `hook_push_stale_state_version_loses_without_event`, `hook_push_cancel_requested_job_is_cancelled_like_pull_v2`, `f4_hook_push_evidence_denial_enters_pane_nudge_path`。
- provider 适用性:
  - codex: 适用，hook-push 可用；测试覆盖注入和状态机，但状态机测试用直接 `reply=HOOK PONG`，不完全等价真实 hook 命令。
  - claude: 适用，hook-push 可用；注入测试覆盖真配置形态，状态机测试多以 codex provider 字符串覆盖通用逻辑。
  - antigravity: 代码和 `ah.toml:10-13` 声明会注入，但按 PM 物理表 hook-push 对 antigravity 不可用(SIGKILL)。因此相关测试是配置/注入假场景，不应视为 antigravity 真实完成路径。
- 可靠度初判: codex/claude 中高，antigravity 不可靠/不适用。依据是 hook push 是主动通知且 CAS 幂等；但真实 reply 采集依赖 screen fallback，antigravity 物理不可用。

## 3. Live PTY marker / prompt match

- 活路径 call site:
  - agent spawn 时 reader 使用 provider matcher 和 `stability_ms`: `src/rpc/handlers/agent.rs:290-305`
  - dispatch/send 前关闭 idle scan: `src/orchestrator/mod.rs:130-132`, `src/rpc/handlers/agent.rs:637-640`
  - ACK visual diff 后重开 idle scan，并立即复扫一次: `src/rpc/handlers/ack.rs:160-204`
  - reader 每个 output chunk 后扫 vt100 屏幕，命中后直接或稳定等待后调用状态机: `src/agent_io/reader.rs:151-192`, 稳定等待落点: `src/agent_io/reader.rs:59-84`
  - 状态机落点: `src/db/state_machine.rs:496-655`, async wrapper: `src/db/state_machine.rs:1416-1422`, `src/db/state_machine.rs:1553-1583`
  - `output_chunk` 内显式 `<<ah-idle:job-id=...>>` marker 也会触发同一状态机路径: `src/db/events.rs:192-242`
- key 信号:
  - `MarkerMatcher` 的 provider prompt regex + bottom 6 viewport lines + anti-pattern，见 `src/marker/matcher.rs:59-77`, `src/marker/matcher.rs:89-95`。
  - 特殊 sentinel `<<ah-idle:job-id=...>>` 优先命中，见 `src/marker/matcher.rs:63-65`, job-id guard 在 `src/db/state_machine.rs:953-999`。
  - LogAndUi provider 若 log monitor 还在 registry 中，UI marker 会被 defer，见 `src/db/state_machine.rs:531-542`。
  - prompt-only reply 会被吞掉，防止只看到 prompt 就完成 job，见 `src/db/state_machine.rs:570-584`。
- 测试覆盖:
  - matcher: `src/marker/matcher.rs` 的 `test_marker_matcher_codex_suppresses_idle_when_working_spinner_present`, `test_marker_matcher_codex_marks_idle_when_no_spinner`, `codex_ready_composer_with_esc_to_interrupt_is_busy`, `codex_ready_composer_without_busy_anchor_is_idle_after_stability`, `codex_hook_trust_modal_is_not_idle`, `test_marker_matcher_antigravity_marks_idle_from_status_line`, `test_marker_matcher_antigravity_suppresses_idle_when_cancel_status_present`, `antigravity_real_idle_capture_matches`, `sentinel_still_wins_before_viewport_filter`。
  - reader stability: `tests/mvp7_acceptance.rs` 的 `test_stability_timer_cancels_on_noise`。
  - state/job: `src/db/state_machine.rs` 的 `ui_marker_does_not_complete_dispatched_job_while_log_monitor_active`, `test_cancel_requested_prompt_only_reply_marks_cancelled`; `tests/pr1a_evidence_statemachine.rs` 的 `physical_and_test_evidence_allows_completion`; `tests/dispatch_atomicity.rs` 的 `dispatch_atomicity_preserves_seq_id_through_completion`。
  - explicit idle marker: `src/db/events.rs` 有查询测试；真实完成路径在 `tests/ah_dogfooding.rs` 使用 `<<ah-idle:job-id=...>>` 后调用/验证 marker 完成。
- provider 适用性:
  - codex: 适用但不是主路径。manifest 为 ObservedStability + `esc to interrupt`/hook trust anti-pattern，见 `src/provider/manifest.rs:371-374`。测试覆盖了 U+2022 bullet/`esc to interrupt` 类真场景。
  - claude: 适用但不是主路径。manifest 为 ObservedStability + claude busy anti-pattern，见 `src/provider/manifest.rs:409-412`；缺少直接的 claude 真实 pane fixture 测试。
  - antigravity: matcher 适用，但 live reader 不是 PM 指定唯一路径；antigravity burst 输出下更依赖后面的 pane recapture。matcher 有真实 capture fixture 测试。
- 可靠度初判: 中。依据是它依赖 UI 字符串和 bottom viewport，但有 anti-pattern、stability、log-monitor-authoritative guard、job-id guard。

## 4. ACK capture seed direct idle

- 活路径 call site:
  - send 后启动 capture seed: `src/orchestrator/mod.rs:236-242`, `src/rpc/handlers/agent.rs:748-757`
  - capture seed 轮询 tmux pane: `src/rpc/handlers/ack.rs:13-32`, `src/rpc/handlers/ack.rs:51-84`
  - 替换画面/增量画面中若 matcher 命中，调用 `mark_agent_idle_matched`: `src/rpc/handlers/ack.rs:207-233`, `src/rpc/handlers/ack.rs:253-291`
  - 对 ObservedStability provider 禁止 direct idle: `src/rpc/handlers/ack.rs:23-24`, `src/rpc/handlers/ack.rs:380-385`
- key 信号:
  - tmux pane capture 相对 baseline 出现 meaningful diff，且 matcher 命中 idle marker/prompt。
  - 只允许 `LineEndRegex` provider 直接完成；ObservedStability provider 只用于 ACK/BUSY 判定，不直接完成。
- 测试覆盖:
  - `src/rpc/handlers.rs` 的 `test_observed_stability_capture_seed_never_marks_direct_idle`, `test_capture_seed_does_not_match_historical_prompt`, `test_capture_seed_poll_and_stability_windows`。
  - 没看到专门用真实 antigravity burst pane 走完整 `spawn_new_capture_seed -> mark_agent_idle_matched` 的测试；真实 antigravity 完成主要由 pane_diff recapture 测。
- provider 适用性:
  - codex: 不直接适用，ObservedStability 禁止 direct idle。
  - claude: 不直接适用，ObservedStability 禁止 direct idle。
  - antigravity: 适用，manifest 是 LineEndRegex，见 `src/provider/manifest.rs:426-430`。但测试覆盖不够真。
- 可靠度初判: 中低。依据是它解决 burst/replace-screen 早期窗口，但只有 5s 轮询窗口，且真实 provider 场景测试不足。

## 5. PaneDiff UiOnly recapture

- 活路径 call site:
  - watcher loop/tick: `src/pane_diff/mod.rs:246-260`
  - 只查询 BUSY 全部 agent + STUCK UiOnly agent: `src/pane_diff/mod.rs:359-368`
  - capture pane 后用 provider matcher 连续稳定 tick 判定 UI complete: `src/pane_diff/mod.rs:125-167`
  - 命中后调用状态机: `src/pane_diff/mod.rs:286-315`, `src/pane_diff/mod.rs:351-357`
  - 状态机用 pane snapshot distill reply，能 BUSY/STUCK -> IDLE，也能 prompt-only -> STUCK: `src/db/state_machine.rs:332-493`
- key 信号:
  - UiOnly provider 的完整 pane recapture。
  - `MarkerMatcher::from_manifest` + vt100 parse + bottom viewport + antigravity `esc to cancel` anti-pattern。
  - 同一 content hash 连续稳定 tick，默认 2 tick，见 `src/pane_diff/mod.rs:16`, `src/pane_diff/mod.rs:137-162`。
  - reply 从 pane snapshot 对 prompt 做 distill，解决 output_chunk 为 0 或 prompt-only 的 antigravity burst 问题，见 `src/db/state_machine.rs:393-462`。
- 测试覆盖:
  - pure recapture: `src/pane_diff/mod.rs` 的 `ui_only_marker_recapture_completes_after_stable_antigravity_ticks`, `ui_only_marker_recapture_completes_real_antigravity_idle_capture_after_two_ticks`, `ui_only_marker_recapture_respects_antigravity_anti_pattern`, `ui_only_marker_recapture_ignores_historical_marker_outside_bottom_viewport`, `ui_only_marker_recapture_uses_stable_tick_counter`。
  - real pane/job distill: `src/pane_diff/mod.rs` 的 `ui_only_recapture_completes_busy_job_from_real_pane_when_chunks_prompt_only`, `ui_only_recapture_completes_busy_job_from_real_pane_when_chunks_zero`, `ui_only_recapture_completes_busy_job_from_real_wrapped_prompt_pane`, `ui_only_recapture_marks_busy_agent_stuck_when_real_pane_is_prompt_only`, `ui_only_marker_recapture_loses_cas_after_reader_already_marked_idle`, `ui_only_recapture_candidates_include_stuck_ui_only_not_idle_or_log_provider`。
  - state STUCK recapture: `src/db/state_machine.rs` 的 `test_ui_recapture_can_mark_stuck_agent_idle_without_opening_live_marker_guard`。
- provider 适用性:
  - codex: 不适用，LogAndUi；STUCK codex 不会进入 UiOnly candidate，见 `src/pane_diff/mod.rs:359-368`。
  - claude: 不适用，LogAndUi。
  - antigravity: 适用，且这是 PM 指定唯一真实完成路径。测试用了 `REAL-a3-*` fixtures，覆盖真实 burst/prompt-only/wrapped prompt 场景。
- 可靠度初判: antigravity 中高但脆弱。依据是它符合 burst provider 物理现实，且真实 fixture 覆盖较好；风险在 UI 文案/viewport/模型状态行变化。

## 6. 相关但非 BUSY->IDLE 完成检测

- Pane hash / mtime / thinking 静默是 STUCK 检测，不会完成:
  - 逻辑: `src/pane_diff/mod.rs:185-228`, watcher 落点 `src/pane_diff/mod.rs:317-348`。
  - 测试: `src/pane_diff/mod.rs` 的 `test_query_log_mtime_changes_reset_timer`, `test_three_signals_static_marks_stuck`。
  - provider: 对能被 pane_diff 观测的 agent 都是 hang detector；不是完成路径。
- BUSY marker timeout 是 STUCK fallback，不是完成:
  - 逻辑: `src/marker/timer.rs:13-17`, `src/marker/timer.rs:92-122`。
  - 测试: `src/marker/timer.rs` 中 BUSY timeout 相关测试。
- evidence gate 是完成前置拦截，不是独立检测:
  - 逻辑: `src/db/state_machine.rs:1001-1020`，被 marker/log/hook/recapture 路径复用。
  - 测试: `tests/pr1a_evidence_statemachine.rs` 的 `missing_physical_evidence_blocks_idle_completion_and_records_deny`, `missing_test_passed_blocks_tdd_job_completion`, `physical_and_test_evidence_allows_completion`；hook denial 见 `src/db/state_machine.rs` 的 `f4_hook_push_evidence_denial_enters_pane_nudge_path`。

## Provider 物理真场景结论

- codex: 主路径是 log event `task_complete`；hook push 可用；UI marker 是后备且有 log-monitor authoritative guard。测试覆盖总体贴近真实物理，尤其 idle anti-pattern。
- claude: 主路径是 log `end_turn`/hook push；代码不把 `tool_use` 当完成。测试覆盖 log guard 和 hook 通路，但 claude 真实 pane UI fallback 覆盖偏少。
- antigravity: 代码声明 `UiOnly`，真实完成应依赖 pane recapture。hook 注入/配置虽存在，但按 PM 物理表不是真场景；log event 不适用。

## 读过的文件

- `/tmp/research-stageC.md`
- `ah.toml`
- `src/db/state_machine.rs`
- `src/completion/parser.rs`
- `src/completion/reader.rs`
- `src/completion/monitor.rs`
- `src/provider/manifest.rs`
- `src/provider/home_layout.rs`
- `src/marker/matcher.rs`
- `src/marker/timer.rs`
- `src/agent_io/reader.rs`
- `src/agent_io/mod.rs`
- `src/rpc/router.rs`
- `src/rpc/handlers.rs`
- `src/rpc/handlers/agent.rs`
- `src/rpc/handlers/ack.rs`
- `src/db/events.rs`
- `src/pane_diff/mod.rs`
- `tests/pr4c_hooks_plugins.rs`
- `tests/mvp7_acceptance.rs`
- `tests/mvp3_acceptance.rs`
- `tests/pr1a_evidence_statemachine.rs`
- `tests/dispatch_atomicity.rs`

## 跑过的 grep / shell 命令

- `pwd && ls`
- `sed -n '1,240p' /tmp/research-stageC.md`
- `rg --files`
- `rg -n "task_complete|stop_reason|hook_push|push completion|pane_stab|stability|anti_pattern|idle_anti|mtime|CompletionSignalKind|completion|recapture|BUSY|IDLE|Idle|Busy" src tests ah.toml Cargo.toml`
- `rg -n "mark_agent_idle|mark_agent_idle_from_log_event|mark_agent_idle_from_hook_event|mark_agent_idle_recaptured|marker|MARKER|LOG_EVENT|HOOK|UI_COMPLETION|distill|complete" src/db/state_machine.rs`
- `rg -n "mark_agent_idle_matched\\(|mark_agent_idle_recaptured\\(|mark_agent_idle_hook_event\\(|spawn_log_monitor_task|collect_provider_log_cursors|registry::register|CompletionSignalKind" src tests`
- `rg -n "agent.notify|handle_agent_notify|hook_push|hook-push|ah-completion-push|completion push" src tests assets docs ah.toml`
- `rg -n "stable marker|marker match|spawn_agent_io_reader|ReaderMarkerConfig|idle_scan|ah-idle|MARKER_MATCHED|prompt-only|codex_ready|antigravity_real" src/agent_io src/marker tests src/db/state_machine.rs src/db/events.rs`
- `rg -n "set_idle_scan_enabled\\([^,]+, true|idle_scan_enabled.*true|spawn_new_capture_seed|capture_seed|new_capture" src tests`
- 多次 `nl -ba ... | sed -n ...` 精确核对上文引用行号。
