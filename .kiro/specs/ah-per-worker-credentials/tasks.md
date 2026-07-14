# ah Per-Worker Credentials — Tasks (outline only, not scheduled)

Status: framing only. Revised 2026-07-10 — scope is redefined to implement Plan B (Fake Gateway) after Phase 0 spike disproved the original redirect-based proxy. Still an independent line — no dependency on the other two module specs.

## Phase 0: Spike (Completed)

- [x] Confirm whether the Claude CLI supports being pointed at a local proxy / alternate base URL. (Completed: Spike disproved original redirect proxy and approved Plan B: Fake Gateway).

## Phase 1: Token Gateway Core (Plan B)

- [ ] Implement host-side HTTP Gateway service (as an asynchronous task in `ahd` or an independent loopback UDS daemon) holding the one real seed credential.
- [ ] Implement proxy forwarding for API calls (e.g. `/v1/messages`) and header rewriting logic (stripping Fake JWT and attaching the latest valid Real Access Token).
- [ ] Implement single-flight refresh logic protected by in-memory `RwLock` + `Mutex` + `watch` to coordinate concurrent worker requests during token expiry.
- [ ] Test: concurrent simulated worker requests against mock upstream trigger exactly one real refresh call.

## Phase 2: Worker-Side Integration (Plan B)

- [ ] Modify `ahd`'s sandbox bootstrap to drop `.credentials.json` creation and mount a dedicated UDS socket: `/home/sevenx/.cache/ah/sandboxes/{worker_id}/tmp/ah-gateway.sock` -> `/var/run/ah-gateway.sock`.
- [ ] Implement light TCP-to-UDS port forwarding inside the sandbox (bridge `127.0.0.1:8206` to `/var/run/ah-gateway.sock`).
- [ ] Inject environment variables: `CLAUDE_CODE_USE_GATEWAY=1`, `ANTHROPIC_BASE_URL=http://localhost:8206`, and `ANTHROPIC_AUTH_TOKEN=<FAKE_LONG_LIVED_JWT>`.
- [ ] Test: worker A triggering a refresh through the gateway does not disrupt worker B's concurrent requests.

## Phase 3: Failure Observability (P2)

- [ ] Track gateway refresh failures (e.g. seed token revoked upstream) and surface them as distinct HTTP response codes/errors to make the CLI crash visibly.
- [ ] Ensure daemon-side observability tracks the credential state and triggers distinct user-facing notifications for manual re-login when seed expires.

## Not Scheduled This Round

- [ ] Extending the gateway pattern to non-Claude providers, if an analogous shared-credential problem is found there — out of scope per requirements.md, file separately if discovered.
