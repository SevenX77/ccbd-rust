# PR-5: Plugin Bundle Recovery / Realign / CLI / Migration Spec

Status: implementation-ready scope lock. This spec is design/review only and writes no product code.

Base input: `origin/docs/plugin-bundle-design:.kiro/specs/ah-plugin-bundle/design.md`, especially §4.5 recovery/realign propagation, §5 zero-regression and migration, and §6 PR-5. PR-4 antigravity bundle adaptation is in flight and orthogonal; PR-5 must not wait for antigravity internals beyond using the provider behavior available on its base branch.

## Evidence Anchors

The following anchors were verified before writing this spec:

- CLI top-level commands live in `src/bin/ah.rs`; `Cmd` currently has `Up` and `Config`, but no `Bundle` subcommand.
- `ah up` builds a `session.realign` payload from `ProjectConfig` in `src/cli/up.rs`, passing bundle names but not precomputed digests.
- `MasterConfig` and `AgentConfig` already carry `bundle: Vec<String>` through `src/cli/config.rs`.
- Bundle resolution and digest recomputation live in `src/provider/bundles.rs` through `resolve_bundles_for_provider` and `digest_for_bundles`.
- Agent crash/recovery persistence already stores `AgentSpawnSpec { bundle, bundle_digest, ... }` in `src/db/recovery.rs`.
- Normal agent spawn hashes effective extensions with `bundle: extensions.bundle_digest.as_ref()` and persists a spawn snapshot in `src/rpc/handlers/agent.rs`.
- Master spawn resolves bundles and hashes the resolved master extensions in `src/rpc/handlers/sessions.rs`.
- `session.realign` / `agent.realign` live in `src/rpc/handlers/realign.rs`; `populate_bundle_digests` recomputes missing digests from current disk, `spawn_realign_agent` passes bundle fields back into agent spawn, and `drift_reason` can report `"bundle changed"`.
- Daemon crash recovery calls `spawn_realign_agent` from `src/orchestrator/mod.rs`; master revive worker reprovisioning calls the same path from `src/monitor/master_watch.rs`.
- Existing drift/E2E tests are under `tests/ah_full_e2e_drift.rs`, `tests/ah_full_e2e_realign_extra.rs`, `tests/ah_full_e2e_main.rs`, and bundle materialization tests under `tests/pr3_codex_bundle.rs` / `tests/e2e_bundle_materialization_a4.rs`.

## PR-5 Scope

PR-5 closes executable behavior around bundle content after PR-1 through PR-4 have introduced bundle parsing, digest, materialization, MCP, codex, and antigravity support.

In scope:

- Recovery/realign hardening:
  - `ah up` realigns bundle-backed agents when bundle content changes but `ah.toml` does not.
  - Crash recovery respawns bundle-backed agents from stored `AgentSpawnSpec` while rematerializing current bundle content.
  - Master revive reprovisions bundle-backed workers from stored `AgentSpawnSpec` while rematerializing current bundle content.
  - Master bundle drift remains audit-only by default and force-realign only with `ah up --force`, matching existing master drift policy.
- CLI:
  - `ah bundle validate`
  - `ah bundle list`
- Documentation:
  - Bundle user doc.
  - Scattered config to bundle migration guide.
  - Bundle to scattered config rollback guide.
  - Explicit coexistence semantics for scattered `skills` / `hooks` / `plugins` and bundle content.

Out of scope:

- No new bundle manifest schema beyond what PR-1/PR-2 already define.
- No plugin-in-bundle support.
- No new provider materialization path.
- No fingerprint redesign.
- No antigravity PR-4 implementation details, except tests should include antigravity once PR-4 lands on the base.
- No unrelated `ah up`, lifecycle, DB, tmux, or provider refactors.

## Hard Decisions

These are locked for implementation:

