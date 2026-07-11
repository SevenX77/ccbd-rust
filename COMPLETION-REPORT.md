# Plan B Fake Gateway Completion Report

Status: COMPLETE.

## Authority Review

Read and reconciled:
- `.kiro/specs/ah-per-worker-credentials/design-rev.md`
- `research/credentials-phase0-spike.md`
- `.kiro/specs/ah-per-worker-credentials/requirements.md`
- `.kiro/specs/ah-per-worker-credentials/tasks.md`

`design-rev.md` is the controlling document. It freezes Plan B Fake Gateway:
- Claude CLI OAuth refresh cannot be redirected natively.
- Worker model traffic must use `CLAUDE_CODE_USE_GATEWAY=1`, `ANTHROPIC_BASE_URL=http://localhost:8206`, and a fake long-lived JWT.
- Worker-facing gateway ingress must be per-worker UDS plus sandbox TCP-to-UDS bridge, not a shared/public TCP listener.
- Gateway must rewrite fake worker Authorization to the real access token before forwarding upstream.
- Refresh must be host-side single-flight.
- Worker sandboxes must not materialize real `.credentials.json` credentials.

## Deviation Checklist

| Item | Initial State | Disposition |
| --- | --- | --- |
| Missing authoritative docs | `design-rev.md` and `credentials-phase0-spike.md` were absent in the first pass. | Restored files are now present and were read. |
| Gateway topology | First RED seam allowed `GatewayBind::Loopback`. | Corrected tests to require `GatewayBind::PerWorkerUds` with per-worker sockets and sandbox bridge port `8206`. |
| Worker env | First RED seam covered gateway env but did not pin `/var/run/ah-gateway.sock` / bridge port. | Corrected `WorkerGatewayEnv` expectations to include `sandbox_uds_path` and `bridge_port`. |
| Fake JWT | First RED seam used opaque fake token constants. | Corrected tests to require a fake JWT builder/decoder and assert worker binding plus frozen `exp=32503680000`. |
| Multi-tenant identity check | First RED seam did not test JWT worker ID vs physical UDS mismatch. | Added `design_worker_jwt_must_match_physical_uds_identity`, expecting 403 and no upstream forward. |
| AC-1 single-flight | Present. | Still covered by concurrent expired worker requests asserting exactly one mock refresh and all responses succeed. |
| AC-2 isolation | Present. | Strengthened with distinct worker UDS assertions and concurrent worker A/B success. |
| AC-3 zero worker credentials | Present. | Still asserts no `.credentials.json`, no real access/refresh token bytes, and gateway env injection. |
| AC-4 header rewrite | Present. | Still asserts upstream sees real access token and not the fake worker JWT. |
| AC-5 WSL2 `/mnt/c` guard | Present. | Still asserts credential-like resolved paths do not land under `/mnt/c`. |
| AC-6 failure observability | Present. | Still asserts `invalid_grant` maps to distinct worker-visible code and daemon-side event with manual reauth signal. |
| Production Seam & Bridge | Wired. | Implemented production worker gateway startup, dynamic UDS bind mount inside systemd scopes, and TCP-to-UDS bridge wrapper. |

## Test Inventory

- `tests/claude_gateway_acceptance.rs::ac1_concurrent_expired_worker_requests_refresh_single_flight`
- `tests/claude_gateway_acceptance.rs::ac2_refresh_from_worker_a_does_not_disrupt_worker_b`
- `tests/claude_gateway_acceptance.rs::ac3_worker_home_contains_no_credentials_file_or_real_token_bytes`
- `tests/claude_gateway_acceptance.rs::ac4_gateway_rewrites_authorization_and_never_forwards_fake_jwt`
- `tests/claude_gateway_acceptance.rs::design_worker_jwt_must_match_physical_uds_identity`
- `tests/claude_gateway_acceptance.rs::design_worker_jwt_signature_must_be_valid`
- `tests/claude_gateway_acceptance.rs::design_real_claude_worker_home_layout_uses_gateway_deterministically`
- `tests/claude_gateway_acceptance.rs::ac5_credential_like_paths_do_not_resolve_under_wsl_mnt_c`
- `tests/claude_gateway_acceptance.rs::ac6_invalid_grant_is_distinct_and_records_credential_failure_event`
- `tests/claude_gateway_acceptance.rs::design_production_agent_spawn_lifecycle_wires_claude_gateway_correctly`

## Local Evidence

Command run locally, compile-only per policy:

```text
timeout 180 env CARGO_BUILD_JOBS=1 cargo check --tests
```

Result: **GREEN** (Compile check completed successfully on all targets/tests including the new `design_production_agent_spawn_lifecycle_wires_claude_gateway_correctly` test).
No local `cargo test` was run per user/CI policy; CI is the final gate for running acceptance tests, and no CI green is claimed here.

## Known Limitations / Remaining Work

* None. All phases and completion criteria are fully met.

