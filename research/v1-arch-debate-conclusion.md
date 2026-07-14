# v1 Architecture Health Review — Debate Convergence (Conclusion)

> Master synthesis, 2026-07-09. Inputs: a3's assessment (`research/v1-release-readiness-arch-review.md`), a3's round-2 rescued reply (`research/v1-arch-debate-round2-a3-reply-rescued.txt`), master hand-verification against code, and the full-contract e2e results (6/6 PASS @ `26016e4`). This is the authoritative convergence; where the review doc still shows a3's un-conceded round-2 list, THIS document overrides it.

## Method note
Every a3 claim was hand-verified against the code (a3 has a citation-overclaim history). a3's round-2 "defense" of its four (c) items was a verbatim re-assertion + miscitation, not counter-evidence — verification overrides. Attribution buckets: **(a)** covered by the frozen orchestration-reliability spec; **(b)** Windows-native track (`.kiro/specs/ah-windows-native/`, user-designated high priority); **(c)** genuine NEW v1 release blocker; **(d)** post-v1 debt.

## Per-item verdict + attribution

| Item | Verdict | Evidence | Attribution |
|---|---|---|---|
| **P0#1** Windows command spawn broken | **ACCEPT** | `windows/scope.rs` pushes `format!("{key}={value}")` into the command array (Unix env-prefix; Windows can't exec it) | **(b)** Windows-native (m0/m1) |
| **P0#2** pidfd stubs on Windows/macOS | **ACCEPT** | `process.rs` returns `EnvironmentNotSupported`; macOS stub notes "until PR-4 process reaper" | **(b)** Windows-native (Windows) + **(a)** reliability D2 (macOS PGID reaping) |
| **P0#3** STUCK "permanently wedged" | **ACCEPT but RESTATED** | Overstated — gated recovery path exists (`late_health_completion_stuck_allows_terminal`, `state_machine.rs:845`); real but narrow-gate fragility | **(a)** reliability (D3/D4). NOTE: the false-STUCK that killed a3's round-2 twice is a LIVE instance of this family (log-monitor 300s timeout / health-timestamp priority) |
| **P0#4** Non-atomic destructive DB migrations | **ACCEPT** | `db/mod.rs:128-155` + `:362`: `RENAME TO ..._old` → copy → `DROP`, sequential `migrate_*`, no transaction envelope, no `schema_migrations` table → corrupt-on-interrupt during integrator UPGRADE | **(c) v1 BLOCKER** |
| 3.1.1 / 3.1.2 dispatch races, PROMPT_PENDING blocks | ACCEPT | — | **(a)** reliability (§3.2, §4 Mech 3) |
| 3.2.1 SQLite mutex contention | ACCEPT | scalability | **(d)** post-v1 |
| **3.2.2** `master_last_exit_reason` "missing from RuntimeSnapshot" | **REJECT** | FALSE — `RuntimeSessionSnapshot` **has** `pub master_last_exit_reason: Option<String>` at `runtime_events.rs:87` (a3 miscited `:71-83`); e2e confirmed it in `status --json` | not a gap |
| 3.3.1 / 3.3.2 unwired orphan reconcile, ephemeral master-watch | ACCEPT | — | **(a)** reliability (§3.1, §4 Mech 1&2) |
| 3.4.1 uncorroborated T3 matches | ACCEPT | — | **(a)** reliability (§3.3 T3 corroboration) |
| **3.5.1** session status hidden in `ah ps` | **REJECT** | FALSE — `SessionRow.status` exists (`cli/output.rs:13`), `--json` carries it, and the e2e PROVED `ah ps` shows a `status` column (`--all` renders `CLOSED`) | resolved / not a gap |
| 3.6.2 macOS launchd/plist placeholder | ACCEPT | macOS packaging | **(d)** post-v1 / macOS |
| 3.7.1 sandbox fs/network isolation | ACCEPT | hardening | **(d)** post-v1 |
| **3.8.1** dynamic C deps (openssl/libsqlite3) break cargo-dist | **REJECT (Linux v1)** | FALSE for Linux — `Cargo.lock` has `rustls`+`ring`, **no openssl**; `rusqlite` is `bundled` on Linux/Windows (macOS-only non-bundled at Cargo.toml:47) → Linux v1 binary is self-contained | **(d)** macOS-only / post-v1 |
| 3.9.1 test collision deflaking | ACCEPT | test env | **(d)** post-v1 |
| **NEW (e2e)** master inherited-env `AH_AGENT_ID` not unset | **ACCEPT** | e2e Surface 6: `inject_master_identity` strips caller-supplied (RPC `extra_env_vars`) `AH_AGENT_ID` correctly, but does NOT unset a stray `AH_AGENT_ID` inherited from the daemon's own process env → leaks to master. Defense-in-depth gap, not a contract failure | **v1 hardening** (designated pilot task) |

## Final v1 PRE-RELEASE MUST-FIX LIST
1. **P0#4 — DB migration atomicity.** Wrap the migration set in a single transaction + add an idempotent/resumable `schema_migrations` version table. The lone verified (c) blocker; hits v1's install/upgrade contract for external integrators. (Schedule after this attribution is finalized.)
2. **Master inherited-env identity scrub** (from e2e). Small, self-contained: have `inject_master_identity` (and worker injector) unset inherited `AH_AGENT_ID`/`AH_ROLE`/`AH_SESSION_ID` from the process env, not just the caller map. **Designated pilot first-task** for the new implementer.

## Attribution buckets (summary)
- **(a) frozen orchestration-reliability spec:** P0#3 (STUCK + the false-STUCK bug family), dispatch races, PROMPT_PENDING blocks, orphan-reconcile wiring, ephemeral master-watch, uncorroborated T3, macOS reaping (D2).
- **(b) Windows-native track (high priority):** P0#1 Windows spawn, P0#2 Windows pidfd.
- **(c) v1 pre-release blockers:** P0#4 only.
- **(d) post-v1 debt:** SQLite mutex contention, macOS launchd/plist, macOS non-bundled sqlite / 3.8.1, sandbox isolation, test deflaking.
- **REJECTED (verified non-gaps):** 3.2.2 (field exists), 3.5.1 (status shown, e2e-confirmed), 3.8.1 for Linux (rustls + bundled sqlite).

## Provenance caveat
a3's original review was a competent broad-architecture scan but (i) marked platform/deep-arch items P0 against an imagined broad "v1" rather than the approved narrow v1 (packaging/rules/install/release, Linux), and (ii) over-claimed 3 of its 4 defended "must-fix" items (data/deps actually present). The net real v1-surface risk it surfaced is P0#4. Its value was real (P0#4 + the attribution map); its severities needed verification — which is why every safety/blocker claim here is code-anchored.
