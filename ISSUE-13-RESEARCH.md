# Issue #13 — Respawn Storm: Root-Cause Research (Phase 1, read-only)

**Author:** g1 (泳道1 gatekeeper)
**Branch:** `fix/issue-13-respawn-storm` (worktree `ccbd-rust-wt-issue13`, off `origin/main @ 95829c4`)
**Scope of this phase:** read + investigate + write this note. No production code changed, no commit. RED tests are the next phase.

---

## TL;DR verdict

- **Operator's粗定位 is REFUTED as stated.** `compute_config_hash` does **not** eat sibling agent blocks, the global agent list, or the whole `ah.toml`. Its input is strictly per-agent (verified field-by-field below). Appending a sibling block does **not** change an existing agent's expected hash.
- **The real root cause is a spawn-vs-realign hash-input asymmetry.** The `config_hash` **stored at `ah start` / `agent.spawn`** is computed over the agent's **fully-injected effective env** (project `[env]` merge + `CCB_SOCKET` + `AH_ROLE/AH_SESSION_ID/AH_AGENT_ID` + sandbox `HOME`/`CLAUDE_CONFIG_DIR`/…). The `expected_hash` **recomputed at `ah up` / `session.realign`** is computed over the **raw `agent.env` from `ah.toml` only**. These two env sets can never be equal, so **every agent is judged `DRIFT config changed REALIGNED` on the first `ah up`** — regardless of whether any block was added.
- **"Adding 4 blocks" is a coincidental trigger, not the cause.** The append is simply what motivated the operator to run `ah up` for the first time since `ah start`. Any first `ah up` reproduces the全员 drift. The 4 new blocks add 4 more spawns on top of the 6 forced respawns, feeding the storm.
- **Storm death path is not "just concurrency count."** There is **no global concurrency cap / semaphore / rate-limit** on spawns. The only serialization is a per-`session_id` async mutex around tmux *window creation*. Worse, realign **does not SIGKILL the old process** before respawning the same `agent_id`, so each forced realign **orphans a live process + tmux pane** (still `pipe-pane`-ing), and each fresh agent starts a **200 ms fast-poll `capture-pane` init-probe loop**. N simultaneous respawns ⇒ N orphaned panes + N new panes + N polling loops + global watchers, all hammering the single-threaded tmux server with no throttle.

---

## 1. `compute_config_hash` actual input — field by field, with source

Definition: `src/provider/fingerprint.rs:18-25` (`ConfigFingerprintInput`), hashed at `:45-79` (`compute_config_hash`).

| Field | Type | Source at the AGENT call site (`realign.rs:211-221`) | Sibling/global? |
|---|---|---|---|
| `role` = `Agent { provider, env }` | `&str`, `&HashMap<String,String>` | `agent.provider`, **`agent.env`** — the single `RealignAgentParams` for *this* agent | **per-agent only** |
| `hooks` | `&HashMap<String,Vec<HookGroup>>` | `agent.hooks` | per-agent only |
| `plugins` | `&[String]` (sorted at `:57-58`) | `agent.plugins` | per-agent only |
| `skills` | `&[String]` (sorted at `:59-60`) | `agent.skills` | per-agent only |
| `settings` | `&Map<String,Value>` | `agent.settings` | per-agent only |
| `bundle` | `Option<&BundleDigest>` | `agent.bundle_digest` | per-agent only |

The hash body (`fingerprint.rs:45-79`) serializes exactly these six fields into a deterministic JSON object and SHA-256s it. **Nothing in the function reads any sibling agent, an agent-count, or the parsed `ah.toml` AST.** For `ConfigRole::Agent`, the `env` map is embedded verbatim into the role JSON (`fingerprint.rs:51-55`), so **the hash is fully determined by `env`'s contents** — this is the lever the bug pulls.

➡️ **Operator hypothesis (hash吃了 sibling/全局) is refuted at the function level.**

---

## 2. Call chain — what env actually reaches the hash on each path

### 2a. STORED hash — `ah start` → `agent.spawn` (server: `src/rpc/handlers/agent.rs`)

Client (`src/cli/start.rs:155-182`) builds the spawn payload with a **merged** env:

