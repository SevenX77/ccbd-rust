# ah / ahd Architecture Index (v1)

Status: **v1, pending r1 completeness/drift gate.** This is the required first read for any design task in this program (MD3). A design that contradicts or ignores this index, or that invents a capability already listed here, is review-rejectable.

Sources: c1 audit (`.kiro/specs/ah-modular-decoupling/c1-layer1-3-audit-2026-07-13.md`), c2 audit (`.kiro/specs/ah-modular-decoupling/c2-layer4-5-audit-2026-07-13.md`), g1 audit (`.kiro/specs/ah-modular-decoupling/g1-layer6-provider-audit-2026-07-13.md`). Seed draft `.kiro/specs/ah-modular-decoupling/o1-module-map-draft-2026-07-12.md` was the starting point only; every entry below (path, symbols, and line counts — all cross-checked with `wc -l` on 2026-07-13) was re-verified against source as of 2026-07-13 (post PR#151). r1 completeness/drift gate: REJECT → fixed → ACCEPT (see `research/architecture-index-r1-review-2026-07-13.md`); operator spot-check 2026-07-13 caught 5 stale line-counts in Layer 6 (all now aligned to `wc -l`).

## How to read the Process Axis column

Every module/entry is tagged with where it actually **runs**, not just where it compiles. Both `ah` (CLI) and `ahd` (daemon) link the same `ah` library crate (`src/lib.rs`), so compiling into the shared crate does **not** imply both processes execute it at runtime — that distinction is exactly what caused the `current_exe()` ah/ahd confusion incident (gateway bridge resolving the wrong sibling binary; fixed in `fix(gateway): resolve bridge ah_bin on main`). Values used:

- **`ah` CLI** — production code path only reachable from `src/bin/ah.rs`.
- **`ahd` daemon** — production code path only reachable from `src/bin/ahd.rs` (RPC handlers, orchestrator, monitor, provider tasks, daemon startup).
- **both** — genuinely invoked from both bins at runtime (noted per-entry which part goes where).

Binary entry anchors:
- `src/bin/ah.rs` (2131 lines): CLI `main()`; imports `ah::cli::*`, `ah::tmux::*`, `ah::systemd_unit::detect_current_scope_or_service`. Does not initialize/query `db`; talks to the daemon over RPC (`cli::rpc_client`) and journals hook events directly via `outbox`.
- `src/bin/ahd.rs` (466 lines): daemon `main()`; initializes sandbox/db/tmux/RPC context, calls `db::init`, `db::system::reconcile_startup_with_tmux_socket_and_gateway`, `outbox::cold_scan_all_agents`, `monitor::master_watch::rearm_active_master_watches_on_startup`, `orchestrator::spawn_orchestrator_task`, `rpc::run_server`.

---

## Layer 1: Entry & IPC

### `cli`
- Responsibility: CLI-side command parsing, config loading, local bootstrap/setup/doctor flows, RPC client transport, output formatting for `ah`.
- Path: `src/cli/` — `mod.rs`(16), `bundle.rs`(229), `config.rs`(1195), `config_cmd.rs`(68), `doctor.rs`(520), `logs.rs`(80), `master_cutover.rs`(393), `output.rs`(145), `prereq.rs`(224), `prompt.rs`(115), `rpc_client.rs`(375), `service_bootstrap.rs`(1134), `service_unit.rs`(207), `setup.rs`(1299), `start.rs`(644), `up.rs`(283), `wsl.rs`(686).
- Key symbols: `cli::config::{load_project_config, find_config, validate_project_config}`; `cli::rpc_client::{RpcClient, UnixRpcClient, rpc_call, rpc_stream_first, rpc_stream_lines, resolve_socket_path_for_config}`; `cli::start::{StartOptions, start_from_options, start_project, print_start_summary, build_ahd_systemd_run_command_with_parent}`; `cli::up::{UpOptions, run_up}`; `cli::master_cutover::{MasterCutoverOptions, run_master_cutover, print_master_cutover_summary}`; `cli::service_bootstrap::{RealSystemctlRunner, bootstrap_persistent_unit, collect_passthrough_env, gc_stale_units, systemd_user_bootstrap_available, detect_linger_note}`; `cli::setup::{SetupOptions, run_setup, render_setup_text, exit_code_for_overall_status}`; `cli::wsl::{detect_wsl, wsl_onboarding_checks, start_preflight_error}`.
- Process axis: **`ah` CLI**, with one narrow exception — `cli::service_unit::derive_unit_name` is also called from `ahd` startup to identify its own service unit.

### `rpc`
- Responsibility: daemon-side JSON-RPC over UDS, request routing, streaming subscriptions, handler fan-out for sessions/agents/jobs/events/runtime/system.
- Path: `src/rpc/` — `mod.rs`(136), `router.rs`(850), `handlers.rs`(1426), `handlers/ack.rs`(606), `handlers/agent.rs`(1365), `handlers/events.rs`(318), `handlers/evidence.rs`(96), `handlers/jobs.rs`(165), `handlers/master_cutover.rs`(610), `handlers/params.rs`(170), `handlers/prompt.rs`(179), `handlers/realign.rs`(689), `handlers/runtime.rs`(115), `handlers/sessions.rs`(1869), `handlers/system.rs`(22).
- Key symbols: `rpc::Ctx`, `rpc::run_server`, `rpc::router::dispatch`; `rpc::handlers::{handle_session_create, handle_session_kill, handle_session_spawn_master_pane, handle_session_master_cutover, handle_master_ack_ready, handle_master_tell_begin, handle_master_tell_failed, handle_session_realign, handle_session_list, handle_agent_spawn, handle_agent_realign, handle_agent_send, handle_agent_read, handle_agent_watch, handle_agent_kill, handle_agent_notify, handle_agent_resolve_prompt, handle_agent_learn_rule, handle_job_submit, handle_job_wait, handle_job_cancel, handle_event_subscribe, stream_event_subscribe, handle_runtime_snapshot, stream_runtime_subscribe, handle_system_dump, handle_system_shutdown}`.
- Process axis: **`ahd` daemon**. `ah` never calls `rpc::run_server`; it only talks through `cli::rpc_client`.

---

## Layer 2: Scheduling Core

### `orchestrator`
- Responsibility: daemon scheduling loop/wakeup bus; dispatches queued jobs, recovers crashed agents, starts ancillary watchers (incl. `provider::health_check`).
- Path: `src/orchestrator/mod.rs`(3054), `src/orchestrator/pubsub.rs`(69).
- Key symbols: `orchestrator::{WAKER, wake_up, spawn_orchestrator_task}`; `orchestrator::pubsub::{EventFrame, notify_job_update, subscribe_job_updates, notify_agent_output, subscribe_agent_output, notify_event, subscribe_events, notify_runtime_changed, subscribe_runtime_updates}`; `run_once` is `pub(crate)` only.
- Process axis: **`ahd` daemon** — spawned from `src/bin/ahd.rs`.

### `monitor`
- Responsibility: daemon-local process/session/master liveness monitoring, pidfd registry, cascade/revival hooks on process death; `master_watch` keeps the master-revival saga ordering while delegating DB decision/fence helpers to `master_revival` and daemon-local revive execution/reap actions to `monitor::master_reaper`.
- Path: `src/monitor/mod.rs`(95), `agent_watch.rs`(379), `master_watch.rs`(5066), `master_reaper.rs`(919), `session_watch.rs`(657).
- Key symbols: `monitor::{MonitorHandle, BorrowedMonitorHandle, pidfd_open, pidfd_send_sigkill, register, remove, with_borrowed, contains, list_keys}`; `monitor::agent_watch::spawn_agent_pidfd_watch_task`; `monitor::session_watch::{unit_name_for_session, spawn_session_watch_task}`; `monitor::master_watch::{MasterDeathSource, handle_master_death_detected, rearm_active_master_watches_on_startup, master_process_is_alive, master_watch_patrol_loop, resolve_master_watch_patrol_interval, spawn_master_pidfd_watch_task, monitor_key}`; `monitor::master_reaper::{spawn_replacement_master_pane, register_revived_master_watch_and_prepare_readiness, reap_failed_revive_master, reprovision_declared_workers_after_master_revive, write_master_revival_redispatch_marker, spawn_master_confirm_timer}`.
- Process axis: **`ahd` daemon**.

### `master_revival`
- Responsibility: pure state-transition helpers and DB decision/fence adapters for deciding/claiming/throttling/completing ah-managed master revival after master death.
- Path: `src/master_revival.rs`(1236).
- Key symbols: `MasterDeathDecision`, `MasterTransitionOutcome`, `MasterReviveAttemptDecision`, `ReviveSessionMasterRequest`, `ReviveSessionMasterOutcome`, `MasterRuntime`, `classify_master_death`, `try_claim_master_transition`, `query_master_runtime`, `master_runtime_matches`, `master_runtime_generation_matches`, `master_recovery_verifying_window_expected_generation`, `master_recovery_effective_readiness_timeout`, `begin_master_recovery_window_for_snapshot`, `mark_master_recovery_phase`, `complete_master_recovery_window_for_master_watch`, `mark_session_closed_after_idle_master_death`, `persist_revived_master_cmd`, `complete_claimed_master_transition`, `revive_session_master`, `query_master_revive_next_retry_at`, `record_master_revive_attempt`, `confirm_master_stable`, `master_spawn_lock`, `master_monitor_key`, `remove_master_monitor_key_if_generation_matches`.
- Process axis: **`ahd` daemon** — invoked by `monitor::master_watch`; no `ah` runtime path.

### `master_cutover`
- Responsibility: shared helpers + CLI/daemon surfaces for replacing an unmanaged/current master with an ah-managed one and transferring handoff context.
- Path: `src/master_cutover.rs`(296, shared helper), `src/cli/master_cutover.rs`(393, CLI wrapper), daemon handler path in `src/rpc/handlers/master_cutover.rs`.
- Key symbols: shared — `master_cutover::{HandoffBundleInput, ConversationSeedResult, claude_project_dir_key_for_cwd, claude_project_conversation_dir, write_handoff_bundle, seed_claude_project_conversation}`; CLI — `cli::master_cutover::{MasterCutoverOptions, MasterCutoverSummary, run_master_cutover, print_master_cutover_summary}`; daemon — `rpc::handlers::{handle_session_master_cutover, handle_master_ack_ready, handle_master_tell_begin, handle_master_tell_failed}` plus `rpc::handlers::sessions::spawn_master_pane_inner` for the non-cutover `session.spawn_master_pane` RPC and shared master-pane spawn helper.
- Process axis: **both, split by file**. `src/cli/master_cutover.rs` builds the `session.master_cutover` RPC request (includes `providers.claude.shared_credentials_dir`); `src/master_cutover.rs` shared helpers run daemon-side inside `rpc::handlers::master_cutover`; the actual cutover state machine/spawn/ack flow runs inside `ahd`.

---

## Layer 3: Execution Substrate

### `tmux`
- Responsibility: tmux socket/session/window/pane command wrapper, pane id model, session naming, optional systemd scope wrapping.
- Path: `src/tmux/` — `mod.rs`(427), `error.rs`(51), `pane.rs`(29), `scope.rs`(193), `session.rs`(1263).
- Key symbols: `tmux::{TmuxError, TmuxPaneId, TmuxServer, TmuxWindowSize, agent_session_name, master_session_name, compute_socket_name}`; `TmuxPaneId::parse`; `TmuxServer::{new, new_with_daemon_unit, new_with_policy, from_socket_name, socket_name, ensure_session, ensure_session_with_window_size, spawn_window, window_exists, server_running, session_exists, get_pane_pid, pipe_pane_to_fifo, send_keys_literal, send_keys_keysym, send_ctrl_c, set_pane_title, kill_pane, kill_pane_if_owned, kill_session, kill_session_if_owned, kill_window, capture_pane, list_panes, load_buffer, paste_buffer, delete_buffer, send_enter}`; `tmux::scope::{ScopePolicy, UnitConfig, wrap_in_scope, unit_name_for_socket, detect_scope_policy, detect_scope_policy_with_daemon_unit}`.
- Process axis: **both**. `ahd` owns most tmux lifecycle create/cleanup via `rpc`/`monitor`/`agent_io`/shutdown; `ah` uses tmux helpers directly for `attach`/hints/local session targeting/socket-name computation.

### `sandbox`
- Responsibility: startup environment validation, sandbox directory resolution, systemd-run command assembly for agent/master process isolation.
- Path: `src/sandbox/mod.rs`(206), `path.rs`(212), `systemd.rs`(722).
- Key symbols: `sandbox::{SandboxOverrides, ReadOnlyBind, ReadWriteBind, EnvState, check_environment}`; `sandbox::path::resolve_sandbox_dir`; `sandbox::systemd::{RecoverySpawn, wrap_command, wrap_command_with_recovery, wrap_command_with_recovery_and_sandbox_overrides, master_command, master_command_with_env}`.
- Process axis: **`ahd` daemon** (primarily). `src/bin/ahd.rs` calls `check_environment` before serving RPC and stores `EnvState` in `rpc::Ctx`; `ah` compiles the module but does not perform daemon sandbox spawning.

### `platform`
- Responsibility: OS-specific abstraction for process handles, pidfd/kqueue/stub registries, systemd/launch-service helpers, cgroup identity, scope wrapping, liveness probes.
- Path: `src/platform/mod.rs`(86) + `linux/`, `macos/`, `windows/` subtrees (identity/proc_info/process/scope/service per platform).
- Key symbols: facade `platform::{ProcessIdentity, ProcessExit, ScopeHandle, CascadeTarget, ProcessWatcher, ProcessReaper, ScopeManager, ServiceSupervisor, DaemonIdentity, ProcInfo, PlatformDiagnostics}`; `platform::sys::process::{MonitorHandle, BorrowedMonitorHandle, PIDFD_REGISTRY, pidfd_open, pidfd_send_sigkill, register, remove, with_borrowed, contains, list_keys}`; `platform::sys::proc_info::{ProcessLiveness, kill_zero_check, is_zombie_process, proc_state, waitid_exit_code, raw_fd}`; `platform::sys::scope::{RecoverySpawn, ScopeUnit, SystemctlRunner, RealSystemctlRunner, parse_systemctl_scope_units, is_own_ccbd_scope, is_orphan_scope, wrap_in_scope, unit_name_for_socket, detect_scope_policy, detect_scope_policy_with_daemon_unit, active_daemon_unit_or_none, active_daemon_unit_or_none_with_runner, wrap_command, wrap_command_with_recovery, wrap_command_with_recovery_and_sandbox_overrides, master_command, master_command_with_env}`; `platform::sys::service::{ServiceUnitError, derive_unit_name, build_ahd_systemd_run_command, build_ahd_systemd_run_command_with_parent, build_ahd_systemd_run_command_with_env, ahd_reset_failed_is_best_effort, render_unit_file, resolve_user_systemd_dir, atomic_write_unit}`; `platform::sys::identity::{detect_current_service_unit, detect_current_scope_or_service, detect_current_service_unit_from_cgroup, is_daemon_service_unit}`.
- Process axis: **both**. `ahd` uses process/scope/identity for pidfd monitoring, active-daemon-unit detection, sandbox command wrapping, scope cleanup. `ah` uses service/bootstrap helpers through `cli::start`/`cli::service_unit`/`systemd_unit` when starting/installing the daemon.

### `systemd_unit`
- Responsibility: thin public compatibility facade for detecting current ah daemon/service identity from env or platform cgroups.
- Path: `src/systemd_unit.rs`(79).
- Key symbols: `systemd_unit::{detect_current_service_unit, detect_current_scope_or_service, detect_current_service_unit_from_cgroup}`.
- Process axis: **both, mostly CLI-facing**. `src/bin/ah.rs` calls `detect_current_scope_or_service()` directly to pass a parent scope into daemon bootstrap. Daemon identity itself goes through `platform::sys::scope::active_daemon_unit_or_none` + `cli::service_unit::derive_unit_name` inside `src/bin/ahd.rs`.

### `agent_io`
- Responsibility: daemon-local tmux pane I/O runtime registry, passive FIFO byte reader tasks, text injection into registered panes.
- Path: `src/agent_io/` — `mod.rs`(104), `reader.rs`(84), `registry.rs`(652), `writer.rs`(127).
- Key symbols: `agent_io::{spawn_agent_io_reader_task, AgentIoEntry, RuntimeCleanupPolicy, register, remove, contains, pane_id, update_pane_id, init_probe_binding, set_idle_scan_enabled, cleanup_agent_runtime_resources, cleanup_agent_runtime_resources_with_policy, send_text_to_pane, send_text_to_pane_with_options, send_text_to_registered_pane, shutdown_reader}`.
- Process axis: **`ahd` daemon**. Registry populated from daemon spawn/reconcile paths; FIFO reader only emits raw byte chunks over a bounded channel to daemon perception processors; no direct DB/marker mutation and no `ah` runtime entry found.

---

## Layer 4: State & Data Persistence

### Top-level

| Name | Responsibility | Path / size | Key symbols | Process axis |
| --- | --- | --- | --- | --- |
| `state_layout` | Resolves state directory + project id from config/cwd/env overrides. | `src/state_layout.rs`, 276L | `StateLayoutRequest`, `StateLayout`, `resolve_state_layout`, `resolve_neutral_state_layout`, `resolve_state_dir_for_config`, `resolve_state_dir_for_cwd` | both — CLI resolves socket/state layout; daemon reaches it via `env::resolve_state_dir`. |
| `env` | Creates/returns effective daemon state directory. | `src/env.rs`, 9L | `resolve_state_dir` | `ahd` daemon — called from `src/bin/ahd.rs`; no CLI production caller. |
| `error` | Shared typed error surface + JSON-RPC error conversion. | `src/error.rs`, 337L | `CcbdError`, `CcbdError::to_rpc_error` | both — daemon returns via RPC; CLI converts/displays. |

### `db/*` — all **`ahd` daemon** (no direct `ah` CLI DB init/query path; CLI goes through RPC except tests)

| Name | Responsibility | Path / size | Key symbols |
| --- | --- | --- | --- |
| `db::mod` | SQLite connection wrapper, init, schema install, migrations. | `src/db/mod.rs`, 962L | `Db`, `Db::conn/try_conn/fresh_conn`, `init` |
| `db::schema` | Canonical SQLite DDL + row structs. | `src/db/schema.rs`, 355L | `SCHEMA_DDL`, `Project`, `Session`, `Agent`, `Event`, `Evidence`, `Job` |
| `db::common` | DB error mapping, constraint helpers, blocking DB task exec. | `src/db/common.rs`, 34L | `is_constraint_error`, `is_unique_constraint_error`, `map_db_error`, `spawn_db` |
| `db::agents` | CRUD/state/config lookup for agent rows. | `src/db/agents.rs`, 400L | `insert_agent_sync`, `update_agent_state_sync`, `update_agent_config_hash_sync`, `query_agent_sync`, `query_agents_by_state_sync`, `agent_exists_sync`, `query_agent_state_sync`, `delete_agent_sync` (+ async) |
| `db::agents_lifecycle` | Atomic KILLED/CRASHED transitions, crash metadata, cleanup policy, recovery intent capture. | `src/db/agents_lifecycle.rs`, 885L | `mark_agent_killed_sync`, `mark_agent_killed_for_master_death_sync`, `mark_agent_crashed_with_exit_sync` (+ async) |
| `db::events` | Event rows, query/backfill, dedup, pubsub notify. | `src/db/events.rs`, 686L | `UNKNOWN_PATTERN_STABLE`, `AH_IDLE_MARKER_PREFIX`, `insert_event_sync`, `query_events_since_sync`, `query_last_event_of_type_sync`, `query_events_backfill_sync` (+ async `insert_event_and_notify`) |
| `db::events_progress` | Send-progress events for job/agent delivery. | `src/db/events_progress.rs`, 160L | `record_send_progress_sync` (+ async) |
| `db::evidence` | Physical/test evidence records + status. | `src/db/evidence.rs`, 294L | `query_evidence_by_id_sync`, `insert_evidence_record_sync`, `has_job_evidence_sync`, `has_job_evidence_for_path_sync`, `update_evidence_status_sync`, `discard_evidence_sync` |
| `db::job_state` | CAS job status gate + legal transition enforcement. | `src/db/job_state.rs`, 1027L | `JobStatus`, `transit_job_state`, `force_cancel_pending_dispatched_job_conn_sync`, `requeue_job_state_conn_sync` |
| `db::jobs` | Job insert/dispatch/complete/fail/cancel/requeue, reply extraction, audit rows. | `src/db/jobs.rs`, 2662L | `DispatchedJob`, `dispatch_job_to_agent_sync`, `insert_job_sync`, `query_job_sync`, `claim_next_job_sync`, `mark_job_completed`, `mark_job_failed`, `request_dispatched_job_cancel`, `collect_reply_for_dispatched_job`, `set_job_evidence_requirements`, `distill_reply`, `strip_ansi_escapes` |
| `db::learned_rules` | Learned prompt-handling rules + extraction metadata validation. | `src/db/learned_rules.rs`, 506L | `LearnedRuleCategory`, `RuleFingerprint`, `CursorAnchor`, `ExtractionAnchor`, `ReplyExtractionSpec`, `LearnedRule`, `insert_learned_rule_sync`, `lookup_learned_rules_sync`, `validate_learn_rule` |
| `db::master_cutovers` | Persist/arbitrate master cutover claims/state. | `src/db/master_cutovers.rs`, 650L | `MASTER_CUTOVERS_DDL`, `MasterCutoverClaim/Update/MasterCutover`, `claim_master_cutover`, `update_master_cutover_state`, `mark_master_cutover_ack_ready`, `release_master_cutover`, `get_active_master_cutover`, `master_cutover_has_inflight_state` |
| `db::master_recovery` | Master recovery windows, readiness waits, anchor cascade decisions. | `src/db/master_recovery.rs`, 1132L | `MASTER_RECOVERY_WINDOWS_DDL`, `AnchorCascadeDecision`, `begin_master_recovery_window_sync`, `update_master_recovery_phase_sync`, `complete_master_recovery_window_sync`, `begin_master_recovery_readiness_wait_sync`, `mark_master_recovery_ready_sync`, `fail_master_recovery_readiness_sync`, `expire_master_recovery_window_sync`, `decide_anchor_cascade_sync` |
| `db::prompt_experience` | Prompt fingerprints + successful resolution actions. | `src/db/prompt_experience.rs`, 455L | `PromptFingerprintType`, `NewPromptExperience`, `PromptExperience`, `PromptExperienceLookup`, `upsert_prompt_experience_sync`, `lookup_prompt_experience_sync`, `hash_hex` |
| `db::recovery` | Spawn specs/recovery intents, interrupted job requeue, recovery backoff. | `src/db/recovery.rs`, 1619L | `AGENT_SPAWN_SPECS_DDL`, `AGENT_RECOVERY_INTENTS_DDL`, `AgentSpawnSpec`, `RecoveryBackoff`, `AgentRecoveryIntent`, `persist_agent_recovery_intent_sync`, `requeue_interrupted_job_from_captured_intent_sync`, `replace_killed_agent_and_requeue_job_sync`, `persist_agent_spawn_spec_sync`, `try_claim_agent_recovery_sync`, `record_recovery_failure_backoff_sync`, `clear_recovery_backoff_sync`, `confirm_agent_stable_sync` |
| `db::sessions` | Session CRUD, active summary, master pane/cmd/config fields, notify/tell transitions. | `src/db/sessions.rs`, 593L | `SessionSummary`, `insert_session_sync`, `create_session_sync`, `query_active_sessions_sync`, `query_session_by_cwd_sync`, `set_session_master_pane_id_sync`, `MasterNotifyTransition`, `apply_master_notify_event_sync`, `master_tell_begin_sync`, `master_tell_failed_sync` |
| `db::state_machine` | Main agent state machine (SPAWNING/IDLE/BUSY/PROMPT_PENDING/STUCK/terminal), completion acceptance. | `src/db/state_machine.rs`, 2978L | `STATE_*`, `EVIDENCE_DENY_MESSAGE`, `MarkerMatchedOutcome`, `StuckOutcome`, `is_active_state`, `is_waiting_for_ack`, `transit_agent_state_sync`, `mark_agent_waiting_for_ack_sync`, `mark_agent_prompt_pending_sync`, `mark_agent_idle_matched_sync`, `mark_agent_idle_log_event_sync`, `mark_agent_idle_hook_event_sync`, `mark_agent_stuck_sync`, `mark_agent_unknown_sync`, `mark_agent_failed_from_intervention_sync` |
| `db::state_machine_assert` | Assert/repair UNKNOWN/WAITING_FOR_ACK agents back to IDLE. | `src/db/state_machine_assert.rs`, 234L | `AssertStateOutcome`, `assert_state_to_idle_sync` (+ async) |
| `db::system` | Startup reconciliation, system dump, cascade cleanup, stale socket sweep, master-death/session cleanup. | `src/db/system.rs`, 3485L | `recovery_eligible_orphan_scope_should_be_preserved`, `system_dump_sync`, `cascade_kill_session_agents_sync`, `MasterDeathSessionActivity/Snapshot`, `WorkerRuntimeCleanupOutcome`, `snapshot_master_death_session_activity`, `clean_worker_runtime_resources_with_runner_sync`, `reconcile_startup_sync`, `reconcile_orphan_scopes_sync`, `remove_agent_sandbox_dir_sync`, `reconcile_startup_with_tmux_socket_and_gateway`, `sweep_stale_tmux_sockets_sync` |
| `db::perception::mod` | Crate-private perception gate/channel foundation (Phase 1). | `src/db/perception/mod.rs`, 52L | `pub(crate) mod events/gate/types`; `#[cfg(test)] mod phase1_acceptance` |
| `db::perception::types` | Typed perception event layer/verdict/value objects. | `src/db/perception/types.rs`, 41L | `PerceptionLayer`, `Verdict`, `PerceptionEvent` |
| `db::perception::gate` | Single intended crate-private gate for `agents.state` perception writes (CAS/version). | `src/db/perception/gate.rs`, 85L | `transit_agent_perception_state_sync` |
| `db::perception::events` | Encode/decode perception events as append-only rows in `events` table. | `src/db/perception/events.rs`, 139L | `emit_perception_event_sync`, `query_perception_events_sync` |
| `db::perception::phase1_acceptance` | (test-only) Contract tests for the perception gate/channel and CI grep rule. | `src/db/perception/phase1_acceptance.rs`, 418L | test functions only; production axis is `ahd` daemon for the code under test |

Note: `db` is not one module for indexing purposes — it is 24+ source files including several 1000–3000+ line ownership centers; treat each row above as its own decoupling-candidate unit for MD2, not "db" as a whole.

---

## Layer 5: Perception & Eventing

### `prompt_handler` — all **`ahd` daemon** (CLI `ah prompt resolve` is an RPC wrapper, does not call this module directly)

| Name | Responsibility | Path / size | Key symbols |
| --- | --- | --- | --- |
| `prompt_handler::mod` | Re-exports detection/KB/runner/resolver/schema APIs. | `src/prompt_handler/mod.rs`, 29L | modules `events, gating, integration, kb, llm_client, matcher, resolve, runner, schema, seeds`; re-exports `scan_prompt_and_apply_outcome`, `PromptScanRequest`, `PromptKb`, `handle_prompt_chain`, `resolve_prompt_with_io` |
| `prompt_handler::schema` | Prompt KB schema, actions, fingerprints, validation, regex compilation. | `src/prompt_handler/schema.rs`, 383L | `PromptResult`, `PromptHandlerError`, `PromptKb`, `PromptCase`, `PromptFingerprint`, `PromptAction`, `ValidatedAction`, `build_regex` |
| `prompt_handler::seeds` | Default prompt KB cases. | `src/prompt_handler/seeds.rs`, 117L | `default_cases` |
| `prompt_handler::kb` | Load/bootstrap/save prompt KB files atomically. | `src/prompt_handler/kb.rs`, 373L | `load_or_bootstrap_kb`, `save_kb_atomic` |
| `prompt_handler::matcher` | Sanitize pane text; match active prompt regions vs KB cases. | `src/prompt_handler/matcher.rs`, 553L | `PromptScanPurpose`, `MatchOutcome`, `match_prompt`, `match_prompt_for_scan`, `active_prompt_region`, `sanitize_pane_text` |
| `prompt_handler::gating` | Decide whether captured pane text goes through classification/handling. | `src/prompt_handler/gating.rs`, 531L | `GateContext`, `PromptGateDecision`, `GateSkipReason`, `classify_capture`, `hash_sanitized_text` |
| `prompt_handler::llm_client` | Haiku classifier abstraction + HTTP transport for unknown-prompt classification. | `src/prompt_handler/llm_client.rs`, 510L | `LlmOutcome`, `LlmError`, `LlmClassifier`, `LlmTransport`, `RealHaikuClassifier`, `UreqTransport`, `call_haiku_45_sync`, `call_haiku_45_with_transport(_and_base_url)` |
| `prompt_handler::runner` | Capture pane text, apply prompt actions, drive bounded resolution chains. | `src/prompt_handler/runner.rs`, 1900L | `PromptIo`, `TmuxPromptIo`, `RunnerContext`, `PromptRunOutcome`, `PromptSnapshot`, `handle_prompt_chain` |
| `prompt_handler::integration` | Wire prompt scanning into daemon state transitions, unknown-prompt events, PROMPT_PENDING unpark. | `src/prompt_handler/integration.rs`, 1961L | `PromptScanRequest`, `PromptScanDisposition`, `SUPPRESSION_ESCALATION_TICKS`, `PromptPendingUnparkState/TickResult`, `is_park_whitelisted`, `is_prompt_handling_provider`, `scan_prompt_and_apply_outcome`, `prompt_pending_unpark_watcher_loop/tick`, `mark_prompt_pending_and_emit_unknown`, `apply_prompt_pending_unpark_outcome_sync` |
| `prompt_handler::events` | Emit durable unknown-prompt events; hash/truncate prompt screenshots. | `src/prompt_handler/events.rs`, 187L | `UNKNOWN_PROMPT_DETECTED`, `UnknownPromptPayload`, `emit_unknown_prompt_detected(_sync)`, `truncate_pane_screenshot`, `hex_hash` |
| `prompt_handler::resolve` | Apply operator prompt-resolution actions; optionally save learned KB cases. | `src/prompt_handler/resolve.rs`, 601L | `ResolvePromptResult`, `ResolvePromptRequest`, `normalize_action_value`, `resolve_prompt_with_io` |

### `outbox`

| Name | Responsibility | Path / size | Key symbols | Process axis |
| --- | --- | --- | --- | --- |
| `outbox::mod` | Journal-first hook event files, consumption/dedup, dead-letter handling, cold replay. | `src/outbox/mod.rs`, 609L | `SELFCHECK_PREFIX`, `OutboxKind`, `OutboxRecord`, `OutboxRecord::hook_event`, `is_reserved_selfcheck_id`, `new_event_id`, `outbox_root`, `outbox_dir_for_agent`, `default_agent_outbox_dir`, `journal_record`, `ConsumeOutcome/Error`, `consume_record`, `MAX_APPLY_ATTEMPTS`, `DEAD_LETTER_DIR`, `ScanReport`, `cold_scan_dir`, `cold_scan_all_agents` | **both** — `ah` journals hook records before RPC; `ahd` consumes/cold-scans them. |

### `runtime_events`

| Name | Responsibility | Path / size | Key symbols | Process axis |
| --- | --- | --- | --- | --- |
| `runtime_events` | Build runtime topology snapshots from daemon DB/tmux state; build inactive snapshots/fingerprints for dead-daemon CLI status. | `src/runtime_events.rs`, 1431L | `RuntimeSnapshotReason`, `RuntimeState`, `RuntimeSnapshotRequest`, `RuntimeInactiveInput`, `RuntimeSnapshot`, `RuntimeSessionSnapshot`, `RuntimeAgentSnapshot`, `RuntimeJobSnapshot`, `RuntimeJobEventSnapshot`, `inactive_runtime_snapshot`, `build_runtime_snapshot`, `runtime_snapshot_fingerprint` | **both** — `ahd` builds active snapshots for RPC subscriptions; `ah` builds inactive snapshots when daemon is absent/lost. **Note:** no `write_state_snapshot` symbol exists (seed draft was wrong). |

### `marker` — all **`ahd` daemon** (runs in orchestrator/RPC/agent lifecycle paths)

| Name | Responsibility | Path / size | Key symbols |
| --- | --- | --- | --- |
| `marker::mod` | Re-exports matching/timer/perception stream APIs. | `src/marker/mod.rs`, 14L | modules `matcher, parser_registry, perception_stream, registry, timer`; re-exports `MarkerMatcher`, `MatchResult`, `PerceptionStreamConfig`, `spawn_perception_stream_processor_task`, `MarkerTimerHandle`, `PromptTimerScanContext`, `TimerKind`, `spawn_marker_timer_task(_with_prompt)` |
| `marker::matcher` | Classify vt100 screen contents as idle/prompt/unknown per provider manifest rules. | `src/marker/matcher.rs`, 488L | `MatchResult`, `MarkerMatcher::{new, from_manifest, mode, scan}` |
| `marker::parser_registry` | Live vt100 parser handles by agent id. | `src/marker/parser_registry.rs`, 72L | `ParserHandle`, `PARSER_REGISTRY`, `register`, `get`, `remove`, `contains` |
| `marker::perception_stream` | Consume passive agent output byte chunks, update vt100 parser handles, persist output events, and apply idle marker state transitions. | `src/marker/perception_stream.rs`, 184L | `PerceptionStreamConfig`, `spawn_perception_stream_processor_task` |
| `marker::registry` | Live marker timer handles by key; cancel/reset. | `src/marker/registry.rs`, 111L | `MARKER_TIMER_REGISTRY`, `register`, `take`, `reset`, `contains` |
| `marker::timer` | Spawn startup/busy marker timers and prompt-aware timer scans. | `src/marker/timer.rs`, 556L | `STARTUP_TIMEOUT`, `BUSY_TIMEOUT`, `TimerKind`, `MarkerTimerHandle`, `PromptTimerScanContext`, `spawn_marker_timer_task(_with_prompt)` |

Note: no `src/marker/registry.rs`-vs-`parser_registry.rs` collapse — seed draft only listed `registry`; both exist and are distinct.

### `completion` — all **`ahd` daemon** (orchestrator registers/cancels monitors; ticks transition daemon state)

| Name | Responsibility | Path / size | Key symbols |
| --- | --- | --- | --- |
| `completion::mod` | Declares log_layout/monitor/parser/reader/registry submodules. | `src/completion/mod.rs`, 5L | modules only |
| `completion::log_layout` | Resolve provider-specific agent log roots + unavailable reasons. | `src/completion/log_layout.rs`, 147L | `LogRootResolution`, `LogSignalUnavailable::{expect_available,expect_unavailable_reason}`, `resolve_agent_log_root` |
| `completion::parser` | Classify provider log lines and terminality, incl. pending-task checks. | `src/completion/parser.rs`, 669L | `CompletionTerminality`, `check_pending_tasks_from_log_root`, `classify_terminality`, `LogParseResult`, `parse_provider_log_line`, `provider_log_line_has_assistant_progress` |
| `completion::reader` | Read provider log tails and assistant-progress deltas with cursor state. | `src/completion/reader.rs`, 809L | `LogCursorMap`, `LogReadState::from_cursors`, `LogCompletion`, `LogTailReadResult`, `collect_provider_log_cursors`, `read_provider_log_tail(_with_state)`, `read_provider_assistant_progress_after_cursors`, `has_pending_tasks_in_transcript` |
| `completion::registry` | Active log monitor entries + cursor snapshots by agent id. | `src/completion/registry.rs`, 82L | `LogMonitorEntry`, `register`, `contains`, `cursor_snapshot`, `entry_snapshot`, `update_state`, `take`, `cancel` |
| `completion::monitor` | Periodic log monitor ticks / spawned monitor tasks. | `src/completion/monitor.rs`, 435L | `LOG_MONITOR_POLL_INTERVAL`, `MAX_LOG_MONITOR_WAIT`, `LogMonitorTickOutcome`, `run_log_monitor_tick`, `spawn_log_monitor_task` |

Note: no public `LOG_MONITORS` symbol exists (seed draft was wrong); registry API is as listed above.

### `pane_diff`

| Name | Responsibility | Path / size | Key symbols | Process axis |
| --- | --- | --- | --- | --- |
| `pane_diff::mod` | Per-agent pane content diffs, stuck detection, UI marker recapture, escalation events. | `src/pane_diff/mod.rs`, 1312L | `DEFAULT_WATCH_INTERVAL`, `DEFAULT_STUCK_THRESHOLD`, `AgentDiffState`, `UiMarkerMatchState`, `PaneDiffObservation`, `StuckSignal`, `UiCompletionRecapture`, `PaneDiffTickResult`, `process_pane_diff_observations`, `pane_diff_watcher_loop`, `escalate_pane_diff_stuck`, `escalate_pane_diff_ui_recapture`, `compute_content_hash`, `query_log_mtime`, `detect_thinking_spinner`, `resolve_stuck_watch_config`, `sanitize_for_diff`, `is_meaningful_diff` | **`ahd` daemon**. Note: no `src/pane_diff/watcher.rs` exists — everything is in `mod.rs` (seed draft was wrong). |

---

## Layer 6: Provider / Gateway / Credential

**Correction (r1 review 2026-07-13, R1):** an earlier draft of this index incorrectly claimed the gateway lived under `src/provider/`. It does not. `src/claude_gateway.rs` is a **top-level** module (`src/lib.rs:2: pub mod claude_gateway;`), namespace `crate::claude_gateway`. `src/provider/claude_gateway.rs` does not exist and `src/provider/mod.rs` does not declare it. All real callers (`monitor/agent_watch.rs`, `monitor/master_watch.rs`, `runtime_events.rs`, `db/system.rs`, `platform/linux/scope.rs`, `orchestrator/mod.rs`, `rpc/*`, `provider/health_check.rs`, `provider/manifest.rs`, `prompt_handler/integration.rs`) use `crate::claude_gateway::…`. It is listed in this Layer 6 section (not Layer 3) because it is part of the provider/credential subsystem conceptually, but its source path is top-level.

**Gateway bridge pitfall (root cause of the `current_exe` incident, now fixed):** `provider::home_layout::build_ah_hook_command` runs inside whichever process materializes hooks (commonly `ahd`); it calls `std::env::current_exe()` then `resolve_ah_binary()` to pick the **sibling `ah`** binary — deliberately not the current `ahd` executable. Any future change to hook/gateway bridging must preserve this "current materializer process ≠ binary to invoke" distinction.

| Name | Responsibility | Path / size | Key symbols | Process axis |
| --- | --- | --- | --- | --- |
| `provider::mod` | Declares/exports every provider subsystem module. | `src/provider/mod.rs`, 11L | `pub mod builtin, bundles, extensions, fingerprint, health_check, home_layout, init_probe, init_probe_task, manifest, plugins, skills` (11 modules; `claude_gateway` is NOT one of them — see below) | both — compiled for both bins, no runtime behavior itself. |
| `provider::builtin` | Embeds ah-owned kernels, default role rules, built-in skill metadata. | `src/provider/builtin.rs`, 42L | `MASTER_KERNEL`, `WORKER_KERNEL`, `DEFAULT_MASTER`, `DEFAULT_WORKER`, `BuiltinSkillScope`, `BuiltinSkill`, `BUILTIN_SKILLS` | both — `ahd` uses during master/worker home materialization (`home_layout`); `ah` reaches indirectly via config/bundle validation + library tests. |
| `provider::bundles` | Parses `.ah/bundles/<name>/bundle.toml`, validates capability support by provider/role, merges into `ExtensionConfig`, computes bundle digests. | `src/provider/bundles.rs`, 874L | `BundleRole`, `ResolvedBundles`, `BundleInspection`, `resolve_bundles_for_provider`, `digest_for_bundles`, `list_bundle_names`, `inspect_bundle` | both — `ah` via `cli::bundle` (`ah bundle list/validate`); `ahd` via `rpc/handlers/{agent,sessions,realign}.rs` before spawn/realign. |
| `claude_gateway` (top-level, NOT `provider::`) | Per-worker Claude OAuth credential gateway: reads/writes seed credentials, runs a UDS gateway core that validates worker identity and forwards/refreshes tokens upstream, and provides the in-sandbox bridge process that worker panes connect through. | `src/claude_gateway.rs`, 1129L (verified via `wc -l`, not the stale 920L estimate) | consts `INVALID_GRANT_ERROR_CODE, REFRESH_FAILED_ERROR_CODE, WORKER_ID_MISMATCH_ERROR_CODE, AUTH_INVALID_ERROR_CODE, SANDBOX_UDS_PATH, GATEWAY_SANDBOX_ROOT_ENV, FAILURE_CACHE_TTL`; types `TokenSet, GatewayRequest, GatewayResponse, GatewayError, UpstreamError, ClaudeUpstream (trait), GatewayWorkerTopology, CredentialEvent, RecordedCredentialEvents, GatewayCore<U>, ClaudeGatewayService, ProductionUpstream, GatewayListener`; fns `validate_worker_identity, fake_worker_jwt, fake_jwt_worker_id, validate_credential_path_not_wsl_windows_mount, gateway_worker_topology, read_seed_credentials, write_seed_credentials_guarded, register_worker, run_internal_bridge, bridge_wrapper_shell` | **both**, cleanly split. `ahd` owns the production `ClaudeGatewayService`/`GatewayCore` lifecycle and calls `register_worker` at Claude worker spawn (via `rpc/handlers/*`, `orchestrator`, `monitor::*`, `provider::health_check`, `db::system`, `platform::linux::scope`). `ah` CLI calls `claude_gateway::run_internal_bridge` (src/bin/ah.rs:275) — this is the in-sandbox bridge shell workers actually connect through, distinct from the daemon-side production gateway. |
| `provider::extensions` | Serialized extension surface: hooks, plugins, skills, bundles, settings, rules, MCP servers. | `src/provider/extensions.rs`, 151L | `ExtensionConfig`, `McpServerConfig`, `McpTransport`, `HookGroup`, `HookItem`, `default_matcher` | both — `ah` parses/validates project config + bundle CLI input; `ahd` consumes same schema at spawn/home materialization/hook injection/realign hashing. |
| `provider::fingerprint` | Deterministic hashes for master/agent provider config incl. hooks/plugins/skills/settings + non-empty bundle digests. | `src/provider/fingerprint.rs`, 198L | `ConfigRole`, `ConfigFingerprintInput`, `BundleDigest`, `BundleDigest::is_empty`, `BundleDigestEntry`, `compute_config_hash`, `deterministic_json` | primarily **`ahd`** runtime, shared compile. `rpc/handlers/{agent,sessions,realign}.rs` store/compare drift hashes; CLI does not own live drift decisions. |
| `provider::health_check` | Observes active agents for tmux/predicate/completion failures + queued starvation; emits alerts or marks STUCK. | `src/provider/health_check.rs`, 937L | `QUEUED_STARVATION_THRESHOLD_SECS`, `HealthCheckResult`, `HealthCheckObservation`, `health_check_observe`, `escalate_health_stuck`, `health_check_watcher_loop` | **`ahd`** daemon runtime — started from `src/orchestrator/mod.rs`; needs daemon DB/tmux/pubsub/state-machine context. |
| `provider::home_layout` | Builds provider-specific sandbox homes: auth materialization, trust files, rules/skills/plugins/hooks/MCP settings, Claude gateway env, hook push commands. | `src/provider/home_layout.rs`, 3039L | `HomeOverrides`, `HookPushContext`, `HomeLayoutRole`, `AuthMaterializationErrorCode`, `materialize_auth_file_with_ladder`, `prepare_home_layout(_with_role/_with_extensions/_with_extensions_for_slot)`, `prepare_claude_home_layout_with_gateway`, `compose_rules(_with_layers)`, `build_ah_hook_command`, `sandbox_home_for_sandbox_dir` | both, **daemon-critical**. `ahd` calls `prepare_home_layout_with_extensions_for_slot` from `rpc/handlers/{agent,sessions}.rs` and monitor revival paths before spawning panes; `ah` reaches related config/bundle/schema validation, and the generated hook command executes as `ah agent notify` — see gateway bridge pitfall above. |
| `provider::init_probe` | Deterministic pane-capture readiness predicates for bash/Codex/Claude/Antigravity startup. | `src/provider/init_probe.rs`, 279L | `InitGateProbe`, `ClaudeInitProbe`, `AntigravityInitProbe`, `CodexInitProbe`, `BashInitProbe` | **`ahd`** daemon runtime — used by `manifest::InitProbeKind::build`, `health_check_observe`, `init_probe_task` against tmux captures. |
| `provider::init_probe_task` | Async InitGate task driver: polls spawned provider panes until readiness/learned readiness/prompt intervention/timeout/unknown-stable. | `src/provider/init_probe_task.rs`, 1139L | `STABLE_UNKNOWN_STARTUP_GRACE`, `spawn_init_probe_task`, `respawn_init_probe_for_agent` | **`ahd`** daemon runtime — spawned from `rpc/handlers/agent.rs`; respawned from `rpc/handlers/prompt.rs`; needs daemon DB/tmux/prompt handler/state transitions. |
| `provider::manifest` | Provider command/auth-mount/env-passthrough/readiness/recovery-arg registry, valid names, spawn env collection. | `src/provider/manifest.rs`, 849L | `ProviderManifest`, `CompletionSignalKind`, `is_recovery_eligible_provider`, `compute_recovery_args`, `IdleDetectionMode`, `InitProbeKind::build`, `ENV_PASSTHROUGH`, `CLAUDE_INJECTED_ENV`, `CODEX_INJECTED_ENV`, `ANTIGRAVITY_INJECTED_ENV`, `OPENCODE_INJECTED_ENV`, `PANE_LOG_INJECTED_ENV`, `VALID_PROVIDER_NAMES`, `canonicalize_provider_name`, `MANIFESTS`, `get_manifest`, `try_get_manifest`, `is_valid_provider`, `valid_provider_names(_csv)`, `unknown_provider_message`, `known_provider_manifests`, `cancel_keysyms_for_provider`, `collect_spawn_env` | both — `ah` uses in config validation, doctor provider checks, setup/service env passthrough, CLI provider normalization; `ahd` uses for spawn command construction, readiness probes, recovery args, cancel keys, platform scope env collection. |
| `provider::plugins` | Parses id-only/git plugin specs, resolves provider cache paths, clones/caches git plugins. | `src/provider/plugins.rs`, 285L | `PluginSpec`, `GitUrlSpec`, `ResolvedPlugin`, `parse_plugin_spec`, `resolve_plugins_for_provider` | both — `ahd` uses through `home_layout` materializing Claude/Codex plugin dirs; `ah` can exercise same resolver via bundle/config validation. |
| `provider::skills` | Validates project skill references, resolves `.ah/skills/<name>/SKILL.md`, plans provider-specific symlink targets. | `src/provider/skills.rs`, 230L | `SkillRef`, `ResolvedSkill`, `SkillMaterialization`, `parse_skill_refs`, `resolve_project_skills`, `plan_claude_skill_materialization`, `plan_codex_skill_materialization` | both — `ah` parses config skill refs + bundle validation input; `ahd` materializes resolved project/bundle skills into sandbox homes before spawn. |
| `process_identity` | Injects per-pane `AH_ROLE`, `AH_SESSION_ID`, optional `AH_AGENT_ID` into master/worker process environments (kept distinct from daemon socket/state identity). | `src/process_identity.rs`, 82L | crate-private: `AH_AGENT_ID`, `AH_ROLE`, `AH_SESSION_ID`, `AH_ROLE_MASTER`, `AH_ROLE_WORKER`, `inject_worker_identity`, `inject_master_identity` | **`ahd`** daemon runtime — crate-private, used from `rpc/handlers/agent.rs`, `rpc/handlers/sessions.rs`, `monitor/master_watch.rs`, platform scope tests. `ah` CLI does not inject live pane identity. |

---

## Capability → Owner Map

| Capability | Owner symbol(s) | Path | Process axis |
| --- | --- | --- | --- |
| Scheduling dispatch loop + wakeup bus | `orchestrator::{spawn_orchestrator_task, WAKER, wake_up}` | `src/orchestrator/mod.rs` | `ahd` daemon |
| JSON-RPC service + routing | `rpc::run_server`, `rpc::router::dispatch` | `src/rpc/mod.rs`, `src/rpc/router.rs` | `ahd` daemon (server); `ah` CLI is a client only via `cli::rpc_client` |
| Persistence & assertion (session/agent/job state transitions, locks, invariants) | `db::state_machine::*`, `db::state_machine_assert::*` | `src/db/state_machine.rs`, `src/db/state_machine_assert.rs` | `ahd` daemon |
| Job lifecycle (dispatch/complete/fail/cancel/requeue) | `db::jobs::*`, `db::job_state::*` | `src/db/jobs.rs`, `src/db/job_state.rs` | `ahd` daemon |
| Crash recovery / interrupted-job requeue | `db::recovery::{persist_agent_recovery_intent_sync, requeue_interrupted_job_from_captured_intent_sync, replace_killed_agent_and_requeue_job_sync, try_claim_agent_recovery_sync}` | `src/db/recovery.rs` | `ahd` daemon |
| Physical process monitoring (pidfd, SIGKILL) | `monitor::*`, `platform::sys::process::*` | `src/monitor/`, `src/platform/*/process.rs` | `ahd` daemon |
| Physical terminal injection (tmux pane keystrokes) | `agent_io::writer::*`, `tmux::TmuxServer::*` | `src/agent_io/writer.rs`, `src/tmux/` | both — `ahd` owns lifecycle, `ah` uses tmux helpers for attach/hints |
| Environment sandbox validation | `sandbox::check_environment`, `sandbox::path::*` | `src/sandbox/` | `ahd` daemon |
| Systemd unit generation/management | `systemd_unit::*`, `platform::sys::service::*` | `src/systemd_unit.rs`, `src/platform/*/service.rs` | both — `ah` triggers install/start, `ahd` self-identifies its unit |
| Prompt detection/classification/resolution | `prompt_handler::integration::scan_prompt_and_apply_outcome`, `prompt_handler::runner::handle_prompt_chain` | `src/prompt_handler/` | `ahd` daemon |
| Idle/prompt/unknown vt100 marker classification | `marker::matcher::MarkerMatcher::{scan, from_manifest}` | `src/marker/matcher.rs` | `ahd` daemon |
| Stuck/pane-diff detection | `pane_diff::{process_pane_diff_observations, pane_diff_watcher_loop}` | `src/pane_diff/mod.rs` | `ahd` daemon |
| Log-based completion detection | `completion::monitor::run_log_monitor_tick` | `src/completion/monitor.rs` | `ahd` daemon |
| Event delivery guarantee (journal-first, dedup) | `outbox::{journal_record, consume_record, cold_scan_dir}` | `src/outbox/mod.rs` | both — `ah` journals, `ahd` consumes/cold-scans |
| Master self-healing (revival) | `master_revival::{classify_master_death, revive_session_master}` | `src/master_revival.rs` | `ahd` daemon |
| Master cutover (unmanaged → ah-managed handoff) | `master_cutover::write_handoff_bundle`, `rpc::handlers::handle_session_master_cutover` | `src/master_cutover.rs`, `src/rpc/handlers/master_cutover.rs` | both — `ah` builds the request, `ahd` runs the state machine |
| Runtime topology snapshot | `runtime_events::{build_runtime_snapshot, inactive_runtime_snapshot}` | `src/runtime_events.rs` | both — `ahd` builds active snapshots, `ah` builds inactive ones when daemon is down |
| Bundle parsing/validation (capability support by provider/role; NOT credential parsing — see gateway rows below) | `provider::bundles::{resolve_bundles_for_provider, digest_for_bundles, inspect_bundle}` | `src/provider/bundles.rs` | both — `ah bundle list/validate` CLI, `ahd` resolves before spawn/realign |
| Provider OAuth/auth file materialization | `provider::home_layout::materialize_auth_file_with_ladder` | `src/provider/home_layout.rs` | `ahd` daemon |
| Claude seed credential read/write | `claude_gateway::{read_seed_credentials, write_seed_credentials_guarded}` | `src/claude_gateway.rs` | `ahd` daemon |
| Provider health watching | `provider::health_check::{health_check_observe, escalate_health_stuck, health_check_watcher_loop}` | `src/provider/health_check.rs` | `ahd` daemon |
| Gateway bridging (production gateway core + per-worker registration) | `claude_gateway::{ClaudeGatewayService, GatewayCore, register_worker}` | `src/claude_gateway.rs` | both — `ahd` owns the production gateway lifecycle, `ah` runs the in-sandbox bridge process via `claude_gateway::run_internal_bridge` |
| Gateway bridge sandbox/env wiring | `provider::home_layout::{prepare_home_layout_with_extensions_for_slot, prepare_claude_home_layout_with_gateway}` | `src/provider/home_layout.rs` | `ahd` daemon |
| Hook bridge command resolution (the current_exe pitfall) | `provider::home_layout::build_ah_hook_command` (+ private `resolve_ah_binary`) | `src/provider/home_layout.rs` | `ahd` daemon (materializer), resolves a sibling `ah` binary path — see pitfall note above |
| Process identity injection | `process_identity::{inject_master_identity, inject_worker_identity}` | `src/process_identity.rs` | `ahd` daemon |
| Provider command/env registry, spawn env, recovery args, readiness kind | `provider::manifest::*` | `src/provider/manifest.rs` | both — `ah` uses it for config validation/doctor/setup, `ahd` uses it for spawn construction |
| Startup readiness (pure predicates) | `provider::init_probe::*` | `src/provider/init_probe.rs` | `ahd` daemon |
| Startup readiness (async polling/learned/intervention/respawn) | `provider::init_probe_task::{spawn_init_probe_task, respawn_init_probe_for_agent}` | `src/provider/init_probe_task.rs` | `ahd` daemon |
| Config loading/validation | `cli::config::{load_project_config, find_config, validate_project_config}` | `src/cli/config.rs` | `ah` CLI |
| State directory / project-id resolution | `state_layout::{resolve_state_layout, resolve_state_dir_for_config, resolve_state_dir_for_cwd}` | `src/state_layout.rs` | both — `ah` resolves socket/state layout, `ahd` reaches it via `env::resolve_state_dir` |

---

## Corrections against the 2026-07-12 seed draft (o1-module-map-draft)

- `db` is not one module — it is 24+ source files including several 1000–3000+ line ownership centers (`state_machine.rs`, `system.rs`, `jobs.rs`, `recovery.rs`, `master_recovery.rs`, `job_state.rs`, `agents_lifecycle.rs`, `events.rs`), plus a crate-private `db/perception/*` subtree.
- `runtime_events` has no `write_state_snapshot` symbol; real entries are `inactive_runtime_snapshot`, `build_runtime_snapshot`, `runtime_snapshot_fingerprint`.
- `pane_diff` has only `src/pane_diff/mod.rs` — no `src/pane_diff/watcher.rs`.
- `completion` has no public `LOG_MONITORS` symbol — real registry API is `register/contains/cursor_snapshot/entry_snapshot/update_state/take/cancel`.
- `src/claude_gateway.rs` **is** a top-level module (`crate::claude_gateway`) — the seed draft correctly omitted it as a `provider/` file, since it never was one. (A v1-pre-review draft of *this* index briefly claimed the opposite — `src/provider/claude_gateway.rs` — which was wrong and was caught and fixed by the r1 completeness gate on 2026-07-13; see `research/architecture-index-r1-review-2026-07-13.md` R1.) The rest of the `provider/` subsystem (~7800 lines across 11 files) plus `process_identity.rs` was **absent from the seed draft entirely** and is now Layer 6 above.
- `cli` and `db` were each collapsed into one seed-draft bullet; both are broken out per-file above (16 files under `cli/`, 24+ under `db/`).

## Freshness mechanism

- This index is updated **at each module/PR close point** as part of that PR (see `.ah/rules/master.md` collapse-point checklist) — not as a separately scheduled task. Whoever closes a module/PR touching an indexed path updates that path's row(s) in the same PR.
- MD3 (design-before-code gate): every design deliverable in this program must state, up front, which index entries it read and which it changes. r1 rejects a design that invents a capability this index shows already exists, or that omits an index update for a module it changed.
