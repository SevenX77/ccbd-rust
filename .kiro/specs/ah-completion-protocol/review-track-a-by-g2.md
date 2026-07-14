# Track A Cross-Lane Review — by g2 (泳道2 闸门)

Reviewing `design-draft-track-a-g1.md` (R1 outbox/ACK/replay + G4 control-path self-check). Cross-lane review, not self-review — I challenge freely and verified every load-bearing claim against `src/` first-hand rather than trusting the draft's self-citations.

**Method note (物理实证).** Files/lines I re-read to ground this review:
- `src/bin/ah.rs:513-576` (`cmd_agent_notify`, `format_agent_notify_output`), `:518/:532/:547-557/:562-564`
- `src/db/state_machine.rs:600-736` (`mark_agent_idle...` CAS + swallow + `events` INSERT), `:619-629/:706/:716/:727/:733`
- `src/db/schema.rs:124-195` (`events`, `jobs`, `job_transitions` DDL)
- `src/provider/fingerprint.rs:18-79` (`ConfigFingerprintInput`, `compute_config_hash`)
- `src/provider/home_layout.rs:227-233/:358/:408/:674/:722/:892/:1167` (claude vs antigravity vs codex hook injection)

Every code line cited below I opened myself.

---

## Verdicts against the four review targets

| # | Master's question | Verdict |
|---|---|---|
| 1 | R1 outbox/ACK/replay — real "journal-first, replay-on-restart", or gaps (journal-write failure, ID-space conflict with Track B `job_transitions`)? | **REJECT (2 blocking gaps): F-1 journal-write-failure unspecified; F-2 idempotency-ledger placement cannot cover Track B's `job_done`.** |
| 2 | Attribution race — did g1 distort/rewrite arbiter Q4? | **ACCEPT.** Faithful reuse, verified line-by-line against Q4 original. One trivial naming nit (F-7). |
| 3 | G4 light-tier "blocking self-repair" — does it wedge ahd's own early boot? | **ACCEPT g1's boot-deadlock reasoning (it is sound).** But a *different* failure lives in the same tier: **F-3 (blocking) re-check non-convergence under benign operator edits.** |
| 4 | G4 vs Track B §5 — does the self-check only cover claude-shaped hooks, missing agy/codex? | **REJECT (blocking): F-4.** Confirmed — G4 is grounded entirely on the claude shape; it does not discharge Track B §5.7's registered cross-track requirement. |

Overall: **draft is directionally right and well-grounded, but NOT freeze-ready.** Four blocking items (F-1..F-4), each with a small scoped fix. Three non-blocking corrections (F-5..F-7).

---

## ACCEPTs (with evidence)

### A. Q4 reuse is faithful — no distortion (master Q2)
I read `ah-perception-arbiter/design.md` Q4 (`:87-110`) in full and compared to Track A's "What This Draft Reuses" (`:7`) + R1-Q1 (`:34-38`).

- "durable local write before any socket attempt" — Q4 `:89`. g1 reproduces exactly (R1-Q1 step 3 demotes RPC to optimization). ✓
- "daemon-side inotify + cold-scan-on-restart" — Q4 `:98`. g1 R1-Q3 `:60` reproduces, and correctly pins the *timing* (before serving) which Q4 left implicit — an addition, not a rewrite. ✓
- Per-dispatch-attempt cookie `AH_JOB_ATTEMPT_COOKIE = "{job_id}:{dispatch_seq}"`, format pinned by implementer — Q4 `:106`. g1 `:34` reproduces verbatim incl. the "R1 does not mint it" boundary. ✓
- "does not depend on the sender process still being alive" — Q4's `rename()`-durability property `:108`. g1 R1-Q4 producer-side-none `:75` reproduces. ✓

g1 did **not** smuggle in either refuted red line (K8s asymmetric absence default / K8s-API single-write) — R1's opening `:9` explicitly disavows both, matching the convergence red-lines. **ACCEPT, no reservations.**