- Bundle content drift is detected by recomputing `BundleDigest` from current disk at realign time. `ah up` should keep sending bundle names only; daemon-side `session.realign` remains responsible for digest calculation.
- Recovery paths use stored bundle references as replay intent, not as cached materialized content. On respawn, provider home layout must be produced from the current `.ah/bundles/<name>/` files.
- Stored `bundle_digest` in `AgentSpawnSpec` is an audit/hash snapshot. It must not prevent recovery from observing current disk content.
- `"bundle changed"` is an acceptable drift reason when the expected agent payload has a non-empty bundle digest and no more specific reason is available.
- `ah bundle validate` and `ah bundle list` are local filesystem/config commands. They do not require ahd, tmux, a running session, or sandbox startup.
- `ah bundle validate` must exercise provider capability validation for referenced bundles, not only TOML syntax.
- Bundle/scattered coexistence remains additive: bundle content and scattered fields are merged; true conflicts hard-error; identical duplicate declarations dedupe; plugins stay scattered only.

## Recovery / Realign Contract

### `ah up` Realign

Required behavior:

1. `ah up` loads the current `ah.toml`, sends bundle names for master and agents, and does not need to compute bundle digests client-side.
2. `session.realign` resolves the active session project root, recomputes missing master/agent bundle digests from `.ah/bundles`, and uses those digests in `compute_config_hash`.
3. If only bundle file content changed, the affected running agent is classified as DRIFT, killed/recreated according to existing state gates, rematerialized from current bundle files, and gets a new `agents.config_hash`.
4. If the changed bundle is referenced by master, `ah up` reports master DRIFT audit-only by default. `ah up --force` follows existing master force realign policy.
5. A no-change run after the bundle realign returns NO_CHANGE and does not add another `drift_realigned` event.

Acceptance must include at least one test where `ah.toml` is byte-for-byte unchanged and a bundle file changes from `v1` to `v2`; the realigned provider home must contain `v2`.

### Agent Crash Recovery

Required behavior:

1. Agent spawn persists `AgentSpawnSpec.bundle` and `AgentSpawnSpec.bundle_digest`.
2. Crash recovery loads the stored spec and calls the same realign spawn path used by normal drift realign.
3. The spawn path re-resolves bundle names and rematerializes current bundle content; it must not copy stale provider-home files forward.
4. The recovered agent row and stored spawn spec end with a config hash that matches the current bundle digest, not the stale digest from before the crash.

Acceptance must mutate bundle content after the agent's spawn snapshot is stored and before recovery runs. The recovered home must contain the mutated content.

### Master Revive / Worker Reprovision

Required behavior:

1. Master revive reuses captured/stored worker spawn specs to reprovision workers.
2. Bundle-backed workers are respawned through `spawn_realign_agent` with bundle names preserved.
3. Worker homes are rematerialized from current bundle files after revive, even when the bundle changed after the original worker spawn.
4. Interrupted job requeue semantics from the existing master revive/recovery path are unchanged.

Acceptance can use the existing master-watch test hooks/fakes. It does not need to run a real provider CLI, but it must verify the worker spawn spec and rematerialized home include current bundle content.

## CLI Contract

### `ah bundle validate`

Shape:

```text
ah bundle validate [--all] [<bundle-name>...]
```

Resolution:

- Uses global `--config` if provided; otherwise finds `ah.toml` from cwd like `ah up`.
- Project root is the directory containing the resolved `ah.toml`.
- With explicit names, validate those `.ah/bundles/<name>` directories.
- With no names and no `--all`, validate all bundles referenced by `[master].bundle` and `[agents.*].bundle`.
- With `--all`, validate all bundle directories under `.ah/bundles`.

Validation depth:

- Bundle name validity.
- `.ah/bundles/<name>/bundle.toml` existence, parse, `name` match, supported `version`.
- Referenced skills, hooks, rules, and MCP manifest entries parse and stay inside the bundle root.
- Digest computation succeeds.
- For referenced bundles, provider capability checks run for each role/provider that references the bundle.
- For explicit names or `--all`, manifest-level validation always runs; provider capability checks run only for references present in `ah.toml`, unless the implementation also offers a documented `--provider` flag in a later PR.

Output:

- Human-readable, stable enough for tests.
- On success, includes `VALID` and the bundle names checked.
- On failure, exits non-zero and includes the bundle name plus the exact validation message.
- It must not print environment secret values. Missing MCP environment variables should be reported by variable name only.

### `ah bundle list`

Shape:

```text
ah bundle list
```

Behavior:

