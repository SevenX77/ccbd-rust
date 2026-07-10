# ah Per-Worker Credentials — Requirements

Status: converged after a3 adversarial review (2026-07-10) — the review found the original copy-on-create mechanism does not survive OAuth refresh-token-rotation in production; P1 was substantially redesigned to a host-side token-proxy architecture as a result (see design.md and `research/a3-adversarial-review-of-c-d-specs-2026-07-10.md` §五/§七.4). Scope grew from a one-line fix to a small service — flagged explicitly, not downplayed. **Not yet cleared for implementation** — awaiting operator/user sign-off. Independent line — no dependency on `ah-perception-arbiter` or `ah-control-plane-refactor`; can schedule separately.

Source material: `research/architecture-assessment-converged-2026-07-09.md` §一.13.

## Scope

In scope:
- Replace the current shared-credential symlink model with per-worker independent credential materialization for the Claude provider (the only provider this mechanism currently exists for — see Existing Grounding).
- Failure/rotation isolation: one worker's credential rotation or logout must not invalidate another worker's live session.

Out of scope:
- Non-Claude provider credential handling (codex/antigravity have their own auth models, not touched by this spec unless investigation finds an analogous shared-credential pattern — if so, file as a follow-up, don't silently expand scope here).
- Credential storage security hardening beyond what's needed to fix the sharing bug (e.g. this spec does not take on secrets-at-rest encryption as a goal unless it falls out naturally from the fix).

## Existing Grounding

- `link_credentials` (`src/provider/home_layout.rs`, function present on current HEAD, ~line 658-664 per architecture assessment though exact line has likely drifted) symlinks `{source_home}/.claude/.credentials.json` into each materialized worker's Claude home directory (`layout.claude_dir.join(".credentials.json")`) via `symlink_auth_file`. All workers sharing one `source_home` therefore share **one physical credentials file** — not a copy, a symlink to the same inode.
- This is named in the architecture assessment as directly corresponding to a known incident class: an OAuth token rotation or logout on one worker's session invalidates the shared file, and every other worker sharing that symlink loses its session simultaneously, mid-task, with no independent recovery path.
- `materialize_trust` (same file, adjacent function) has a related but distinct pattern for `.claude.json` trust state — copy-if-missing rather than symlink. This spec should determine whether trust-state sharing has the same failure class or is already safe by virtue of being a copy (design.md to confirm by reading the actual copy-vs-symlink semantics, not assumed from this note).

## Requirement P1: No Worker Independently Holds a Refreshable Credential

**Revised 2026-07-10 after a3 adversarial review** (`research/a3-adversarial-review-of-c-d-specs-2026-07-10.md` §五) found the original "give each worker its own copy of the file" mechanism does not achieve isolation in practice: OAuth refresh token rotation (RTR), standard on Auth0-family flows, means N independent copies of the *same starting token* will cascade-invalidate each other the first time any two workers' refresh windows overlap — copying the file does not copy independent *server-side* session lifetime. See design.md for the full mechanism and why a token-proxy architecture (a3's proposed fix, adopted) is required instead of a file-level change.

Acceptance criteria:
- No worker sandbox holds a real, independently-usable refresh token at any point after this change — workers authenticate through a host-side proxy that is the sole holder of the refreshable credential, not through a locally-materialized (copied or symlinked) `.credentials.json` carrying a real refresh token.
- A credential refresh triggered by worker A's activity does not affect worker B's ability to continue making authenticated requests — verified by a test that drives a refresh through the proxy from one simulated worker context and asserts a concurrent second worker context's requests are unaffected (this replaces the original file-mutation test, which tested the wrong layer — file independence was never the property that mattered, request-layer independence is).
- The proxy's refresh path is single-flight: concurrent requests from multiple workers that would each trigger a refresh result in exactly one real upstream refresh call, not N races.
- Initial provisioning (how the proxy itself obtains its one seed credential) is a design decision, stated in design.md — likely a single operator/seed-account login, materialized once for the proxy, never distributed to worker sandboxes.
- **Open question, not resolved by this requirement alone**: whether the Claude CLI supports being pointed at a local proxy/alternate endpoint is a load-bearing technical assumption this requirement depends on. Implementer must confirm this before implementation proceeds past a spike — if the CLI has no such mechanism, this requirement cannot be satisfied as written and must come back for a design revision (see design.md's "open implementation question" and fallback section).

Testability: `--lib` for the single-flight/isolation logic against a fake upstream token endpoint; the real CLI-proxy integration is CI-integration or manual verification, gated on the open question above being resolved first.

## Requirement P2: Rotation/Logout Failure Isolation

A credential failure (expired token, revoked session, explicit logout) on one worker is observable and recoverable independently of other workers' sessions.

Acceptance criteria:
- When worker A's credential becomes invalid, workers B-E's sessions continue unaffected (direct consequence of P1, but stated separately because it's the actual operational property being bought — P1 is the mechanism, P2 is the outcome).
- Worker A's credential failure is surfaced as an observable event (not a silent hang or an opaque provider-CLI error the operator has to dig for) — design.md should specify what "observable" means concretely (log line, `ah ps` sub_state, event stream entry) given whatever the codebase's existing failure-surfacing conventions are.

Testability: `--lib` for the "other workers unaffected" property; the observability surface may need a small integration check depending on where in the stack the failure is currently caught (or not caught) today — implementer should trace the current failure path before designing the fix, since "how does the CLI currently behave when credentials are invalid" hasn't been audited for this spec pass.
