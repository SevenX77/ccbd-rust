# Plan B Fake Gateway Completion Report

## Change List

- Added `ah::claude_gateway` core with:
  - shared seed `TokenSet`
  - single-flight refresh lock
  - worker fake JWT stripping and upstream Authorization rewrite
  - distinct `invalid_grant` mapping to `AH_CLAUDE_GATEWAY_REFRESH_INVALID_GRANT`
  - in-memory credential failure event recorder for daemon-observable state
- Changed Claude home materialization so worker sandboxes no longer receive `.claude/.credentials.json` by symlink or copy.
- Injected Claude gateway mode env:
  - `CLAUDE_CODE_USE_GATEWAY=1`
  - `ANTHROPIC_BASE_URL=http://localhost:<deterministic-port>`
  - `ANTHROPIC_AUTH_TOKEN=ah-fake-jwt...`
- Removed Anthropic credential env passthrough from provider spawn env.
- Updated existing config/home-layout tests that encoded the pre-Plan-B Claude symlink contract.
- Updated master revive logging/env to stop expecting an auth symlink and to inject gateway mode env on revive.

## Acceptance Coverage

- AC-1 single-flight refresh:
  - `tests/plan_b_gateway_acceptance.rs::ac1_concurrent_expired_requests_single_flight_refresh_once`
- AC-2 worker isolation:
  - `tests/plan_b_gateway_acceptance.rs::ac2_refresh_by_worker_a_does_not_fail_worker_b`
- AC-3 worker zero credentials:
  - `tests/plan_b_gateway_acceptance.rs::ac3_claude_worker_home_has_no_real_credentials_file_or_token`
  - updated `tests/mvp12_home_layout.rs::test_provider_home_layout_materialization`
  - updated `tests/ah_config_drift.rs::claude_uses_gateway_without_credentials_and_codex_auth_still_tracks_host_refresh`
- AC-4 header rewrite:
  - `tests/plan_b_gateway_acceptance.rs::ac4_worker_fake_jwt_is_rewritten_to_real_access_token`
- AC-5 WSL2 path guard:
  - `tests/plan_b_gateway_acceptance.rs::ac5_credentials_paths_reject_wsl_windows_mounts`
- AC-6 failure observability:
  - `tests/plan_b_gateway_acceptance.rs::ac6_invalid_grant_returns_distinct_error_and_records_event`

## Red / Green Evidence

- RED local check:
  - Command: `timeout 120 env CARGO_BUILD_JOBS=1 cargo check --tests`
  - Commit: `05d28d3`
  - Result: failed with `E0432` / `E0433` because `ah::claude_gateway` did not exist.
- GREEN local check:
  - Command: `timeout 120 env CARGO_BUILD_JOBS=1 cargo check --tests`
  - Result: finished successfully in `18.86s`.

## Known Limits

- Per policy, no local `cargo test` or full build was run.
- CI evidence is pending operator push/CI execution.
- Real Claude CLI end-to-end live conversation through the local gateway is not locally exercised; it remains CI/live-stack validation.
- The authoritative brief referenced `.kiro/specs/ah-per-worker-credentials/design-rev.md` and `research/credentials-phase0-spike.md`, but this worktree contains `.kiro/specs/ah-per-worker-credentials/design.md` and no matching phase0 spike file. Implementation followed the task brief acceptance contract and available design material.
