# ah Completion Protocol — Design

Status: **merged design, freeze candidate** (2026-07-10). Format follows `ah-perception-arbiter/design.md`'s Must-Answer convention (question → decision → rationale → relationship to existing modules). This document is the next layer *below* the arbiter: the arbiter (Phase 1, frozen) owns `agents.state`; this protocol owns `jobs.status` COMPLETED/FAILED, and it rides the same outbox transport the arbiter's Q4 introduced.

This is the product of a full two-lane authoring + double-blind divergence + bidirectional cross-review pipeline (STAGE-PLAN-2026-07-11 §三/§四). It merges two independently-authored, mutually-reviewed drafts into one spec; it does **not** re-open any decision either lane already froze. Every design decision below is annotated with (a) the north-star §五 acceptance clause it serves (`research/perception-layer-first-principles.md`) and (b) the dogfood ledger node its empirical closure hangs on (`research/dogfood-ledger-2026-07-10.md`) — because **code-closed ≠ empirically-closed** is a standing discipline here, not a formality.

---

## 0. Provenance & Process Record

Recorded per the 执笔权 discipline (a merge must carry its own audit trail — who authored, who reviewed, and whether any real REJECT→rework→re-ACCEPT cycle occurred).

**Authorship (two lanes, neither self-adjudicating its own part — STAGE-PLAN §二):**

| Part | Owner | Draft |
|---|---|---|
| R1 (outbox / ACK / replay) + G4 (control-path self-check) | **g1** (泳道1 闸门) | `design-draft-track-a-g1.md` (rev-2) |
| R2 (explicit completion protocol) + Evidence Gate + R3 teardown + §5 provider matrix | **g2** (泳道2 闸门) | `design-draft-track-b-g2.md` |

