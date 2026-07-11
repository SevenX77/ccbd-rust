# COMPLETION-REPORT — ah Completion Protocol · Group R1 + JC-1 (transport side)

- **Arm / worktree:** `/home/sevenx/coding/ccbd-rust-wt-ab-a`
- **Branch:** `ab/r1-outbox-lane` (based on `main` 97104cd)
- **Scope (frozen brief):** implement `ah-completion-protocol` **Group R1 (R1-T1..T4)** + **JC-1 (transport side)** only. Explicitly **not** R2 / G4 / evidence-gate / R3.
- **Frozen inputs used (read-only):** `research/ab-experiment-r1-outbox/frozen-spec/{design.md,requirements.md,tasks.md,convergence-provider-matrix.md}`.

---

## 1. Implementation summary

R1 is the transactional-outbox floor. It did not exist in this codebase (grep of `src/` for `outbox` / `attempt_cookie` returned nothing — arbiter-Q4's outbox was a one-line promise, never implemented), so this is **net-new transport**, wired at R1's own two seams and kept strictly out of the R2-owned state-machine.

New module **`src/outbox/mod.rs`** owns four things and nothing about `agents.state` / `jobs.status` semantics:

1. **`journal_record` (R1-T1)** — durable write: `create_dir_all` → write `{event_id}.tmp` → `fsync` file → `rename()` to `{event_id}.json` → `fsync` dir. The `rename()` is the durability commit point; every step is checked, a partial `.tmp` is cleaned on failure, and the error is propagated so the caller exits loud + non-zero.
2. **`consume_record` (JC-1)** — the consume boundary: `INSERT INTO outbox_consumed(event_id) … ON CONFLICT DO NOTHING` runs **first, for every record, before the `kind` fork**; `0` rows ⇒ `Duplicate` (drop, no effect). A first-seen record routes by kind and commits the ledger row **and** the per-kind effect in **one transaction**.
3. **`cold_scan_dir` / `cold_scan_all_agents` (R1-T2/T3)** — startup replay: enumerate `*.json` (never `.tmp`), replay `event_id`-ordered through `consume_record`, reap each file only **after** its commit, and error-book un-applyable records to `outbox/dead/`.
4. **selfcheck reconciliation (R1-T4)** — a reserved `selfcheck:` id is ordering-exempt but still takes a ledger row.

New schema (`src/db/schema.rs`, idempotent `CREATE TABLE IF NOT EXISTS`):
- `outbox_consumed(event_id TEXT PRIMARY KEY, kind, consumed_at)` — the JC-1 ledger (a real schema delta, per F-5 — **not** a `UNIQUE` on `events`, whose `event_id` is buried in JSON, nor on `job_transitions`, a disjoint AUTOINCREMENT namespace).
- `outbox_job_declaration_stub(...)` — the **clearly-labeled stub sink** for the not-yet-built F3 consumer (R2 owns the real `apply_job_done_declaration_sync → job_transitions`).
- `outbox_apply_failures(event_id, attempts, …)` — retry bookkeeping for the error-book.

Wiring:
- **`src/bin/ah.rs` `cmd_agent_notify`** now journals a durable `hook_event` record **before** the RPC. Journal failure ⇒ `CliError::Io` (non-zero exit); RPC failure **after** a successful journal ⇒ exit 0 with the allow-stop output. Outbox dir is derived from `--socket` (both sides agree) or an explicit `--outbox-dir`. Carries `AH_JOB_ID` / `AH_JOB_ATTEMPT_COOKIE` from the env into the record (read, never minted — arbiter Q4).
- **`src/bin/ahd.rs`** runs `cold_scan_all_agents` after startup-reconcile and **before** serving RPC.

`Cargo.toml`: added the `v7` feature to the existing `uuid` dep for time-ordered `event_id`s.

TDD: every mechanism landed RED-first (production fn `unimplemented!()`, real test failing through it) then GREEN — visible in the commit history (`test(...): RED` → `feat(...): GREEN` pairs).

---

## 2. Per-acceptance mapping (tasks.md R1 + JC-1)

| tasks.md item | Requirement | Where implemented | Test(s) pinning the invariant |
|---|---|---|---|
| **R1-T1** journal-first `.tmp`+`fsync`+`rename()`+dir-`fsync`; durability before RPC; **exit 0 ⇔ durable record**; journal fail ⇒ loud non-zero, RPC fail ⇒ exit-0-safe [CP-R1.1] | `journal_record`; `cmd_agent_notify` | `journal_writes_durable_json_that_round_trips`, `journal_leaves_no_tmp_and_matches_scanner_glob`, `journal_creates_missing_outbox_dir`, `journal_failure_returns_error_never_silent_ok`, `agent_notify_journals_then_exits_zero_when_rpc_is_down`, `agent_notify_exits_nonzero_when_journal_fails` |
| **R1-T2** cold-scan replay before serving + error-book quarantine for un-applyable; `event_id`-ordered replay; reserved-prefix exempt [CP-R1.3] | `cold_scan_dir` / `cold_scan_all_agents`; ahd startup wiring | `cold_scan_replays_all_records_and_reaps_after_commit`, `cold_scan_replays_in_event_id_order`, `cold_scan_quarantines_malformed_file_without_stalling_siblings`, `cold_scan_error_books_unapplyable_record_after_n_retries`, `cold_scan_all_agents_walks_every_agent_dir` |
| **R1-T3** ACK = durable DB commit; reap-after-commit; sender never blocks; **one** `.tmp`→`.json` naming (writer glob == scanner glob, F-7) [CP-R1.4] | reap only after `consume_record` Ok; `record_json_filename` / `is_scannable_json` | `cold_scan_replay_of_crash_surviving_file_does_not_double_apply` (reap-after-commit + no double-apply), `journal_leaves_no_tmp_and_matches_scanner_glob` (naming), `default_outbox_dir_derives_from_socket_parent` (writer==scanner path) |
| **R1-T4** reconcile selfcheck `event_id`: ordering-exempt + **takes a ledger row** (crash-surviving selfcheck re-scans as no-op) [CP-R1.3/DF-A4] | `Selfcheck` no-op sink in `consume_record`; sort exemption in `cold_scan_dir` | `cold_scan_selfcheck_is_noop_ledgered_and_order_exempt`, `selfcheck_id_is_recognized_as_reserved` |
| **JC-1** single transport ledger `outbox_consumed(event_id PK)`, `ON CONFLICT DO NOTHING` **before the kind fork**, dedup+effect one tx; covers **both** F2 `events` and F3 `job_transitions` paths; F3 consumer stubbed [CP-R1.2] | `outbox_consumed`; `consume_record` (dedup before `match kind`); `apply_hook_event` (F2), `apply_job_declaration_stub` (F3 stub) | `jc1_dedup_f3_job_done_no_double_apply`, `jc1_dedup_f2_hook_event_no_double_apply`, `jc1_dedup_is_keyed_on_event_id_not_job`, `jc1_dedup_and_effect_are_one_transaction` |

**Completion-definition invariants (brief §完成定义.2), each test-pinned:**
- *exit 0 ⇔ durable outbox record; journal-commit failure ⇒ non-zero* → `agent_notify_exits_nonzero_when_journal_fails` + `journal_failure_returns_error_never_silent_ok`.
- *redelivery does not double-apply (`outbox_consumed`, replay no double-apply)* → the four `jc1_*` tests + `cold_scan_replay_of_crash_surviving_file_does_not_double_apply`.
- *cold-scan replay: post-kill-9-restart replay with no holes + error-book isolates un-applyable (DF-A1 automated approximation)* → the six `cold_scan_*` tests.

---

## 3. Test run output (excerpts)

```
$ CARGO_BUILD_JOBS=1 cargo test --lib outbox:: -- --test-threads=1
test outbox::tests::cold_scan_all_agents_walks_every_agent_dir ... ok
test outbox::tests::cold_scan_error_books_unapplyable_record_after_n_retries ... ok
test outbox::tests::cold_scan_quarantines_malformed_file_without_stalling_siblings ... ok
test outbox::tests::cold_scan_replay_of_crash_surviving_file_does_not_double_apply ... ok
test outbox::tests::cold_scan_replays_all_records_and_reaps_after_commit ... ok
test outbox::tests::cold_scan_replays_in_event_id_order ... ok
test outbox::tests::cold_scan_selfcheck_is_noop_ledgered_and_order_exempt ... ok
test outbox::tests::default_outbox_dir_derives_from_socket_parent ... ok
test outbox::tests::jc1_dedup_and_effect_are_one_transaction ... ok
test outbox::tests::jc1_dedup_f2_hook_event_no_double_apply ... ok
test outbox::tests::jc1_dedup_f3_job_done_no_double_apply ... ok
test outbox::tests::jc1_dedup_is_keyed_on_event_id_not_job ... ok
test outbox::tests::journal_creates_missing_outbox_dir ... ok
test outbox::tests::journal_failure_returns_error_never_silent_ok ... ok
test outbox::tests::journal_leaves_no_tmp_and_matches_scanner_glob ... ok
test outbox::tests::journal_writes_durable_json_that_round_trips ... ok
test outbox::tests::record_union_wire_form_round_trips_for_job_done ... ok
test outbox::tests::selfcheck_id_is_recognized_as_reserved ... ok
test result: ok. 18 passed; 0 failed; 0 ignored; 0 measured; 1033 filtered out

$ CARGO_BUILD_JOBS=1 cargo test --bin ah agent_notify -- --test-threads=1
test tests::agent_notify_exits_nonzero_when_journal_fails ... ok
test tests::agent_notify_journals_then_exits_zero_when_rpc_is_down ... ok
test result: ok. 5 passed; 0 failed; ...
```

**Full workspace (serial), live-provider e2e skipped via the sanctioned `CCB_TEST_SKIP_REAL_PROVIDER=1`:**
```
$ CCB_TEST_SKIP_REAL_PROVIDER=1 CARGO_BUILD_JOBS=1 cargo test --workspace -- --test-threads=1
lib:  test result: ok. 1048 passed; 0 failed; 3 ignored
(all integration suites green; ~1501 tests total across the workspace, 0 failures)
```

**Live-provider note (honest):** a single full-real run of `tests/mvp11_real_codex.rs::test_codex_spawn_ask_flow` failed once at `wait_job`'s 120 s `handle_job_wait().unwrap()` — a timeout waiting on the **live external codex model**. Re-run in isolation it **passes in 14.6 s**. This is unrelated to R1 (the completion path rides the unchanged RPC; the journal is additive) and its passing is positive evidence that the journal-first hook still completes a real job end-to-end through a real sandboxed codex agent — had the journal errored, the hook would exit before the RPC and the job would never complete. Per the repo discipline ("本机全量测试禁跑,CI 是唯一全量门"), live-provider tests are non-deterministic and CI-gated.

---

## 4. Design-vs-reality conflicts and how they were handled

1. **The F2 apply path is R2-entangled — JC-1's "events path" could not reuse it as-is.** `mark_agent_idle_hook_event_outcome_sync` (`state_machine.rs:581`) opens its own transaction and is fused with R2/evidence-gate logic (`mark_job_completed_conn_sync`, `collect_reply_for_dispatched_job_sync`, `classify_terminality`, `evidence_denial_for_job`) — all explicitly out of scope. JC-1 requires dedup+effect in **one** tx, which calling that function (its own tx) would break, and rerouting it is literally R2-T2's "load-bearing refactor."
   **Handling (design intent preserved):** the funnel's F2 effect lands the durable event on the `events` spine (`event_type='hook_event'`) within its own tx — a faithful slice of "the F2 events path" — and deliberately does **not** run the R2-owned state/completion logic. The live RPC idle-marker path is left untouched for R2-T2 to reroute through this same deduped boundary. Documented inline on `apply_hook_event`.

2. **F3 (`job_done`) consumer does not exist yet (brief-acknowledged).** Handled exactly as the brief authorizes: the dedup gate is built **before the kind fork** and both kinds' dedup semantics are tested; F3's apply point is a **clearly-labeled tested stub** (`apply_job_declaration_stub` → `outbox_job_declaration_stub`, with a schema comment marking it for replacement by R2's `apply_job_done_declaration_sync → job_transitions`).

3. **Outbox location: design says `{agent_home}/outbox/`, but arbiter-Q4's `agent_home`/outbox is unimplemented.** I pinned the convention as `{state_dir}/outbox/{agent_id}/`, derived identically on both sides from the `--socket` path both already share (`default_agent_outbox_dir` == `outbox_dir_for_agent`), so writer glob == scanner glob (F-7). An explicit `--outbox-dir` override exists for the eventual sandbox-mapped path.

4. **Sandbox outbox write-access (arbiter-Q4 escalation, out of scope).** Design R1-Q1 says an inaccessible outbox is "a loud escalation to the sandbox design, not a route-around." I kept that **fail-closed** posture (journal failure ⇒ loud non-zero, no silent RPC fallback). The real-codex e2e passing confirms the derived path is writable from the sandbox in this environment; provisioning a guaranteed host-visible outbox mount for every sandbox model remains arbiter-Q4's job, flagged in §5.

5. **`event_id` minting.** Design says the hook mints a ULID/UUIDv7. I used `uuid::Uuid::now_v7()` (time-prefixed ⇒ scan order ≈ fire order). The CLI mints one when `--event-id` is absent and passes the same id to both the outbox record and the RPC params, so the fast-path and cold-scan share one idempotency key.

---

## 5. Self-assessed residuals (in-scope boundaries / handed to later stages)

- **Steady-state (inotify) consume + reap-on-RPC-success is not wired.** In this PR, outbox files are consumed/reaped **only** on ahd startup cold-scan (which is exactly R1-T2's acceptance and the DF-A1 shape). The design's steady-state inotify consumer + "reap immediately on RPC success" reroute the live RPC through the ledger — that is R2/later, and I deliberately did not touch the R2-owned handler. Consequence: between daemon restarts, outbox files (and `outbox_consumed` rows) accumulate; they are drained on the next restart. Ledger **retention/reap policy** is an explicitly-deferred Track-A open item (design §"From Track A's own deferred list").
- **Sandbox host-visible outbox provisioning** (arbiter Q4) — see conflict §4.4. The `--outbox-dir` override is the injection point.
- **F3 real apply** (`apply_job_done_declaration_sync → job_transitions`, `reason=explicit_done|explicit_fail`) is R2-T2's; the stub sink is its placeholder.
- **`selfcheck` producer** (the G4 medium-tier synthetic round-trip that *writes* selfcheck records) is G4's; R1 only implements the **consume-side** contract (no-op sink + ledger row + ordering exemption) so a selfcheck record replays correctly once G4 emits it.
- **Cold-scan sweep cadence / periodic inotify-miss backstop** — a tuning value subordinate to arbiter Q2, not implemented here (cold-scan-on-restart is the required piece).
- **Windows dir-`fsync`**: `fsync_dir` tolerates `EINVAL` on directory fds; the module is exercised on Linux (the target). Non-Linux durability nuances are untested here.

---

## 6. Commit trail (RED → GREEN)

```
test(r1-t1): RED — journal-first outbox write contract
feat(r1-t1): journal-first outbox write — GREEN
test(jc-1): RED — transport dedup ledger contract (both kinds + atomicity)
feat(jc-1): transport dedup ledger + consume funnel — GREEN
test(r1-t2,t3,t4): RED — cold-scan replay / reap / dead-letter / ordering / selfcheck
feat(r1-t2,t3,t4): cold-scan replay / reap / dead-letter / ordering — GREEN
feat(r1-t1,t2): wire journal-first into ah agent notify + cold-scan into ahd startup
```
