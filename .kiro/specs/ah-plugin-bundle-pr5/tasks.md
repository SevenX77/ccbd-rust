# PR-5 Tasks: Bundle Recovery / Realign Hardening, CLI, Docs

This task list is executable scope for a1/a3. It is based on `.kiro/specs/ah-plugin-bundle-pr5/design.md` and the upstream bundle design §4.5, §5, §6 PR-5.

No implementation should touch unrelated lifecycle/provider code. Start every task with a failing test or a direct CLI/doc assertion, then make the narrowest change.

## Phase Matrix

| Phase | Goal | Primary files | Acceptance command |
| :--- | :--- | :--- | :--- |
| M1 | Realign detects bundle content changes and rematerializes current content. | `tests/pr5_bundle_realign_recovery.rs`, `src/rpc/handlers/realign.rs` only if needed | `CARGO_BUILD_JOBS=1 cargo test --test pr5_bundle_realign_recovery bundle_content_change_realigns_agent -- --test-threads=1` |
| M2 | Crash recovery and master revive reprovision replay bundle references and current content. | `tests/pr5_bundle_realign_recovery.rs`, recovery/master-watch glue only if needed | `CARGO_BUILD_JOBS=1 cargo test --test pr5_bundle_realign_recovery crash_recovery_rematerializes_current_bundle master_revive_reprovisions_current_bundle_worker -- --test-threads=1` |
| M3 | Add `ah bundle validate` / `ah bundle list`. | `src/bin/ah.rs`, new/updated `src/cli/bundle.rs`, `src/cli/mod.rs`, bundle validation helper if needed, `tests/pr5_bundle_cli.rs` | `CARGO_BUILD_JOBS=1 cargo test --test pr5_bundle_cli -- --test-threads=1` |
| M4 | Docs and migration guide, plus regression pass. | docs/spec markdown, tests only if needed | Full PR-5 verification matrix in T16 |

## Tests-First Scenarios

- [ ] **T1 Bundle file content drift realigns agent**
  - Description: Start or seed a running bundle-backed worker with stored `agents.config_hash`. Mutate a file inside `.ah/bundles/<name>/` without changing `ah.toml`. Call `session.realign` with the same bundle names and no `bundle_digest`. Expected: status `REALIGNED`, event `drift_realigned`, reason/message includes `bundle changed`, new `agents.config_hash`, and rematerialized provider home contains mutated content.
  - Files: `tests/pr5_bundle_realign_recovery.rs`; product code only if the test exposes a real gap.
  - Test: `CARGO_BUILD_JOBS=1 cargo test --test pr5_bundle_realign_recovery bundle_content_change_realigns_agent -- --test-threads=1`
  - Acceptance gate: no `ah.toml` mutation in fixture; no client-side digest in payload; second realign returns NO_CHANGE.

- [ ] **T2 Bundle hook/rules/skill digest coverage matrix**
  - Description: Prove content changes in at least three bundle surfaces trigger digest/hash change: `skills/<skill>/SKILL.md`, a hook script, and `rules/worker.md`. MCP manifest digest coverage may use existing PR-2 tests if already present, otherwise include a manifest-only mutation.
  - Files: `tests/pr5_bundle_realign_recovery.rs` or provider bundle unit tests.
  - Test: `CARGO_BUILD_JOBS=1 cargo test --test pr5_bundle_realign_recovery bundle_digest_covers_materialized_content -- --test-threads=1`
  - Acceptance gate: each mutation changes expected hash; empty/no-bundle hash stability tests still pass.

- [ ] **T3 `ah up` uses bundle names and daemon-side digest recomputation**
  - Description: CLI test records `session.realign` payload from `run_up`. Expected payload includes `bundle` arrays from `ah.toml`, does not require `bundle_digest`, and still resolves the active session exactly as existing `ah up` tests do.
  - Files: `src/cli/up.rs` tests if missing, or `tests/pr5_bundle_realign_recovery.rs`.
  - Test: `CARGO_BUILD_JOBS=1 cargo test --test pr5_bundle_realign_recovery ah_up_payload_preserves_bundle_refs_for_daemon_digest -- --test-threads=1`
  - Acceptance gate: no local filesystem bundle digest code added to `src/cli/up.rs`.