**Divergence → convergence (new topic: per-provider completion-gap matrix, double-blind):**
- **o1** (agy) wrote the first blind divergence draft — problems/risks only, no conclusions, explicit anti-sycophancy — `divergence-provider-matrix-o1.md`.
- **g1** wrote the second, independent blind draft, mutually unseen (prompt disclosed neither o1's content nor any prior leaning) — `divergence-provider-matrix-g1.md`. Model-diversity was chosen deliberately (claude second seat over a same-agy double-blind), per operator correction after the single-blind (o1-only) first pass was flagged as a double-blind violation.
- **g2** authored the convergence record `convergence-provider-matrix.md`, **independently re-verifying g1's code-grounding against `src/` first-hand** rather than trusting the draft's citations. That verification *corrected the §5 premise*: all three providers currently wire an identical observe-only Stop hook and **no** `{"decision":"block"}` path exists anywhere — the "claude has a hook, agy doesn't" asymmetry the brief assumed is *future/aspirational, not current* (§5.0 below). g2's read was stronger than g1's draft: **codex** wires the same Stop hook too (convergence ⚠-A).

**Bidirectional cross-review (each lane reviews the *other* lane's product — STAGE-PLAN §四.3):**
- **g1 reviewed Track B** (`review-track-b-by-g1.md`): **ACCEPT for merge, no blocking issue.** The §5.0 premise correction was re-verified TRUE line-by-line against `src/` (incl. `grep -rn '"decision"' src/` → zero hits, and the `"{}\n"` output pinned by test `ah.rs:1863-1870`). Six completeness/precision items (NB-1..NB-4c) + per-§7 rulings were raised as non-blocking, carried to implementation (§ Open Items).
- **g2 reviewed Track A** (`review-track-a-by-g2.md`): **initial verdict REJECT** — four blocking gaps (F-1..F-4) + three non-blocking (F-5..F-7). **This lane's real REJECT→fix→re-ACCEPT cycle, recorded in full:**
  - **F-1** — journal-write/fsync/rename failure control-flow unspecified (default reading = silent exit-0 = the exact fire-and-forget-into-the-void disease R1 exists to kill, re-entering the back door).
  - **F-2** — the idempotency ledger, as originally grounded ("reuse the `events` table's `event_id`"), structurally cannot dedup Track B's `job_done` (no `event_id` column on `events`; Track B consumes onto `job_transitions`, a disjoint namespace). **Cross-track contract, must be pinned jointly.**
  - **F-3** — whole-config `compute_config_hash` drift-check + block-until-re-check-passes + merge-don't-clobber are mutually inconsistent → benign operator edit ⇒ permanent dispatch block.
  - **F-4** — G4 grounded entirely on the claude hook shape; does not discharge Track B §5.7's registered cross-track requirement to cover agy + codex. **Cross-track contract.**
  - g1 revised Track A → **rev-2**, fixing all four (each fix verified against `src/` before editing), folding in F-5/F-6/F-7. g2 **re-checked rev-2 against `src/`** (not against g1's self-report): **all four ACCEPT**, Track A freeze-ready. g2 additionally confirmed a false-positive vector g1 caught that g2 had missed in round 1 — codex's `hooks.json` Stop hook is inert unless `features.hooks = true` (`home_layout.rs:1163`).
  - Reciprocal Track B patch applied bidirectionally: the two joint contracts (below) are now mirrored on both sides with no duplicated content.

**Two joint Track-A/Track-B contracts pinned by the review, written once in this merge:**
- **JC-1 — single transport dedup ledger** (from F-2). Owned/specified in **R1-Q2**; referenced (not re-specified) by R2-MA-1.
- **JC-2 — multi-provider G4 coverage** (from F-4, discharging Track B §5.7). Owned/specified in **G4-Q1/Q3/Q4**; referenced (not re-specified) by §5.7.

---

## 1. North-Star §五 Acceptance Legend (unified)

The two drafts used two labelings for the *same* north-star §五 clauses (Track A cited `§五.N`; Track B cited `A1..A4` as "the four acceptance definitions"). Unified here so each decision carries one label:

| Label | Clause (verbatim intent) |
|---|---|
| **§五.1 / A1** | kill -9 ahd then relaunch → event stream has **no hole** (outbox replay proves it). |
| **§五.2 / A2** | agent with a live background task hits end_turn → job is **not** judged complete; watchdog alerts "停了未声明". |
| **§五.3 / A3** | ghost text / banner appears arbitrarily → **zero** lifecycle impact. |
| **§五.4 / A4** | hook config manually deleted → next-startup self-check **alarms and auto-repairs**, loudly, in the interim. |
| **§五.5** | every mechanism above is pinned by a **required automated test** (cross-cutting test-discipline clause). |

## 2. Dogfood Anchor Nodes (unified)

All under the Gen-4 open window (`research/dogfood-ledger-2026-07-10.md`); efficacy verdicts pushed via `research/gen-efficacy-reports.md`. A verdict may read 治愈-实证 **only** when the automated test passes **and** the live-stack observation lands — never on code-closure alone.

- **R1-outbox-replay** — dispatch a job, `kill -9` ahd while the Stop hook fires, relaunch, assert the completion event is consumed **exactly once** (restart arm + duplicate-delivery arm + crash-between-commit-and-reap arm). (§五.1)
- **R1-deadletter** — inject a malformed outbox file; assert it lands in `outbox/dead/` with a loud log and does not stall replay of its siblings; assert it surfaces as an `ah doctor` `Warn`. (§五.1)
- **G4-wiring-selfheal** — delete a hook config, restart ahd, assert alarm + auto-repair + re-check pass **before** dispatch; run once per provider shape; plus a benign-edit arm (operator adds a non-ah hook → **no** alarm, dispatch stays unblocked). (§五.4)
- **G4-synthetic-roundtrip** — break the outbox write permission, run the medium check, assert it **warns** and surfaces in `ah doctor` (does *not* silently pass on a byte-correct config); run per provider. (§五.4)
- **DF-1** — live agent parks a background task and ends its turn → assert job stays DISPATCHED, no COMPLETED. (§五.2)
- **DF-2** — STOPPED_UNDECLARED_ALERT rate: exactly-once per genuine stop-without-declare; **no 1887-in-48h flood** (Gen-3 baseline). (§五.2)
- **DF-3** — sampling audit: every COMPLETED job's `reply_text` is the declared result, never a brief fragment (kills obs #33). (§五.2)
- **DF-4** — inject banner/ghost text into a live pane → job lifecycle unchanged, dispatch still fires (kills ah#17). (§五.3)
- **DF-5** — mutating job declares done with empty diff → denied + nudged; 3rd try released + escalated; read-only job never gated. (§五.2)
- **DF-6** — agy 假BUSY 占道时长 (Gen-4 vs Gen-2/3): watchdog budget catches 哑火 without killing long legitimate reasoning turns. (§五.2)

---

## 3. Boundary Declaration — three modules, three write authorities (kept decoupled on purpose)

```text
  [Perception Arbiter]         -> writes agents.state           (F1/F2/F4: alive / turn-boundary / input-wait)
  [THIS: Completion Protocol]  -> writes jobs.status COMPLETED/FAILED   (F3: task result)
  [ah-job-events]              -> job_transitions carrier        (the durable spine both ride)
```

The core first-principle this document enforces: **F3 ≠ F2.** Today they are conflated — an agent going IDLE (a turn boundary, F2) triggers job-completion *inference* (F3) in the same DB transaction (`mark_agent_idle_hook_event_sync` flips the agent to IDLE **and** calls `mark_job_completed_conn_sync`; `src/db/state_machine.rs:716`, and the matched path `:501`, and the log path `:885` — all three verified in g1's Track B review). Severing that conflation is R2's whole job: agent-idle stays the arbiter's business; **job completion is driven only by an explicit worker declaration.**

Neither R1 nor G4 decides `agents.state` or `jobs.status`. R1 is a transport; its delivered events feed the arbiter (perception verdicts) and this protocol's R2 (job done-declaration). G4 is a self-check; its findings feed logs, `ah doctor`, and startup gating. Write authority over state stays where the arbiter design put it (single-writer for `agents.state`) and where R2 puts it (single-writer `apply_job_done_declaration_sync` for `jobs.status`).

## 4. Design Thesis — transport-first floor, explicit-declaration spine, self-check prophylaxis

Three theses stack in dependency order; R1 is the load-bearing floor.

**R1 — make the agent→ahd segment transactional (the floor).** Today `ah agent notify` is a synchronous RPC and nothing else (`src/bin/ah.rs:532`); on any error — ahd not running, socket stale, daemon mid-restart — the hook exits non-zero and **the event is gone** (`:547`). The only durable trace is a human-debug appender, never replayed (`:578`). That is the G1 structural defect the north-star names ("notify 无 ACK 无记账,ahd 不在=事件蒸发"). R1 flips the order: the hook stops *sending* an event and starts *journaling* one — a durable outbox record first, fast-path RPC as an optimization, exit-safe regardless of delivery; ahd consumes at-least-once and reaps only after a durable commit; on restart it cold-scans before serving. This is the standard transactional-outbox pattern, and **everything above rides it** — R2's declaration, MA-4's reply, the evidence gate's admission decision are all just records journaled through the same outbox.

**R2 — completion is an explicit declaration, never an inference (the spine, head-of-list problem).** `classify_terminality` returns `Terminal` from `end_turn`/`task_complete` (`src/completion/parser.rs:182`, `:230`) — F2 masquerading as F3. Live cost: Gen-0 假 COMPLETED 11 例/日; agy 假完成占道 Gen-2 3/3, Gen-3 incl. g2-m1 卡 12h. R2 severs completion from the idle transaction and makes it a worker-initiated `ah job done` / `ah job fail` declaration riding R1's transport.

**G4 — guarantee the events actually fire (prophylaxis).** R1 guarantees *delivery of events that fire*; G4 guarantees *events actually fire* — the hook is still wired into each provider's config, the config hasn't drifted/been-deleted, and a synthetic event round-trips the real path. Without G4, R1's durability is a guarantee about an empty pipe (the "零件好的忘了装" disease family).

```text
  R1:  [hook fires] --journal--> [outbox] --at-least-once--> [ahd consume] --reap-after-commit-->
  R2:  ahd consume --dedup(JC-1)--> apply_job_done_declaration_sync --> jobs.status COMPLETED/FAILED
  G4:  assert the hook is wired (all 3 provider shapes), its config matches, a synthetic event survives end-to-end
```

**Reused, not re-answered — the hook attribution race.** `ah-perception-arbiter/design.md` Q4 already pinned attribution: durable local write before any socket attempt, daemon-side inotify + cold-scan-on-restart consumption, attribution by a **per-dispatch-attempt cookie** (`AH_JOB_ATTEMPT_COOKIE = "{job_id}:{dispatch_seq}"` or equivalent — exact format pinned by the implementer, arbiter Q4 correction 2026-07-10) that does **not** depend on the sender process still being alive. This protocol treats that as settled input and mints no second cookie. R1 adds only the *delivery transactionality* the arbiter left as a one-line promise; where R1 and arbiter Q4 describe the same physical artifact (the outbox directory), R1 is the transport spec and arbiter Q4 is one consumer of what it delivers.

**Red lines observed** (`perception-final-convergence-2026-07-09.md` §三, two refuted claims). This design invokes **neither** the K8s probe-absence optimistic/pessimistic asymmetric default **nor** the K8s-API-layer-forced-`/status` single-write argument. R1's replay leans only on the *confirmed* level-based re-derivation over re-readable durable artifacts (final-convergence §1.5) — the outbox file is such an artifact; a transient socket call is not, which is why durability must precede delivery. Track B's §5 likewise cites neither refuted red line.

---

# Part R1 — Hook Delivery Reliability (Outbox / ACK / Replay) · g1

## R1-Q1: Where does durability begin — and what exactly is written before any socket attempt?

**Decision: durability begins at an atomic `rename()` into a per-agent, host-visible outbox directory, before the hook makes any RPC call.** The rewritten `ah agent notify` flow:

1. Construct the record in memory: `{ event_id, agent_id, provider, event, attempt_cookie, hook_fired_at, payload }`. `event_id` is a content-independent unique id minted by the hook (ULID/UUIDv7 — monotonic-ish for scan ordering; the hook already has a `--event-id` surface, `src/bin/ah.rs:518`). `attempt_cookie` is the arbiter-Q4 per-dispatch-attempt cookie read from the environment; R1 does not mint it.
2. Write to `{outbox_dir}/{event_id}.tmp`, `fsync` the file, `rename()` to `{outbox_dir}/{event_id}.json`, then `fsync` the containing directory. The rename is the durability commit point — POSIX guarantees the reader never observes a partial file under that name. *(Naming pinned `{event_id}.tmp → {event_id}.json` to match arbiter Q4 and Track B — F-7; the host-side scanner globs `*.json` and must never see `.tmp`.)*
3. **Only then** attempt the fast-path `client.call("agent.notify", …)` as today, but treat its result as an *optimization*: success lets ahd process and reap immediately; failure (ahd down, socket stale) is a non-error — the durable file is the guarantee, and the hook exits `0`.

**The durability commit can itself fail — and that must be loud, not exit-0 (F-1).** The exit-0-safe posture is scoped **only** to step-3 RPC failure. If step 2 fails (`ENOSPC` mid-write, `fsync` `EIO`, `rename` `EROFS`, a full/read-only outbox, a missing/unwritable `outbox_dir`), then **nothing durable landed** and the hook must **exit non-zero with a provider-visible error on stderr**, never exit 0. Exiting 0 on a failed journal is the exact "发射即忘进虚空" disease R1 exists to kill, re-entering the back door. Concretely: the `.tmp` write, its `fsync`, the `rename`, and the directory `fsync` are each checked; any error ⇒ `tracing::error!` + best-effort cleanup of the partial `.tmp` + non-zero exit. This is the mirror image of today's bug — today the hook exits non-zero on *RPC* failure (`:547`), which R1 flips to exit-0-safe; R1 must not simultaneously let a *journal* failure exit 0. **Invariant: exit 0 ⇔ a durable outbox record exists.** (Outbox-inaccessible remains a loud escalation to the sandbox design per arbiter Q4, not a route-around; F-1 adds the *transient* write-failure branch the arbiter's access-denied clause did not cover.)

`outbox_dir` is the same host-side directory arbiter Q4 introduced (`{agent_home}/outbox/`). If the sandbox model cannot guarantee outbox write access, that is a **loud escalation to the sandbox design, not a route-around** (arbiter Q4 failure-mode, carried forward verbatim).

**Rationale.** Today's order is inverted: the socket call *is* the delivery, so far-end unavailability equals loss. Reversing to journal-first is the whole content of the G1 fix. `rename()` over the same filesystem is the only primitive in this path that survives a `kill -9` of both ends; the fast-path RPC is kept purely to preserve sub-second common-case latency, demoted from *the* mechanism to *an* accelerator.

> **§五 acceptance:** §五.1 / A1 — the rename commit point is what makes "no holes" provable. **Dogfood:** `R1-outbox-replay` (kill -9 mid-Stop-hook → relaunch → consumed exactly once). Also retires the "hook 信号偶发丢失" incident family (handoff §域1), root-caused by arbiter Q4 as the sd_notify send-before-PID1-processes race.

## R1-Q2: At-least-once means duplicates — what makes redelivery safe? · **JC-1 lives here**

**Decision (Joint Contract JC-1): a single dedicated transport-level dedup ledger, checked at the outbox-consume boundary *before* the record is routed by `kind`; state-CAS is demoted from the idempotency mechanism to a secondary guard.** The consume loop's first step, for *every* record regardless of `kind`, is `INSERT INTO outbox_consumed(event_id) … ON CONFLICT DO NOTHING`; zero rows affected ⇒ already applied ⇒ drop and reap without dispatching. Only a first-seen `event_id` proceeds to routing. The dedup insert and the handler's effect commit in **one transaction**, so a crash between apply and reap cannot double-apply.

**Why the ledger must live at the transport boundary, not on the `events` table (F-2 — corrected against schema).** The earlier "reuse the `events` table's `event_id`" was structurally wrong, verified against `src/db/schema.rs`:
- `events` has **no `event_id` column** (`schema.rs:124-131`: `seq_id, agent_id, request_id, event_type, payload, created_at`); `event_id` is buried inside the JSON `payload` (`state_machine.rs:727`). No `UNIQUE(event_id)` without a migration (generated column / expression index).
- The one existing idempotency index is `UNIQUE(agent_id, request_id) WHERE request_id IS NOT NULL` (`schema.rs:135`) — keyed on `request_id`, and the hook-event INSERT passes `request_id = NULL` (`state_machine.rs:733`), so it doesn't even cover today's hook path. (This *confirms* the CAS-is-the-only-current-guard motivation below — but it means there is nothing to "reuse.")
- **Decisively for the cross-track seam:** Track B's `job_done`/`job_fail` records consume onto a **different table**, `job_transitions` (`apply_job_done_declaration_sync`, R2-MA-1), whose PK is `job_event_id INTEGER PRIMARY KEY AUTOINCREMENT` (`schema.rs:176`) — a DB-minted integer in a *disjoint namespace* from the hook-minted ULID `event_id`. A `UNIQUE(event_id)` bolted onto `events` gives Track B's declarations **zero** dedup, so under at-least-once redelivery a replayed `job_done` file would be **double-applied** to `job_transitions`. The transport owns delivery, so the dedup that makes redelivery safe must be **one ledger over all outbox records, keyed on the outbox `event_id`, applied before the `kind` fork** — covering both the F2 `events` path and the F3 `job_transitions` path with a single guard. This is a real schema delta (a new dedup table, e.g. `outbox_consumed(event_id TEXT PRIMARY KEY, consumed_at INTEGER)`), **not** free reuse (F-5).

**JC-1 is a pinned Track-A/Track-B contract, not a unilateral choice.** R2-MA-1 routes `job_done`/`job_fail` through this same consume boundary (dedup-then-route) and must **not** assume the `events`-table `event_id` covers it. Both drafts previously assumed different homes for the same key; this section fixes the home to the transport ledger, and R2 adopts it (mirrored, no content rewrite).

**Why CAS alone is insufficient — corrected against current code.** Today idempotency rides entirely on the state-version CAS inside `mark_agent_idle_hook_event_outcome_sync` (`state_machine.rs:619-632`): a redelivered event finds the agent no longer `BUSY`/`WAITING_FOR_ACK` and is swallowed (`:623`). That is *accidental* idempotency, and it breaks under exactly the sequence at-least-once makes common: event E delivered once (BUSY→IDLE), a new job re-arms the agent to BUSY, then E is redelivered (outbox replay after an ahd restart that hadn't reaped E's file) — CAS now sees BUSY and **misapplies E to the new job**. An explicit `event_id` ledger closes this: E is recognized as already-seen regardless of current state. CAS stays as defense-in-depth (it correctly rejects genuinely-stale transitions) but is no longer the thing standing between replay and corruption.

**Rationale.** at-least-once + idempotent-consumer is the only crash-safe *and* duplicate-safe combination; exactly-once delivery over a crashing transport is unachievable, so we make duplicates harmless. Keying on a producer-minted `event_id` (not content, not "current active job") is what lets a duplicate be recognized after the world has moved on.

> **§五 acceptance:** §五.1 / A1 (replay must not corrupt) + §五.5 (the redeliver-after-rearm sequence is a required RED test). **Dogfood:** `R1-outbox-replay` (duplicate-delivery arm — a doubly-scanned file transitions state exactly once).

## R1-Q3: How does ahd replay after restart, and what happens to events it can never apply?

**Decision: cold-scan the outbox before serving, replay in `event_id` order, and quarantine the unapplyable into an error-book rather than dropping or hot-looping on them.** On startup, before accepting RPC traffic, ahd enumerates `{agent_home}/outbox/*.json` across all agent homes and feeds each through the same idempotent consume path (JC-1) as a live inotify event. Steady-state consumption is inotify-driven; the cold-scan is the restart-recovery and the inotify-miss backstop (a periodic sweep also catches inotify drops, since the durable file is level-triggered and re-readable — final-convergence §1.5).

**Ordering.** Replay oldest-first by `event_id` (ULID/UUIDv7 timestamp prefix ⇒ scan order ≈ fire order). Strict global ordering is *not* required for correctness (each effect is idempotent and cookie-attributed, not arrival-position-attributed) — but approximate fire-order replay avoids needless CAS churn. *Reserved-prefix ids are exempt (F-6):* the G4-Q4 selfcheck record uses a fixed non-ULID id (`selfcheck:{agent_id}:{boot_id}`) that does not time-sort — it routes to a no-op sink (G4-Q4), so its replay position is irrelevant, but it still takes a JC-1 ledger row so a crash-surviving selfcheck file re-scans as a harmless no-op rather than re-running.

**Error-book (the quarantine, not a drop).** An outbox file that (a) fails to parse, (b) references an `attempt_cookie` attributable to no known dispatch, or (c) has failed consume N times (N=3, matching the "third-attempt escalation" convention) is `rename()`d into `{agent_home}/outbox/dead/` with a **loud** `tracing::error!` carrying file name + failure reason. Never silently deleted, never left to hot-loop the scanner. The dead-letter directory is an operational surface (a `DoctorCheck` in G4-Q5 counts it) and a dogfood data point, not an auto-retry queue.

**Rationale.** Cold-scan-before-serve is the arbiter's own adopted mechanism (Q4); R1 pins its *timing* (before serving, so no window exists where ahd is up but hasn't replayed) and its *failure handling* (error-book, so a poison file can neither vanish nor wedge the scanner). Quarantine-with-loud-log over silent-drop is the north-star's "响亮地降级" applied to the transport's own edges.

> **§五 acceptance:** §五.1 / A1 (the canonical `kill -9 ahd` → relaunch → "事件流无洞,outbox 重放可证" scenario — this section *is* that mechanism). **Dogfood:** `R1-outbox-replay` (restart arm) + `R1-deadletter` (malformed file → `outbox/dead/` + loud log, siblings unaffected).

## R1-Q4: What is the ACK, and when is an outbox record safe to delete?

**Decision: the ACK is ahd's durable DB commit of the event's effect + JC-1 ledger row; reaping (file deletion) happens strictly after that commit, and the hook process never blocks on it.** Two acknowledgements exist and must not be conflated:
- **Producer-side "ACK": none, by design.** The hook's guarantee is the `rename()` (R1-Q1); it exits `0` immediately after, reachable-or-not. Making the hook wait for a daemon ACK would reintroduce the coupling R1 removes (and stall the agent's turn on ahd latency).
- **Consumer-side ACK = commit-then-reap.** ahd applies the effect + inserts the `event_id` ledger row in one transaction (JC-1); the file is deleted only after commit. Crash between commit and reap → file survives, next cold-scan re-reads it, ledger makes the re-read a no-op. Crash between read and commit → file survives, event applied on restart. No ordering of crashes loses or double-applies an event.

**Rationale.** The transactional-outbox invariant stated precisely: *the file may only be destroyed by the party that has durably recorded its effect.* The north-star's "先journal后投递等ACK" is satisfied not by the hook waiting for an ACK (it must not) but by the *reap* waiting for the *commit*.

**Consistency with the arbiter's 2s hook budget (arbiter Q2).** The 2s hook-grace window measures "how long ahd waits to *notice and process* an already-durably-written outbox file" (inotify latency + tick cadence) — it does **not** measure disk I/O or hook liveness, and does **not** produce a direct job-terminal write (arbiter Q2 correction: the expiry yields a perception verdict feeding Phase-2 evidence-check, never a one-way guillotine into `Failed`). R1 is consistent: the outbox makes the write durable in R1-Q1, so the 2s clock only ever runs against ahd's own consume latency. If it ever needs a different value, arbiter Q2 already named the correct conclusion — "the consumption loop's cadence assumption was wrong," not that R1 needs its own second number.

> **§五 acceptance:** §五.1 / A1 (no-loss under crash — commit-then-reap is what closes the crash-window analysis). **Dogfood:** `R1-outbox-replay` (crash-between-commit-and-reap arm — fault-injection hook `abort()`s ahd after commit, before reap; assert exactly-once on relaunch).

---

# Part R2 — Explicit Completion Declaration (F3 ≠ F2) · g2

## R2-MA-1: Dispatch carries a job identity; completion is an explicit declaration

**Decision: dispatch injects a per-attempt job identity; completion is a worker-initiated declaration, never an inference.** Two shapes — a CLI command (the concrete artifact) and its outbox record (the wire form).

**Dispatch side (what the worker receives).** Every dispatch injects into the agent's environment/sandbox:
- `AH_JOB_ID` — the `jobs.id` of the dispatched job.
- `AH_JOB_ATTEMPT_COOKIE` — the per-dispatch-attempt cookie from arbiter Q4 (format `{job_id}:{dispatch_seq}`). **Not** optional and **not** just `job_id`: a fast redispatch's stale, late-arriving declaration must not misattribute to the new attempt (o1 §五.2 epoch drift; arbiter Q4 replay hazard). Reuse the arbiter's cookie; do not mint a second one.

**Completion side (what the worker runs).**

```
ah job done <job_id> [--reply-file <path> | --reply-stdin]     # declare COMPLETED, attach the actual result
ah job fail <job_id> --reason <text>                           # declare FAILED, honest exit (no queue pollution)
```

Both resolve `<job_id>` against `AH_JOB_ID`/`AH_JOB_ATTEMPT_COOKIE` from the environment; a mismatch (wrong id, stale cookie) is refused loudly at the CLI **before anything is written** (defends o1 §三.2 ID-tampering / cross-job "隔山打牛"). Providing both `done` and `fail` is deliberate: o1 §三.1's "调了 ah job done 但 reply 里诚实说我搞不定" ambiguity is resolved by giving the model an honest FAILED exit, so "I give up" never rides the COMPLETED path.

**Wire form (rides the R1 outbox, not a new channel).** `ah job done` does **not** hit the RPC socket directly. It writes a durable outbox record, then returns:

```text
{agent_home}/outbox/{event_id}.tmp  --rename(atomic)-->  {event_id}.json
  { "kind": "job_done",             # or "job_fail"
    "event_id": "<uuid>",           # idempotency key, dedup at consume (JC-1)
    "job_id": "job_...",
    "attempt_cookie": "job_...:<seq>",
    "reply_text": "<the actual declared result>",   # T1 structured, NOT scraped
    "declared_at": <monotonic> }
```

ahd consumes (inotify + cold-scan-on-restart, arbiter Q4 / R1-Q3), validates the attempt cookie against the current dispatch epoch, and applies the transition. A declaration for a stale epoch (arrived after a realign) is **rejected and logged, not applied** (o1 §五.2). The write is durable the instant `rename()` returns; the worker may die immediately after (fire-and-forget on the sender is safe *because* the outbox already persisted — the sd_notify barrier property, convergence 2.3).

**Rides JC-1 (R1-Q2), not a private key.** The consume step first passes through R1-Q2's single transport dedup boundary — `INSERT INTO outbox_consumed(event_id) … ON CONFLICT DO NOTHING`, applied to *every* record **before** the `kind` fork — so a replayed `job_done`/`job_fail` (re-read by cold-scan after a restart) is recognized as already-consumed and **cannot double-apply** to `job_transitions`. This protocol does **not** rely on the `events`-table `event_id` (which does not exist as a column and would not cover `job_transitions` at all); the dedup insert and `apply_job_done_declaration_sync` commit in one transaction. *(Full specification of the ledger is in R1-Q2; not repeated here.)*

**Structural refactor this forces (the load-bearing change).** Job completion moves out of the agent-idle transaction:
- `mark_agent_idle_hook_event_sync` / idle-marker / log paths keep updating `agents.state` (F2 — legitimate, arbiter's carrier) but **must no longer call `mark_job_completed_conn_sync`** (`state_machine.rs:716`, `:501`, `:885` — all three coupling sites confirmed by g1's Track B review). They stop being completion authorities.
- A new `apply_job_done_declaration_sync(conn, job_id, attempt_cookie, reply_text)` is the **sole** writer of `DISPATCHED -> COMPLETED`, triggered only by a consumed `job_done` record. It records the transition on `job_transitions` with `reason = "explicit_done"` and `reply_source = "protocol"` (retiring `"screen"`; see MA-4).
- Symmetrically `job_fail` → `DISPATCHED -> FAILED`, `reason = "explicit_fail"`.

This is the direct structural analog of convergence 2.3's sd_notify precedent: "管理者只认显式报告、不认输出启发式."

> **§五 acceptance:** §五.2 / A2 (agent end_turn no longer completes a job — completion now requires an explicit act). **Dogfood:** DF-1 (backgrounded work + end_turn → job stays DISPATCHED), DF-3 (every COMPLETED job carries a declared, non-scraped reply). **Verification-debt:** unit tests prove the code path (declaration → COMPLETED; idle → no completion); empirical closure is DF-1/DF-3 on the Gen-4 live stack — code-closed ≠ empirically-closed.

## R2-MA-2: Claude Stop-hook enforcement layer (block + reason, bounded)

**Decision: the Claude Stop hook is a *soft, bounded* enforcement — it blocks a stop that has no declaration, feeds a reason, and after a bounded number of blocks yields to the watchdog.** Not an infinite hard block (that deadlocks a genuinely stuck agent), and not a completion signal (the hook completes nothing — it only refuses to let the turn end silently).

**Grounding (this is net-new, not a tweak — convergence §5.0, verified by both lanes).** Today claude's `Stop` hook is **observe-only**: it routes to `ah agent notify --event stop`, marks the agent idle, and the CLI emits a hardcoded `"{}"` = "allow stop" (`src/bin/ah.rs:562-564`, test-pinned at `ah.rs:1863-1870`). **No `{"decision":"block"}` is emitted anywhere in `src/` for any provider.** So the claude "block 带病下班" capability the matrix leans on does **not exist yet** — MA-2 *builds* it (Claude-Code's Stop contract honors `decision:block`; the ~5s hook timeout is ample headroom). Enforcement being absent-by-default is the pre-existing state, not a regression — which is exactly why the G4 config self-check (below) matters.

Mechanism, using Claude Code's native `Stop` hook contract:

```text
Claude worker reaches turn end
   [Stop hook fires]  reads AH_JOB_ID + reads the agent's own outbox
   Has a job_done|job_fail record for AH_JOB_ID been written this dispatch?
     |-- yes --> allow stop (turn ends cleanly; declaration already durable)
     |-- no, block-count < N --> return {"decision":"block",
     |         "reason":"You have not declared completion for <AH_JOB_ID>. Run `ah job done <id>`
     |                   with your result, or `ah job fail <id> --reason ...`. Do not end undeclared."}
     |-- no, block-count >= N --> allow stop, BUT emit STOPPED_UNDECLARED (hand to MA-3 watchdog; never loop forever)
```

Key properties:
- **Bounded, not infinite.** Claude Code passes `stop_hook_active` on re-entry precisely to prevent infinite block loops. The hook tracks a per-dispatch block counter; after N (proposed **N=2**, tunable — same spirit as the evidence-gate nudge cap) it stops blocking and escalates. A hard-infinite block on a genuinely-wedged agent (context-corrupted, tool-call broken) would reproduce the 12h-park disease from the other side. Enforcement's job is to *prompt* the declaration, not to imprison the process.
- **The hook never writes state.** It reads the outbox to decide block-vs-allow; it never sets `agents.state` or `jobs.status`. Single-writer discipline stays intact (arbiter owns state; MA-1 declaration owns completion).
- **Config self-check dependency → JC-2.** The Stop hook is the "零件好的忘了装" risk class: if its config is deleted or drifts, enforcement silently vanishes and we regress to inference. Its presence/wiring **is** an assertion in G4 (JC-2), for every provider shape, plus a semantic-behavior check (Open Items). This part registers the requirement; G4 discharges it.

> **§五 acceptance:** §五.2 / A2 (undeclared stop is caught, not inferred-complete) + §五.4 / A4 (the enforcement hook is itself under the config self-check umbrella). **Dogfood:** DF-1, DF-2 (an agent that tries to end without declaring is blocked once/twice; if it still won't, exactly one STOPPED_UNDECLARED fires — not a flood). **Open item (NB-2):** the durable, epoch-scoped, reset-on-redispatch **block counter is a net-new stateful component**, not a tuning value — `stop_hook_active` (bool) and `has_completion_deferred_event` (hash de-dup) do not provide it. Where the counter lives + its reset semantics → Open Items.

## R2-MA-3: Detection demoted to a watchdog ("停了却没声明" = alert, never inference)

**Decision: the completion *detectors* (log/idle-marker/pane inference) stop being completion authorities and become a single reconcile watchdog whose only output is an alert.** Nothing about "stopped" ever *infers* "done" again.

The watchdog rides the existing tick (not a new thread). Predicate:

```text
For each agent A:
  if arbiter says A.state == IDLE          (F2: A stopped, awaiting input)
  and A has a DISPATCHED job J with no explicit job_done|job_fail declaration
  and (now - A.last_activity) > watchdog_budget(provider)      # per-provider, §5
  then emit STOPPED_UNDECLARED_ALERT(A, J)   # alert only; optionally one nudge; NEVER mark COMPLETED/FAILED
```

- **Alert, not a verdict.** The systemd-watchdog semantic (convergence 1.4 / 2.3): keep-alive absence is a *loud* condition surfaced to master/operator, not a silent fallback to a lower-tier signal. The job stays DISPATCHED (or is explicitly cancelled by master); it is never auto-completed off a timeout.
- **`classify_terminality`'s completion role is retired.** `end_turn`/`task_complete` stop being `Terminal` verdicts; they survive only as *turn-boundary* signals feeding the arbiter's F2 and as watchdog corroboration/telemetry — never job-completion authority. `DeferredBackgroundWork` becomes unnecessary for gating completion (completion no longer fires on turn-end at all); it is only a hint for nudge text.
- **Anti-flood.** DF-2 directly addresses the Gen-3 pathology (STOPPED_UNDECLARED_ALERT 刷 1887 条/48h): the alert must be **edge-triggered** (one per stop-without-declare episode), de-duplicated by `(agent, job, dispatch_epoch)`, not re-emitted every tick.

> **§五 acceptance:** §五.2 / A2 — the exact clause ("job 不判完,看门狗告警") is this section. **Dogfood:** DF-2 (alert cadence sane) + DF-6 (watchdog budget catches genuine 哑火 without guillotining long legitimate reasoning turns).

## R2-MA-4: Reply payload attribution made explicit (kill the "screen" source)

**Decision: the reply travels *inside* the completion declaration; the pane-scraping reply path is deleted, not merely deprioritized.**

Grounding: `reply_source` has three values — `"hook"` (structured), `"log"` (transcript), `"screen"` (`collect_reply_for_dispatched_job_sync`, pane scrape). The `"screen"` fallback fires whenever a structured reply is absent (`state_machine.rs:646-658`, again `:699-702`; g1's Track B review enumerated all sites — matched `:398`, hook `:650`/`:656`, log `:819`/`:825`, plus the `unwrap_or_else(… "screen")` defaults `:702`/`:871`). Obs #33 is precisely this fallback capturing prompt-echo/fragment instead of the verdict.

The protocol closes it:
- `ah job done <id> --reply-file <path>` carries the worker's *own stated result* in the outbox record's `reply_text`, which lands in `jobs.reply_text` when `apply_job_done_declaration_sync` runs. `reply_source = "protocol"`.
- **`"screen"` is removed.** `collect_reply_for_dispatched_job_sync` and *all* its call sites are deleted as part of the MA-1 refactor. No code path left reads pane text into `reply_text`.
- **`"log"` is demoted to fallback-with-caveat, not authority.** For providers whose harness genuinely cannot run `ah job done` inline (§5.4 codex), a *structured* log token (not free-text scrape) may carry the reply as a transitional bridge — `reply_source = "log_structured"`, explicitly a §5 gap, adapter-owned, never a blessed default.

Delivery/attribution identical to MA-1 — reply rides the outbox record, attributed by `attempt_cookie`, durable before sender exit. A late reply from a stale attempt is dropped with the whole stale declaration (MA-1 epoch check), so a redispatch cannot inherit the previous attempt's reply text.

> **§五 acceptance:** §五.2 / A2 (reply is the declared result) + §五.3 / A3 (no pane text ever becomes payload → ghost/banner cannot corrupt `reply_text`). **Dogfood:** DF-3, DF-4. **Open item (NB-3):** the deletion PR's anchor list (R3 §6.3) must enumerate **all three** `collect_reply` sites + both `"screen"` defaults, and note `:398`'s entanglement with `is_prompt_only_reply`/`classify_terminality` (those consumers fall away with the declaration-driven model).

---

# Part G4 — Control-Path Self-Check · g1

## G4-Q1: What does "the control path is wired" actually assert — what are the check targets? · **JC-2 lives here**

**Decision: three concrete, machine-checkable assertions, in ascending cost.** "Wired" decomposes into exactly these targets, and the three startup tiers (G4-Q2) each cover a prefix:

1. **Config presence & shape** (cheap, read-only): for every agent ahd manages, the agent's provider config file on disk contains the ah-owned `Stop` hook entry (and `UserPromptSubmit` where used), with a `command` string that (a) invokes `ah agent notify`, (b) carries the correct `--agent-id`, (c) points at the *current* ahd socket path, (d) names the right `--provider`. **This target is per-provider-shape-aware (JC-2/F-4):** it locates the ah-owned entry using the same `is_ah_owned_hook_item` predicate the injector uses (`command.contains("ah agent notify")`, `home_layout.rs:1202-1207`), inside the correct file+nesting for each provider — see the G4-Q3 shape table. The `command` itself is a pure string comparison against what `build_ah_hook_command` (`:674`) renders today (shared across all three providers).
2. **Config fidelity** (cheap): the ah-owned hook entries on disk match what ahd would render — a **scoped** comparison over *only ah's own entries* (G4-Q3), **not** a whole-config fingerprint. (Whole-config hashing false-blocks on benign operator edits — F-3; deliberately not used.)
3. **End-to-end liveness** (expensive, side-effecting): a synthetic event, injected at the real hook entry point of **each provider's installed hook**, actually traverses hook → outbox → ahd consume and is observed consumed (G4-Q4).

**Rationale.** The G4 disease family is "a good part nobody installed" — the failures are all *absence* or *drift* of ah's own wiring, not logic bugs, so the checks are all *comparisons of ah's-own-entries-on-disk against ahd's own expectation*. ahd already holds the expectation: it rendered the hook command (`build_ah_hook_command`) and knows which entries are its own (`is_ah_owned_hook_item`). Scoped to ah's entries so an operator's out-of-band additions are invisible (F-3), and enumerated across all three provider shapes so no provider is silently unmonitored (JC-2/F-4).

> **§五 acceptance:** §五.4 / A4 (target 1 catches the manual-delete; targets 2/3 catch subtler drift). **Dogfood:** `G4-wiring-selfheal` (self-heal arm in G4-Q6).

## G4-Q2: What runs at startup, and what blocks vs. warns — the three-tier check?

**Decision: light / medium / deep, with light always-on and blocking-with-repair, medium always-on and non-blocking, deep opt-in via `ah doctor`.**

| Tier | When | Covers | On failure |
|---|---|---|---|
| **Light** | Every ahd startup, and every `ah up`/realign that (re)materializes an agent home | Target 1 (presence/shape) + Target 2 (scoped ah-owned-entry fidelity) — read-only, sub-ms per agent, over **ah's own entries only** (F-3) | **Loud alarm + auto-repair** (re-materialize ah's own hook entries, G4-Q6) + a self-check event. Blocks the agent from being dispatch-ready until repair is confirmed by a re-check. Does **not** block ahd's own boot (other agents proceed). Scoped to ah's entries, so a benign operator edit elsewhere never trips it (F-3 non-convergence structurally impossible). |
| **Medium** | Every ahd startup, after light passes | Target 3 (synthetic round-trip, G4-Q4) per agent | Loud alarm + self-check event; **non-blocking** — a synthetic-trigger failure warns and surfaces in `ah doctor`, but does not hold dispatch (light already guaranteed config present/correct; medium catches path-level rot the config can't reveal). |
| **Deep** | On demand: `ah doctor` (never in hot startup) | Full end-to-end incl. real daemon consume confirmation, dead-letter census (R1-Q3), cross-agent wiring matrix | Reported as `DoctorStatus::Fail`/`Warn` `DoctorCheck`s (`src/cli/doctor.rs:15`); operator-facing, no auto-action. |

**Why light blocks but medium does not.** Light checks a *fact ahd fully controls* (the config it wrote is still on disk) and can *fully remediate* (re-materialize) — so blocking-until-repaired is safe and correct: dispatching to an agent whose hook is missing would resurrect the exact fire-and-forget-into-the-void R1 fixed. Medium exercises the *live daemon path*, which can fail for reasons ahd cannot instantly fix (a transient socket race during its own boot, an inotify not-yet-armed); making that block startup risks a boot deadlock (ahd refusing to serve because its own not-yet-serving socket didn't answer a synthetic probe). So medium warns loudly and defers the hard verdict to deep/`ah doctor`, where the daemon is unambiguously up.

> **§五 acceptance:** §五.4 / A4 (light tier is the always-on catcher) + §五.5 (each tier's trigger is a required automated test). **Dogfood:** `G4-wiring-selfheal` (light-tier arm). *(Boot-deadlock avoidance confirmed correct in g2's Track A review §B — the block/warn cut is at the "ahd can both detect and fix" vs "ahd can only report" line.)*

## G4-Q3: How is config drift detected — and does it catch a manual delete? · **JC-2 shape table**

**Decision: drift detection compares *only ah's own materialized hook entries* on disk against what ahd would render — scoped by `is_ah_owned_hook_item`, located per-provider-shape — NOT the whole-config `compute_config_hash`; a delete is the degenerate drift case (the ah-owned entry is absent). (F-3 + F-4/JC-2.)** Per managed agent, per provider shape: re-read the config file, find the ah-owned `Stop` item(s) (`command.contains("ah agent notify")`, `home_layout.rs:1202-1207`), compare its `{command, timeout}` against `build_ah_hook_command`/`hook_timeout_for_provider` now. Absent entry ⇒ delete (strongest alarm, names the missing entry). Present-but-different ⇒ drift. Everything else in the file — operator hooks, skills, settings, plugins — is **out of scope** and never enters the comparison.

**Why not whole-config `compute_config_hash` (the F-3 correction).** Verified: `compute_config_hash` (`fingerprint.rs:45-79`) hashes the *entire* `hooks` map + `settings` + `plugins` + `skills` + `bundle` (`:61-74`), not ah's entries in isolation. Combined with light-tier "block dispatch until a re-check passes" and G4-Q6's "merge, don't clobber operator config," a whole-config fingerprint is **self-contradictory**: an operator makes any out-of-band edit → whole-config hash ≠ ahd's expectation → drift fires every boot → merge-preserving repair keeps the operator's edit → on-disk *still* ≠ ahd's isolated expectation → re-check never passes → the agent is **permanently blocked on a benign edit.** Scoping to ah's own entries dissolves this: the operator's additions are invisible, so repair→re-check converges, and the block only ever holds for a genuinely missing/wrong ah entry — which repair *can* fix. (G4 no longer shares one definition of "correct config" with realign's whole-home `compute_config_hash`; intentional — G4 asks the *narrower* question "is ah's own wire intact," the only question whose answer ahd can both detect and fully remediate.)

**Per-provider config-file shapes — enumerated, not "claude then same for others" (JC-2/F-4), each re-verified in `src/`:**

| Provider | File | Nesting of the ah-owned Stop entry | Materialize / inject path | Extra gate |
|---|---|---|---|---|
| **claude** | `settings.json` | `hooks.Stop[]` (claude settings schema), pushed as `materialized_ah_hook(ctx,"Stop")` | `materialize_claude_settings` (`home_layout.rs:892`), Stop push `:232` | — |
| **antigravity / agy** | `hooks.json` | top-level wrapper key **`ah-completion-push`** → `Stop[]` → group `{matcher: "", hooks:[{command,…}]}` | `inject_antigravity_hook_push` (`:408-428`) / `merge_antigravity_hooks` (`:358`) | — |
| **codex** | `hooks.json` | top-level wrapper key **`hooks`** → `Stop[]` → group `{matcher: "*", hooks:[{command,…}]}` | `merge_codex_hook_push` (`:1167-1190`) | also requires `features.hooks = true` in codex config (`:1163`); a check ignoring this reports "wired" while codex silently never fires the hook |

The two `hooks.json` shapes are **not** interchangeable (agy wrapper `ah-completion-push` + matcher `""`; codex wrapper `hooks` + matcher `"*"`), so the presence/drift check needs a per-provider locator. The `command` string being compared is provider-agnostic (all route through `build_ah_hook_command`); *where it lives* is not.

**The manual-delete scenario, concretely.** "手工删掉某 agent 的 hook 配置" manifests two ways, both caught, **for whichever provider**: (a) the ah-owned `Stop` entry is gone from that provider's file — presence check (Target 1) fails, alarm names the specific provider + missing entry; (b) the whole config file is gone/empty — "no ah hook wired at all," strongest alarm. Light-tier repair re-materializes **ah's own entries** via the same path `ah up` uses for that provider (claude `materialize_claude_hooks`/`materialized_ah_hook` `:227-235`; agy `inject_antigravity_hook_push`; codex `merge_codex_hook_push` — each merge-preserving via `remove_ah_owned_hook_groups` then re-push, `:419/:1180`) and re-checks; only a passing re-check clears the dispatch block.

> **§五 acceptance:** §五.4 / A4 (the detection half; G4-Q6 is the repair half). **Dogfood:** `G4-wiring-selfheal` (drift arm: hand-edit an ah-owned hook `command` to a stale socket, restart, assert ah-entry-mismatch alarm + repair) — run **once per provider shape** since the locator differs; plus a benign-edit arm (operator adds a non-ah hook/skill → assert **no** alarm, dispatch stays unblocked, proving the F-3 scoping).

## G4-Q4: What is the synthetic-trigger check — how do we know the wire actually carries current?

**Decision: inject a self-check event through the real `ah agent notify` entry point with a reserved self-check `event_id` prefix, confirm it traverses the full R1 transport to a durable consume, then discard it before it can affect state.** The medium tier, per agent, executes **that agent's own provider-specific installed hook command** — the exact `command` string materialized into its config file, as located by the G4-Q3 shape table — with a distinguished synthetic event (`event = "selfcheck"`, `event_id = "selfcheck:{agent_id}:{boot_id}"`). **This exercises each provider's actually-installed hook, not the claude shape as a stand-in (JC-2/F-4):** the failure modes it catches (a `PATH` that lost `ah`, a broken sandbox mount, an unwritable outbox, an unarmed inotify) are per-agent and can differ by provider sandbox, so a claude-only probe would leave codex/agy path-rot undetected. ahd's consume path recognizes the `selfcheck:` prefix as a **liveness probe**: it verifies the outbox record was written and consumed (proving hook → outbox → inotify/scan → consume all work end-to-end) but routes it to a no-op sink instead of `mark_agent_idle_hook_event`, so a probe can never move `agents.state`.

**Rationale.** Config-presence (G4-Q3) proves the hook *string* is correct; it cannot prove the hook *runs* and its output *arrives*. A `PATH` that lost `ah`, a broken sandbox mount, an unwritable outbox, an unarmed inotify — all leave the config byte-perfect while the wire carries nothing. Only pushing a real event through the real path detects those. The synthetic event uses the *same* transport as production (no test-only shortcut) or it proves nothing about production; the only difference is the terminal sink, gated on a reserved id prefix so the probe is inert **by construction**, never by a runtime flag that could be misconfigured. Running it against each provider's installed hook extends that proof from claude to all three wired providers.

> **§五 acceptance:** §五.4 / A4 (catches wiring rot the config check is blind to) + §五.5. **Dogfood:** `G4-synthetic-roundtrip` — break the outbox directory's write permission, run the medium check, assert it warns loudly and surfaces in `ah doctor` (and does *not* silently pass because the config string still looks right); run per provider.

## G4-Q5: What does deep end-to-end on `ah doctor` add over the startup tiers?

**Decision: `ah doctor` runs the full matrix as operator-facing `DoctorCheck`s — including live-daemon consume confirmation and a dead-letter census — with the daemon unambiguously up and no auto-repair.** Deep mode adds `DoctorCheck` entries (`src/cli/doctor.rs`, alongside the existing `daemon_check`, `provider_health_checks`) covering: (a) synthetic round-trip *with confirmed ahd consume* (medium confirms transport up to consume; deep confirms against a definitely-serving daemon and reports round-trip latency); (b) a census of every agent's `outbox/dead/` (R1-Q3) — any dead-letter is a `Warn` with count + first failure reason; (c) the cross-agent wiring matrix (which managed agents are fully wired vs. degraded). Each maps to `DoctorStatus::{Pass,Warn,Fail}` with a `suggestion` (`doctor.rs:15-19`), composing with the existing `has_failures` gate (`:331`).

**Rationale.** The startup tiers optimize for *not slowing boot* and *self-healing what's cheap*; they deliberately skip the expensive, definitely-daemon-up confirmations. `ah doctor` is where an operator asks "is the control path actually healthy right now," so it pays full cost, reports everything, and — crucially — does *not* auto-repair (an operator running doctor wants a diagnosis, not silent mutation mid-investigation). The dead-letter census closes the loop with R1-Q3: quarantined events are visible exactly where an operator looks for health.

> **§五 acceptance:** §五.4 / A4 ("深度端到端检查挂 `ah doctor`" is this section literally) + surfaces §五.1's dead-letter outcomes operationally. **Dogfood:** `G4-synthetic-roundtrip` (doctor arm) + `R1-deadletter` (census arm — a quarantined event shows as a doctor `Warn`).

## G4-Q6: On detection, what is the "loud degradation + auto-repair" behavior — and when is repair unsafe?

**Decision: light-tier detection triggers (1) a loud structured log, (2) a self-check event on the same event spine, (3) an idempotent re-materialize repair, (4) a mandatory re-check that must pass before the agent is dispatch-eligible — repair is *attempted*, never *assumed*.** Sequence on a light-tier failure:

1. **Loud log:** `tracing::error!` (not `warn!`) naming the agent, the specific unwired/drifted target (G4-Q3's presence pre-check gives specificity), and the action taken. "响亮" = error-level and specific.
2. **Self-check event:** emit a `control_path_selfcheck` event through the *same* `events` channel arbiter/transport use, so the incident is durable, auditable, countable — a config that silently self-heals with no record would hide a real operational signal (someone/something repeatedly deleting hook configs). Telemetry, not a state transition.
3. **Auto-repair:** re-materialize the hook config via the shared `ah up` materialization path (G4-Q3), idempotent (the same NO_CHANGE-detecting path issue #13 hardened).
4. **Re-check gate:** immediately re-run the light-tier check; **only a passing re-check clears the dispatch block.** If repair fails (settings file read-only, home gone) the agent stays blocked, the log escalates, and it surfaces as a `Fail` in `ah doctor` — a **fail-closed** posture: an agent whose hook cannot be guaranteed wired is not dispatched to, because doing so would silently resurrect the fire-and-forget void.

**When repair is unsafe / must not be silent.** Auto-repair re-materializes *ah's own hook entries only*, per provider; it must not clobber operator/provider config it didn't author. Every provider's inject path already merges rather than overwrites — claude `materialize_claude_settings`, agy `merge_antigravity_hooks`/`inject_antigravity_hook_push`, **codex `merge_codex_hook_push`** — each via `remove_ah_owned_hook_groups` (drop only ah's prior entries) then re-push (`home_layout.rs:419`, `:1180`, `:1193-1207`). G4 relies on that merge-preserving property; it adds no blind overwrite. **Consistent with the F-3 narrowing:** because the *drift check* looks only at ah's own entries and *repair* rewrites only ah's own entries, the detect→repair→re-check loop operates entirely within ah's namespace — an operator's out-of-band additions are neither flagged (F-3) nor touched, so the loop converges instead of oscillating. If drift is somehow detected in a way repair *cannot* reconcile without discarding non-ah config, that is a `Fail` + escalate, not a forced overwrite. And repair is always *followed by verification* (step 4) — an unverified repair is indistinguishable from no repair.

**Rationale.** This is §五.4 / A4 built as a closed loop: detect (G4-Q3) → alarm loud (1) → record (2) → fix (3) → prove-fixed-or-stay-blocked (4). The fail-closed dispatch gate is the load-bearing safety property: it is *better to not dispatch* than to dispatch to an agent that cannot report back, because a silent completion-signal loss is exactly the disease this whole spec exists to kill (handoff §域1: agy 假完成/假BUSY 占道). The self-check *event* (2) matters because a system that heals silently trains its operators to believe nothing is wrong while someone is, in fact, repeatedly deleting hook configs — the loud log is for the moment, the event is for the trend.

> **§五 acceptance:** §五.4 / A4 in full ("下次启动自检报警并自动修复,期间有响亮降级日志") + §五.5 (the whole loop is one required automated test: delete config → restart → assert alarm event emitted + config re-materialized + re-check passes + agent becomes dispatch-eligible only after). **Dogfood:** `G4-wiring-selfheal` (full-loop arm) — proves §五.4 end to end; a 治愈-实证 verdict only if the automated loop passes *and* a live-stack manual delete is observed self-healing.

---

# Part E — Physical-Evidence Gate (job-level admission gate) · g2

**Decision: physical evidence is a job-level *admission gate* on the completion declaration — it can only DENY (bounce a declaration back with a nudge). It is never a T0–T3 lifecycle signal, never a completion trigger, and it is not part of the arbiter state machine.** This is the single most-attacked area in o1's divergence (§四); the answers are structural.

Placement in the pipeline:

```text
job_done declaration consumed
   is J.requires_physical_evidence (static, set at dispatch)?
     |-- no  --> apply COMPLETED (read-only jobs are never gated)
     |-- yes --> compare git diff (agent's own sandbox worktree) vs dispatch-time baseline
                 |-- diff present (tracked change)        --> apply COMPLETED
                 |-- empty diff, deny_count < 2           --> DENY: bounce nudge
                 |     "SYSTEM DENY: mutating job, no git diff. Produce your change or `ah job fail`."
                 |     (reuses EVIDENCE_DENY_MESSAGE string, state_machine.rs:30)
                 |-- empty diff, deny_count >= 2 (3rd)    --> apply COMPLETED_EVIDENCE_WAIVED
                                                              + emit EVIDENCE_GATE_ESCALATION (human/master)
```

The two guardrails convergence 2.4② mandates:
1. **Static `is_mutating` tag at dispatch.** Reuse `jobs.requires_physical_evidence` (`schema.rs:168`), set by dispatcher/master at submit time. Read-only jobs (research, audit, log-grep) carry `= 0` and are never gated → forecloses o1 §四.1's read-only livelock.
2. **Bounded interception → release + escalate.** After 2 denials the 3rd declaration is *released* so a mis-tagged or permission-blocked job can never wedge forever (Gen-3's 12h park is the anti-pattern). But release is **not** a clean pass: it lands as `COMPLETED_EVIDENCE_WAIVED` and fires a mandatory `EVIDENCE_GATE_ESCALATION` a human sees. This answers o1 §四.1's "拦截上限成安全漏洞": gaming the cap buys a *flagged* completion under human review, not a silent green tick. A flagged pass beats an infinite deny.

o1 §四's contamination attacks, answered:
- **§四.1 signal-level ownership / conflict deadlock.** Resolved by refusing to give the evidence check a T-level at all — orthogonal to T0–T3, never sets state, never completes, only refuses to apply a completion. No level to conflict with; the deny simply bounces the declaration.
- **§四.2 non-causal / non-replayable mutation.** The gate is **edge-triggered at the instant of an explicit declaration**, not a continuous git poll — so "ahd crashes mid-write, rereads git dirty, who wins" does not arise (no background git reader). Baseline snapshotted at dispatch; comparison is `git diff` of **tracked files** in the **agent's own sandbox worktree**. Environmental noise (`target/`, test logs) excluded by tracked-files-only + `.gitignore`. Concurrent human edits on another worktree cannot leak in (scoped to the agent's own worktree).
- **Why not promote 产物轨 into the FSM (o1 §四, whole section).** Promoting it makes it a control signal with the very contamination modes o1 lists. Keeping it a *deny-only admission gate on an explicit declaration* keeps all of git's noise on the DENY side (worst case: false deny → nudge → 3rd-try release), never on the COMPLETE side. Git never *causes* a completion; it can only *delay* one.

> **§五 acceptance:** §五.2 / A2 (a mutating agent cannot silently "带病下班" — no diff, no clean completion). **Dogfood:** DF-5 (mutating + empty diff → denied+nudged, 3rd release+escalate; read-only never gated). **Verification-debt:** the git-diff comparison must be exercised on a real sandbox worktree (DF-5), not only a unit fixture — the tracked-files-only scoping and worktree isolation are the parts most likely to behave differently in vivo.

**Three concrete holes carried to implementation (g1's Track B review, all non-blocking for merge):**
- **NB-4a (mechanism divergence — the important one).** §E's "compare git diff vs dispatch-time baseline" is **not** what the existing machinery does. The existing gate (`evidence_denial_for_job`, `state_machine.rs:1004-1027`) checks for **recorded evidence *events*** via `has_job_evidence_sync(…, &["mtime_changed","diff_generated"])` / `&["test_passed"]` — event-presence, **not** a live `git diff`. `:30` is only the deny *string*; the live-git-diff comparison is net-new **and divergent in kind**. The merged design (this section) flags: implementation must decide whether git-diff becomes the *producer* of `diff_generated` evidence events (fits the existing model) or *replaces* it — two unreconciled evidence mechanisms must not coexist. Also: the existing gate handles a **second dimension, `requires_test_evidence`** (`:1019-1025`, `test_passed`) that this section never mentions — implementation must not silently drop it.
- **NB-4b (guardrail-1 hole).** A mutating job **mis-tagged `requires_physical_evidence=0`** skips the gate entirely with **no flag** — a strictly stronger, unescalated bypass than cap-gaming. Either name it an accepted residual or add a cheap mis-tag detector (a completed read-only job whose worktree is dirty = telemetry, not a gate).
- **NB-4c (guardrail-2 hole).** "tracked-files-only `git diff`" **false-denies new-file creation** — brand-new modules/tests/scaffolds are *untracked*, so a legitimate new-file deliverable shows an empty tracked-diff → DENY → waiver path. Fix: count untracked-non-`.gitignore`d files as evidence (e.g. `git status --porcelain` minus ignored, or `git add -A && git diff --cached`). This is a first-class git-plumbing failure mode, not Windows-only.

---

# Part R3 — Pane Lifecycle Inference Removal (design only, not executed this round) · g2

North-star R3 is explicit: pane lifecycle inference is removed **only after R1/R2 are stable** — "拆早了没有替代信号." This round designs the removal + the substitute-signal coverage table; it deletes nothing. The enforcement tiers referenced below (watchdog / evidence-gate / block) are defined in the **Per-Provider Matrix (Part 5, following)**.

## R3.1 Removal preconditions (all must hold before any deletion PR)

1. **R1 stable** — outbox/ACK/replay live, declarations durable across ahd restart (§五.1 / A1 proven).
2. **R2 stable** — explicit protocol + Stop-hook enforcement live for **≥2 gens** with the 假完成 rate trending to zero (DF-1/DF-3 green, efficacy verdict 治愈-实证 not 未观测).
3. **Every substitute signal in §R3.2 has a green dogfood observation** — no row deleted whose replacement is unproven in vivo.
4. **G4 self-check (Part G4) live** — so a silently-missing hook loudly degrades (§五.4 / A4) instead of falling back to the pane we are about to delete.

## R3.2 Substitute-signal coverage table (each pane-inference site → its T0–T2 successor)

| Current pane-inference site (code) | Post-teardown successor signal | Tier | Precondition |
|---|---|---|---|
| Completion via scraped reply (`collect_reply_for_dispatched_job_sync`, `"screen"`, `state_machine.rs:646`) | Explicit `ah job done`; reply rides protocol (MA-1/MA-4) | **T1** | R2 stable, DF-3 green |
| Dispatch-readiness recheck reads ghost/banner pane text → 恒拒发 (ah#17, obs #24/#36) | Arbiter `agents.state == IDLE` from hook/OS; dispatch gates on *state*, never pane content | **T1/T0** | Arbiter Phase 2 live, DF-4 green |
| STUCK inferred from timeout + pane text | STOPPED_UNDECLARED watchdog alert (MA-3) + arbiter `Stalled` explicit-true signal (convergence 1.3) | **T1 + watchdog** | DF-2 green (no flood) |
| "Background task vs 哑火" distinction from pane output | Declaration present/absent (MA-1) + `workload.scope` cgroup `populated` (arbiter Q3 PoC) | **T1/T0** | Arbiter Q3 PoC passes (C8 gate) |
| `classify_terminality` end_turn/task_complete → completion | Demoted to F2 turn-boundary + watchdog corroboration only (MA-3) | **T1/T2** | R2 stable |
| trust/update interaction dialog driving | **KEPT** — T3's one legitimate job (north-star T3 line) | **T3** | — (not removed) |

## R3.3 What actually gets deleted (the removal PR's scope, next round)

- `collect_reply_for_dispatched_job_sync` + the `"screen"` `reply_source` branch — **all** sites per NB-3 (`state_machine.rs:398` matched, `:646-658`/`:699-702` hook, `:815-827`/`:868-871` log, both `"screen"` defaults `:702`/`:871`), not only the hook path the original §6.3 anchor list named.
- The dispatch-readiness pane-diff gate (the second surviving pane-content-inference site the arbiter design cross-references but does not own; ah#17).
- `classify_terminality`'s authority to return `Terminal` for completion — reduced to turn-boundary emission + `DeferredBackgroundWork` hint text. **Additionally (surfaced by g1's Track B review as a reinforcing removal target):** `classify_terminality` (`parser.rs:48-133`) hard-codes an **antigravity-only** natural-language heuristic (English + Chinese phrase lists + two regexes `:69-129`) and unconditionally returns `Terminal` for every non-agy provider (`:132`) — that is itself reply-text inference driving completion, and it is named here explicitly as an R3 target rather than left oblique.

Everything else pane-related (trust/update dialog) stays. The deletion is **one focused PR gated on §R3.1**, tracked but not authored this round.

> **§五 acceptance:** §五.3 / A3 — once R3 lands, "ghost text/banner appears → zero lifecycle impact" is structurally true, not defended by patch. **Dogfood:** DF-4 — the ah#17 injection test is the acceptance gate for the removal PR, run *before* deleting the dispatch-readiness gate to confirm the successor (arbiter state) already covers it.

---

# Part 5 — Per-Provider Completion-Gap Matrix (with post-convergence truth correction) · g2

This section answers the **two independent double-blind divergence drafts** (`divergence-provider-matrix-o1.md`, `divergence-provider-matrix-g1.md`) reconciled in `convergence-provider-matrix.md`. The convergence pass (a) confirmed the gaps both drafts raised independently (high confidence), (b) judged single-sided ones on merit, and (c) — decisively — **re-verified g1's code-grounding against `src/` and found it corrects the premise this section originally rested on.** The correction is recorded honestly in §5.0 and the rest is re-anchored on it.

## 5.0 Verified baseline — the premise correction (convergence §1; g1 review re-verified TRUE line-by-line)

The brief's framing "claude 有 Stop-hook 抓手、agy 没有" is **not true in the current code.** Verified line-by-line (and independently re-verified in g1's Track B cross-review, incl. `grep -rn '"decision"' src/` → zero hits):

- `CompletionSignalKind` has a **single** variant `LogOnly` (`manifest.rs:29-32`), assigned to **every** provider — bash/codex/claude/antigravity (`:351/:381/:400/:418`). The manifest flattens all heterogeneity to one kind.
- **All three real providers wire the *identical* Stop hook today.** claude (`home_layout.rs:232`), antigravity (`inject_antigravity_hook_push:408-428`, cmd `:420`), **and codex** (`merge_codex_hook_push:1167-1191`, cmd `:1181`) each inject a `"Stop"` hook routing to the **same** `ah agent notify --event stop` (`build_ah_hook_command:674-687`).
- That path is **observe-only**: `handle_agent_notify:838-950` → `mark_agent_idle_hook_event` marks idle; the CLI's `--hook-json` output is a hardcoded `"{}\n"` (`ah.rs:562-564`, test-pinned `ah.rs:1863-1870`) = "allow stop." **No** code path anywhere emits `{"decision":"block"}` — not for agy, and **not for claude either.**

**Consequence for this section's 立论基础.** The real axis is **not "has a Stop *event*" (all three have one) but "does the Stop event carry an enforceable *block* verdict" — and currently NONE of them does.** The claude-vs-agy asymmetry is therefore **future/aspirational, not current**:
- **claude** block is **buildable-by-contract** (Claude-Code's Stop hook *does* honor `{"decision":"block","reason":…}`; the ~5s timeout is ample headroom). MA-2 *builds* it — not a capability we already have.
- **agy** block is **UNVERIFIED** — a Stop hook fires with a generous timeout, but whether gemini's harness *honors* a returned block verdict is undocumented. **First-class open item.** Positive → the asymmetry evaporates and agy shares claude's enforcement tier. Negative → agy is observe-only, a *harness-capability* property, not a "no Stop event" one.
- **codex** block is **UNVERIFIED**, same as agy.

*(Aside: the `5000` vs `5` hook-timeout numbers are most likely a **units artifact** between two harness schemas both meaning ≈5s — not a 1000× gap, not a barrier to enforcement. Convergence §1.3; g1 review confirmed codex also falls in the `_ => 5` arm, so the unit story is honestly "per-harness external contract, unverified here.")*

## 5.1 Per-provider capability grid (replaces the old "physical block" matrix — g1 D6)

Honest grid — **"current" = wired-and-behaving today; "buildable/unverified" = not a present capability.** Cells are deliberately marked unverified rather than asserted.

| Capability | Claude | Antigravity/agy | Codex |
|---|---|---|---|
| Fires a completion-relevant signal today | ✅ `Stop`→`--event stop` (`:232`) | ✅ `Stop`→`--event stop` (`:420`) | ✅ `Stop`→`--event stop` (`:1181`) |
| That signal treated as completion today | ❌ idle-marker only (`LogOnly`) | ❌ idle-marker only (`LogOnly`) | ❌ idle-marker only (`LogOnly`) |
| **Can *block* until declaration** | **buildable** (honors `decision:block`) → MA-2 | **UNVERIFIED** | **UNVERIFIED** |
| Carries a structured payload | via `ah job done --reply-file` (MA-1) — provider-agnostic wire form | same wire form | same wire form; `task_complete` field TBD |
| Distinguishes success/failure | via `ah job fail` (MA-1) | via `ah job fail` | via `ah job fail`; native discriminant TBD (D4) |
| Re-enterable after a nudge | in-turn re-prompt (once block built, MA-2) | **only as a fresh dispatch** (no block → nudge = new turn, A6) | as fresh dispatch |
| Silence profile | active stream, rare dead-silence | reasoning models → long PTY silence (backgrounding *normal*) | batch → total silence bursts |
| Primary backstop under this protocol | Stop-hook block (once built) + watchdog | **watchdog + evidence gate** | **watchdog + evidence gate** |

The shared abstraction is the **declaration wire form** (outbox `job_done` + cookie + reply, MA-1); the holes are in *enforcement*, which is tiered (§5.5), not in the wire form.

## 5.2 agy — the gap is "no enforceable block verdict," not "no Stop event"

**Conclusion: agy's completion is *currently* unenforceable at the source (its Stop event has no verified block verdict), so it leans on *three* backstops in priority order — watchdog, evidence gate, explicit `fail` — and never on inference. Whether agy can be lifted to a claude-like block tier is an open verification item (§5.0), not assumed either way.**

- **Open item first (the honest reframe).** Before any of the below, verify whether agy's `hooks.json` `"Stop"` hook honors `{"decision":"block"}`. Until answered, agy is designed as **observe-only** (the fail-closed assumption). Positive verification migrates agy into MA-2's block tier.
- **The "耍赖不报" gap (o1 §二.1 / g1 A2).** With pane inference removed, an agy that finishes but never calls `ah job done` sits DISPATCHED until the watchdog budget expires → STOPPED_UNDECLARED_ALERT (MA-3). No scrape "rescue," and that is correct: a loud unresolved alert (master cancels/redispatches) is strictly better than a wrong silent COMPLETED. The true progress anchor for agy remains the **产物轨 (fix worktree git HEAD)** — but per Part E that is a *deny gate*, not a completer; master reads it to decide the redispatch, ahd never auto-completes off it.
- **The "suspended-pending-background" third state (g1 E2).** agy's *normal* pattern — end_turn, background task runs, self-wake later — is neither done nor not-done. The watchdog budget *is* the mechanism that tolerates this third state without mis-slotting it as done or failed: "alive + no declaration + long silence" is an *alert threshold*, not a verdict, precisely because for agy it can mean legitimate backgrounding. This is why the same observable means opposite things for claude (anomaly) and agy (maybe-normal).
- **Watchdog budget is per-provider, not global (o1 §二.1/§六.1, g1 A5/C2).** A single hardcoded budget is rejected: too short guillotines legit long turns; too long lets 假BUSY squat 12h. Proposal: budget keyed on `(provider, job.requires_physical_evidence)`, seeded from the existing `MAX_LOG_MONITOR_WAIT = 900s` (`src/completion/monitor.rs:10` — verified `Duration::from_secs(15*60)`; reused not restated, matching arbiter Q2) as the agy floor, with the arbiter's per-signal-class Unknown budgets as the ceiling reference. Exact numbers → DF-6 supplies the live data; **not hardcoded blind.**
- **A nudge for agy is a *fresh dispatch*, not an in-turn re-prompt (g1 A6).** With no block, the MA-3 nudge cannot re-open the same turn — it lands as a new turn with fresh context and can be misread as new work. So the nudge text must be self-identifying against the same `AH_JOB_ID` ("re-declare completion for *this* job, do not restart it"). A concrete divergence from claude's in-turn re-prompt, called out so the two nudge paths are not conflated.
- **turn-end must NOT be reused as a completion probe (o1 §二.2, g1 A5/C3).** Reusing agy turn-end to "probe completion" re-introduces G2 (multi-turn 修改→编译→报错→再修改 each yields control → each turn-end would false-complete). Under this protocol turn-end feeds only F2 + watchdog; never F3. PTY injection / DSR probing stays **rejected** (convergence 1.4/2.4③: fail-dangerous, breaks running external long-connection processes = env pollution / security-boundary击穿 per o1 §二.2, no precedent, violates the投键铁律).
- **Do NOT synthesize a fake block for agy (o1 §五.1, g1 E7 — consensus C12).** Both drafts independently flag the temptation to give agy a claude-like "阻断感" via a PTY polling-block proxy or synthesized signal. Rejected: a self-made sync barrier is a fresh deadlock/race surface and re-introduces exactly the inference pane-text was rejected for. If agy cannot block natively (pending §5.0 verification), it stays in the watchdog tier — we do not fake it.
- **Sudden-death escape (o1 §二.3, g1 E3).** If the agy sandbox is OOM-killed / torn down before it can declare, there is no `ah job done`. Verdict: the job is **not** auto-FAILED off the missing declaration alone. T0 (arbiter's OS-layer: process/scope death) drives the agent to a crash/UNKNOWN verdict; the *job* becomes recovery-eligible (requeue) per the control-plane recovery path — **not** a terminal FAILED that later can't accept a late-but-real result. This mirrors the arbiter Q2 correction (a perception verdict feeds an evidence-check consumer; not a one-way guillotine into terminal Failed). Dirty partial writes in the dead sandbox are exactly what Part E's evidence gate + master's 产物轨 read reconcile on redispatch.

## 5.3 Done-tool "high-confidence lies" — control-flow trust ≠ correctness trust (o1 §三; g1 B-axis)

**Conclusion: the protocol trusts the *declaration* for control flow but never for correctness — the evidence gate + human escalation are the correctness backstop. Two things it does NOT solve are stated as accepted residuals, not hidden.**

- **Two distinct trust boundaries (g1 B2).** "The signal was *delivered* reliably" (transport — R1's outbox solves this exactly-once) and "the signal is *true*" (semantic — unsolved) are different layers. R1 guaranteeing a `job_done` arrives once does **not** make the task done. This protocol owns *transport* attribution + the *deny gate*; it explicitly does **not** own a semantic-truth oracle. Naming this boundary prevents R1's reliability from making the semantic gap *look* solved.
- **讨好式/幻觉 done (o1 §三.1, g1 B1/B3/B4).** A mutating agy declaring done on a broken first pass is caught by Part E: no diff → deny → nudge; or diff-present-but-wrong → COMPLETED, a *known, accepted* residual — the protocol's job is to stop F2-inference and silent scraping, not to judge code correctness (that is the gatekeeper's審計 job, out of this module's authority). **Anti-sycophancy residual (g1 B4):** *mandatory* declaration could make 假完成 **worse** for a reflexive model that learns "I must emit done to end my turn." We do not claim it won't; DF-1/DF-3 must *measure* whether forced declaration raises confident-but-false declarations, and the metric (E6) must be able to falsify "the protocol fixed agy."
- **The non-mutating class is the evidence gate's permanent blind spot — and it correlates with the highest 假完成 risk (o1 §四.1 + g1 B5/C2/C7 — consensus C8).** Read-only jobs (审计, 设计发散, e2e) legitimately produce zero git diff, so Part E never gates them — but that is exactly the class where the worst live 假完成 occurred (job_e817301f: COMPLETED while monitors ran). An **accepted structural residual**, stated plainly: the evidence gate cannot corroborate the highest-risk class, and no git-derived signal will. Its only backstops there are the explicit declaration + human审计.
- **Is "explicit protocol + corroboration" quietly circular? (g1 B7).** Every candidate corroborator is entangled with the model: OS-liveness (T0) answers "alive," not "done"; log-tail (T2) derives from the model's own output; git HEAD is independent of the model's *claim* but not its *actions*. There is **no** completion-corroborating signal genuinely orthogonal to the model. Accepted as an epistemic limit, not pretended away.
- **Honest "I can't" (o1 §三.1, g1 E8).** Resolved by MA-1's `ah job fail --reason`: the model has a first-class FAILED exit, so "I give up" never rides the COMPLETED path and never pollutes downstream job dependencies. This also gives a *negative* completion channel (g1 E8: "I finished and I failed" distinct from "I didn't finish").
- **reply payload错位 is a *separate* trust surface (g1 B6, obs #33).** "task is done" and "here is the result" both ride self-report but can diverge (done=true, payload=garbage). MA-4 treats reply-fidelity as its own concern (reply rides *inside* the declaration, `reply_source="protocol"`), so a trusted done-flag does not launder a garbage payload.
- **Replay / stale-history done (o1 §三.2).** The `AH_JOB_ATTEMPT_COOKIE` epoch check (MA-1) rejects a `done` whose cookie doesn't match the current dispatch epoch — a context-replayed historical `ah job done` is dropped, logged, not applied.
- **Wrong-id "隔山打牛" (o1 §三.2).** The CLI refuses a `<job_id>` not matching the injected `AH_JOB_ID`/cookie before writing anything (MA-1).
- **Parser ghost-call (o1 §三.2).** Because the declaration is a real subprocess exec of `ah job done` writing a real outbox file (not a transcript scrape), "the model *mentioned* `ah job done` in prose" cannot trigger a declaration. There is no transcript-scraping tool-call parser in this path to fool.
- **Intra-provider signal conflict (g1 D7).** agy demonstrably emits *both* a bare `Stop` event (F2 idle-marker) *and*, under this protocol, an explicit `ah job done` (F3). JC-1's `event_id` dedups *transport*; **semantically the explicit declaration wins and the bare Stop is F2-only** — the Stop event never competes with the declaration for the completion verdict.

## 5.4 codex `task_complete` semantic boundary (o1 §一/§五.2, g1 D2/D4)

**Conclusion: `task_complete` is a *turn-boundary* signal (F2), demoted from its current completion role; codex must still emit an explicit done, via a structured bridge if it cannot exec a CLI inline. codex already wires the same Stop hook as the others (§5.0) — so the "future provider" framing understates how much is already in-tree.**

- Today `classify_terminality` treats codex `task_complete` as `Terminal` (`parser.rs:182`, test `:349`), *and* codex injects the same `Stop`→`--event stop` idle-marker (`merge_codex_hook_push:1181`). Under R2 both are demoted: `task_complete` = "the turn's `last_agent_message` is final for this turn" (F2), the Stop event = idle-marker (F2) — neither gates F3.
- **Does codex `task_complete` mean "loop done" or "task done"? (g1 D2).** If it fires at "the agentic loop terminated" rather than a turn boundary, codex imports the same F3≠F2 confusion as agy via a different primitive — demoted for the same reason, not trusted as native completion.
- **Union, not intersection, wire form (g1 D4).** Normalizing three primitives to a lowest-common "done" must **not** discard codex's success/failure discriminant if it has one. The outbox `job_done`/`job_fail` record is a *union* schema (carries per-provider fields, mostly-null for providers that lack them), never an *intersection* stripping the richest provider down to a bare "done."
- If a codex harness genuinely cannot run `ah job done` as an inline subprocess (batch/API execution, o1 §一), the transitional bridge is a **structured done token** the codex adapter (not a generic scraper) emits into `last_agent_message` and converts into a real outbox `job_done`. This is `reply_source = "log_structured"`, explicitly a bridge, explicitly flagged: the one place the protocol tolerates a log-derived declaration, and it must be a structured contract, never free-text inference. (Hardening in Open Items §7-b.)
- **Provider-switch epoch (o1 §五.2, consensus C13).** A late `task_complete`/done from a previous provider (g1 failed → master switched to g2) must not punch through the new provider's epoch. Same cookie/epoch mechanism (MA-1) — the stale signal is scoped to the dead attempt.

## 5.5 Least-common-denominator degradation — refused, but the tiering is *capability-conditional* (o1 §五.1, g1 D6 — consensus C11)

Both drafts' sharpest structural worry: does "one protocol for all providers" force downgrading Claude's block to agy's weak async notify, throwing away the ability to block 带病下班?

**Conclusion: no. The protocol is one *declaration contract*, but enforcement is provider-*tiered*, not lowest-common-denominator — and (correcting the original draft) the tier is assigned by *verified capability*, not provider *identity*.** Claude, once MA-2 builds its block, gets the Stop-hook enforcement tier. agy/codex get the watchdog + evidence-gate tier — **unless §5.0's verification shows their harness honors a block verdict**, in which case they move up. We do **not** cripple Claude to match agy, and we do **not** build a PTY polling-block proxy to fake sync-block (rejected on the same grounds as DSR probing). The wire form (outbox `job_done` + cookie + reply) is identical across all tiers; only *enforcement strength* varies, and it varies by what each provider *provably* supports — not by a fixed "claude strong / agy weak" table, which §5.0 showed does not describe current reality.

## 5.6 Attribution race across teardown (o1 §六.2, g1 E9 — consensus C14)

Fully inherited from arbiter Q4 (not re-designed): outbox `rename()` makes the declaration durable *before* the sender/sandbox can be reaped, so a `done` written just before host-level teardown survives (no "ACK barrier before exit" requirement because durability is achieved at `rename()`, not at ACK). Same sd_notify barrier property (convergence 2.3); Part R1 owns the outbox implementation, this protocol owns only the `job_done` record shape riding it.

## 5.7 Cross-cutting scope & new requirements (g1 E1/E4/D5)

- **Master is a fourth completion class, flagged not solved here (g1 E1).** Verified: `handle_master_notify:978` also just flips BUSY/IDLE on `stop`, no block — master's stop is an idle-marker too, and master's "done" is a different predicate (it orchestrates). This matrix covers **worker** completion; master-self-completion (裸等唤醒) is a sibling concern owned elsewhere, named here so neither spec silently owns the seam.
- **G4's self-check must validate *every* provider's hook path (g1 E4) → discharged by JC-2.** A hook that can't reach ahd's socket from inside a sandbox is a per-provider *silent* completion loss that looks identical to 假完成. This requirement is **discharged in Part G4 (JC-2)**: G4-Q1/Q3/Q4 enumerate all three provider config shapes as first-class check targets, the synthetic round-trip exercises *each* provider's own installed hook, and codex's `features.hooks = true` gate (`home_layout.rs:1163`) is included so a shape-correct-but-inert codex config is not read as "wired." *(Full specification in Part G4; not repeated here.)*
- **Silent contract drift (g1 D5).** claude's Stop-hook contract, agy's `hooks.json` shape, codex's `task_complete` are all *external* contracts that evolve under us (the repo already has `.SUPERSEDED` rule churn). A silent semantic change would manifest as a new 假完成 wave with no code change on our side. There must be a G4-style self-check for the *semantic* contract (does the hook still block/fire as expected?), not just the wiring — Open Items.

> **§五 acceptance:** §五.2 / A2 + §五.3 / A3 across §5. **Dogfood:** DF-6 (agy budget) + DF-1/DF-3/DF-4 as applicable per provider; DF-2/E6 must make per-provider 假完成 rate *measurable* so "did the protocol fix agy" is falsifiable.

---

## Relationship to Other Modules and Tracks

- **Perception arbiter (`ah-perception-arbiter/design.md`):** R1 is the transport whose delivered hook events the arbiter consumes as its T1 signal; R1 reuses arbiter Q4's attribution decision unchanged and is consistent with arbiter Q2's 2s hook budget (R1-Q4). Neither R1 nor R2 writes `agents.state`; R1 hands events to the arbiter's single-writer path, and R2 *removes* completion from the agent-idle transaction (MA-1) so the two authorities stop overlapping. This module's write authority is `jobs.status` COMPLETED/FAILED via `apply_job_done_declaration_sync`.
- **ah-job-events:** `job_transitions` is the pinned carrier. `explicit_done`/`explicit_fail`/`evidence_waived` are new `reason` values on the existing spine (`reply_source = "protocol"` retiring `"screen"`); align exact strings with ah-job-events' existing reason convention, do not fork a pattern.
- **Control plane (`ah-orchestration-reliability` / `ah-control-plane-refactor`):** R1's delivered events, R2's declarations, and G4's self-check verdicts are inputs to the control plane's consumers; none of them owns `jobs.status` beyond the explicit-declaration path. agy sudden-death → recovery-eligible requeue (§5.2) and the `Failed -> Completed` late-evidence reopen live there. **The one seam with genuine correctness risk:** the arbiter's Q2 late-evidence reconciliation (`Failed -> Completed` narrow reopen) and Part E's evidence gate describe overlapping `Failed↔Completed` races from two sides — see Open Items for the required edge-ownership table.
- **Joint contracts:** JC-1 (transport dedup ledger, R1-Q2) and JC-2 (multi-provider G4 coverage, G4-Q1/Q3/Q4) are the two Track-A/Track-B contracts, each specified once above and referenced from the other side (R2-MA-1 → JC-1; §5.7 → JC-2). Both were pinned jointly by the bidirectional cross-review, not unilaterally by either lane.

---

## Open Items Carried to Implementation

Nothing below blocks the design freeze; every item has a correct *direction* and is a completeness/precision/tuning question for implementation. Grouped by source so the review discipline's output is preserved rather than flattened.

### From g1's cross-review of Track B (non-blocking; must resolve before implementation)
- **NB-1 — outbox record size.** Confirm the outbox has no per-record size bound before inlining a multi-KB `--reply-file` `reply_text` into the `job_done` JSON; else carry the reply as a content-addressed file-ref rather than inline. (R2-MA-1 seam with R1.)
- **NB-2 — the bounded-release / block counter is net-new, not a tuning value.** Both MA-2's "N=2 blocks then yield" and Part E's "deny_count ≥ 2 then release" assume a **durable, dispatch-epoch-scoped, reset-on-redispatch integer counter**. `stop_hook_active` (bool re-entry flag) and `has_completion_deferred_event` (content-hash nudge de-dup, `state_machine.rs:1113`) do **not** provide it. Specify where the counter lives, its keying, and its reset-on-epoch semantics.
- **NB-3 — `"screen"`-deletion anchor completeness.** The R3 §6.3 deletion must enumerate all three `collect_reply` sites (`:398` matched, `:650` hook, `:819` log) + both `"screen"` defaults (`:702`, `:871`), and note `:398`'s entanglement with `is_prompt_only_reply`/`classify_terminality`. (Folded into R3 §6.3 above.)
- **NB-4a — reconcile live-git-diff with the existing evidence-*event* model.** Part E's live `git diff` is net-new AND divergent-in-kind from `has_job_evidence_sync` over `mtime_changed`/`diff_generated`/`test_passed` (`state_machine.rs:1004-1027`). Decide: does git-diff *produce* `diff_generated` events (fits the existing model) or *replace* it? Two evidence mechanisms must not coexist unreconciled. Also address the untouched `requires_test_evidence` dimension (`:1019-1025`).
- **NB-4b — mis-tag-as-read-only bypass.** A mutating job mis-tagged `requires_physical_evidence=0` skips the gate with no flag. Name as accepted residual OR add a cheap dirty-worktree-on-read-only-completion detector (telemetry, not a gate).
- **NB-4c — untracked new files false-denied.** Count untracked-non-`.gitignore`d files as evidence so new-module/test/scaffold jobs aren't spuriously routed through the waiver path. A first-class git-plumbing failure mode.

### From g1's per-item ruling on Track B §7 (hardening/lean/seam)
- **§7-b — codex `log_structured` bridge hardening.** Require: (i) the sentinel is a machine token the **codex adapter** recognizes, never a generic scraper; (ii) it **carries the `attempt_cookie`** and passes the same MA-1 epoch check (not a cookie-bypass backdoor); (iii) **fail-closed** — sentinel absent ⇒ job does NOT complete, never falls back to scrape.
- **§7-c — `COMPLETED_EVIDENCE_WAIVED`: lean to a flag/`reason`, not a new terminal status.** A new status ripples into every `status == 'COMPLETED'` consumer (dispatch-readiness, recovery-eligibility, master queries, `runtime_events`) — high blast radius; a flag preserves COMPLETED semantics and matches the "evidence via events, not new statuses" pattern. Escalation stays mandatory either way.
- **§7-e — arbiter Q2 late-evidence seam (ELEVATED — the one correctness-risk seam).** The arbiter's `Failed→Completed` reopen and Part E's evidence gate describe overlapping `Failed↔Completed` races from opposite sides. Non-blocking for merge (named), **blocking for implementation**: produce an explicit **edge-ownership table** — arbiter owns perception-driven reopen for late evidence; this protocol owns declaration-driven `DISPATCHED→COMPLETED/FAILED` + evidence-waived.
- **§7-f — agy/codex block-verdict verification (highest-leverage TODO).** Whether gemini's/codex's `hooks.json` `"Stop"` hook honors `{"decision":"block"}` is an external-harness property, closeable only by (1) external-harness doc/source research, or (2) an **isolated sacrificial e2e** (throwaway agent, independent `STATE_DIR` + independent tmux socket + trap cleanup, synthetic block return observed on a *sacrificial* turn). **Forbidden path:** injecting a block into a live working agent, or DSR/PTY probing the live stack (投键铁律). Keep it a scoped TODO with method = doc-research OR isolated-e2e, **not** live-stack probing. (The isolated-e2e path is executable by lane 1 on request.)

### From Track A's own deferred list (R1/G4 implementation details)
- Exact outbox record JSON schema (field names/types). **JC-1 is settled, not open** (R1-Q2): a dedicated transport-level `outbox_consumed(event_id TEXT PRIMARY KEY, consumed_at INTEGER)` applied before `kind`-routing — not a `UNIQUE` on `events`, not on `job_transitions`. Open: only the table's exact columns and its retention/reap policy, pinned jointly with the R2 side (which routes `job_done`/`job_fail` through the same boundary).
- Cold-scan sweep cadence + periodic-resweep interval (inotify-miss backstop) — a concrete value tuned against ahd tick latency, subordinate to arbiter Q2's hook-timing numbers, not a new independent budget.
- Dead-letter retention/reap policy for `outbox/dead/` — reuse the `events` reap policy if one exists; if none, a pre-existing gap G4-Q5's census surfaces, not one this module must solve.
- `selfcheck:` event-id prefix reservation + the no-op sink wiring (G4-Q4) — exact constant/routing point is an implementation decision; the invariant (reserved prefix ⇒ inert by construction, not by runtime flag) is fixed.
- Whether the light-tier dispatch block is a per-agent flag on the `agents` row vs. an in-memory gate — match how existing dispatch-readiness gating is stored; do not introduce a new persistence pattern for this one flag.

### From Track B §7 (design-round deferrals)
- **N (Stop-hook block count) + per-provider watchdog budgets** — tuning parameters seeded here (N=2; agy floor = `MAX_LOG_MONITOR_WAIT` = 900s), finalized against DF-2/DF-6 live data; note the *counter mechanism* itself is net-new (NB-2), not a knob.
- **Exact `job_transitions.reason` value set** (`explicit_done`/`explicit_fail`/`evidence_waived`) — align with ah-job-events' existing reason strings.
- **Semantic contract-drift self-check (g1 D5)** — G4 must assert not just that the hook is *wired* but that it still *behaves* (blocks/fires as expected); an external harness silently changing completion semantics would surface as a new 假完成 wave with no code change on our side.
- **产物轨 git-plumbing failure modes + Windows (convergence C5)** — index-lock contention, detached HEAD, amended/squashed commits, submodules, git-not-on-PATH, **+ untracked-new-files (NB-4c)** as a first-class mode; on the Windows-native target the evidence path may degrade to platform-conditional. Harden Part E's gate against these before trusting it on non-Linux.
- **Coalesced-turn attribution (convergence E9)** — a provider that coalesces two dispatches into one turn may emit one completion signal for two jobs; the per-attempt cookie is necessary but the coalescing behavior itself must be tested per provider.
- **Per-provider 假完成 fidelity metric (convergence E6)** — without a per-provider completion-signal-fidelity measurement, "did the protocol fix agy specifically" stays unfalsifiable from the data we collect (ties DF-1/DF-2/DF-3 to a per-provider breakdown).
- **Master as a fourth completion class (g1 E1)** — flagged not solved here; master-self-completion is a sibling concern owned elsewhere.
- **Parked (log-only, convergence §3):** E5 onboarding/first-turn window (hook not yet wired but agent already accepts a task — owned more by the sandbox/onboarding path); A7 non-Stop hook points (tool-call/pre-exit/heartbeat as alternate carriers — speculative until each harness's hook surface is known).

### Resolved during the design round (recorded for provenance — g2's cross-review of Track A)
These were the non-blocking corrections in `review-track-a-by-g2.md`; all folded into Track A rev-2 and carried into this merge, so they are **closed**, not open:
- **F-5** — "reuse over invention" wording corrected: the `outbox_consumed` ledger is a real schema delta (migration), not free reuse of a non-existent `events.event_id` column. (Reflected in R1-Q2.)
- **F-6** — selfcheck `event_id` is exempt from R1-Q3's ULID ordering assumption and still takes a JC-1 ledger row (a crash-surviving selfcheck file re-scans as a harmless no-op). (Reflected in R1-Q3.)
- **F-7** — `.tmp` → `.json` naming pinned to one convention so the sandbox-side writer and host-side `*.json` scanner agree on the glob. (Reflected in R1-Q1.)