- Uses global `--config` / cwd discovery to determine project root.
- Scans `.ah/bundles/*/bundle.toml`.
- Does not require a running daemon.
- Prints one row per discovered bundle, sorted by name.
- Columns must include at least: `NAME`, `VERSION`, `SKILLS`, `HOOKS`, `RULES`, `MCP`, `REFERENCED_BY`, `STATUS`.
- `REFERENCED_BY` lists `master` and agent ids from `ah.toml` that reference the bundle.
- `STATUS` is `VALID` when manifest-level validation succeeds, otherwise `ERROR`.
- Invalid bundles are listed with `ERROR`; the command exits non-zero if any discovered bundle is invalid.

## Documentation Contract

Add docs in the repo's existing docs/spec style. The exact final path can be chosen by the implementer, but the content must cover:

- Bundle directory layout and `bundle.toml` fields.
- How `bundle = "x"` and `bundle = ["x", "y"]` work for master and agents.
- Provider translation summary for skills, hooks, rules, and MCP.
- Recovery/realign behavior: bundle content changes require `ah up`; bundle-backed crash recovery and master revive rematerialize current bundle content.
- Coexistence semantics:
  - Scattered `skills` and bundle skills merge.
  - Scattered hooks and bundle hooks merge/dedupe.
  - Plugins are not part of bundles and remain scattered.
  - Rules layer as kernel, bundle rules in array order, then project slot rules.
  - True conflicts hard-error.
- Migration from scattered config to bundle:
  - Create `.ah/bundles/<name>/bundle.toml`.
  - Move or copy skills into `skills/`.
  - Move hook scripts into `hooks/` and encode event/matcher mapping in `bundle.toml`.
  - Move common rules snippets into `rules/master.md` or `rules/worker.md`.
  - Add `bundle = "<name>"` to selected roles.
  - Keep scattered fields during staged migration when desired.
  - Run `ah bundle validate` and then `ah up`.
- Rollback from bundle to scattered:
  - Restore scattered `skills`/`hooks`/rules config.
  - Remove `bundle` references.
  - Run `ah up`.
  - Keep `.ah/bundles/<name>` on disk if desired; unreferenced bundles are inert.

## Test Strategy

Add focused tests rather than broad new lifecycle machinery. Prefer reusing existing E2E harnesses and fake tmux/provider homes.

Required new test files:

- `tests/pr5_bundle_realign_recovery.rs`
- `tests/pr5_bundle_cli.rs`

Required test groups:

- Bundle digest drift through `session.realign`.
- `ah up` payload path for bundle names and daemon-side digest recomputation.
- Crash recovery from stored `AgentSpawnSpec`.
- Master revive worker reprovision from stored `AgentSpawnSpec`.
- CLI validate success/failure.
- CLI list sorting/reference/status behavior.
- Documentation grep test or review checklist item confirming migration/coexistence text exists.

Required commands for PR-5 implementation:

```text
CARGO_BUILD_JOBS=1 cargo test --test pr5_bundle_realign_recovery -- --test-threads=1
CARGO_BUILD_JOBS=1 cargo test --test pr5_bundle_cli -- --test-threads=1
CARGO_BUILD_JOBS=1 cargo test --test pr3_codex_bundle -- --test-threads=1
CARGO_BUILD_JOBS=1 cargo test --test e2e_bundle_materialization_a4 -- --test-threads=1
CARGO_BUILD_JOBS=1 cargo test --test ah_full_e2e_drift -- --test-threads=1
CARGO_BUILD_JOBS=1 cargo test --lib -- --test-threads=1
```

If PR-4 antigravity tests land under a different test file, include that test in the final PR-5 verification matrix.

## Review Checklist

- Product code changes are limited to bundle CLI plumbing, bundle validation helpers if needed, tests, and small recovery/realign fixes directly required by acceptance.
- No provider materialization rewrite.
- No DB schema migration unless a failing acceptance test proves an existing persisted field is insufficient.
- `ah up` continues to work without any `bundle` fields.
- Existing empty-bundle fingerprint stability tests still pass.
- `git diff -- src` contains no unrelated formatting churn.
- Docs explicitly state scattered and bundle config may coexist.