- [ ] **T4 Master bundle drift audit/force policy**
  - Description: Seed a session whose master references a bundle. Mutate master bundle content. `session.realign` without force reports master `DRIFT` audit-only and does not respawn. With force it realigns master and stores a new `sessions.config_hash`.
  - Files: `tests/pr5_bundle_realign_recovery.rs`; product code only if policy regressed.
  - Test: `CARGO_BUILD_JOBS=1 cargo test --test pr5_bundle_realign_recovery master_bundle_drift_is_audit_only_until_force -- --test-threads=1`
  - Acceptance gate: agent drift behavior remains independent from master audit status.

- [ ] **T5 Crash recovery rematerializes current bundle**
  - Description: Spawn/seed a bundle-backed agent and persisted `AgentSpawnSpec`, mutate bundle content, mark the agent recoverable crashed, run the recovery respawn path. Expected: replacement agent uses the same bundle reference, current bundle content appears in provider home, and stored `config_hash` matches current digest.
  - Files: `tests/pr5_bundle_realign_recovery.rs`; `src/orchestrator/mod.rs` only if needed.
  - Test: `CARGO_BUILD_JOBS=1 cargo test --test pr5_bundle_realign_recovery crash_recovery_rematerializes_current_bundle -- --test-threads=1`
  - Acceptance gate: recovery must not reuse stale materialized home files as the source of truth.

- [ ] **T6 Master revive reprovisions current bundle-backed worker**
  - Description: Use existing master-watch/recovery test hooks to simulate master death with a stored bundle-backed worker spec. Mutate bundle content before revive reprovisions workers. Expected: worker respawn goes through `spawn_realign_agent`, preserves bundle refs, and provider home contains current bundle content.
  - Files: `tests/pr5_bundle_realign_recovery.rs`; `src/monitor/master_watch.rs` only if needed.
  - Test: `CARGO_BUILD_JOBS=1 cargo test --test pr5_bundle_realign_recovery master_revive_reprovisions_current_bundle_worker -- --test-threads=1`
  - Acceptance gate: existing interrupted-job requeue assertions in master revive tests still pass.

- [ ] **T7 Recovery snapshot persists bundle fields**
  - Description: Unit test `AgentSpawnSpec` roundtrip with non-empty `bundle` and `bundle_digest`. Expected: both survive JSON storage/query, including backward compatibility for old specs without `bundle_digest`.
  - Files: `src/db/recovery.rs` tests or `tests/pr5_bundle_realign_recovery.rs`.
  - Test: `CARGO_BUILD_JOBS=1 cargo test --lib recovery_bundle_snapshot -- --test-threads=1`
  - Acceptance gate: no schema migration required if JSON roundtrip already suffices.

## CLI Tasks

- [ ] **T8 Add `Bundle` top-level subcommand**
  - Description: Add `Cmd::Bundle { cmd: BundleCmd }` with `Validate` and `List`. Route to a new `src/cli/bundle.rs`.
  - Files: `src/bin/ah.rs`, `src/cli/mod.rs`, `src/cli/bundle.rs`, `tests/pr5_bundle_cli.rs`.
  - Test: `CARGO_BUILD_JOBS=1 cargo test --test pr5_bundle_cli bundle_cli_subcommands_parse -- --test-threads=1`
  - Acceptance gate: existing `ah config validate`, `ah up`, and other top-level command parsing tests still pass.

- [ ] **T9 Implement project-root and selection rules**
  - Description: `ah bundle validate` and `ah bundle list` use global `--config` when present, otherwise find `ah.toml` from cwd. Validate selection: explicit names, `--all`, or referenced bundles by default.
  - Files: `src/cli/bundle.rs`, possibly `src/cli/config.rs` helper reuse.
  - Test: `CARGO_BUILD_JOBS=1 cargo test --test pr5_bundle_cli bundle_validate_selects_referenced_explicit_and_all -- --test-threads=1`
  - Acceptance gate: no daemon/RPC calls are made by bundle CLI tests.

- [ ] **T10 Implement `ah bundle validate`**
  - Description: Validate manifest syntax, path containment, referenced files, digest computation, and provider capability for bundles referenced by `ah.toml`.
  - Files: `src/cli/bundle.rs`, `src/provider/bundles.rs` only if public validation helpers are needed.
  - Test: `CARGO_BUILD_JOBS=1 cargo test --test pr5_bundle_cli bundle_validate_reports_success_and_failures -- --test-threads=1`
  - Acceptance gate: success output contains `VALID`; failure exits non-zero and includes bundle name plus cause; secret env values are never printed.

