# ah Per-Worker Credentials — Design

> [!IMPORTANT]
> **2026-07-12 晚设计收敛: 本文件顶部为当前冻结设计。**
> 下方 "Design Thesis, Revised" 及之后的 Plan B Gateway 内容保留为历史记录,不再作为实施依据。

Status: current implementation draft, 2026-07-12 晚, based on `requirements.md` 最新三个块:
非静默变更 / drvfs 真机 spike 结果 / 作用域隔离验证。

## Final Mechanism: Shared Claude Secure Storage Direct-Dir

Frozen direction:

1. ah 对每个 Claude 席位继续使用每沙箱 Claude config:
   `CLAUDE_CONFIG_DIR=<sandbox-home>/.claude`。
   该目录仍承载 settings、角色规则、session/todos、trust、MCP 配置等隔离状态。
2. ah 对每个 Claude 席位新增注入:
   `CLAUDE_SECURESTORAGE_CONFIG_DIR=<shared-credentials-dir>`。
   该变量只改变 Claude OAuth/secure-storage 的 `.credentials.json` 存储目录。
3. `<shared-credentials-dir>` 必须是用户平时登录的 Claude 凭据真目录,例如 Windows 用户侧 `.claude` 目录在 WSL 中的 drvfs 路径。设计不硬编码任何 `/mnt/c/...`。
4. `<shared-credentials-dir>` 必须是 direct-dir: 注入值本身是真实存在目录,不是 `.credentials.json` 文件级 symlink,也不是空串。drvfs 真机 spike 已证明 Claude 的 tmp+rename 会把文件级 symlink 替换成普通文件,写不穿 SSOT。
5. 不再给 worker 注入 fake Gateway / dummy token 环境变量。Claude 自身在 `CLAUDE_SECURESTORAGE_CONFIG_DIR` 指向同一真目录时通过 mtime 重读、竞态保护、`.storage-write` 文件锁和 tmp+rename 协调刷新。

This design treats the latest binary reverse-engineering facts in `requirements.md` as premises, not as options to re-debate.

## Config Schema

Add provider-level config to `ah.toml`:

```toml
[providers.claude]
shared_credentials_dir = "/absolute/path/to/user/.claude"
```

Semantics:

- `shared_credentials_dir` is optional for non-Claude projects and for deployments that do not opt into shared host credentials yet.
- When any configured master or agent uses provider `claude`, enabling this spec requires `shared_credentials_dir` to be set.
- The value must be a non-empty absolute path. Relative paths are rejected to avoid silently resolving to the project directory or a sandbox.
- Spawn/materialization code must fail closed before launching Claude if the path does not exist, is not a directory, or the final path component itself is a symlink.
- The injected value is the configured directory itself, not the `.credentials.json` file path.
- `CLAUDE_SECURESTORAGE_CONFIG_DIR=""` must never be emitted. Empty or whitespace-only config is a validation error. Missing config means "do not inject", not "inject empty".

Suggested Rust shape:

```rust
pub struct ProjectConfig {
    // existing fields...
    #[serde(default)]
    pub providers: ProviderConfigs,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ProviderConfigs {
    #[serde(default)]
    pub claude: ClaudeProviderConfig,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ClaudeProviderConfig {
    #[serde(default)]
    pub shared_credentials_dir: Option<PathBuf>,
}
```

Do not put this under `[agents.<id>.settings]`: this is provider process wiring, not Claude settings.json content, and it must apply consistently to master plus all Claude workers.

## Implementation Change List

### `src/cli/config.rs`

Add:

- `ProjectConfig.providers`.
- `ProviderConfigs`.
- `ClaudeProviderConfig.shared_credentials_dir`.
- Validation for non-empty absolute paths when present.
- Validation that Claude provider usage requires the field once this spec is enabled.

Reason: the shared credentials path is a per-provider deployment setting. Hardcoding any machine-specific path would break the operator requirement and make dogfood-only state leak into product code.

