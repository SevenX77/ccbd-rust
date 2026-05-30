# PR-5 Rename Phase A Tasks

## T1 Cargo And Binaries
- Change Cargo package name from `ccbd` to `ah`.
- Change daemon bin from `ccbd` to `ahd`.
- Move `src/bin/ccbd.rs` to `src/bin/ahd.rs`.
- Move daemon test helper to `src/bin/ahd_test_helper.rs`.
- Acceptance: `cargo check --workspace` resolves both `ah` and `ahd`.

## T2 Rust Crate Imports
- Replace `ah::` imports and paths with `ah::`.
- Replace daemon helper env names from `CARGO_BIN_EXE_ahd*` to `CARGO_BIN_EXE_ahd*`.
- Acceptance: no Rust compile errors from unresolved crate or binary env vars.

## T3 Systemd Strings
- Replace `ahd.service` with `ahd.service`.
- Replace `ahd-session-` with `ahd-session-`.
- Replace daemon-owned tmux/systemd prefixes where they encode daemon identity.
- Acceptance: unit-name tests expect `ahd` names.

## T4 Persistence Rename
- Replace active database path `ahd.sqlite` with `ahd.sqlite`.
- Add daemon startup migration from legacy `ahd.sqlite*` to `ahd.sqlite*`.
- Acceptance: legacy file names appear only in migration compatibility logic or docs.

## T5 State Env Rename
- Prefer `AH_STATE_DIR`.
- Keep `AH_STATE_DIR` as fallback alias for one version.
- Update scripts and tests to use `AH_STATE_DIR` for new invocations.
- Acceptance: state layout tests cover new env plus fallback alias.

## T6 Docs And Specs
- Update docs, scripts, and specs for Phase A names.
- Keep local checkout path `ccbd-rust` and external `ccb` framework references unchanged.
- Acceptance: targeted grep shows no stale daemon/package strings outside explicit legacy notes.

## T7 Verification
- Run `CARGO_BUILD_JOBS=1 cargo check --workspace`.
- Run `CARGO_BUILD_JOBS=1 cargo test --workspace --lib -- --test-threads=1`.
- Run `CARGO_BUILD_JOBS=1 cargo test --workspace -- --test-threads=1`.
- Acceptance: all commands exit successfully.

## T8 Commit Boundary
- Commit only after T1-T7 pass.
- Do not push or rename the GitHub repository in Phase A.
- Acceptance: git status excludes unrelated pre-existing untracked files.
