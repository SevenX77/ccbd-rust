# State-Contract Final E2E — Single Total-Verification Plan

**Author:** a4 (claude, e2e planning) · **Status:** design only (no test code written yet)
**Runs:** ONCE, as the pre-release gate, AFTER the whole PR1→PR4 series merges (per the operator e2e-downfrequency rule: no per-PR e2e; one final total verify).
**Deliverable target file (to be written later):** `tests/e2e_state_contract_final_a4.rs`

This plan grounds every scenario in the real RPC/CLI/DB surfaces (grepped, cited below). It reuses the isolated-`Stack` harness pattern already proven in the merged PR1 e2e (`tests/e2e_state_contract_pr1_a4.rs`).

---

## 0. Non-negotiable ground rules

1. **NEVER the live operator stack.** Every leg runs on a fully isolated stack: file-backed temp DB (`tempfile::NamedTempFile`), tempdir state dir (`tempfile::TempDir`), and a unique `tmux -L <socket>` server via `common::TmuxServerGuard` whose `Drop` kills the socket + removes the socket file. Subprocess legs additionally pin `CCB_SOCKET` and `AH_STATE_DIR` into the tempdir so no `ah`/`ahd` process can touch the operator's socket, DB, or systemd scopes.
2. **Markdown only for this task.** No cargo, no git, no test code yet.
3. **Serialize tmux legs** with `#[serial_test::serial(state_contract_final_tmux)]` (mirrors PR1's `state_contract_pr1_tmux`) and gate on `which::which("tmux")`.
4. **Every scenario carries a NON-TAUTOLOGY note** — the concrete regression that would flip the assertion red.

---

## 1. Surface map (what the test drives — all cited)

### Machine-authoritative surface (in-process, no daemon)
- `rpc::router::dispatch(line, &ctx) -> String` — `src/rpc/router.rs:50`. Entry the daemon reaches after IPC.
- `runtime.snapshot` **and** `runtime.subscribe` both route to `handle_runtime_snapshot` — `src/rpc/router.rs:108-109`, handler `src/rpc/handlers/runtime.rs:10`. `ah status --json` (PR3) is expected to map to `runtime.snapshot`; `ah events --format json` maps to `runtime.subscribe`.
- `session.list` → `handle_session_list` — `src/rpc/handlers/sessions.rs:1161`; underlying `list_session_summaries_sync` — `src/db/sessions.rs:363`. `active_agents` today is `SUM(CASE WHEN agents.state NOT IN ('CRASHED','KILLED') THEN 1 ELSE 0 END)` — `src/db/sessions.rs:370`.
- `is_terminal_session_status` — `src/rpc/handlers/sessions.rs` (design §2B: must include `"CLOSED"`).
- Snapshot job projections: `query_runtime_jobs_sync` — `src/runtime_events.rs:513` (24h terminal cutoff `now-24*60*60` at `:500`, `LIMIT 500`, workspace-scoped by `projects.absolute_path`); `query_runtime_job_events_sync` — `src/runtime_events.rs:561` (`job_event_cursor = COALESCE(MAX(job_transitions.job_event_id),0)`, events `ORDER BY job_event_id DESC LIMIT 500` then `.reverse()`, workspace-scoped).
- Cleanup fields `cleanup_required` / `safe_to_cleanup` — computed in `build_runtime_snapshot` (`src/runtime_events.rs:170-181`); exact formulas in design §3B.

### Session lifecycle (CLOSED vs FAILED)
- CLOSED writer: `mark_session_closed_after_idle_master_death` — `src/monitor/master_watch.rs:1952` → `SET status='CLOSED', master_state='IDLE', master_last_exit_reason='IDLE_MASTER_EXIT' WHERE id=? AND status='ACTIVE'`. Fires `notify_runtime_changed(InventoryChanged)`.
- Called only when classification is `IdleNoWork` — `src/monitor/master_watch.rs:614-623` (early-returns BEFORE any master respawn).
- Classification: `snapshot_master_death_session_activity` — `src/db/system.rs:172-228`. `IdleNoWork` ⇔ **no** worker in `SPAWNING|WAITING_FOR_ACK|BUSY|PROMPT_PENDING` **and** **no** job in `QUEUED|DISPATCHED`.
- **Drivable pub entry from `tests/`:** `ah::monitor::master_watch::handle_master_death_detected(&ctx, session_id, expected_pid, expected_generation, master_cmd, MasterDeathSource)` — `src/monitor/master_watch.rs:59` (both fn and `MasterDeathSource` at `:43` are `pub`; module chain `pub mod monitor` / `pub mod master_watch`).
- Genuine FAILED writers: `FUSED` (revive attempts exhausted) — `src/master_revival.rs:396-404`; `MASTER_REVIVE_WINDOW_EXPIRED` — `src/db/master_recovery.rs:485-491` via `expire_master_recovery_window_sync`. The window-expiry path is reachable from `tests/` through the **pub** `ah::db::system::reconcile_startup_with_tmux_socket` — `src/db/system.rs:1215` → `reconcile_master_recovery_windows_with_runner_sync` — `src/db/system.rs:636`.
- Migration backfill: `migrate_sessions_failed_idle_to_closed` at `db::init` — `src/db/mod.rs:264` (`FAILED + IDLE_MASTER_EXIT → CLOSED`). Already proven in the PR1 e2e; not re-driven here except as a consistency cross-check.

### Job lifecycle (transitions + events)
- RPC: `job.submit` → `handle_job_submit` (`src/rpc/handlers/jobs.rs:14`, returns `QUEUED`); `job.cancel` → `handle_job_cancel` (`:104`); `job.wait` → `handle_job_wait` (`:48`). There is **no** `job.dispatch`/`job.complete` RPC — dispatch/complete/fail are internal (orchestrator + state machine).
- DB transition functions (the seam PR2 wires `record_job_transition_conn_sync` into): `insert_job(_sync)` `src/db/jobs.rs:919/49` (→QUEUED); `dispatch_job_to_agent(_sync)` `:976/233` (QUEUED→DISPATCHED); `mark_job_completed(_conn_sync)` `:1010/374` (DISPATCHED→COMPLETED); `request_dispatched_job_cancel(_sync)` `:1041/402` (sets `cancel_requested=1`, **no status change** — the canonical `job_updated`); `mark_dispatched_job_cancelled_if_agent_idle` `:1048/431` (→CANCELLED); `mark_queued_job_cancelled(_conn_sync)` `:1027/391`; `mark_job_failed(_conn_sync)` `:1066/476`.
- `job_transitions` table + `kind` CHECK (`'job_transition'|'job_updated'`) — `src/db/schema.rs:175-191`.
- ⚠️ **BLOCKER-BY-DESIGN:** `record_job_transition_conn_sync` **does not exist in the source today**; the only `INSERT INTO job_transitions` are in `#[cfg(test)]` blocks (`src/runtime_events.rs:1032,1095`). PR2 (design §"PR 2") must add the writer and wire it into the `_conn_sync` transition functions above. **Scenario 2 is only exercisable after PR2 merges** — which is exactly the premise of this post-series total-verify.

### CLI surfaces (subprocess)
- `ah ps` → `cmd_ps` — `src/bin/ah.rs:1138` (today: no `--all`, no `status` column). `SessionRow` — `src/cli/output.rs:8-15` (today: `session_id, project_id, path, master_state, active_agents`). PR3 adds `--all`, a `status` column, `db_tracked_agents` (rename), `live_agents`.
- `ah status --json` — **does not exist today** (no `Cmd::Status`, no `cmd_status`). PR3 adds it → `runtime.snapshot`.
- `ah start` → `cmd_start` — `src/bin/ah.rs:1176` (today: `ensure_daemon_running` runs FIRST at `:1181`, config resolved later in `start_from_options` `src/cli/start.rs:49-57`). PR4 reorders so config resolves/validates before the daemon is touched. `find_config` — `src/cli/config.rs:149`; no-config error `"could not find ah.toml from {dir}; create one or set CCB_CONFIG_PATH"` — `src/cli/config.rs:298-301`.
- Subprocess drivers: `env!("CARGO_BIN_EXE_ah")` / `env!("CARGO_BIN_EXE_ahd")` (pattern in `tests/r4_attach_mapping.rs:14`, `tests/r1_master_exit_shutdown.rs`). Socket override `CCB_SOCKET` — `src/cli/rpc_client.rs:116`; state dir `AH_STATE_DIR` — `src/bin/ah.rs:742`.
- Business-logic seam (no socket needed): `start_from_options`/`start_project`/`run_up` are generic over the `RpcClient` trait; a `RecordingClient` mock exists (`src/cli/start.rs:357`, `tests/pr7_tests_first.rs:96`).

### Orphan-scope reconcile
- `reconcile_orphan_scopes_with_runner_sync(db, runner, daemon_marker, dry_run)` — `src/db/system.rs:577` (**`pub(crate)` — not reachable from `tests/`**). Live vs orphan: `active_session_and_agent_refs_sync` — `src/db/system.rs:716`; `is_own_ccbd_scope`/`is_orphan_scope` — `src/platform/linux/scope.rs:91/108`. `SystemctlRunner` trait — `src/platform/linux/scope.rs:22`. Dry-run env `CCBD_RECONCILE_DRY_RUN==1` — `src/db/system.rs:573`.
- **Wiring today:** orphan reconcile is called only from `reconcile_startup_sync_with_state_dir_and_runner` — `src/db/system.rs:543-563` (line 561), which has **no production caller**. The **async production** path `reconcile_startup_with_tmux_socket` (`:1215`, called by `ahd`) does agents+recovery-windows+socket-sweep but **not** orphan reconcile. Design §6 (PR4) wires it into that async path — but with the hardcoded `RealSystemctlRunner`.
- In-crate fakes already exist: `FakeSystemctl` (`src/db/system.rs:2661`), `RecordingSystemctl` (`:2691`), and tests `..._keeps_active_session_scope` (`:2832`, live scope survives), `..._force_mode_stops` (`:2758`, orphan reaped), `..._handles_missing_systemctl_gracefully` (`:2853`).

---

## 2. Harness architecture (reuse + extend PR1 `Stack`)

**Layer A — in-process isolated `Stack`** (verbatim reuse of `tests/e2e_state_contract_pr1_a4.rs`): temp DB + tempdir state + `TmuxServerGuard` socket; helpers `build_ctx`, `seed_session`, `seed_agent`, `seed_job`, `subscribe_snapshot` (dispatch `runtime.subscribe`/`runtime.snapshot`), `stream_first_line`, `spawn_live_agent_pane`, `db_status`, `session_by_id`. Add small helpers as needed (`seed_recovery_window`, `run_reconcile_startup`, job-lifecycle drivers calling the `src/db/jobs.rs` functions on `stack.ctx.db`). This layer carries scenarios **1, 2, 3(payload), 4(RPC), 5**.

**Layer B — full-process subprocess** on an isolated socket+state. One helper spins `ahd` (`env!("CARGO_BIN_EXE_ahd")`) with `CCB_SOCKET`/`AH_STATE_DIR` in a tempdir, waits for the socket, and returns a guard whose `Drop` sends `system.shutdown` / kills the child and removes the tempdir. `ah` subcommands (`env!("CARGO_BIN_EXE_ah")`) run against that socket. This layer carries the **CLI-rendering** legs of scenarios **3 & 4** (`ah status --json`, `ah ps` / `ah ps --all`) and all of scenario **6** (start guard — inherently about process/socket side effects). Scenario 6's negative case runs with **no daemon spun up at all**.

**Deferred — in-crate coverage:** scenario **7** (orphan reap / live-survive), because the async production reconcile path hardcodes `RealSystemctlRunner` (no fake injection from `tests/`) and the isolated harness has no real user systemd scopes. See §5.

Isolation invariant restated: no leg reads `CCB_SOCKET`/`AH_STATE_DIR`/`CCB_CONFIG_PATH` from the ambient environment; every leg sets them into its own tempdir. `EnvState { systemd_run_available: false, .. }` (as PR1) keeps Layer A off systemd entirely.

---

## 3. Scenario-by-scenario design

### Scenario 1 — CLOSED lifecycle (and CLOSED ≠ FAILED)
**Layer:** A (in-process). **tmux:** not required for the writer; required only if we also want a live-agent contrast (we don't here).

**Setup (CLOSED leg):**
1. `Stack::new()`. Seed an **ACTIVE** session `s_idle` with a project row (so `absolute_path` is workspace-consistent), a dead master pid, no agents in active states, and **no** `QUEUED|DISPATCHED` job → forces `IdleNoWork`.
2. Drive the real daemon handler: `ah::monitor::master_watch::handle_master_death_detected(&stack.ctx, "s_idle".into(), <pid>, <generation>, "<master_cmd>".into(), MasterDeathSource::<Pidfd|Patrol>)`.
3. Take a live snapshot via `stack.subscribe_snapshot().await`.

**Assertions (CLOSED leg):**
- `db_status("s_idle") == "CLOSED"` (DB truth).
- `session_by_id(&snap,"s_idle")["status"] == "CLOSED"` (live snapshot).
- `["master_last_exit_reason"] == "IDLE_MASTER_EXIT"`.
- Cascade proceeded without error (handler returns `Ok`), and `is_terminal_session_status("CLOSED")` treats it terminal (cross-checked via scenario 4's default-hide behavior).

**Setup (FAILED contrast leg):**
1. Seed an **ACTIVE** session `s_fail` with an **expired** `master_recovery_windows` row (`phase NOT IN (COMPLETED|FAILED|FUSED)`, `defer_until < now`).
2. Call the pub `ah::db::system::reconcile_startup_with_tmux_socket(stack.ctx.db.clone(), stack.state_dir.clone(), Some(socket_name))` (the same call `ahd` makes on boot).
3. Snapshot again.

**Assertions (FAILED leg):**
- `session_by_id(&snap,"s_fail")["status"] == "FAILED"` and `["master_last_exit_reason"] == "MASTER_REVIVE_WINDOW_EXPIRED"`.
- The idle session from the CLOSED leg (if seeded in the same stack) still reads `CLOSED` — **both terminal statuses coexist and are distinct**.

**NON-TAUTOLOGY:** If the CLOSED wiring regressed to the pre-PR2 behavior (writing `FAILED` for idle exits, or leaving `master_last_exit_reason` unset), the CLOSED leg's `== "CLOSED"` / `== "IDLE_MASTER_EXIT"` fails. If a regression collapsed the two terminal states (e.g. mapping every terminal to `FAILED`, or migrating the window-expiry row to `CLOSED`), the contrast leg's `s_fail == "FAILED"` **and** `s_idle == "CLOSED"` cannot both hold. A no-op handler (never firing the writer) leaves `s_idle` `ACTIVE` → fails.

**Feasibility flag:** `handle_master_death_detected` reaches the CLOSED writer only if `classify_master_death` returns `Revive` for the seeded row (normal for an ACTIVE session whose master pid is dead). If a future classification change makes a bare seeded row resolve to `Stale`/`IntentionalExit`, the idle→CLOSED flip won't fire from this entry; **fallback:** drive it from an in-crate test via the `#[cfg(test)]` `revive_master_after_exit` seam (`src/monitor/master_watch.rs:498`, already used by `idle_master_death_reaps_without_revive` at `:4448`) and keep only the snapshot-projection assertion in the external file. Confirm the reachable path once PR2 is on `main`.

---

### Scenario 2 — Job transitions populate `job_events[]` with correct edges + cursor
**Layer:** A (in-process). **Depends on PR2's `record_job_transition_conn_sync` writer.**

**Setup:**
1. `Stack::new()`; seed project + **ACTIVE** session `s_jobs` + agent `a_jobs` (state `IDLE`).
2. Drive a real transition chain through the DB lifecycle functions (the functions PR2 wires the writer into), on `stack.ctx.db`:
   - `insert_job(...)` → QUEUED (`src/db/jobs.rs:919`) — or via `job.submit` dispatch for the RPC-authentic path.
   - `dispatch_job_to_agent(...)` → DISPATCHED (`:976`).
   - `mark_job_completed(...)` → COMPLETED (`:1010`).
   - A second job: `insert_job` then `mark_queued_job_cancelled` → CANCELLED (`:1027`).
   - A third job: `insert_job` → `dispatch_job_to_agent` → `mark_job_failed` → FAILED (`:1066`).
   - A fourth (dispatched) job: `request_dispatched_job_cancel` → sets `cancel_requested=1` **without** status change (`:1041`).
3. Snapshot via `subscribe_snapshot`.

**Assertions:**
- `snap["job_events"]` is a non-empty array; the recorded edges match, in order, the driven transitions: for the completed job the events include `{old_status:"QUEUED",new_status:"DISPATCHED"}` and `{old_status:"DISPATCHED",new_status:"COMPLETED"}` with `kind=="job_transition"`; the CANCELLED and FAILED jobs each carry their terminal-edge `job_transition`.
- The cancel-request event appears as `kind=="job_updated"` with `cancel_requested==true` and **no** `new_status` change (old==new or new null) — matching design §2's "cancel-request appears as a `job_updated` event".
- `snap["job_event_cursor"]` equals the `event_id` of the **last** (highest) event in `job_events[]` (cursor = `MAX(job_event_id)`, per `src/runtime_events.rs:565`).
- `changed` arrays are populated (e.g. `["status","completed_at"]` for completion; `["cancel_requested"]` for the cancel-request update).
- `snap["jobs"]` reflects the final per-job statuses (COMPLETED/CANCELLED/FAILED and the still-dispatched cancel-requested job with `cancel_requested==true`).

**NON-TAUTOLOGY:** If PR2's writer is missing or not wired into one of the transition functions, `job_events[]` stays empty and `job_event_cursor==0` (today's behavior) → every event assertion fails. If cursor were mis-derived (e.g. a count instead of `MAX(event_id)`), the `cursor == last event_id` equality fails. If the cancel-request were mis-recorded as a `job_transition` (status edge) instead of `job_updated`, the kind assertion fails. Because we assert the exact `old→new` pairs against the driven chain, a writer that emitted rows with wrong/blank edges fails rather than passing on mere non-emptiness.

**Feasibility flag:** Full RPC-authentic dispatch/complete requires the orchestrator + state-machine + a live provider pane (not available under `CCB_TEST_SKIP_REAL_PROVIDER=1`). Driving the `src/db/jobs.rs` transition functions directly is the isolable equivalent and hits the identical writer seam. Note this substitution in the test doc-comment.

---

### Scenario 3 — `ah status --json` emits schema v2 (Studio contract), parseable
**Layer:** A for payload shape; **B** for the real CLI binary.

**Setup (A, payload):** Reuse the PR1 wire-format approach on an isolated stack seeded with a session, an agent, and (post-PR2) a job with events; `dispatch("runtime.snapshot", {})`.
**Setup (B, CLI):** Layer-B daemon seeded via RPC (`session.create`, etc.); run `ah status --json` (subprocess, isolated `CCB_SOCKET`). Capture stdout.

**Assertions:**
- `schema_version == 2` (and `!= 1`).
- Top-level `jobs` (array), `job_events` (array), `job_event_cursor` (integer) all present.
- `cleanup_required`/`safe_to_cleanup` present per session; `db_tracked_agents` and `live_agents` present; legacy `active_agents` **absent** from the machine surface (matches PR1 wire contract).
- (B) `ah status --json` stdout parses as a single JSON object equal in shape to the `runtime.snapshot` result — i.e. Studio can consume the CLI output verbatim. Exit code 0.

**NON-TAUTOLOGY:** A v1 emitter (`schema_version:1`, no `jobs`/`job_events`, or re-emitting `active_agents`) fails the pins. If PR3 wired `ah status` to a table/pretty printer instead of raw snapshot JSON, `serde_json::from_str(stdout)` fails to yield the contract object.

---

### Scenario 4 — `ah ps` default hides terminal; `--all` shows them with status column; agent-count fields consistent
**Layer:** A for the `session.list` filter + snapshot count semantics; **B** for the human table.

**Setup:**
1. Seed one **ACTIVE** session `s_live` (with a live tmux worker via `spawn_live_agent_pane`, tmux leg) and one **terminal** session `s_done` (`CLOSED` or `FAILED`).
2. (A) `dispatch("session.list", {})` and `dispatch("session.list", {"all": true})`.
3. (B) `ah ps` and `ah ps --all` subprocess against a Layer-B daemon seeded to the same shape.

**Assertions:**
- (A) default `session.list` returns `s_live` but **not** `s_done`; `{"all":true}` returns both (relies on `is_terminal_session_status` incl. `CLOSED`, design §4A / `sessions.rs`).
- (A) snapshot per-session: `db_tracked_agents == COUNT(agents NOT IN CRASHED|KILLED)` and `live_agents == COUNT(tmux_alive)`; for `s_live` with one live pane `live_agents==1`; consistency `live_agents <= db_tracked_agents` where applicable.
- (B) `ah ps` stdout does **not** contain `s_done`; `ah ps --all` contains `s_done` **and** a `status` column showing its terminal status; both render `live_agents`.
- **active_agents alias:** assert the rename is consistent everywhere — the machine snapshot exposes `db_tracked_agents` (not `active_agents`). *(Open decision — see §6: confirm whether PR3 keeps `active_agents` as a compat alias equal to `db_tracked_agents`, or drops it entirely as PR1's wire test asserts. Assertion text is finalized once that decision is fixed.)*

**NON-TAUTOLOGY:** If default filtering regressed (returns terminal sessions), the default-list `!contains(s_done)` fails. If `--all` were ignored, the all-list `contains(s_done)` fails. If `live_agents` regressed to the DB count (ignoring tmux), the live-pane case `live_agents==1` still holds but a **terminal+residual** cross-check (seed a terminal session whose agent has no live pane → `live_agents==0` while `db_tracked_agents>=1`) distinguishes them; include that pair so the two counts can't silently collapse.

---

### Scenario 5 — Cleanup signals reflect real state
**Layer:** A (tmux required for the live-residual leg). Extends the PR1 cleanup tests with the recovery-window-strict leg (operator risk #3).

**Setup & assertions (three seeded sessions in one snapshot):**
1. **ACTIVE** `s_active` → `cleanup_required==false && safe_to_cleanup==false` (design §3D). *(Already in PR1; re-assert.)*
2. **Terminal + live residual** `s_resid` (FAILED/CLOSED, agent with a real tmux pane + live pid via `spawn_live_agent_pane`) → `cleanup_required==true`, `live_agents==1`. *(PR1 covers FAILED; add a CLOSED variant to prove CLOSED is treated terminal by the cleanup formula.)*
3. **Terminal + non-terminal recovery window** `s_defer` (terminal status **plus** a `master_recovery_windows` row still in a non-terminal phase with `defer_until > now`) → `safe_to_cleanup==false` (**operator risk #3 strict**: an in-flight recovery window blocks teardown even for a terminal session). Contrast with a terminal session that has **no** window / an expired window → `safe_to_cleanup==true`.

**NON-TAUTOLOGY:** If the recovery-window join were dropped from the `safe_to_cleanup` computation (`src/runtime_events.rs:170-181`), `s_defer` would report `safe_to_cleanup==true` and the strict assertion fails — catching exactly the operator risk #3 regression. If `cleanup_required` stopped counting live tmux residual, `s_resid`'s `true` flips. The ACTIVE `false/false` anchor prevents a formula that trivially returns constants from passing.

**Feasibility flag:** tmux-gated (`which::which("tmux")`), serialized. Kill the seeded live session at end (as PR1 does).

---

### Scenario 6 — Bare `ah start` guard: config failure leaves NO daemon/socket
**Layer:** B (subprocess) — this scenario is *defined* by process/socket side effects, so it must run the real binary.

**Setup:**
1. Fresh tempdir as CWD with **no `ah.toml`** and no ancestor `ah.toml`; set `CCB_SOCKET` and `AH_STATE_DIR` into a *separate* empty tempdir; ensure `CCB_CONFIG_PATH` is unset.
2. Run `ah start` (subprocess) with that env/CWD.

**Assertions:**
- Exit status is non-zero.
- stderr contains the config-discovery error (`"could not find ah.toml"`, `src/cli/config.rs:298-301`).
- The `CCB_SOCKET` path **does not exist** (no socket file created).
- **No `ahd` process was spawned** for this state dir — assert by (a) socket absence and (b) `ah status`/`ah ps` against the same `CCB_SOCKET` immediately after also failing with "not running / connection refused" (proving nothing is listening), and/or scanning that no child `ahd` with this `AH_STATE_DIR` marker survives.
- **Positive control:** in a sibling tempdir that *does* contain a valid `ah.toml`, `ah start` proceeds and a socket appears — proving the guard rejects only the bad case, not all starts.

**NON-TAUTOLOGY:** Pre-PR4 ordering (`ensure_daemon_running` before config resolution, `src/bin/ah.rs:1181`) spawns/starts the daemon *then* fails on config → the socket exists after a failed `ah start`; the "socket does not exist" assertion catches that regression precisely. Without the positive control, a guard that rejects *every* start (even with valid config) would pass the negative case falsely.

---

### Scenario 7 — Orphan-scope reconcile: live scope survives, genuine orphan reaped (operator risk #1)
**Layer:** DEFERRED to in-crate coverage (see rationale). Documented here with the exact fallback.

**Why it is NOT drivable in the isolated external harness:**
- The reap function `reconcile_orphan_scopes_with_runner_sync` is `pub(crate)` (`src/db/system.rs:577`) — not callable from `tests/`.
- After PR4 wires it into the async `reconcile_startup_with_tmux_socket` (`:1215`), that path uses the **hardcoded `RealSystemctlRunner`** — it shells out to the operator's real `systemctl --user`. An external test cannot inject a fake, and the isolated harness has no real ccbd scopes to reap, so `list_scope_units` returns empty/`NotFound` → count 0. Nothing observable, and running it for real would risk touching operator scopes (violates ground rule 1).

**How it IS covered (targeted/in-crate, reuse existing fakes):**
- Live scope survives: `test_reconcile_orphan_scopes_keeps_active_session_scope` — `src/db/system.rs:2832` (seeds a live session/agent; asserts the matching scope is **not** stopped, count 0). This is operator risk #1's "LIVE scope SURVIVES" leg.
- Genuine orphan reaped: `test_reconcile_orphan_scopes_force_mode_stops` — `src/db/system.rs:2758` (orphan scope → `stop_unit` called). Plus `..._stops_known_agent_with_stale_marker` (`:2800`) and dry-run `..._dry_run_does_not_stop` (`:2742`).
- **PR4 wiring proof (recommended addition):** an in-crate `#[cfg(all(test,unix))]` test that drives `reconcile_startup_sync_with_state_dir_and_runner` (`:543`, already the seam holding orphan reconcile) — or, once PR4 lands, an in-crate test that injects a `RecordingSystemctl` into the wired async startup path — asserting the startup reconcile invokes orphan cleanup **after** recovery-window reconcile (ordering test at `src/db/system.rs:1949-1978` is the template) and that a seeded live session's scope survives while a seeded orphan is stopped.

**NON-TAUTOLOGY (of the deferred coverage):** the live-survive test seeds a scope whose descriptor matches a live `active_session_and_agent_refs_sync` entry; if `is_orphan_scope`/`active_session_and_agent_refs_sync` regressed to ignore liveness, that live scope would be stopped and the `stopped==0` assertion fails. The reap test seeds a scope matching *no* live ref; if the daemon-marker/known-ref gate regressed, it would skip the stop and the "`stop_unit` called" assertion fails.

**Action item for the final e2e:** the external file includes a documented `#[ignore]`/comment stub pointing at these in-crate tests, so the "total verify" file records that risk #1 is intentionally covered elsewhere and why — no silent gap.

---

## 4. Proposed test-file skeleton (plan only — do not write yet)

`tests/e2e_state_contract_final_a4.rs`, `mod common;`, reuse PR1 `Stack`. Functions:

| Fn (proposed) | Layer | tmux | Scenario |
|---|---|---|---|
| `e2e_idle_master_exit_marks_closed_not_failed` | A | no | 1 (CLOSED + FAILED contrast) |
| `e2e_job_transitions_populate_events_and_cursor` | A | no | 2 (needs PR2 writer) |
| `e2e_status_json_is_schema_v2_studio_contract` | A + B | no | 3 |
| `e2e_ps_hides_terminal_default_shows_with_all` | A + B | yes | 4 |
| `e2e_cleanup_signals_recovery_window_strict` | A | yes | 5 |
| `e2e_bare_start_no_config_leaves_no_daemon` | B | no | 6 |
| `e2e_orphan_reconcile_COVERAGE_NOTE` (doc/ignored stub) | — | — | 7 (deferred pointer) |

Shared additions to the harness: `seed_recovery_window(session_id, phase, defer_until)`, `run_reconcile_startup()`, `drive_job_chain(...)` (wrappers over `src/db/jobs.rs` fns), and a Layer-B `DaemonGuard` (spawn `ahd` on isolated `CCB_SOCKET`/`AH_STATE_DIR`, `Drop` = shutdown+cleanup).

---

## 5. Feasibility summary

| Scenario | In-process (Layer A) | Subprocess (Layer B) | Deferred / in-crate |
|---|---|---|---|
| 1 CLOSED vs FAILED | ✅ via `handle_master_death_detected` + `reconcile_startup_with_tmux_socket` (flag: Revive reachability) | — | fallback only if Revive not reachable |
| 2 job events + cursor | ✅ (after PR2 writer) | — | — |
| 3 `ah status --json` v2 | ✅ payload via `runtime.snapshot` | ✅ real CLI parse | — |
| 4 `ah ps` filter + counts | ✅ `session.list` filter + counts | ✅ human table + `--all` | — |
| 5 cleanup signals (risk #3 strict) | ✅ (tmux-gated) | — | — |
| 6 bare `ah start` guard | — | ✅ (side-effect proof) | — |
| 7 orphan reconcile (risk #1) | ❌ (`pub(crate)` + hardcoded `RealSystemctlRunner`) | ❌ (would touch real systemd) | ✅ existing `FakeSystemctl` tests `:2758/:2832` + PR4 wiring/ordering in-crate test |

---

## 6. Open decisions to confirm before writing the test (post-merge, on `main`)

1. **`active_agents` alias (scenario 4):** PR1's wire test asserts `active_agents` is **absent** (renamed to `db_tracked_agents`). The task brief lists "active_agents alias … present". Confirm the final PR3 contract: (a) drop `active_agents` entirely (assert absent), or (b) keep it as a compat alias equal to `db_tracked_agents` (assert present **and** `==db_tracked_agents`). The assertion text is trivially finalized once this is fixed; flagged so we don't hard-code the wrong one.
2. **Scenario 1 Revive reachability:** confirm a bare seeded ACTIVE session with a dead master pid resolves to `MasterDeathDecision::Revive` through `handle_master_death_detected` on merged `main`; if not, adopt the in-crate `revive_master_after_exit` fallback for the writer leg and keep only the snapshot-projection assertion external.
3. **`ah status --json` mapping (scenario 3):** confirm PR3 maps it to `runtime.snapshot` (one-shot) and prints the raw snapshot object (not a pretty/table form), so the Studio-contract parse assertion is valid.

---

## 7. What this plan deliberately does NOT do
- No per-PR e2e (operator rule) — this file is the single post-series gate.
- No reliance on real providers (`CCB_TEST_SKIP_REAL_PROVIDER=1`), real systemd scopes, or the operator's socket/DB/state.
- No raw `INSERT INTO job_transitions` to fake scenario 2 — the point is to prove PR2's **writer** fires on real transitions; seeding the table directly would be tautological.