### B. Light-tier does not deadlock ahd's boot (master Q3) — g1's weighing is correct
Independent judgment (not rubber-stamped because g1 conceded it): the light-tier block is scoped to **per-agent dispatch-readiness**, explicitly *not* ahd boot (G4-Q2 `:108` "Does not block ahd's own boot (other agents proceed)"). The self-deadlock the master worries about would require the *repair* to depend on a not-yet-serving subsystem; but repair = filesystem re-materialization (`materialize_claude_hooks`, home_layout.rs:227), which has no socket/RPC dependency. And g1 correctly makes the **medium** tier (synthetic round-trip through the live daemon) non-blocking for *exactly* the boot-self-probe-deadlock reason (G4-Q2 `:112`: "ahd refusing to serve because its own not-yet-serving socket didn't answer a synthetic probe"). That is the right cut. **ACCEPT the block/warn boundary as drawn** — the boot-deadlock is genuinely avoided. (The tier still has a separate defect: F-3.)

---

## BLOCKING findings

### F-1 [BLOCKING] R1 has no answer for the durability-commit failing — the exact inverse the fix exists to prevent
**Master Q1.** R1-Q1 (`:32-40`) specifies: build record → write `.tmp` + `fsync` + `rename()` (the "durability commit point") → *then* RPC fast-path. It handles RPC failure carefully ("failure … is a non-error … the hook exits `0`", `:36`). R1-Q4 doubles down: "the hook's guarantee is the `rename()`; it exits `0` immediately after" (`:75`).

**But nowhere does R1 specify what the hook does when the `fsync`/`rename()` itself fails** — `ENOSPC` mid-write, `fsync` `EIO`, `rename` `EROFS`, a full or read-only outbox. The draft's exit-0-safe posture is scoped *only* to RPC failure; the durability-commit-failure control flow is unspecified, and the default reading ("it tried to write, then exits") is **silent loss** — the hook exits 0 believing it's durable when nothing landed. That is precisely the fire-and-forget-into-the-void G1 disease R1 exists to kill, re-entering through the back door.

Arbiter Q4 does **not** cover this: it addresses outbox *access denied* ("loud escalation to sandbox design", `:110`) and disk-full-as-operational-alert, but not the transient-write-failure control flow of the hook process itself.

Code baseline for contrast: today (ah.rs:547-557) the hook returns `Err` → exits **non-zero** on RPC failure. R1 correctly flips *that* to exit-0-safe — but must not simultaneously let a *journal* failure exit 0.

**Required before freeze:** R1-Q1 must state the durability-commit-failure branch explicitly — journal write/fsync/rename failure ⇒ **loud, non-zero exit / provider-visible error**, never silent exit-0. "先 journal" only holds if a *failed* journal is loud, not swallowed.

### F-2 [BLOCKING] The idempotency ledger, as grounded, structurally cannot dedup Track B's `job_done` — the dedup must be at the outbox-consume boundary, not on the `events` table
**Master Q1 "ID-space conflict" sub-question — answered in two parts.**

**(a) No value-collision.** Track A `event_id` (ULID/UUIDv7, hook-minted, R1-Q1 `:34`) and Track B's `job_transitions.job_event_id` (`INTEGER PRIMARY KEY AUTOINCREMENT`, DB-minted — verified schema.rs:176) are disjoint namespaces. So "ID space conflict" in the collision sense: **no.** g1 is safe there.