- [ ] **T11 Implement `ah bundle list`**
  - Description: Scan `.ah/bundles/*/bundle.toml`, sort by name, and print `NAME VERSION SKILLS HOOKS RULES MCP REFERENCED_BY STATUS`.
  - Files: `src/cli/bundle.rs`, `tests/pr5_bundle_cli.rs`.
  - Test: `CARGO_BUILD_JOBS=1 cargo test --test pr5_bundle_cli bundle_list_sorts_references_and_status -- --test-threads=1`
  - Acceptance gate: invalid bundles appear as `ERROR`; command exits non-zero if any discovered bundle is invalid.

- [ ] **T12 CLI no-secret regression**
  - Description: Bundle with MCP env placeholders fails validation or reports requirements using variable names only. Test output must not contain a fake secret value set in process env.
  - Files: `tests/pr5_bundle_cli.rs`.
  - Test: `CARGO_BUILD_JOBS=1 cargo test --test pr5_bundle_cli bundle_cli_does_not_print_secret_values -- --test-threads=1`
  - Acceptance gate: output may include `ACME_KEY`; output must not include `super-secret-test-value`.

## Documentation Tasks

- [ ] **T13 Add bundle user documentation**
  - Description: Document layout, manifest fields, master/agent `bundle` config, provider translation summary, and `ah bundle validate/list`.
  - Files: choose repo-appropriate docs path, for example `docs/plugin-bundles.md`.
  - Test/check: `rg -n "ah bundle validate|ah bundle list|bundle.toml|mcp|rules/worker.md" docs .kiro/specs/ah-plugin-bundle-pr5`
  - Acceptance gate: docs include a minimal complete bundle example and command examples.

- [ ] **T14 Add scattered-to-bundle migration guide**
  - Description: Include staged migration from scattered skills/hooks/rules to bundle, coexistence during migration, and rollback from bundle back to scattered config.
  - Files: same doc as T13 or a linked migration doc.
  - Test/check: `rg -n "coexist|scattered|rollback|plugins are not part of bundles|ah up" docs .kiro/specs/ah-plugin-bundle-pr5`
  - Acceptance gate: guide explicitly states unreferenced `.ah/bundles/<name>` directories are inert.

## Regression / Ship Tasks

- [ ] **T15 PR-5 focused tests**
  - Description: All new PR-5 tests pass.
  - Commands:
    - `CARGO_BUILD_JOBS=1 cargo test --test pr5_bundle_realign_recovery -- --test-threads=1`
    - `CARGO_BUILD_JOBS=1 cargo test --test pr5_bundle_cli -- --test-threads=1`
  - Acceptance gate: zero ignored PR-5 tests unless explicitly justified in PR notes.

- [ ] **T16 Existing bundle and drift regression**
  - Description: Ensure PR-5 did not regress PR-1 through PR-4 bundle behavior or `ah up` drift behavior.
  - Commands:
    - `CARGO_BUILD_JOBS=1 cargo test --test pr3_codex_bundle -- --test-threads=1`
    - `CARGO_BUILD_JOBS=1 cargo test --test e2e_bundle_materialization_a4 -- --test-threads=1`
    - `CARGO_BUILD_JOBS=1 cargo test --test ah_full_e2e_drift -- --test-threads=1`
    - `CARGO_BUILD_JOBS=1 cargo test --lib -- --test-threads=1`
  - Acceptance gate: if PR-4 lands antigravity tests under another test target, add that target to this list before shipping.

- [ ] **T17 Diff hygiene**
  - Description: Confirm PR-5 touched only intended implementation, tests, and docs.
  - Commands:
    - `cargo fmt`
    - `git diff --check`
    - `git diff --stat`
  - Acceptance gate: no unrelated provider/lifecycle refactors; no `src` changes outside the PR-5 scope described in `design.md`.

## Implementation Notes

- Prefer exposing small bundle inspection/validation helpers from `src/provider/bundles.rs` rather than duplicating manifest parsing in CLI code.
- CLI commands should be testable without spawning `ahd`; use pure functions that return structured rows/results, then print at the edge.
- Keep `ah up` digest responsibility daemon-side. Client-side digest computation would split the source of truth and make crash/master revive behavior harder to reason about.
- If a recovery test needs a provider home assertion, use the lightest provider fixture available on the base branch. Codex is usually the lowest-friction worker target; add antigravity coverage only after PR-4 provides stable fixtures.
