# PR-5 Rename Phase A Design

## 1. Scope
- Rename the Rust crate from `ccbd` to `ah`.
- Rename the daemon binary from `ccbd` to `ahd`.
- Keep the user-facing CLI binary as `ah`.
- Rename daemon-owned systemd unit strings with the daemon name.
- Keep this PR limited to code, tests, scripts, docs, and specs.

## 2. Schema
- `Cargo.toml [package] name = "ah"`.
- `Cargo.toml [[bin]] name = "ahd" path = "src/bin/ahd.rs"`.
- Move `src/bin/ccbd.rs` to `src/bin/ahd.rs`.
- Move daemon test helper binary to `src/bin/ahd_test_helper.rs`.
- Replace Rust crate references from `ah::` to `ah::`.
- Replace `ahd.service` with `ahd.service`.
- Replace `ahd-session-{...}.service` with `ahd-session-{...}.service`.
- Replace `BindsTo=ahd.service` with `BindsTo=ahd.service`.
- Replace daemon socket and database names with `ahd.sock` and `ahd.sqlite`.

## 3. Scope Out
- Do not edit `.ccb/` framework state.
- Do not rename the external `ccb` command family or `ccb ask` references.
- Do not rename the GitHub repository.
- Do not move the local `~/coding/ccbd-rust` checkout.
- Do not remove the legacy `AH_STATE_DIR` alias in Phase A.

## 4. Compatibility
- Prefer `AH_STATE_DIR` for new state-dir selection.
- Keep `AH_STATE_DIR` as a fallback alias for one version.
- On daemon startup, auto-rename `ahd.sqlite*` to `ahd.sqlite*` when the new database does not exist.

## 5. Risks
- `.ccb/` accidental edits would corrupt framework state, so grep and replace exclude it.
- Socket/database rename can strand old local state without the startup migration.
- Integration tests depend on Cargo binary env vars and must follow `ahd` names.
