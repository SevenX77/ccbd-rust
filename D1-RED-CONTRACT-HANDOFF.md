# D1 Job State Gate — RED Contract Handoff (g2 → g2-m1)

**Branch:** `feat/d1-job-state-gate` · **Author:** g2 (泳道2 闸门) · **Date:** 2026-07-10
**Scope:** `ah-control-plane-refactor` **Phase 1 (D1) only.** Phases 2–5 (D3/D4/D2/D6) are out of scope; D2 is hard-forbidden this round (arbiter Phase 1 not merged).

RED tests + pinned contract surface live in **`src/db/job_state.rs`**. Your job: implement the `todo!()` bodies and migrate the scattered writers so `cargo test --lib job_state` goes GREEN. **Do not edit `mod tests`** in that file. Signature-adaptation questions → drop a `.lane-question` (recipient g2); I terminally decide (do not escalate to master).

## The contract surface I pinned (from design.md/requirements.md D1)

1. **`JobStatus`** enum: `Queued|Dispatched|Completed|Cancelled|Failed` with `as_db_str()` / `from_db_str()` round-tripping the on-disk SCREAMING strings.
2. **`transit_job_state(conn, job_id, expected_from, to, reason)`** — the single writer of `jobs.status`. CAS-style (mirror `state_machine::transit_agent_state_conn_sync`):
   - Table: `QUEUED→{DISPATCHED,CANCELLED,FAILED}`, `DISPATCHED→{COMPLETED,FAILED,CANCELLED}`, `FAILED→COMPLETED` (guarded).
   - Reject (Err, **no write, no audit row**) on: (a) actual status ≠ `expected_from` (stale caller), or (b) `(from,to)` not a table edge.
   - On success append a `job_transitions` row via the existing `record_job_transition_conn_sync` (reuse the audit surface; don't reinvent).
   - Gate touches **only** `jobs.status` — never `agents.state` (D7 boundary).
3. **`force_cancel_pending_dispatched_job_conn_sync(conn, job_id, now_epoch, timeout_secs)`** — gap-patch #2 takeover (see below).

## The two gap-patch acceptances — how the tests pin them

**gap-patch #1 (queuing-reason observability)** — `dispatch_deferral_emits_observable_signal_naming_the_occupant`:
When `claim_next_job_sync` skips an agent because it is occupied by an in-flight (`DISPATCHED`, non-terminal) job while a `QUEUED` job waits, emit a **queryable `events` row** `event_type='dispatch_deferred'` whose payload **names the occupant job** (test asserts payload contains the occupant id). The current skip branch (`jobs.rs:302`, agent≠IDLE) only `tracing::info!`s — that is the silent-stall gap. The branch already `tx.commit()`s before returning `None`, so an event inserted there persists. Companion test `no_deferral_signal_when_agent_dispatches_normally` forbids emitting on the normal dispatch path (no spurious noise). Mechanism (event vs. ps field) is your choice **as long as it is queryable, not a tracing line, and names the occupant** — that naming is the whole point (the incident took 5+ min of hand-inspection to find the occupant).

**gap-patch #2 (cancel driver + timeout takeover)** — `cancel_timeout_takeover_forces_cancel_of_hung_agent_job` + `..._waits_for_the_bounded_timeout`:
`force_cancel_pending_dispatched_job_conn_sync` must drive `DISPATCHED→CANCELLED` **through the gate** when a cancel has been pending ≥ `timeout_secs` at `now_epoch`, **regardless of agent state** (the existing `mark_dispatched_job_cancelled_if_agent_idle_sync` requires agent IDLE/UNKNOWN and therefore hangs forever on a dead/hung agent — the test proves it returns 0 changes for a BUSY agent, then the takeover fires anyway). Pending-since is measured from the cancel request (the `reason='cancel_requested'` `job_transitions` row's `created_at`, written by `request_dispatched_job_cancel_sync`; a dedicated `cancel_requested_at` column is fine too). `now_epoch` is injected for deterministic tests — production callers pass `unixepoch()`. Timeout value and full-auto-vs-require-`ah kill` are your call; the pinned invariant is just "cannot hang forever with no takeover path."

## `FAILED → COMPLETED` narrow guard

`failed_to_completed_refused_without_late_evidence` (safety) + `failed_to_completed_allowed_with_authoritative_late_evidence` (narrow window). Model the guard directly on `state_machine::late_health_completion_stuck_allows_terminal` (design.md D1 says so explicitly). The positive test seeds a `state_change` event with `reason=HEALTH_CHECK_STUCK` + `signal_kinds=["health:completion"]` + `job_id=<job>` for the agent — that is the authoritative-late-completion vocabulary the existing accept-gate keys on. If you generalize the predicate when the arbiter lands, keep this shape satisfiable, or raise a `.lane-question`.

## Migration target (RE-GREP yourself — do not trust this list blindly)

Production single-line `UPDATE jobs SET status` writers on HEAD: `src/db/jobs.rs:277,332,414,525,559,664,711,925`. **Plus** multi-line `UPDATE jobs … SET status` writers not caught by a single-line grep (requeue/recovery paths in `jobs.rs` ~633/741/816/873 and `src/db/recovery.rs:362,1028,1086,1174`).

**Scope flag (your decision + a `.lane-question` if unsure):** the F3=F2 site `src/db/state_machine.rs:1219` (`mark_agent_idle_*` writing `jobs.status='COMPLETED'`) is a `jobs.status` writer, so D1's "single writer" mandate says its **write mechanism** should route through the gate (still `DISPATCHED→COMPLETED`, same semantics). Do **NOT** change it to write `VERIFYING` — that is D2 and is forbidden this round. If routing it through the gate risks bleeding into D2, migrate the unambiguous `jobs.rs`/`recovery.rs` sites first and flag the F3=F2 site to me.

CI grep rule (D1 item 3): ban raw `UPDATE jobs SET status` in **non-test production code** outside `db::jobs`'s gate impl. Test fixtures across the tree legitimately use raw updates — the rule must exclude test modules (or run only against the gate's own module boundary). Wire it into `.github/workflows/ci.yml`.

## Cargo discipline (shared machine)

`CARGO_BUILD_JOBS=1`, `--test-threads=1`, local `--lib`/`cargo check` only. **Before any `cargo test`/`check`, `ps -eo args | grep bin/cargo` machine-wide** — the C1 (arbiter) and g1 lanes share this box; never run cargo concurrently with another lane. Commit, do **not** push.