```rust
// start.rs:155-157
let mut merged_env = config.env.clone();     // project-level [env]  (GLOBAL)
merged_env.extend(agent.env.clone());        // agent's own [agents.X.env] overlaid
...
"extra_env_vars": merged_env,                // start.rs:174
```

Server (`agent.rs`) then **injects more env before hashing**:

- `agent.rs:142-147` → `build_agent_spawn_env_vars_for_hook_push(...)`
  - `agent.rs:451-454`: inserts **`CCB_SOCKET`** (unconditional)
  - `agent.rs:455` → `process_identity::inject_worker_identity` (`src/process_identity.rs:9-17`): inserts **`AH_ROLE=worker`, `AH_SESSION_ID`, `AH_AGENT_ID`** (unconditional)
- `agent.rs:172` → `spawn_env_vars.extend(home_overrides.extra_env)` for providers requiring home materialization
  - `src/provider/home_layout.rs` `home_env(...)` always adds **`HOME`** (`:home_env` helper), plus **`CLAUDE_CONFIG_DIR`** (claude, `:246`) / **`CODEX_HOME`** (codex, `:273`)
- `agent.rs:173-178`: may add **`IS_SANDBOX=1`** (claude `--dangerously-skip-permissions` in sandbox home)

Then the stored hash is computed over that fully-injected env:

```rust
// agent.rs:308-318
let config_hash = compute_config_hash(&ConfigFingerprintInput {
    role: ConfigRole::Agent { provider, env: &spawn_env_vars },   // <-- INJECTED env
    hooks: &extensions.hooks, plugins: &extensions.plugins,
    skills: &extensions.skills, settings: &extensions.settings,
    bundle: extensions.bundle_digest.as_ref(),
})?;
// agent.rs:353-354  (InsertDefault branch) persists it:
update_agent_config_hash(ctx.db.clone(), agent_id, config_hash.clone()).await?;
```

**Stored env set** ⊇ `{ project [env]… , agent.env… , CCB_SOCKET, AH_ROLE, AH_SESSION_ID, AH_AGENT_ID, HOME, CLAUDE_CONFIG_DIR|CODEX_HOME, [IS_SANDBOX] }`.

### 2b. EXPECTED hash — `ah up` → `session.realign` (comparison side)

Client (`src/cli/up.rs:45-56`) sends **only the raw agent block env**:

```rust
// up.rs:49
"env": agent.env,     // raw [agents.X.env] ONLY — no config.env merge, no injection
```

(The `ah start` recovery path `build_realign_payload` in `start.rs:274-285` is identical: `"env": agent.env`.)

Server compares against a hash recomputed straight from that raw env, **with none of the §2a injections**:

```rust
// realign.rs:211-221
let expected_hash = compute_config_hash(&ConfigFingerprintInput {
    role: ConfigRole::Agent { provider: &agent.provider, env: &agent.env },  // <-- RAW env
    hooks: &agent.hooks, plugins: &agent.plugins, skills: &agent.skills,
    settings: &agent.settings, bundle: agent.bundle_digest.as_ref(),
})?;
...
// realign.rs:257
if running.config_hash.as_deref() == Some(expected_hash.as_str()) { /* NO_CHANGE */ }
// else -> mark_agent_killed + delete_agent + spawn_realign_agent  (:287-294)  -> DRIFT REALIGNED
```

**Expected env set** = `{ agent.env… }` only.

➡️ **Stored (§2a) ⊋ Expected (§2b) by at least `{CCB_SOCKET, AH_ROLE, AH_SESSION_ID, AH_AGENT_ID, HOME, …}`.** Different env map ⇒ different role JSON ⇒ different SHA-256 ⇒ `running.config_hash != expected_hash` for **every** agent ⇒ forced kill+respawn.

> Note: after a forced realign, `spawn_realign_agent` overwrites the stored hash with `expected_hash` (raw-env hash) at `realign.rs:413-419`. So the storm is a **one-shot on the first `ah up`**; a *second* `ah up` would then be stable — which is exactly why the operator saw it fire once, right after the append.

### 2c. Why the operator's live stack is guaranteed to hit it

