# PR-3: Codex Bundle Adaptation Spec

Status: implementation-ready scope lock for a3. Base: `feat/plugin-bundle-pr2` at `f0d6d54`.

This PR changes codex bundle handling only. It must not implement antigravity bundle support, MCP changes, fingerprint shape changes, recovery changes, schema optionality, or unrelated cleanup.

## Inputs

- Final bundle design: `origin/docs/plugin-bundle-design` / `32ac26a`, `.kiro/specs/ah-plugin-bundle/design.md`, especially §3.1 codex column and §6 PR-3.
- a2 PR-3 recon conclusion, as reflected by PR-2 code: codex already has concrete writers for skills, hooks, hook-push, and worker rules; PR-3 should remove the non-MCP codex gate and route bundle content into those existing writers, not invent a second materialization path.
- PR-2 base: `feat/plugin-bundle-pr2` / `f0d6d54`.
  - `src/provider/bundles.rs` has per-content provider capability gating, `BundleDigest`, bundle resolution, and bundle contribution merge into `ExtensionConfig`.
  - `src/provider/extensions.rs::ExtensionConfig` already carries `bundle`, `rules`, `mcp`, `bundle_digest`, and `resolved_skills`.
  - `src/provider/home_layout.rs` already has codex writers: `materialize_codex_skills`, `materialize_hooks`, `enable_codex_hooks`, `merge_codex_hook_push`, and `materialize_builtin_rules`.

## Hard Decisions

These are not open questions:

- Codex worker bundle `skills`, `hooks`, and `rules` change from "unsupported -> Q8 error" to "supported -> materialize".
- Codex master bundle rules return a direct hard configuration error. Keep the existing `materialize_builtin_rules` master early return for non-claude providers. v1 master is always claude, so this route is practically unreachable except direct tests. Do not add optional schema or optional downgrade support for this. YAGNI.
- Do not change fingerprint structure. PR-1/PR-2 `BundleDigest` already covers bundle `skills`, `hooks`, `rules`, and MCP manifest content.
- Do not touch MCP. PR-2 already implemented codex MCP writer and MCP gate behavior.
- Do not touch antigravity. PR-4 owns antigravity bundle adaptation.

## Implementation Scope

### 1. `src/provider/bundles.rs`

Change only `validate_bundle_capabilities`.

Required behavior:

- `provider == "claude"` remains fully accepted.
- `provider == "codex" && role == BundleRole::Worker` accepts:
  - `contribution.skills`
  - `contribution.hooks`
  - `contribution.rules`
  - `contribution.mcp` remains governed by existing MCP writer/provider checks; do not special-case it here.
- `provider == "codex" && role == BundleRole::Master && !contribution.rules.is_empty()` returns `CcbdError::EnvironmentNotSupported` with a direct message containing all of:
  - `codex`
  - `master`
  - `bundle rules`
  - `unsupported`
- `provider == "codex" && role == BundleRole::Master` may continue to accept skills/hooks only if reachable through tests, but must not materialize master rules. The only mandated hard error is master bundle rules.
- `provider == "antigravity"` remains PR-2 behavior for non-MCP bundle content: skills/hooks/rules are still rejected until PR-4.
- Unknown providers remain rejected for non-MCP content as today.

Do not add manifest optional handling. Do not change `BundleManifest`, `BundleRulesManifest`, `McpServerConfig`, or `ExtensionConfig`.

### 2. `src/provider/home_layout.rs`

Change only codex bundle materialization. Reuse existing functions.

#### `prepare_managed_codex_home`

Current PR-2 flow calls:

```rust
materialize_builtin_rules(role, "codex", home_root, project_root, slot_id, &[])?;
...
let skills = resolve_skills(project_root, &extensions.skills)?;
materialize_codex_skills(codex_home, &skills)?;
...
materialize_codex_mcp(&target_config, &extensions.mcp)?;
if let Some(ctx) = active_hook_push_ctx(hook_push_ctx, "codex") { ... }
```

Required changes:

1. Pass bundle worker rule layers into the existing writer:

```rust
materialize_builtin_rules(
    role,
    "codex",
    home_root,
    project_root,
    slot_id,
    &extensions.rules,
)?;
```

This yields `.codex/AGENTS.md` order:

```text
worker kernel
---
bundle worker rules, in bundle array order
---
project .ah/rules/<slot_id>.md, or default worker rules
```

2. Include resolved bundle skills with project skills:

```rust
let mut skills = resolve_skills(project_root, &extensions.skills)?;
skills.extend(extensions.resolved_skills.iter().cloned());
materialize_codex_skills(codex_home, &skills)?;
```

Target must remain `$CODEX_HOME/skills/<name>`, which in this repo is `home_root/.codex/skills/<name>`. The target must be a symlink to `.ah/bundles/<bundle>/skills/<name>`.

