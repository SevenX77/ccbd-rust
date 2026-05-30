# Tasks: ah PR-6 ERROR Recovery + Claude Worker Resume

metadata:
- spec: PR-6 ERROR Recovery + Claude Worker Resume
- design: `.kiro/specs/ah-pr6-recovery-resume/design.md`
- target LOC: 335 (design budget 250-350; implementation may compress toward ~280)
- sequencing: tests-first T1-T2 red -> src impl T3-T6 green -> T7 verify -> PM step 5/6 audit/docs

Hard decisions from 1d/1f/1g:
- `is_recovery = (running.state == "CRASHED")`, not "DB has row".
- `wrap_command` new `is_recovery: bool` is the 6th parameter; do not confuse it with `spawn_realign_agent`'s existing 5th parameter `killed_before_spawn`.
- `agent_spawned.reason` stays `DRIFT_REALIGN`; recovery evidence is event payload `is_recovery: true`, not a new `RECOVERY` reason enum.
- `ProviderManifest.resume_args` is `&'static [&'static str]`; Claude uses `&["--continue"]`, all other providers use `&[]`.

## T1: Flip case_11 to recovery success (tests-first red) - ~120 LOC

Files:
- Modify `tests/ah_full_e2e_realign_extra.rs:495-515` (`install_fake_claude_with_behavior`)
- Modify `tests/ah_full_e2e_realign_extra.rs:971-1001` (`case_11_error_recovery_known_gap`)

Change:
- In fake Claude startup, before the CRASH branch sleeps/exits, if `$GRAND_TOUR_RESUME_ARG_MARKER` is non-empty, write the physical argv: `printf '%s\n' "$@" > "$GRAND_TOUR_RESUME_ARG_MARKER"`.
- Update case_11 from JSON-RPC `AGENT_ALREADY_EXISTS` known-gap to successful recovery:
  - create a marker path under the test project/temp dir and inject `GRAND_TOUR_RESUME_ARG_MARKER` into the `a_crash` fixture env used for recovery;
  - call `session.realign` through normal `run_realign`;
  - assert `statuses[a_crash].status == "REALIGNED"`;
  - wait for `a_crash` back to `IDLE`;
  - assert PID changed from the crashed process;
  - assert marker contains `--continue`;
  - assert latest `agent_spawned` payload has `reason == "DRIFT_REALIGN"` and `is_recovery == true`.
- Keep case_06-09 unchanged. Add a negative assertion that the resume marker is absent before the CRASHED recovery path, proving IDLE/BUSY drift did not inject resume.

Acceptance:
- `CARGO_BUILD_JOBS=1 cargo test --test ah_full_e2e_realign_extra grand_tour_realign_extra_matrix -- --include-ignored --test-threads=1 --nocapture` must fail before src changes, because current src still returns `AGENT_ALREADY_EXISTS`.
- Red failure should be on case_11 recovery contract, not on setup/case_06-10 regressions.

Audit view:
- a2: red purity, marker checks physical argv not mocked JSON.
- a3: case_06-09 untouched; no BUSY/IDLE resume leakage.

## T2: Add marker helpers (tests-first red support) - ~25 LOC

Files:
- Modify `tests/ah_full_e2e_realign_extra.rs:1-110` helper area.

Change:
- Add `wait_for_resume_marker(path: &Path, timeout: Duration) -> String`, polling until marker exists and is readable.
- Add `assert_marker_contains(path: &Path, flag: &str)`, using the wait helper and checking line/substring contains `--continue`.
- Keep helpers local to this test file; do not touch `tests/common`.

Acceptance:
- T1+T2 compile if helper signatures are wired correctly.
- The grand tour remains red until T3-T6 implement src recovery.

Audit view:
- a2: helper fails with useful path/content diagnostics.
- a3: helper reads filesystem marker, not in-memory seam.

## T3: Add ProviderManifest.resume_args - ~30 LOC

Files:
- Modify `src/provider/manifest.rs:5-20` (`ProviderManifest` struct).
- Modify static manifest literals: `bash` `:138-150`, `codex` `:153-177`, `gemini` `:180-194`, `claude` `:197-211`.
- Modify `get_manifest` fallback `src/provider/manifest.rs:216-230`.
- Modify hand-written `ProviderManifest` literal in `src/rpc/handlers.rs:1980`.

Change:
- Add `pub resume_args: &'static [&'static str]`.
- Fill values:
  - bash: `&[]`
  - codex: `&[]`
  - gemini: `&[]`
  - claude: `&["--continue"]`
  - unknown fallback: `&[]`
- Update existing provider manifest tests so current command assertions still pass and optionally assert Claude `resume_args`.

Acceptance:
- `cargo test -p ccbd provider::manifest` or workspace compile should catch all struct literal fan-out.

Audit view:
- a2: no `Vec<String>`.
- a3: Codex/Gemini remain no-op for PR-6 scope.

## T4: Thread is_recovery through wrap_command and argv builder - ~80 LOC

Files:
- Modify `src/sandbox/systemd.rs:8-16` (`wrap_command` signature).
- Modify `src/sandbox/systemd.rs:108-123` (`command_with_env_prefix`).
- Modify `src/rpc/handlers.rs:717` production call.
- Modify `src/sandbox/systemd.rs` existing wrap_command test calls at `:177`, `:205`, `:220`, `:239`, `:331`, `:348`, `:369`, `:386`, `:465`, `:490`, `:506`, `:526`.

