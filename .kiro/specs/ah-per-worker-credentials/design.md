# ah Per-Worker Credentials — Design

Status: draft, master-authored, revised 2026-07-10 after a3 adversarial review found the first draft's core mechanism doesn't work in production. Second pass below; not yet re-reviewed by a3.

## Design Thesis, Revised

The first draft's plan (`fs::copy` instead of symlink, giving each worker an independent physical credentials file) was a one-line mechanism change that looked like it closed P1/P2. **a3's adversarial review (`research/a3-adversarial-review-of-c-d-specs-2026-07-10.md` §五) found it doesn't** — and the finding is not a matter of taste, it's standard OAuth 2.0 behavior:

Most modern OAuth providers (Auth0-family flows, which Claude's CLI auth is built on) use **refresh token rotation (RTR)**: each time a refresh token is used, the server issues a new access token *and* a new refresh token, and invalidates the old refresh token. If a previously-invalidated refresh token is used again, the server treats it as a signal of token theft/replay and revokes the *entire* token family — including the refresh token that's currently valid.

Copy-on-create gives every worker an independent **file**, but all copies start as the *same token value*. The first worker to refresh gets a new, valid token; every other worker still holds the now-invalidated original. The moment a second worker tries to use its (now-stale) copy, the server doesn't just reject that one worker — RTR's replay-detection revokes the *first* worker's freshly-issued token too, cascading the outage across every worker sharing that seed, at 100% probability once any two workers' refresh windows overlap (which they will, since they were all seeded from the same login and have roughly the same token lifetime). This is **worse than the symlink bug in one respect**: the symlink at least fails predictably (one shared file, one clear point of failure); RTR-triggered revocation can take down workers that had *just* successfully refreshed, making it look like an intermittent, hard-to-diagnose flake rather than the deterministic mechanism it actually is.

**The fix cannot be "give each worker its own copy of the same token."** It has to be "workers never independently hold or use a refresh token at all." That's a materially different, larger design than the first draft — noted honestly here rather than downplayed: this is no longer a one-line `fs::copy` fix, it's a small proxy service. If that scope increase isn't acceptable for this spec round, the fallback is documented at the end of this section, but it is *not* recommended.

## P1 Design, Revised: Host-Side Token Proxy (adopting a3 §七.4)

**Mechanism**: `ahd` (or a small sidecar it owns) runs a lightweight token-proxy service that is the *only* holder of the real, refreshable credential (access token + refresh token). Workers never receive a `.credentials.json` with a real refresh token in it — copy or symlink, neither.

```text
[ahd / token-proxy sidecar]
  holds: real access_token + refresh_token (one instance, one refresh lifecycle)
  responsibility: refresh on expiry, single-flight (only one refresh in-flight
                   at a time, regardless of how many workers are asking)
        |
        | (short-lived, worker-scoped forwarding: local socket or loopback
        |  HTTP the CLI's auth layer is pointed at instead of the real
        |  Anthropic endpoint)
        v
[Worker 1]   [Worker 2]   [Worker N]
  each configured (via whatever the Claude CLI's proxy/base-URL override
  mechanism is — implementer to confirm it exists and what it's named)
  to route auth-bearing requests through the proxy, which attaches the
  current valid access token before forwarding upstream.
```

- Workers hold **no real refresh token** at all — nothing to independently rotate, nothing to replay, nothing for RTR's replay-detection to ever see as a conflict, because there is exactly one refresh lifecycle system-wide, owned by the proxy.
- The proxy's single-flight discipline (only one refresh in flight at a time) is itself a reused pattern, not new: the existing `Arc<Mutex<Connection>>` single-writer discipline elsewhere in this codebase (per the architecture assessment and both other specs in this design round) is the same shape — one owner, no concurrent-mutation races. Implementer should look at whether existing daemon-side singleton/lock patterns can be reused for the proxy's refresh path rather than inventing a new one.
- **Open implementation question, not resolved by this design pass**: does the Claude CLI (and other providers this might extend to) actually support pointing its outbound requests at a local proxy/alternate base URL? This is load-bearing — if the CLI hardcodes its upstream endpoint with no override mechanism, this design doesn't work as stated and needs a different interception point (e.g. a transparent local reverse-proxy the CLI's HTTP client is forced through via `HTTPS_PROXY`-style env var, if the CLI's HTTP client respects that convention). **Implementer's first task**: confirm which interception mechanism the Claude CLI actually supports before writing any proxy code — this determines whether the proxy is a literal API-shaped service or a TLS-terminating intercepting proxy, which are different builds.

## P2 Design: Failure Observability (unchanged from first draft)

Trace point (implementer to confirm at implementation time): when the proxy itself fails to refresh (e.g. the underlying seed credential's refresh token is itself expired/revoked — a real failure the proxy can't route around), that failure should surface as a distinct agent-facing or daemon-level signal, not fold into the generic `STUCK`/`PROMPT_PENDING` buckets. Under the revised P1 design, this failure is now a **single point of observability** for the whole fleet (the proxy either has a valid token or it doesn't) rather than N independent per-worker failure points — which is a secondary benefit of the corrected design worth noting: monitoring gets simpler, not just correctness.

## Failure Modes (revised)

- **Proxy is now a single point of failure for auth across all workers.** This is the direct, honest tradeoff for eliminating N-way RTR cascade risk: instead of "any worker's refresh can theoretically cascade-fail the others" (the first draft's unsolved problem), it's "the proxy's own health gates every worker's ability to make authenticated calls." This is the correct tradeoff — a single, well-monitored, restart-recoverable proxy process is a much smaller, more tractable failure surface than a distributed replay-detection cascade across N workers — but implementer must ensure the proxy itself is trivially restartable/recoverable (holds no unrecoverable state beyond the one credential file, which persists across restarts) so this SPOF doesn't become its own incident class.
- **Provider auth-flow assumption risk** (see "open implementation question" above): if the CLI truly cannot be pointed at a local proxy, this design needs a fallback interception mechanism, or — as a last resort, **explicitly not preferred** — a return to independent per-worker OAuth logins (the "maximally isolated but operationally heavy" option the first draft rejected on ops-cost grounds; revisit only if the proxy approach turns out to be technically infeasible, not for convenience).
- **Copy staleness for the seed credential itself**: unchanged from first draft — if the seed credential's refresh token is itself already invalid before the proxy ever starts, that's a provisioning-time problem, not something this design claims to solve.
- **Trust-state (`materialize_trust`) parity — still resolved, unchanged**: `materialize_trust` already uses `copy_if_missing`, not a symlink, and does not carry refresh-token-shaped secrets (it's trust/workspace-approval state, not an OAuth credential) — the RTR attack does not apply to it. No action needed there.

## Fallback If the Proxy Approach Is Rejected as Too Large a Scope Increase

Not recommended, documented only so the tradeoff is explicit if someone chooses it anyway: keep copy-on-create (first draft's P1), but add an explicit **serialization discipline** — only one worker at a time is ever allowed to hold a "live" (refreshable) credential; all others get a read-only, non-refreshing credential and must queue for the lock before making a call that might trigger a refresh. This still has a single point of contention (defeating the "N workers run concurrently" goal that presumably motivated per-worker credentials in the first place) and is strictly worse than the proxy design for zero implementation-simplicity benefit once you're already building coordination logic — it's listed here only to make clear it was considered and rejected, not as a real alternative.