3. Materialize codex bundle hooks using the existing generic hook script symlinker:

```rust
let mut hook_specs = materialize_hooks(source_home, &codex_home.join("hooks"), &extensions.hooks)?;
```

Then merge those specs into `.codex/hooks.json` in codex shape:

```json
{
  "hooks": {
    "<event>": [
      {
        "matcher": "...",
        "hooks": [
          {"type":"command","command":".../.codex/hooks/<script>","timeout":5}
        ]
      }
    ]
  }
}
```

Use a small codex-specific merge helper if needed, but it must consume the existing `MaterializedHook` values returned by `materialize_hooks`; do not resolve hook paths a second way.

4. Preserve existing host `.codex/hooks.json`:

- If source `$HOME/.codex/hooks.json` exists and target `.codex/hooks.json` does not, copy it before merging bundle hooks and hook-push.
- Existing non-ah-owned hook groups must remain.
- Bundle hooks must append without deleting existing non-ah hooks.

5. Enable codex hooks whenever either bundle hooks exist or hook-push is active:

```rust
if !hook_specs.is_empty() || active_hook_push_ctx(...).is_some() {
    enable_codex_hooks(&target_config)?;
}
```

This writes `[features].hooks = true` in `.codex/config.toml` and removes legacy `codex_hooks`, preserving existing config.

6. Keep hook-push behavior exactly as today:

- If `active_hook_push_ctx(hook_push_ctx, "codex")` exists, call `merge_codex_hook_push(codex_home, ctx)`.
- `merge_codex_hook_push` must remain idempotent: repeated materialization leaves exactly one ah-owned Stop hook.
- Hook-push must coexist with bundle hooks and with copied host hooks.

7. Keep MCP untouched:

```rust
materialize_codex_mcp(&target_config, &extensions.mcp)?;
```

Do not reorder MCP in a way that changes PR-2 behavior unless required by codex config TOML parsing. No MCP test expectations change in PR-3.

#### `materialize_builtin_rules`

Do not change its non-claude master early return:

```rust
if role == HomeLayoutRole::Master && provider != "claude" {
    return Ok(());
}
```

Codex master bundle rules are rejected earlier in `validate_bundle_capabilities`; this function remains a defensive no-op for non-claude masters.

#### New helper allowance

A helper such as `merge_codex_hooks(codex_home: &Path, hooks: &[MaterializedHook])` is allowed if it is private to `home_layout.rs` and only performs JSON merge into `.codex/hooks.json`.

Do not add public API unless tests cannot compile otherwise. The acceptance tests should use existing public entry points:

- `resolve_bundles_for_provider`
- `prepare_home_layout_with_extensions_for_slot`

## Acceptance Tests

PR-3 must make `tests/pr3_codex_bundle.rs` pass. These tests define the implementation boundary:

- Codex bundle skill: `.ah/bundles/x/skills/s/SKILL.md` symlinks to `.codex/skills/s`.
- Codex bundle hooks:
  - bundle hook script symlinks into `.codex/hooks/`
  - bundle hook group appears in `.codex/hooks.json`
  - copied host hooks are preserved
  - `.codex/config.toml` contains `[features].hooks = true`
  - hook-push remains idempotent and coexists with bundle hooks.
- Codex worker rules: `.codex/AGENTS.md` order is kernel -> bundle worker rules -> project slot rules.
- Codex master bundle rules: direct hard configuration error.
- Zero regression without bundle:
  - codex home layout still sets `CODEX_HOME` and creates `.codex/config.toml`
  - codex project skills v1 still symlink
  - codex hook-push remains idempotent
  - empty bundle fingerprint hash remains unchanged.

## Out Of Scope

Hard red lines for a3:

- No antigravity bundle adaptation.
- No MCP schema/writer/gate changes.
- No fingerprint structure changes.
- No `BundleDigest` redesign.
- No recovery, realign, master revive, or daemon restart logic.
- No optional bundle schema, optional rules downgrade, or Q8 expansion.
- No plugin-in-bundle work.
- No config format changes beyond existing PR-2 `bundle` field behavior.
- No drive-by refactors, renames, formatting churn, dependency changes, or unrelated test rewrites.

## Review Checklist

- `git diff -- src/provider/bundles.rs src/provider/home_layout.rs tests/pr3_codex_bundle.rs` should be the whole PR-3 implementation diff, except harmless line-number drift.
- No files under antigravity-specific logic changed except untouched context.
- No changes to `src/provider/fingerprint.rs`.
- No changes to `src/provider/extensions.rs`.
- No changes to `src/provider/manifest.rs`, DB schema, recovery, realign, or orchestrator modules.
- `cargo test --test pr3_codex_bundle` passes after implementation.
- Existing PR-2 bundle/MCP tests still pass.
