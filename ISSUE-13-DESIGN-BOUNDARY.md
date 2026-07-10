# Issue #13 — Design Boundary Note: config-fingerprint normalization (FINAL)

**Author:** g1 (泳道1 gatekeeper)
**Branch:** `fix/issue-13-respawn-storm` (worktree `ccbd-rust-wt-issue13`)
**Status:** **FINAL** — o1's adversarial challenge (`ISSUE-13-DESIGN-CHALLENGE.md`) adjudicated below (§5). Supersedes the earlier DRAFT. No production code changed in this phase; the RED tests that encode these contracts are committed alongside this note.
**Prereqs:** `ISSUE-13-RESEARCH.md` (root cause), `ISSUE-13-DESIGN-CHALLENGE.md` (o1's challenge).

Root cause (recap): the stored `config_hash` (`ah start` → `agent.spawn`) is computed over the **fully-injected effective env**, while the realign expected hash (`ah up` → `session.realign`) is computed over the **raw declared env** — two different env sets, so *every* agent is judged DRIFT on the first `ah up`, forcing a全员 respawn storm.

---

## 0. What this note decides (FINAL)

The fix is a **bare-config fingerprint**: hash exactly the user-declared config, never the runtime-injected/derived values, computed the **same way on both sides**. Five design seams are settled below; the meat:

- **Normalization boundary (§1):** hash env = `merge(config.env, agent.env)`; every runtime-injected/derived var excluded.
- **Env construction is centralized on the SERVER (§3, adopting o1 #4):** the RPC carries raw `agent.env` + a top-level `config_env`; the server merges. Clients never pre-merge. This makes it structurally impossible for two call sites to build the hashed env differently — the exact failure that caused #13.
- **Single hash-write authority (§3, adopting o1 #5):** only the spawn success path (`agent.rs`) writes `config_hash`; `realign.rs` stops overwriting it. Kills a latent drift-loop footgun.
- **Storm hardening (§4):** SIGKILL-reap the old process on drift-realign (commit B) **and** stagger the respawn burst (commit C, adopting o1 #3). Core normalization is commit A.
- **`is_sandbox` stays EXCLUDED (§5.2):** held against o1, using o1's *own* declared-config-only principle from #1 — sandbox posture is runtime-derived, belongs in a separate security check, not the config fingerprint.

---

## 1. The normalization boundary — precise, per-item (FINAL)

Two independent asymmetry leaks, both closed by the boundary below:

- **Leak A (injected derived vars, spawn-only):** runtime vars injected server-side after the client payload (`agent.rs:445-457` etc.), folded into the stored hash (`agent.rs:308` hashes `spawn_env_vars`), absent from the realign hash.
- **Leak B (`config.env`, spawn-only):** the project-level `[env]` is present on the stored side but **dropped from the realign payload** (`up.rs:49` sends raw `agent.env`). Live repo's `[env]` (`RUSTUP_HOME`, `CARGO_HOME`) hits this even with empty agent blocks.

| # | Env source | Origin | Ruling | Reason |
|---|---|---|---|---|
| 1 | `config.env` — project `[env]` | **User TOML.** `ProjectConfig.env`, `#[serde(default)]`, `config.rs:24`. No runtime mutation in `src/` (only `.env.insert` is test fixture `recovery.rs:1316`). | **KEEP** | Real user config; editing `[env]` *must* realign. Merged **server-side** (§3), not by clients. |
| 2 | `agent.env` — `[agents.X.env]` | **User TOML.** `AgentConfig.env`, `#[serde(default)]`, `config.rs:101`. | **KEEP** | Real per-agent user config. |
| 3 | `CCB_SOCKET` | Derived: `state_dir/ahd.sock` (`agent.rs:451`) | **EXCLUDE** | Daemon socket identity, moves with state-dir; not a per-agent setting. |
| 4 | `AH_ROLE` = `worker` | Derived constant (`process_identity.rs:14`) | **EXCLUDE** | Identical for every worker → zero drift signal. |
| 5 | `AH_SESSION_ID` | Derived: session id (`process_identity.rs:15`) | **EXCLUDE** | Runtime identity; **changes on reconnect/new session** — the false-drift trigger (§2). |
| 6 | `AH_AGENT_ID` | Derived: agent id (`process_identity.rs:16`) | **EXCLUDE** | **Per-process runtime identity, not declared config** — already asserted by the existing unit test `process_identity_vars_are_not_daemon_identity_vars` (`process_identity.rs:31-44`). (o1 #5.1 — conceded rationale.) |
| 7 | `HOME` | Derived sandbox `home_root` = f(state_dir, session_id, agent_id) (`home_layout.rs:1716`) | **EXCLUDE** | Path derived from session id → changes on reconnect. |
| 8 | `CLAUDE_CONFIG_DIR` / `CODEX_HOME` | Derived: `home_root.join(".claude"/".codex")` (`home_layout.rs:246,273`) | **EXCLUDE** | Same derivation chain as `HOME`. |
| 9 | `IS_SANDBOX` | Derived: f(provider, `--dangerously-skip-permissions`, **`is_ccb_sandbox_home(home_root)`**) (`agent.rs:459-468`) | **EXCLUDE** | **Runtime-derived, not declared config** (depends on a host path check). Sandbox posture is a security boundary but belongs in a *separate* posture-drift check, not the config fingerprint. (o1 #2 — held, see §5.2.) |

**Boundary in one sentence:** the fingerprint's env = **exactly `merge(config.env, agent.env)`** (agent overrides project) — everything the user declared, nothing the runtime injected. `KEEP = {#1,#2}`, `EXCLUDE = {#3..#9}`.

---

## 2. Why bare-config beats recompute-effective-env (with a concrete failure)

Rejected alternative — make realign *recompute the injected effective env* to match the stored hash — reintroduces the storm:

**Failure scenario (identity churn → phantom全员 drift):** Agent `a1` spawns in session `S1`; effective env carries `AH_SESSION_ID=S1`, `HOME=<state>/S1/a1`, … The daemon restarts / session is re-minted `S2` on reconnect. On next `ah up`, an effective-env realign recomputes `AH_SESSION_ID=S2`, `HOME=<state>/S2/a1`, … → different hash → **DRIFT → destructive respawn of an agent whose `ah.toml` is byte-identical**. Across N agents = the same storm, re-triggered by identity churn. Bare-config is invariant under session/HOME/socket churn → immune. (Secondary: recompute would force realign to replay home-materialization *side effects* just to compute a comparison hash — fragile and non-deterministic.)

---

## 3. Implementation landing (signature-level; code deferred to GREEN)

**Design intent:** one server-side function builds the hashed env; the fingerprint is taken *before* any injection; exactly one site writes the hash. Every rule below removes a way for the two sides to diverge — the disease #13 is made of.

### 3a. Server-side merge — single source of truth (adopts o1 #4)

Clients stop pre-merging. Both RPCs carry raw declared inputs; the **server** merges:
- `agent.spawn` / `session.realign` / `agent.realign` params gain a top-level **`config_env`** (the project `[env]`); each agent still carries its raw **`env`** (`agent.env`).
- A single server helper, e.g. `fn bare_config_env(config_env: &Map, agent_env: &Map) -> Map` = `config_env` then `.extend(agent_env)` (agent overrides), is called at **both** the spawn entry (`handle_agent_spawn_with_db_action`) and the realign entry (`handle_session_realign`, per-agent loop) so the hashed env is constructed by **one** code path.
- `start.rs:155-157` (which currently client-merges) and `up.rs:45-56` (which currently drops `config.env`) both change to send raw `env` + top-level `config_env`.
- `spawn_realign_agent` (`realign.rs:393-407`) must forward `config_env` into the inner spawn so the respawn's stored hash is built from the same bare env.

Rationale over the DRAFT's client-side merge: a future CLI entry or a third party hitting `session.realign` directly would silently skip a client-side merge and reintroduce phantom drift. Centralizing on the server makes the merge unskippable. This is not gold-plating — the root cause *is* "two sites built the env differently," so removing every client's ability to build it is directly on-point.

### 3b. Fingerprint the bare env, before injection (core)

`compute_config_hash` and the `ConfigRole::Agent { env }` shape are **unchanged**. On the spawn side, feed it the **bare merged env**, captured before `build_agent_spawn_env_vars_for_hook_push` and home/sandbox injection run:

```rust
// spawn entry, before injection:
let bare_env = bare_config_env(&config_env, &agent_env);   // = merge(config.env, agent.env)
// ... build spawn_env_vars = inject(bare_env.clone())  [CCB_SOCKET, AH_*, HOME, ...]  for the ACTUAL process
// fingerprint input (agent.rs:308-318) uses the BARE env, not the injected one:
role: ConfigRole::Agent { provider, env: &bare_env },      // was: &spawn_env_vars
```

`spawn_env_vars` (injected) remains the real process env and the persisted `AgentSpawnSpec.env`. Allowlist-by-construction: injected vars (#3..#9) enter strictly *after* the capture point, so no future injection can leak into the hash — the fix is structural, not a maintained exclude-list. The realign side already hashes the payload env with zero injection; once it hashes `bare_config_env(config_env, agent.env)`, both sides agree.

### 3c. Single hash-write authority (adopts o1 #5)

Today `config_hash` is written in **two** places: `agent.rs:353-354` (spawn success) and `realign.rs:413-419` (the `!uses_atomic_replacement` overwrite). The realign overwrite currently *masks* the asymmetry by clobbering the injected-env hash with the raw-env `expected_hash` — which is exactly why a *second* `ah up` is stable today. That dual-write is a latent drift-loop: if the two computations ever diverge (different merge/filter details across versions), the DB hash and the next realign's recomputed hash disagree forever → permanent respawn loop.

**Ruling:** remove the `update_agent_config_hash` call at `realign.rs:413-419`. After a drift verdict `realign.rs` only spawns; the spawn success path in `agent.rs` is the **sole** writer, persisting `H(bare_env)`. Because §3a guarantees both sides derive `bare_env` from the same helper, the stored hash and the next realign's `expected_hash` are byte-equal for unchanged config — no clobber needed, no divergence possible. (The atomic-replacement path already defers to `agent.rs`; this makes the non-atomic path consistent with it.)

---

## 4. Storm hardening (FINAL)

The core fix (§1–§3) removes the *trigger* (first-`ah up` / identity-churn全员 drift → now zero drift). Two amplifiers remain and are fixed as **separate commits in the same PR** so multi-agent realign is never re-exposed to the storm, while each stays independently gate-able:

### 4a. SIGKILL-reap on drift-realign — commit B

Realign's destructive path calls DB-only `mark_agent_killed` (`agents_lifecycle.rs:31` — pure SQL, **no `libc::kill`**) then respawns the same `agent_id`, **never terminating the old OS process/pane** (contrast the real kill at `agent.rs:788`). Every forced realign orphans a live process still holding resources — the amplifier that floods the single-threaded tmux server. **Fix:** on drift-realign, physically terminate the old process (SIGKILL on the DB `pid`) and reap its scope before/at respawn, mirroring `handle_agent_kill`.

**Test finding (this phase, physical-evidence):** the SIGKILL contract is **not** unit-testable in the lightweight normalization harness, and I did *not* ship a test that passes for the wrong reason. Evidence: `realign.rs` and `delete_agent` (`agents.rs:172`) do **no** tmux teardown, yet in the no-systemd/no-sandbox harness the old process dies anyway — because `spawn_window_sync` (`tmux/session.rs:131`) respawns the agent window **in place** (`respawn-window`/`-k`, `:262`), and the agent process is a *direct tmux child*, so tmux reaps it regardless of any daemon-side SIGKILL. The orphan the fix targets only manifests when the process **outlives its pane** — i.e. wrapped in a systemd scope (`env_state.systemd_run_available`). Its RED test therefore belongs in a **systemd-scope e2e**, authored alongside GREEN commit B (a light-harness assertion would gate nothing). **This is flagged for master** as a PR-structure decision: author commit B's e2e under systemd, or split commit B to an immediate follow-up with that e2e. It is not silently dropped — the fix is still required; only its verification lane changes.

### 4b. Respawn stagger — commit C (adopts o1 #3, with a mechanism correction)

o1 is right that SIGKILL alone is not a *proven-complete* storm defense: a `config.env` edit legitimately drifts **all** agents at once, so a mass respawn burst remains possible. **But o1's stated mechanism is inaccurate:** the realign loop is already **sequential** — `realign.rs:210` `for agent in &agents { … spawn_realign_agent(...).await }` awaits each respawn inline, so realign does *not* fan out concurrent spawn requests. The residual burst is (i) back-to-back pane creation with no spacing, and (ii) init-probe poll accumulation.

**Ruling:** adopt a bounded **inter-respawn stagger** in the realign loop — insert a minimum delay between consecutive destructive respawns so the pane-creation burst is spread over time (a global semaphore on an already-sequential loop would be a no-op; a time stagger is the remedy that actually spreads the burst). Contract test (observable, wall-clock): realigning K simultaneously-drifting agents takes ≥ `(K-1) × MIN_RESPAWN_STAGGER_MS`. **Contract value: `MIN_RESPAWN_STAGGER_MS = 500`** (matches o1's suggested 500 ms). Test uses K=5 ⇒ realign ≥ 2000 ms.

**Explicitly deferred (NOT this PR — flagged so it is not silently dropped):** the init-probe 200 ms fast-poll accumulation (`init_probe_task.rs:26`) is a *separate* amplifier; stagger does not fix it (probes outlive spawns). It needs a probe-cadence backoff / global probe budget — its own follow-up issue. Do not fold into #13.

---

## 5. Adjudication of o1's challenge (FINAL rulings)

Discipline: concede where o1 has evidence, hold where o1 is wrong — both with evidence. Not a rubber-stamp; not a defense of ego.

### 5.1 `AH_AGENT_ID` exclude — **CONCEDE rationale (exclusion unchanged)**
o1 accepts the exclusion but supplies a stronger reason than my DRAFT's "redundancy": the existing unit test `process_identity_vars_are_not_daemon_identity_vars` (`process_identity.rs:31-44`) already classifies `AH_AGENT_ID` as **per-process identity, not config**. Adopted verbatim into the §1 rationale. No design change, no new test (existing test covers the classification).

### 5.2 `IS_SANDBOX` — **HOLD exclusion (disagree with o1's mechanism), with strengthened reasoning**
o1 wants sandbox posture as an explicit `is_sandbox: bool` field in `ConfigFingerprintInput`, arguing a silent sandbox degradation would go undetected. **The security concern is real; the proposed mechanism is wrong, by o1's own principle.**

In #5.1 o1 correctly insists the config fingerprint must be **decided only by declared config**, excluding runtime identity. `IS_SANDBOX` is `should_inject_is_sandbox(provider, command, is_ccb_sandbox_home(home_root))` (`agent.rs:459-468`) — its third factor `is_ccb_sandbox_home(home_root)` (`home_layout.rs:1677`) is a **runtime host-path check**, i.e. runtime-derived state, exactly the category #5.1 says must stay out. o1 even concedes "沙箱状态并非单纯由配置静态决定." Folding a runtime-derived boolean into the *config* fingerprint conflates "did the user's declared config change" with "did the host posture drift" — the very pollution o1 rejects for `AH_AGENT_ID`.

The declared sandbox **intent** is already in the hash (via `provider` + `command`/`settings`). The runtime sandbox **reality** deserves monitoring, but through a **dedicated sandbox-posture health check**, not `config_hash`. **Ruling: EXCLUDE stands.** My DRAFT's reason ("determined by provider + declared config") was itself imprecise and is corrected here (the correct reason is *runtime-derived → wrong home*). The security concern is logged as a **separate follow-up** (sandbox-posture-drift detector), not dropped. No `is_sandbox` field; this also keeps the realign path free of manifest/home re-derivation. No test this PR (the follow-up owns it).

### 5.3 `config.env` mass-drift acceptability — **CONCEDE (adopt rate-limiting)**
o1 is right that "SIGKILL alone is enough" understated the residual: a `config.env` edit legitimately drifts all agents, and a mass respawn burst can still stress tmux. Adopted as the **stagger** in §4b — with the evidence-based correction that the realign loop is already sequential (so the remedy is temporal spacing, not de-fan-out), and with the init-probe accumulation explicitly split out as a deferred follow-up. Contract test committed.

### 5.4 Client-side merge is a trust-boundary violation — **CONCEDE (adopt server-side merge)**
o1 is right: my DRAFT's client-side merge lets any future/third-party caller skip it and reintroduce phantom drift. Adopted as §3a — the server is the sole env-construction authority; clients send raw `env` + `config_env`. This is directly root-cause-aligned (the bug *is* divergent env construction). Anchored by the `config_env` RED tests.

### 5.5 Split hash-write authority / drift-loop risk — **CONCEDE (adopt single writer)**
o1 is right: the `realign.rs:413-419` overwrite vs `agent.rs:353-354` write is a latent permanent-drift-loop if the two ever diverge. Adopted as §3c — remove the realign overwrite; `agent.rs` is sole writer. No dedicated RED test (today the paths are consistent-by-clobber, so there is no *current* observable failure to turn red — a synthetic internal-authority assertion would violate the "anchor observable behavior, not internals" rule). The refactor is guarded by the **idempotency regression test** (§6, test 4): realign-after-respawn with unchanged config must stay NO_CHANGE, which fails if a future divergence appears.

---

## 6. RED test plan (committed this phase)

New frozen file **`tests/issue13_respawn_storm.rs`** (authored by g1; g1-m1 must not modify it). All tests drive real RPCs through the router against a real tmux server + sqlite (harness modeled on `pr4e_up_fingerprint.rs` / `ah_full_e2e_drift.rs`), provider `bash`, `unsafe_no_sandbox` so Leak A reproduces via `CCB_SOCKET` + `AH_*`. Each spawns through the **real `agent.spawn`** so the stored hash is the *injected* one — the gap the existing tests miss by seeding the raw hash directly. **RED run confirmed** (`--test-threads=1`): the three contract tests fail for the intended reasons; the three guards pass.

| # | Test | Contract | Now | After |
|---|---|---|---|---|
| 1 | `spawn_then_realign_unchanged_config_is_no_change` | Core asymmetry (Leak A): spawn real agent (empty env) → realign same raw config → **NO_CHANGE** | **RED** (agent forced `DRIFT config changed REALIGNED` while master is NO_CHANGE) | GREEN |
| 2 | `unchanged_config_env_does_not_drift` | Leak B + server-merge (#4): spawn with `config_env={K:v}`,`env={}` → realign same → **NO_CHANGE** | **RED** (REALIGNED) | GREEN |
| 3a | `agent_env_edit_still_drifts` | Over-correction guard: real `agent.env` change → **REALIGNED** | GREEN (guard) | GREEN |
| 3b | `config_env_edit_still_drifts` | Guard against over-excluding `config_env`: change `config_env` → **REALIGNED** (proves server reads+hashes it) | GREEN (guard) | GREEN |
| 4 | `realign_is_idempotent_no_drift_loop` | #5 guard: spawn → realign(edit) → respawn → realign(same edit) → **NO_CHANGE** (no loop) | GREEN (guard) | GREEN |
| 6 | `simultaneous_drift_respawns_are_staggered` | Stagger (commit C): K=5 drifting agents → `session.realign` wall ≥ 2000 ms | **RED** (measured ~0.6 s ≪ 2 s) | GREEN |

SIGKILL contract (was "test 5") is **omitted here on purpose** — not unit-observable in this harness (§4a); belongs in a systemd-scope e2e with commit B. Omitted rather than shipped-passing-for-the-wrong-reason.

RED discipline: tests anchor **observable contract behavior** (realign verdict, NO_CHANGE vs REALIGNED, wall-clock spacing) — never internal hash strings or write-authority. Guards (3a/3b/4) are GREEN-now by design: they stop the fix over-correcting ("never drift") or regressing idempotency; they are not padding. The RED margin on test 6 is ~3.4× (measured 0.6 s vs 2 s floor), so it is not timing-flaky.

---

## 7. Next step (GREEN handoff to g1-m1)

g1-m1 implements against the frozen tests, in three commits within this PR:
- **A (core):** §3a server-side `config_env` merge + §3b bare-env fingerprint + §3c single hash-writer → turns tests 1,2 green; keeps 3a,3b,4 green.
- **B (SIGKILL reap):** §4a → verified by a **systemd-scope e2e** co-authored with GREEN (not in the frozen file; see §4a finding). Master decides PR-structure (same PR under systemd, or immediate follow-up).
- **C (stagger):** §4b → turns test 6 green.

g1-m1 may not edit `tests/issue13_respawn_storm.rs`; the test body is g1's contract, master + CI are its gate.

---

### File:line index (re-verified this phase)
- Hash fn: `src/provider/fingerprint.rs:45-79` (unchanged by this fix)
- Spawn hash over injected env: `src/rpc/handlers/agent.rs:308-318`; inject sites `:445-457`, home `:172-178`
- Sole spawn hash write: `src/rpc/handlers/agent.rs:353-354`
- Realign hash over raw env: `src/rpc/handlers/realign.rs:211-221`; compare `:257`; destructive DB-only kill path `:287-294`; **overwrite to remove** `:413-419`
- Realign respawn (sequential loop): `src/rpc/handlers/realign.rs:210`; spawn helper `:375-421`
- Identity inject: `src/process_identity.rs:9-17`; classification test `:31-44`
- Sandbox decision: `src/rpc/handlers/agent.rs:459-468`; host check `src/provider/home_layout.rs:1677`
- Spawn/realign client payloads: `src/cli/start.rs:155-157`, `src/cli/up.rs:45-56`
- Config purity: `src/cli/config.rs:24` (`ProjectConfig.env`), `:101` (`AgentConfig.env`)
- DB-only kill vs real SIGKILL: `src/db/agents_lifecycle.rs:31` (`mark_agent_killed_sync`, no kill) vs `src/rpc/handlers/agent.rs:788` (`libc::kill`)
- Init-probe cadence (deferred amplifier): `src/provider/init_probe_task.rs:26-28`