`ah.toml` in the live repo (`/home/sevenx/coding/ccbd-rust/ah.toml`) has a **non-empty top-level `[env]`**:

```toml
[env]
RUSTUP_HOME = "/home/sevenx/.rustup"
CARGO_HOME  = "/home/sevenx/.cargo"
```

Those two vars alone are merged into the stored hash (§2a) and dropped from the expected hash (§2b). But even a config with an empty `[env]` and an empty `[agents.X]` (just `provider=...`) drifts, because `CCB_SOCKET` + `AH_*` + `HOME` are always injected at spawn and never at realign.

---

## 3. Minimal reproduction (throwaway — deleted after capture)

A throwaway integration test (`tests/issue13_tmp_repro.rs`, **not committed, deleted after this run**) called the real `ah::provider::fingerprint::compute_config_hash` twice for the *same* agent `a1` (empty `[agents.a1]` block, i.e. `agent.env = {}`):

- **spawn side:** env = `[env]`(RUSTUP_HOME,CARGO_HOME) + `CCB_SOCKET` + `AH_ROLE/AH_SESSION_ID/AH_AGENT_ID`
- **realign side:** env = `{}` (raw `agent.env`)

Actual output:

```
running 1 test
SPAWN_HASH   (stored at ah start) = 3af280aa406e4ed0e69f8942d01a9d29306a05328dbbe4c8bbe4aaca3c79bae7
REALIGN_HASH (recomputed at ah up)= 66ee714c4a8b3e60774cd42f5868da8b9c24bbee3928a31058289315ec6693b4
EQUAL? false
REALIGN_HASH a1 (sibling absent) = 66ee714c4a8b3e60774cd42f5868da8b9c24bbee3928a31058289315ec6693b4
REALIGN_HASH a1 (sibling present)= 66ee714c4a8b3e60774cd42f5868da8b9c24bbee3928a31058289315ec6693b4
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

- `SPAWN_HASH != REALIGN_HASH` for the **same** agent `a1` → drift on first `ah up`, confirmed with real SHA-256 values.
- a1's realign hash is **byte-identical** with a sibling absent vs present → per-agent expected hash is sibling-independent (operator hypothesis refuted).

It also asserts a1's realign hash is byte-identical whether or not a sibling block is present — corroborating that the per-agent expected hash is sibling-independent (refuting the operator hypothesis).

Command (run with `RUSTUP_HOME`/`CARGO_HOME` set so rustup resolves a default toolchain; `CARGO_BUILD_JOBS=1`, `--test-threads=1`):
`cargo test --test issue13_tmp_repro -- --nocapture --test-threads=1`

---

## 4. Root-cause判定

**Root cause = env-input asymmetry between the spawn path and the realign comparison path**, not a hash function that folds in siblings/globals.

- `agent.spawn` stores `config_hash = H(effective_injected_env, …)`.
- `session.realign` compares against `expected = H(raw_ah_toml_env, …)`.
- `effective_injected_env ⊋ raw_ah_toml_env` **always** (CCB_SOCKET/AH_*/HOME are never in the raw block), so the equality check at `realign.rs:257` is false for every agent, triggering the `mark_agent_killed → delete_agent → spawn_realign_agent` destructive path (`realign.rs:287-294`).

The fix must make the two sides hash **the same effective env** — either the realign side must reconstruct the injected env before hashing (project `[env]` merge + `CCB_SOCKET` + identity + home vars), or the stored hash must be computed over the raw declared config only (and the injected/derived env excluded from the fingerprint on both sides). That decision belongs to the design/RED-test phase; this note only localizes the defect.

**Operator粗定位 status:**
- ❌ "hash 把 sibling blocks / 全局 agent 列表 / 整份 ah.toml 算进去了" — refuted (§1, §3).
- ✅ partial truth: a *global* input (`config.env`) does leak into the **stored** (spawn) side — but via the env-merge in `start.rs:156`, not via the hash function, and it is only one of several spawn-only env sources. The mechanism is asymmetry, not sibling-sensitivity.

---

## 5. Storm death path — preliminary evidence

**Question:** is the tmux-server crash purely a function of concurrent respawn count, or is there another resource-exhaustion path? Is there any throttle/serialization?

Evidence found:

1. **The realign respawn loop is sequential.** `realign.rs:210-310` `for agent in &agents { … spawn_realign_agent(...).await … }` awaits each spawn inline. So realign itself does not fan out — it serializes its own 10 spawns. (The operator's "并发" is partly the *aftermath*, below, not the realign loop.)

2. **Only serialization present: a per-`session_id` async mutex around tmux window creation.** `agent.rs:247-249` takes `session_window_lock(session_id)` (defined `src/rpc/handlers/sessions.rs:56-66`) around `ensure_session` + `spawn_window`. This serializes *window creation* for agents in the same ah-session, but does **not** cap total agents or throttle anything after the window exists.

3. **No global concurrency cap / semaphore / rate-limit / inter-spawn backoff anywhere in the spawn/respawn path.** `grep -rn 'Semaphore|max_concurrent|throttle|permit|rate_limit|jitter|stagger' src/` returns only unrelated dispatch-guard code — nothing gating agent spawns.

4. **Realign leaks live processes (extra resource path, beyond raw count).** On DRIFT, realign calls the **DB-only** `mark_agent_killed` (`src/db/agents_lifecycle.rs` — pure `agents`-table transaction, **no `libc::kill`**) then `delete_agent`, then spawns a new pane for the same `agent_id`. Contrast `handle_agent_kill` (`agent.rs:788`) which *does* `libc::kill(pid, SIGKILL)`. So realign **never terminates the old OS process / tmux pane** — each forced realign orphans a still-running pane that keeps `pipe-pane`-ing to a now-removed FIFO. 6 forced realigns ⇒ 6 orphaned panes + 6 new panes; +4 new agents = ~16 panes churning.

5. **Every fresh agent starts a 200 ms fast-poll `capture-pane` init-probe.** `src/provider/init_probe_task.rs:26-28` `POLL_FAST = 200ms` for the first 5s (`POLL_SWITCH`), then `POLL_SLOW = 500ms`, up to `readiness_timeout_s` (120s). N agents starting together ⇒ N concurrent 200 ms tmux-poll loops, on top of the periodic `pane_diff_watcher_loop` / `health_check_watcher_loop` (`src/orchestrator/mod.rs:61-71`) that scan all panes.

**Preliminary conclusion:** the crash is a **compounding tmux-command flood**, not a single syscall. The forced全员 respawn (root cause §4) creates, in one burst: orphaned-but-live panes (no SIGKILL) + new panes + one 200 ms poll loop per new agent + global watchers, with **zero throttle**. Raw respawn count is the trigger; the *amplifiers* are (a) no process reaping before respawn and (b) unbounded per-agent tmux polling. Confirming the exact tmux failure mode (e.g. `new-window`/`pipe-pane` fd/socket exhaustion) needs a runtime repro and is out of scope for this read-only phase.

---

## Appendix — file:line index

- Hash fn + input struct: `src/provider/fingerprint.rs:18-25`, `:45-79`
- Agent hash call (expected, realign): `src/rpc/handlers/realign.rs:211-221`, compare `:257`, destructive path `:287-294`, stored-hash overwrite `:413-419`
- Stored hash (spawn): `src/rpc/handlers/agent.rs:308-318`, persist `:353-354`
- Spawn env injection: `agent.rs:142-147`, `:172-178`, `:445-457`; identity `src/process_identity.rs:9-17`; home env `src/provider/home_layout.rs:home_env`, `:246`, `:273`
- Client spawn payload (merged env): `src/cli/start.rs:155-182`
- Client realign payload (raw env): `src/cli/up.rs:45-56`; recovery variant `src/cli/start.rs:274-285`
- Live config `[env]`: `/home/sevenx/coding/ccbd-rust/ah.toml`
- Storm: realign loop `realign.rs:210-310`; window lock `sessions.rs:56-66` + `agent.rs:247`; DB-only kill `src/db/agents_lifecycle.rs` `mark_agent_killed*`; real SIGKILL for contrast `agent.rs:788`; init-probe cadence `src/provider/init_probe_task.rs:26-28`; watchers `src/orchestrator/mod.rs:61-71`