**(b) But the ledger's *home* is a real seam, and it's mis-grounded.** R1-Q2 (`:47-49`) says: "Reuse over invention: the existing `events` table already stores an `event_id` … The ledger is a `UNIQUE(event_id)` constraint … on that path, not a new table." Verified facts contradict the "reuse" premise and expose the seam:
- `events` (schema.rs:124-135) has **no `event_id` column.** `event_id` is embedded inside the JSON `payload` string (state_machine.rs:727). You cannot put `UNIQUE(event_id)` on a JSON-blob field without a migration (generated column / expression index).
- The existing UNIQUE idempotency index on `events` is on **`(agent_id, request_id)` WHERE request_id IS NOT NULL** (schema.rs:135) — keyed on `request_id`, not `event_id`. And the hook-event INSERT passes **`request_id = NULL`** (state_machine.rs:733), so today that index doesn't even cover the hook path. (This *confirms* g1's motivating claim that CAS is the only current guard — good — but it also means the "reuse" is not reuse.)
- **Critically for the cross-track seam:** Track B's `job_done`/`job_fail` records are consumed onto **`job_transitions`** via a *different* path (`apply_job_done_declaration_sync`, Track B MA-1), a table with no `event_id` column. A `UNIQUE(event_id)` bolted onto `events` provides **zero** dedup for Track B's outbox-delivered declarations. Under at-least-once redelivery (R1's whole premise), a replayed `job_done` outbox file would be **double-applied** — Track A believes the ledger covers it; it does not.

**Required before freeze:** Track A owns the *transport*, so it must own a **transport-level dedup ledger** keyed on the outbox record's `event_id`, checked at the outbox-consume boundary **before routing by `kind`** to either the F2 `events` path or the F3 `job_transitions` path. One dedup table for all outbox records — not "reuse the events table's event_id." Pin this as a Track-A/Track-B contract; right now the two drafts silently assume different homes for the same key.

### F-3 [BLOCKING] G4-Q3's whole-config fingerprint drift-check + block-until-re-check-passes is internally inconsistent with G4-Q6's "merge, don't clobber" — benign operator edits ⇒ permanent dispatch block
**Master Q3, adjacent failure (not the boot-deadlock, which is fine).** G4-Q3 (`:121`) detects drift by "recompute the fingerprint from what it expects vs what is on disk … Mismatch = drift; loud alarm + repair," reusing `compute_config_hash`. G4-Q2 makes light-tier **block dispatch until a re-check passes** (G4-Q6 step 4, `:155`). G4-Q6 *also* promises repair "merges rather than overwrites … must not clobber operator or provider config it didn't author" (`:157`).

Verified: `compute_config_hash` (fingerprint.rs:45-79) hashes the **entire** `hooks` map + `settings` + `plugins` + `skills` + `bundle` (`:61-74`) — **not** ah's own hook entries in isolation. Combine the three commitments and they contradict:
- An operator makes any out-of-band edit to the managed home (add their own hook/skill/setting) — a case G4-Q6 *explicitly designs for*.
- Whole-config fingerprint ≠ ahd's expectation ⇒ **drift fires** every boot.
- Merge-preserving repair keeps the operator's addition (as promised) ⇒ on-disk still ≠ ahd's isolated expectation ⇒ **re-check still fails** ⇒ the agent is **blocked from dispatch indefinitely** on a benign edit.

The baseline semantics are the crux and are unspecified: if "expected" = fingerprint of source-declared config, benign out-of-band edits false-block forever (above). If "expected" = fingerprint of the merged live result, the check is tautological (on-disk always equals itself → always passes → detects nothing). Either reading breaks the light-tier gate.

**Required before freeze:** scope drift detection to **ah's own materialized hook entries** — the presence-check granularity G4-Q3 already uses for the delete case (`:123`) — OR define the fingerprint baseline precisely as the merged-materialized result and drop the block-until-match coupling. As written, "reuse whole-config `compute_config_hash` + block-until-re-check-passes + merge-don't-clobber" cannot all three hold.

### F-4 [BLOCKING] G4's self-check is grounded entirely on the claude hook shape — it does not discharge Track B §5.7's registered cross-track requirement to cover agy + codex
**Master Q4 — confirmed.** Track B §5.0 established (code-verified in the convergence pass) that **all three** providers wire the *same* observe-only Stop hook, each with a **different config-file shape**:
- claude — `settings.json` shape, `materialize_claude_settings` (home_layout.rs:892), Stop push at `:232`.
- antigravity/agy — `hooks.json` shape, `inject_antigravity_hook_push` (home_layout.rs:408), `merge_antigravity_hooks` (`:358`).
- codex — `hooks.json` shape, `merge_codex_hook_push` (home_layout.rs:1167).

And Track B §5.7 (this is g1's *own* E4 divergence point) **explicitly registered the requirement against G4**: "G4's self-check must validate *every* provider's hook path … assert agy's `hooks.json` and codex's `hooks.json` wiring … not just claude's `settings.json`."

Track A's G4 does not discharge it:
- G4-Q1 target 1 (`:93`) cites only `home_layout.rs:230-232` (the **claude** Stop/UserPromptSubmit push) and `build_ah_hook_command`.
- G4-Q3's repair path (`:123`) names only `materialize_claude_hooks`/`materialized_ah_hook` — **claude**.
- G4-Q6 (`:157`) mentions `merge_antigravity_hooks`/`materialize_claude_settings` in the don't-clobber aside — antigravity partially acknowledged, but **codex is never named anywhere in G4.**
- The synthetic-trigger (G4-Q4 `:132`) "execute the actual installed hook command" is described against the claude shape; it does not account for the `hooks.json` shapes.

Net: a deleted or drifted **codex** or **agy** Stop hook — the exact "零件好的忘了装" this self-check exists to catch — would go **undetected for 2 of 3 providers**, and a per-provider silent completion loss looks identical to 假完成 (Track B §5.7's own argument). Track A silently narrowed a cross-track requirement back to claude.

**Required before freeze:** G4 must enumerate the three provider config-file shapes as first-class check targets (claude `settings.json`, agy `hooks.json`, codex `hooks.json`) in G4-Q1/Q3, and the synthetic round-trip (G4-Q4) must exercise **each** provider's actually-installed hook, not the claude shape alone. This is a hard dependency Track B is counting on (§5.7, §Cross-references).

---

## NON-BLOCKING corrections

### F-5 [non-blocking] "Reuse over invention" (R1-Q2) overstates — the existing idempotency primitive is `request_id`, not `event_id`
Folds into F-2's fix. The events table has no `event_id` column and its existing UNIQUE is `(agent_id, request_id)` (schema.rs:135). Adding an `event_id` ledger is a **migration**, not free reuse. Stop calling it "not a new table"; state the actual schema delta (either extract `event_id` to a real/generated column, or — preferred per F-2 — a dedicated transport dedup table). Non-blocking once F-2 is resolved.

### F-6 [non-blocking] Selfcheck `event_id` breaks R1-Q3's ordering assumption; ledger interaction unspecified
R1-Q3 (`:62`) replays oldest-first assuming `event_id` carries a time-sortable ULID/UUIDv7 prefix. G4-Q4 (`:132`) mints `event_id = "selfcheck:{agent_id}:{boot_id}"` — a fixed non-ULID string that does not time-sort. Harmless in isolation (routed to a no-op sink, so replay order is irrelevant for it), but the two sections should (a) note reserved-prefix ids are exempt from the ordering assumption, and (b) state whether a selfcheck record takes an idempotency-ledger row (it should, or a crash-surviving selfcheck file re-runs the no-op — benign but worth pinning). Non-blocking.

### F-7 [non-blocking] `.tmp` suffix naming inconsistency across the three drafts
Track A R1-Q1 (`:35`) uses `{event_id}.json.tmp` → `{event_id}.json`; arbiter Q4 (`:93`) and Track B both use `{event_id}.tmp` → `{event_id}.json`. Trivial, but pin one convention in the merged design so the sandbox-side writer and host-side scanner agree on the glob. Non-blocking.

---

## Freeze recommendation

**Do not freeze Track A until F-1..F-4 are addressed.** All four are small, scoped fixes (a failure branch, a ledger-placement contract, a drift-granularity choice, a provider-enumeration) — none require re-architecting. F-2 and F-4 are specifically **cross-track contracts** with my Track B (ledger home; multi-provider G4 coverage) and must be pinned jointly, not resolved unilaterally by either lane. F-5..F-7 can ride the same edit pass. The core theses (journal-first transport; CAS-is-accidental-idempotency; three-tier self-check with the block/warn line where g1 drew it) are **sound and code-grounded** — the gaps are at the edges, which is exactly where a transactional-outbox + self-heal design lives or dies.

---

## rev-2 re-check (2026-07-10) — all four blocking items ACCEPT

g1 revised `design-draft-track-a-g1.md` to rev-2. I re-verified each fix against the new draft **and** against `src/` (not against g1's self-report).

| Item | rev-2 fix | Re-check | Verdict |
|---|---|---|---|
| **F-1** | R1-Q1 `:45` adds an explicit durability-commit-failure branch: `.tmp` write, its `fsync`, the `rename`, **and the directory `fsync`** are each checked; any error ⇒ `tracing::error!` + cleanup + **non-zero exit**. Invariant pinned: **"exit 0 ⇔ a durable outbox record exists."** exit-0-safe is now scoped *only* to step-3 RPC failure. | No new back-door exit-0: I read `:43` (RPC-failure exit-0) and `:45` (journal-failure non-zero) — the two are cleanly separated and the invariant is stated. The added directory-`fsync` is a correctness bonus (rename durability). | **ACCEPT** |
| **F-2** | R1-Q2 `:56` moves dedup to a dedicated **transport-level ledger** — `INSERT INTO outbox_consumed(event_id) … ON CONFLICT DO NOTHING`, run for *every* record **before** the `kind` fork, dedup+effect in one tx. `:61` documents why `events`-table reuse is structurally wrong; `:63` + Open Items `:200` pin it as a settled Track-A/Track-B contract (`outbox_consumed(event_id TEXT PRIMARY KEY, …)`). | Design covers **both** consume paths: hook events (F2→`events`) and `job_done` (F3→`job_transitions`) share one globally-unique-`event_id` gate upstream of routing; cold-scan replay (R1-Q3 `:74`) re-enters the same gate; same-tx apply-and-ledger closes the crash window. Schema facts g1 cites (`events` has no `event_id` col; `job_transitions.job_event_id` autoincrement, disjoint namespace) match what I verified round-1. | **ACCEPT** |
| **F-3** | G4-Q1 target 2 `:108` + G4-Q3 `:135-137` narrow drift detection to **only ah-owned entries** via `is_ah_owned_hook_item`; whole-config `compute_config_hash` is explicitly **not used**. Convergence of the block+repair loop under benign operator edits is now structural (G4-Q2 `:122`, G4-Q6 `:183`). | Verified `is_ah_owned_hook_item` at `home_layout.rs:1202-1207` = `command.contains("ah agent notify")` — exactly the injector's own predicate (`remove_ah_owned_hook_groups:1193`), so "correct config" is definitionally ah's-own-entries. No shadow `compute_config_hash` remains in the drift path. The permanent-block-on-benign-edit contradiction is dissolved. | **ACCEPT** |
| **F-4** | G4-Q1/Q3/Q4 `:107/:139-151/:158` enumerate all three shapes as first-class targets, per-provider locator, synthetic round-trip per provider's own hook. G4-Q3 shape table `:141-145`. | Verified every cell: claude `settings.json` Stop push `:232`; agy `hooks.json` wrapper **`ah-completion-push`** + matcher `""` (`:409/:422`); codex `hooks.json` wrapper **`hooks`** + matcher `"*"` (`:1170/:1183`) — the two `hooks.json` shapes are genuinely non-interchangeable, so a per-provider locator is mandatory (g1 is right). **codex `features.hooks = true` extra gate confirmed at `home_layout.rs:1163`** (in the codex *config*/TOML path, separate from `hooks.json`): a shape-correct `hooks.json` is inert without it — g1 caught a real false-positive vector I missed in round 1, and correctly encoded it as a G4 check target. | **ACCEPT** |

**rev-2 verdict: all 4 blocking items resolved, code-grounded. Track A is freeze-ready** pending the two cross-track contracts being mirrored on the Track B side (done below). The rev-2 draft also folds in non-blocking F-5 (reuse wording corrected), F-6 (selfcheck id ordering-exemption + ledger row, R1-Q3 `:76`), F-7 (`.tmp` naming pinned, R1-Q1 `:42`).

### Track B reciprocal patch (this review's step 2)
Both contracts g1 pinned on the Track A side are now mirrored in `design-draft-track-b-g2.md` (bidirectional reference, no content rewrite):
1. **MA-1 wire form** — `job_done`/`job_fail` explicitly routed through R1-Q2's `outbox_consumed(event_id)` transport dedup boundary (dedup-before-`kind`-routing); Track B no longer assumes the `events`-table `event_id` covers its `job_transitions` path.
2. **§5.7 (g1 E4)** — marked **discharged by Track A rev-2**, with the three-shape enumeration + the codex `features.hooks = true` gate recorded as adopted from g1's catch.
3. Cross-references section — the "Delivery/attribution" and "G4 self-check" bullets updated to name the shared ledger and the discharged multi-provider coverage.