### Start/realign/session parameter plumbing

Modify the config-to-RPC path that currently carries master/agent provider, env, settings, hooks, plugins, skills and bundle:

- `src/cli/start.rs`
- `src/cli/up.rs`
- `src/rpc/handlers/sessions.rs`
- `src/rpc/handlers/realign.rs`
- `src/rpc/handlers/agent.rs`

Add a Claude provider config payload, or another typed field with the same data, so `prepare_home_layout_with_extensions_for_slot` can receive `shared_credentials_dir` without reading `ah.toml` globally.

**These two RPC handlers are also active Gateway trigger sites, not just config plumbing** — the primary (non-revive) master and worker spawn paths inject and activate the Gateway independently of `home_layout.rs`, so they must be torn down here too:

- `src/rpc/handlers/sessions.rs` — `build_master_spawn_env_vars(...)` (call at `:461`, body at `:520`) inserts `CLAUDE_CODE_USE_GATEWAY=1` and `ANTHROPIC_AUTH_TOKEN=fake_worker_jwt(session_id)`. This survives even after the `home_layout.rs` env builder is fixed, because the master plan later does `master_env_vars.extend(home_overrides.extra_env)` — an `extend` cannot unset a key the home layout no longer emits. Remove both inserts from `build_master_spawn_env_vars`.
- `src/rpc/handlers/sessions.rs` — the master plan block (`~:490-508`) calls `ctx.claude_gateway.register_master(...)` and injects `AH_CLAUDE_GATEWAY_HOST_UDS` + `GATEWAY_SANDBOX_ROOT_ENV`, then pushes the gateway UDS `ReadWriteBind`. Stop registering and stop mounting the UDS for the direct-dir mechanism.
- `src/rpc/handlers/agent.rs` — the worker spawn block (`~:185-203`, guarded by `manifest.provider_name == "claude"`) calls `ctx.claude_gateway.register_worker(...)`, injects `AH_CLAUDE_GATEWAY_HOST_UDS` + `GATEWAY_SANDBOX_ROOT_ENV`, and pushes the gateway UDS bind. Same teardown. (The worker's `CLAUDE_CODE_USE_GATEWAY`/`fake_worker_jwt` come from `home_layout.rs` and are removed there; only the register + UDS bind live in this handler.)

Reason: the spawn side is where ah knows both the sandbox home and the provider. It must inject the secure-storage env together with `CLAUDE_CONFIG_DIR` for every Claude master/worker spawn, including realign and revive paths. If the Gateway register/UDS/flag/token teardown above is skipped, every fresh master and worker still boots into the Gateway that `requirements.md` §11 records as never having activated a single Claude (current_exe + claude 秒死) — the direct-dir mechanism would be dead-on-arrival on the two primary spawn paths, leaving only the revive path corrected.

### `src/provider/home_layout.rs`

Change:

- `prepare_home_layout_with_extensions_for_slot(...)` and `prepare_claude_overrides(...)` should accept the Claude shared credentials config.
- Replace `claude_gateway_home_env(home_root, slot_id)` with a Claude env builder that emits:
  - `HOME=<sandbox-home>`
  - `CLAUDE_CONFIG_DIR=<sandbox-home>/.claude`
  - `CLAUDE_SECURESTORAGE_CONFIG_DIR=<validated-shared-credentials-dir>`
- The new env builder must reject missing, empty, non-directory, or symlink paths before env insertion.

Delete or stop using:

- `claude_gateway_home_env(home_root, slot_id)` at `src/provider/home_layout.rs:1753`.
- Its `CLAUDE_CODE_USE_GATEWAY` insertion.
- Its `ANTHROPIC_AUTH_TOKEN = fake_worker_jwt(slot_id)` insertion.

Delete for Claude credentials:

- `materialize_auth_file_with_ladder(...)` use for `.claude/.credentials.json`.
- `link_auth_file_into_sandbox(...)` branch for `.claude/.credentials.json`.
- `symlink_auth_file_checked(...)` and related `same_resolved_path(...)` usage if no other provider auth path still needs symlink semantics.

Keep:

- `CLAUDE_CONFIG_DIR=<sandbox-home>/.claude`.
- `materialize_trust(...)`, settings, hooks, rules, MCP, skills and plugin materialization under each sandbox `.claude`.
- Non-Claude auth materialization for `.codex/...` and `.gemini/...`.

Reason: Claude credentials are no longer materialized into sandbox homes at all. The sandbox `.claude` directory remains isolated config; secure storage points at the shared true directory.

### `src/master_revival.rs`

Change:

- `warn_if_master_auth_missing(...)` currently warns about `.claude/.credentials.json`. Remove the credentials-file branch and keep only checks relevant to the sandbox `.claude` config directory.

Reason: after this design, absence of sandbox `.claude/.credentials.json` is expected and healthy. Revive must not treat it as a missing relink condition.

### `src/monitor/master_watch.rs`

Change:

- In revive env reconstruction around `src/monitor/master_watch.rs:794`, stop checking `home_root/.claude/.credentials.json`.
- Keep injecting `HOME` and `CLAUDE_CONFIG_DIR`.
- Add validated `CLAUDE_SECURESTORAGE_CONFIG_DIR` for revived Claude master.
- Remove revive-time `CLAUDE_CODE_USE_GATEWAY` and `ANTHROPIC_AUTH_TOKEN` injection.
- Remove Gateway topology bind setup from the Claude credential path. If the frozen `claude_gateway.rs` module remains in tree for history or another branch, this revive path must not activate it.

Reason: master revive must faithfully restart with isolated sandbox config plus shared secure storage. Requiring an auth symlink during revive preserves the old ah#18 failure mode.

### Gateway / dummy-token trigger points

Do not delete frozen branch files such as `src/claude_gateway.rs` as part of this design task unless a separate cleanup task authorizes it.

Remove the active trigger surface at **all three** Claude spawn paths (not only home layout + revive):

- Provider home layout (`src/provider/home_layout.rs`, `prepare_claude_overrides` → `claude_gateway_home_env`): no `CLAUDE_CODE_USE_GATEWAY=1`, no dummy `ANTHROPIC_AUTH_TOKEN`. Covers both master and worker home env.
- Master initial spawn (`src/rpc/handlers/sessions.rs`): no `CLAUDE_CODE_USE_GATEWAY=1`/`fake_worker_jwt` from `build_master_spawn_env_vars`; no `register_master` + `AH_CLAUDE_GATEWAY_HOST_UDS`/`GATEWAY_SANDBOX_ROOT_ENV` + UDS bind. (See the plumbing section above.)
- Worker spawn (`src/rpc/handlers/agent.rs`): no `register_worker` + `AH_CLAUDE_GATEWAY_HOST_UDS`/`GATEWAY_SANDBOX_ROOT_ENV` + UDS bind.
- Master revive (`src/monitor/master_watch.rs`): no revive-time Gateway/dummy env injection (covered in the `master_watch.rs` section).
- Platform bridge code that only activates when `CLAUDE_CODE_USE_GATEWAY=1` may remain dormant until a cleanup task removes the module. This is why removing the four *injection* sites above is load-bearing: `src/platform/linux/scope.rs` still reads `CLAUDE_CODE_USE_GATEWAY`/`GATEWAY_SANDBOX_ROOT_ENV` and will wrap the launch into the dead Gateway whenever any spawn path leaves the flag set.

Reason: the operator direction is to withdraw the three-layer neuter/dummy RT/Gateway path from active Claude launches, while avoiding unrelated churn in frozen Gateway code.

## Safety Checks

The implementation must defend the empty-string trap explicitly:

```rust
fn validate_claude_shared_credentials_dir(path: &Path) -> Result<PathBuf, CcbdError> {
    if path.as_os_str().is_empty() {
        return Err(...);
    }
    if !path.is_absolute() {
        return Err(...);
    }
    let meta = std::fs::symlink_metadata(path)?;
    if meta.file_type().is_symlink() || !meta.is_dir() {
        return Err(...);
    }
    Ok(path.to_path_buf())
}
```

The env builder must insert `CLAUDE_SECURESTORAGE_CONFIG_DIR` only after this validation passes. It must not use `unwrap_or_default`, `unwrap_or("")`, TOML default empty strings, or any lossy conversion that can turn absence into `""`.

## Acceptance and Test Plan

### `--lib` testable

Add focused tests only; do not require full local suite.

Config tests in `src/cli/config.rs`:

- Parses `[providers.claude] shared_credentials_dir = "/tmp/user/.claude"`.
- Rejects `shared_credentials_dir = ""`.
- Rejects relative paths.
- Rejects Claude master/agent config when shared credentials are required but missing.
- Does not require the field for non-Claude-only configs.

Home layout tests in `src/provider/home_layout.rs`:

- Claude env contains `HOME`, `CLAUDE_CONFIG_DIR`, and non-empty `CLAUDE_SECURESTORAGE_CONFIG_DIR`.
- Claude env does not contain `CLAUDE_CODE_USE_GATEWAY` or `ANTHROPIC_AUTH_TOKEN`.
- `CLAUDE_CONFIG_DIR` remains `<sandbox-home>/.claude` while `CLAUDE_SECURESTORAGE_CONFIG_DIR` points at the shared dir.
- Preparing Claude layout does not create or symlink `<sandbox-home>/.claude/.credentials.json`.
- Validation rejects a symlink path used as `shared_credentials_dir`.

Revive tests in `src/master_revival.rs` / `src/monitor/master_watch.rs`:

- Revived master env includes `CLAUDE_CONFIG_DIR` and `CLAUDE_SECURESTORAGE_CONFIG_DIR`.
- Revived master env excludes Gateway/dummy token env.
- Revive no longer warns or branches on missing sandbox `.claude/.credentials.json`.
- Existing CLAUDE_CONFIG_DIR readiness/trust checks remain intact.

Command budget for implementers on this repo:

```bash
timeout 300 env CARGO_BUILD_JOBS=1 cargo check --lib
timeout 300 env CARGO_BUILD_JOBS=1 cargo test <single_test_name> --lib -- --test-threads=1 --exact
```

Do not run local full `cargo test` or full builds; CI/operator provides broader evidence.

### Tier-3 true-machine acceptance

These cannot be proven by `--lib` and must be verified on the user Win11/WSL2 setup:

1. User logs in once with their normal Claude CLI; ah Claude worker starts to IDLE using the same login.
2. A worker refresh writes the new refresh token in place into the Windows-side true `.credentials.json`.
3. A second worker and the user's host Claude remain authenticated after that refresh.
4. WSL worker and Windows host Claude do not break each other during near-concurrent refresh; if the cross-OS lock is not mutually recognized, Claude's mtime/reload/竞态 protection still prevents logout.
5. drvfs chmod behavior remains non-fatal: ignored `0600` is acceptable if Claude write succeeds.

This tier-3 run is also where plaintext-vs-Cred-Manager behavior is confirmed for the user's Windows Claude install. Recent incidents strongly imply plaintext `.credentials.json`, but this design records that as a true-machine confirmation item, not a `--lib` property.

---

## Historical Gateway Draft Below

The following sections are retained only to preserve the design history that led to the current direct-dir mechanism. They are superseded by the 2026-07-12 晚 design above.

Status: superseded by [design-rev.md](file:///home/sevenx/coding/ccbd-rust/.kiro/specs/ah-per-worker-credentials/design-rev.md), 2026-07-10.

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
