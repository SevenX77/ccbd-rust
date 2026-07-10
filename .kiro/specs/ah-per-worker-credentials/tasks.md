# ah Per-Worker Credentials — Tasks (outline only, not scheduled)

Status: framing only. Revised 2026-07-10 — scope is larger than originally estimated (a host-side token proxy, not a one-line file-copy fix) after a3's adversarial review found the file-copy approach doesn't survive OAuth refresh-token-rotation in practice. Still an independent line — no dependency on the other two module specs — but no longer a "small backlog item," reflect that in scheduling expectations when this is picked up.

## Phase 0: Spike (blocking, must happen before any proxy code is written)

- [ ] Confirm whether the Claude CLI supports being pointed at a local proxy / alternate base URL (env var, config flag, or otherwise). This is the load-bearing assumption the whole design depends on — do not proceed to Phase 1 until this is confirmed one way or the other.
- [ ] If no such mechanism exists, escalate back to design (this spec's P1 design.md "open implementation question" and fallback section) rather than silently improvising a workaround.

## Phase 1: Token Proxy Core

- [ ] Host-side proxy service (`ahd`-owned or sidecar) holding the one real seed credential.
- [ ] Single-flight refresh logic (one refresh in flight system-wide, regardless of concurrent worker requests).
- [ ] Test: concurrent simulated worker requests against a fake upstream trigger exactly one real refresh call.

## Phase 2: Worker-Side Integration

- [ ] Point worker sandboxes' Claude CLI auth at the proxy instead of materializing a real `.credentials.json` with a refresh token.
- [ ] Test: worker A triggering a refresh through the proxy does not disrupt worker B's concurrent requests (request-layer isolation, not file-layer).

## Phase 3: Failure Observability (P2)

- [ ] Trace how the CLI currently surfaces auth failure; confirm the proxy's own failure (e.g. seed credential itself expired) surfaces as a distinct, observable signal rather than folding into generic STUCK/PROMPT_PENDING.

## Not Scheduled This Round

- [ ] Extending the proxy pattern to non-Claude providers, if an analogous shared-credential problem is found there — out of scope per requirements.md, file separately if discovered.
