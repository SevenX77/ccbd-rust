# Plan B Fake Gateway Completion Report

## Change List

- Added `ah::claude_gateway` core with:
  - shared seed `TokenSet`
  - single-flight refresh lock
  - worker fake JWT stripping and upstream Authorization rewrite
  - worker channel/JWT worker_id mismatch rejection with 403
  - distinct `invalid_grant` mapping to `AH_CLAUDE_GATEWAY_REFRESH_INVALID_GRANT`
  - in-memory credential failure event recorder for daemon-observable state
- Changed Claude home materialization so worker sandboxes no longer receive `.claude/.credentials.json` by symlink or copy.
- Injected Claude gateway mode env:
  - `CLAUDE_CODE_USE_GATEWAY=1`
  - `ANTHROPIC_BASE_URL=http://localhost:8206`
  - `ANTHROPIC_AUTH_TOKEN=<alg:none fake JWT with exp=32503680000 and worker_id>`
- Removed Anthropic credential env passthrough from provider spawn env.
- Added per-worker gateway topology helper for the design path shape:
  - host UDS: `{worker_sandbox_root}/tmp/ah-gateway.sock`
  - sandbox UDS: `/var/run/ah-gateway.sock`
  - sandbox TCP bridge URL: `http://localhost:8206`
- Updated existing config/home-layout tests that encoded the pre-Plan-B Claude symlink contract.
- Updated master revive logging/env to stop expecting an auth symlink and to inject gateway mode env on revive.
- Added the previously missing authority docs:
  - `.kiro/specs/ah-per-worker-credentials/design-rev.md`
  - `research/credentials-phase0-spike.md`

## Deviation Review After Supplemental Docs

- `design-rev.md` requires a real three-segment JWT with `alg:none`, `exp=32503680000`, `sub=ah-worker-session`, and `worker_id`.
  - Previous state: token was `ah-fake-jwt.<worker>.long-lived`.
  - Disposition: fixed in `fake_worker_jwt`; tests decode and assert header/payload.
- `design-rev.md` requires sandbox CLI to use `ANTHROPIC_BASE_URL=http://localhost:8206`.
  - Previous state: deterministic high port per slot.
  - Disposition: fixed to the frozen `8206` URL and covered in tests.
- `design-rev.md` requires checking physical worker channel against JWT `worker_id`.
  - Previous state: gateway core did not inspect JWT claims.
  - Disposition: fixed by parsing fake JWT and rejecting mismatch with `AH_CLAUDE_GATEWAY_WORKER_ID_MISMATCH`.
- `design-rev.md` requires per-worker UDS topology and `/var/run/ah-gateway.sock` sandbox path.
  - Previous state: not represented in code.
  - Disposition: added `gateway_worker_topology` and tests for host/sandbox path shape and WSL `/mnt/c` rejection.
- `credentials-phase0-spike.md` confirms `CLAUDE_CODE_USE_GATEWAY` bypasses OAuth refresh and relies on external refresh/restart.
  - Previous state: implementation already removed worker credentials and host Anthropic env passthrough.
  - Disposition: retained; report now cites this as the reason worker `.credentials.json` remains absent.
- Remaining integration boundary:
  - The lib-level gateway core and worker env/topology are implemented and checked.
  - Real CLI conversation through the UDS-backed local bridge remains CI/live-stack validation, per original task constraints.

## Acceptance Coverage

- AC-1 single-flight refresh:
  - `tests/plan_b_gateway_acceptance.rs::ac1_concurrent_expired_requests_single_flight_refresh_once`
- AC-2 worker isolation:
  - `tests/plan_b_gateway_acceptance.rs::ac2_refresh_by_worker_a_does_not_fail_worker_b`
- AC-3 worker zero credentials:
  - `tests/plan_b_gateway_acceptance.rs::ac3_claude_worker_home_has_no_real_credentials_file_or_token`
  - updated `tests/mvp12_home_layout.rs::test_provider_home_layout_materialization`
  - updated `tests/ah_config_drift.rs::claude_uses_gateway_without_credentials_and_codex_auth_still_tracks_host_refresh`
- AC-4 header rewrite and worker identity guard:
  - `tests/plan_b_gateway_acceptance.rs::ac4_worker_fake_jwt_is_rewritten_to_real_access_token`
  - `tests/plan_b_gateway_acceptance.rs::ac4_gateway_rejects_fake_jwt_from_wrong_worker_channel`
- AC-5 WSL2 path guard:
  - `tests/plan_b_gateway_acceptance.rs::ac5_credentials_paths_reject_wsl_windows_mounts`
- AC-6 failure observability:
  - `tests/plan_b_gateway_acceptance.rs::ac6_invalid_grant_returns_distinct_error_and_records_event`

## Red / Green Evidence

- Initial RED local check:
  - Command: `timeout 120 env CARGO_BUILD_JOBS=1 cargo check --tests`
  - Commit: `05d28d3`
  - Result: failed with `E0432` / `E0433` because `ah::claude_gateway` did not exist.
- Supplemental RED local check after reading `design-rev.md`:
  - Command: `timeout 120 env CARGO_BUILD_JOBS=1 cargo check --tests`
  - Result: failed with `E0432` because `gateway_worker_topology` did not exist.
- GREEN local check:
  - Command: `timeout 120 env CARGO_BUILD_JOBS=1 cargo check --tests`
  - Result: finished successfully.

## Known Limits

- Per policy, no local `cargo test` or full build was run.
- CI evidence is pending operator push/CI execution.
- Real Claude CLI end-to-end live conversation through the local gateway is not locally exercised; it remains CI/live-stack validation.