Change:
- Add `is_recovery: bool` as the 6th parameter to `wrap_command`, before `daemon_unit`.
- Add an inline comment near the parameter or call site: `is_recovery is true only when realign matched CRASHED`.
- Pass `is_recovery` into `command_with_env_prefix`.
- Update `command_with_env_prefix(manifest, extra_env_vars, is_recovery)` so it appends `manifest.resume_args` after `manifest.command` only when `is_recovery` is true.
- Ensure both systemd and unsafe-no-sandbox paths use the same builder.
- Existing tests pass `false`.
- Add `wrap_command_with_recovery_appends_resume_args`:
  - Claude + `is_recovery=true` includes `--continue`;
  - Claude + `false` does not include `--continue`;
  - Codex/Gemini/Bash + `true` do not append resume args.

Acceptance:
- `cargo test -p ccbd sandbox::systemd` passes.
- `rg -n "wrap_command\\(" src/rpc/handlers.rs src/sandbox/systemd.rs` shows all calls explicitly pass the new bool.

Audit view:
- a2: verify parameter order; 6th parameter, not replacing `daemon_unit`.
- a3: verify `--continue` lands after provider base command, not in env.

## T5: Thread is_recovery through realign spawn path - ~30 LOC

Files:
- Modify `src/rpc/handlers.rs:582-588` (`spawn_realign_agent` signature).
- Modify `src/rpc/handlers.rs:458` NEW call.
- Modify `src/rpc/handlers.rs:505` ordinary drift/force call.
- Modify `src/rpc/handlers.rs:589-600` spawn call internals.
- Modify `src/rpc/router.rs:79` only if `handle_agent_spawn` public signature changes.
- Modify existing `handle_agent_spawn` unit-test calls in `src/rpc/handlers.rs:2341`, `:2380`, `:2596`, `:2639`, `:2782`, `:2994` if signature fan-out requires it.

Change:
- Add `is_recovery: bool` to `spawn_realign_agent`.
- NEW path passes `false`.
- Ordinary drift/force path passes `false`; keep existing `killed_before_spawn=true` unchanged.
- Add an internal spawn helper if needed so public RPC `handle_agent_spawn(params, ctx)` remains `is_recovery=false`, while `spawn_realign_agent(..., true)` can reach `wrap_command(..., is_recovery=true)`.
- Do not relax `agent_exists` at `src/rpc/handlers.rs:684-685`.

Acceptance:
- Current `agent.spawn` behavior is unchanged and still rejects duplicate rows.
- Compile catches every changed handler signature.

Audit view:
- a2: no public RPC contract expansion unless intentionally hidden behind internal helper.
- a3: ordinary REALIGNED from IDLE/BUSY does not pass recovery.

## T6: Add CRASHED recovery branch in handle_session_realign - ~50 LOC

Files:
- Modify `src/rpc/handlers.rs:629-654` (`running_agent_hashes` SQL).
- Modify `src/rpc/handlers.rs:439-520` (`handle_session_realign` control flow).
- Modify `src/rpc/handlers.rs:618-624` (`agent_spawned` payload).

Change:
- Change SQL from `state NOT IN ('CRASHED', 'KILLED')` to `state != 'KILLED'`.
- In `handle_session_realign`, insert `if running.state == "CRASHED"` before the hash-equality `NO_CHANGE` block at `:467`.
- CRASHED branch:
  - compute/use reason as `DRIFT_REALIGN` path;
  - delete the old row before spawn;
  - call `spawn_realign_agent(..., killed_before_spawn=true, is_recovery=true)`;
  - return per-agent `status="REALIGNED"`;
  - do not return `NEW`;
  - do not return `NO_CHANGE` even if hash matches.
- `agent_spawned` event keeps `reason: "DRIFT_REALIGN"` and adds `is_recovery: true` in payload when applicable.
- Keep `KILLED` excluded from `running_agent_hashes`, so no killed recovery occurs.

Acceptance:
- case_11 no longer receives JSON-RPC `AGENT_ALREADY_EXISTS`.
- CRASHED hash-equal recovery still realigns.
- Existing NO_CHANGE behavior for IDLE hash-equal remains unchanged.

Audit view:
- a2: CRASHED branch priority above NO_CHANGE.
- a3: event payload changed without reason enum expansion.

## T7: Verification - 0 LOC

Commands:
- Red after T1-T2:
  - `CARGO_BUILD_JOBS=1 cargo test --test ah_full_e2e_realign_extra grand_tour_realign_extra_matrix -- --include-ignored --test-threads=1 --nocapture`
- Green after T3-T6:
  - `CARGO_BUILD_JOBS=1 cargo test --test ah_full_e2e_realign_extra grand_tour_realign_extra_matrix -- --include-ignored --test-threads=1 --nocapture`
  - `CARGO_BUILD_JOBS=1 cargo test --test ah_full_e2e_realign_extra -- --test-threads=1`
  - `CARGO_BUILD_JOBS=1 cargo test --workspace`

Expected:
- Grand tour case_06-09 still pass and do not create/use resume marker.
- case_10 still reaches `CRASHED`.
- case_11 passes with `REALIGNED`, `IDLE`, new PID, marker contains `--continue`, and `agent_spawned` payload `is_recovery=true`.
- Default lane remains common tests passed with grand tour ignored.

## LOC Total

- T1: 120
- T2: 25
- T3: 30
- T4: 80
- T5: 30
- T6: 50
- T7: 0

Total: 335 LOC, within the 250-350 PR-6 budget.

## Tests-First Gate

T1 + T2 must land first and produce a meaningful red test. Only after red purity review should T3-T6 src implementation proceed. Step 3 audit focus: case_11 contract coverage, marker physicality, and no accidental resume on case_06-09.
