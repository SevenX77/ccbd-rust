# ah Master Provider Parameterization — Requirements

Status: **requirements draft by d1, not implementation-cleared**. This spec touches master spawn home layout, hook injection, bundle resolution, and credential materialization. It requires operator gate before `design.md` and again before implementation.

Process state:
- Draft authoring: complete in this file.
- o1 red-team/debate: **completed by master handoff, incorporated 2026-07-13**.
- Freeze state: **ready for operator gate after this revision**; no `design.md` or implementation is authorized by this file.

## Ground Truth

Source material: `research/architecture-index.md` Layer 6, "Capability holdout — master provider is CLAUDE-ONLY at the spawn site" (owner `rpc::handlers::sessions::prepare_master_pane_plan`, obs #57/#58, dated 2026-07-13).

Verified facts to carry forward, not re-litigate:
- `master.provider` is honored by surrounding modules: `cli::config`, `cli::bundle` (`src/cli/bundle.rs:165`), `rpc::handlers::master_cutover` readiness Ack/Probe, and `tests/builtin_skills.rs` provider coverage.
- The load-bearing master spawn scaffold ignores that field. `rpc::handlers::sessions::prepare_master_pane_plan` hardcodes `"claude"` for bundle/home/hook/credential materialization (`src/rpc/handlers/sessions.rs:428` prepares the plan; current hardcoded calls are in that body).
- `SpawnMasterPaneParams` has no `provider` field today (`src/rpc/handlers/sessions.rs:341`), unlike `agent.spawn`, which threads `manifest.provider_name` dynamically into the same materialization path (`src/rpc/handlers/agent.rs:177`).
- `git log -S 'master.provider'` already confirmed this was never wired into `prepare_home_layout*` / `resolve_bundles_for_provider`; it is unfinished wiring, not a regression.
- `provider::home_layout` (`src/provider/home_layout.rs`, 3039L per architecture index) owns `prepare_home_layout(_with_role/_with_extensions/_with_extensions_for_slot)` and `prepare_claude_home_layout_with_gateway`; it is daemon-critical and called from `rpc/handlers/{agent,sessions}.rs`.
- `provider::manifest` (`src/provider/manifest.rs`, 849L per architecture index) owns `ProviderManifest`, `VALID_PROVIDER_NAMES`, `canonicalize_provider_name`, and `collect_spawn_env`; this is the dynamic provider pattern master spawn must mirror.
- `claude_gateway` is a top-level module (`src/claude_gateway.rs`, 1129L per architecture index), not `provider::claude_gateway`; it is Claude-specific OAuth credential gateway/bridge code. A Codex or other-provider master MUST NOT be routed through Claude gateway credential handling.
- o1 red-team evidence, incorporated 2026-07-13: Codex already has a file-based auth/home contract. `src/provider/manifest.rs` declares Codex `auth_mount_paths: vec![".codex", ".config/gcloud"]`; `src/provider/home_layout.rs` whitelists `.codex/auth.json` and `.codex/installation_id` in `PROVIDER_AUTH_WHITELIST`, `prepare_codex_overrides` sets `CODEX_HOME`, and missing auth files warn rather than crashing.
- o1 red-team evidence, incorporated 2026-07-13: master cutover conversation seeding is Claude-hardcoded. `src/rpc/handlers/master_cutover.rs` calls `seed_claude_project_conversation(...)`, while Codex session logs live under `.codex/sessions` per `src/completion/log_layout.rs`.
- o1 red-team evidence, incorporated 2026-07-13: master role rules and prompt hooks are not automatically fixed by generic provider parameterization. `src/provider/home_layout.rs` bypasses built-in rule composition for `HomeLayoutRole::Master && provider != "claude"`, and Claude's `prepare_claude_overrides` injects `UserPromptSubmit` while Codex's override path does not.

Current user-visible failure:
- `[master] provider = "codex"` can pass surrounding validation signals, but `session.spawn_master_pane` still prepares a Claude sandbox and can fail with opaque `ENVIRONMENT_NOT_SUPPORTED`, especially when `[providers.claude]` is absent and the fail-closed shared-credentials check trips.
- Config-time bundle validation can check Codex rules while runtime materializes Claude rules. That is an internal consistency bug, not only a UX warning problem.

## Scope

In scope:
- Make master spawn provider-parametric for every `VALID_PROVIDER_NAMES` entry that is actually spawn-supported by the provider manifest.
- Thread the selected master provider through session spawn and master cutover paths.
- Dispatch master bundle, home, hook, and credential materialization by provider, using the existing `agent.spawn` pattern as the reference.
- Remove silent false-green behavior from `config validate` while the parameterization is absent or incomplete.
- Add an end-to-end lock test that exercises the real non-Claude master spawn path, not only config/bundle validation.

Out of scope:
- Adding a new provider name or changing `VALID_PROVIDER_NAMES`. Owner: provider subsystem; backlog registration: `requirements.md` backlog item B1 below.
- Redesigning Claude OAuth itself. This spec may route Claude through existing Claude credential mechanisms, but it does not change gateway token semantics. Owner: per-worker credential/gateway owners; backlog registration: B2 if new auth semantics are discovered.
- Making all providers share identical credential behavior. This is explicitly rejected: credential materialization is provider-conditional.
- Implementing `design.md`, source edits, or tests during this requirements pass.

## Requirements

### MPP-R1: Master Provider Config Must Produce a Real Matching Master Spawn

An operator MUST be able to configure `[master] provider = "codex"` or any other valid, spawn-supported `VALID_PROVIDER_NAMES` entry and get a master pane whose runtime materialization matches that provider. Passing config validation is insufficient.

Acceptance criteria:
- For `master.provider = "codex"`, the spawned master uses Codex home layout, Codex bundle rules, Codex hook settings, Codex command/env collection, and Codex credential path.
- For `master.provider = "claude"`, existing Claude master behavior remains supported, including Claude-specific credential/gateway behavior where applicable.
- Unsupported provider names fail at config validation with a clear provider-name diagnostic via the existing canonicalization/validation path, not at late spawn.
- A provider that is valid but not master-spawn-supported must fail or warn according to MPP-R5; it must not silently pass validation and then materialize Claude.
- Provider/cmd disagreement is a validation error, not an auto-derive. If `[master] provider = "codex"` leaves a Claude default command or otherwise selects a command known to belong to another provider, validation must fail with "master.provider/master.cmd mismatch" and instructions to set the provider-matching command. The implementation must not infer provider from `cmd`, and must not silently rewrite `cmd`.

### MPP-R2: `SpawnMasterPaneParams` Carries Provider Identity End-to-End

`SpawnMasterPaneParams` MUST gain a `provider` field. Every caller that creates master spawn params MUST source it from `[master] provider`, after existing config canonicalization, rather than hardcoding `"claude"`.

Acceptance criteria:
- `handle_session_spawn_master_pane` accepts and validates a provider value on the RPC contract or derives it from the already-loaded master config if the RPC request is config-backed. The source of truth must be explicit in `design.md`.
- `start.rs` session/master startup path populates `SpawnMasterPaneParams.provider` from `master.provider`; default remains the canonical current default (`claude`) only when config omitted it.
- Master cutover path (`master_cutover.rs` / `rpc/handlers/master_cutover.rs`) populates `SpawnMasterPaneParams.provider` from `request.master.provider`, not by implication from `cmd` or readiness mode.
- Tests that construct `SpawnMasterPaneParams` must specify provider explicitly, so new tests cannot accidentally inherit Claude.

### MPP-R3: Master Home, Bundle, and Hook Materialization Dispatch Per Provider

`prepare_master_pane_plan` MUST stop using hardcoded `"claude"` for bundle resolution, hook push context, and home materialization. It must mirror the provider-parametric pattern already used by `agent.spawn` (`src/rpc/handlers/agent.rs:177` threads `manifest.provider_name` into `prepare_home_layout_with_extensions_for_slot_and_claude_credentials`).

Acceptance criteria:
- `resolve_bundles_for_provider` receives `params.provider`, not `"claude"`.
- `HookPushContext.provider` receives `params.provider`, not `"claude"`.
- `prepare_home_layout*` receives `params.provider`, not `"claude"`.
- Any provider-specific environment scrubbing remains provider-conditional. Today `strip_claude_gateway_env` is Claude-specific and must not be applied blindly to every provider unless design proves it is safe and intended.
- The runtime provider used for materialization is the same provider used by config-time bundle validation.

### MPP-R4: Credential Materialization Is Provider-Conditional

Master credential materialization MUST be explicitly dispatched by provider. Claude OAuth gateway/bridge materialization is Claude-specific and MUST NOT be force-applied to Codex or other providers.

Acceptance criteria:
- Claude master path may use the existing Claude-specific credential inputs, including `claude_shared_credentials_dir`, only under `provider == "claude"`.
- Codex master path MUST reuse the existing Codex file-based credential/home contract:
  - provider manifest auth mounts include `.codex` and `.config/gcloud` (`src/provider/manifest.rs`);
  - sandbox auth linking is constrained by `PROVIDER_AUTH_WHITELIST`, including `.codex/auth.json` and `.codex/installation_id` (`src/provider/home_layout.rs`);
  - `prepare_codex_overrides` prepares the `.codex` home and sets `CODEX_HOME`;
  - missing `.codex/auth.json` / `.codex/installation_id` remains graceful: warn and continue according to the existing whitelist-linking behavior, not crash as a Claude credential failure.
- For every provider in `VALID_PROVIDER_NAMES`, design.md must state the credential path: materialized file/dir, inherited host env, provider CLI default, gateway/bridge, or unsupported-with-diagnostic.
- No non-Claude master spawn may receive `CLAUDE_CODE_USE_GATEWAY`, `ANTHROPIC_AUTH_TOKEN` fake-worker bridge values, `AH_CLAUDE_GATEWAY_HOST_UDS`, or `GATEWAY_SANDBOX_ROOT_ENV` as a side effect of master materialization unless the operator explicitly supplied unrelated env and design allows it.
- Failure mode must be provider-named. A Codex credential failure must not surface as missing Claude shared credentials or Claude `ENVIRONMENT_NOT_SUPPORTED`.

### MPP-R5: `config validate` Must Stop Silent False-Greens

Until full parameterization is implemented, `config validate` MUST NOT silently accept `master.provider != "claude"` as if it were end-to-end spawn-supported.

Requirement decision:
- In the full fix, validation should check the same provider support matrix used by spawn and should pass non-Claude master providers only when the spawn path is actually parameterized.
- As a preceding quick patch, if implementation is split, validation MUST emit a warning for `master.provider != "claude"` stating that current master spawn is Claude-pinned until this spec lands. If the config uses features known to fail hard, validation may remain an error, but the diagnostic must name the spawn-path limitation.

Acceptance criteria:
- `config validate` output distinguishes "valid provider name" from "master spawn supported by this build".
- A Codex master config cannot receive an all-green validation result on a build that would still hardcode Claude at spawn.
- The warning/error text points to master spawn provider parameterization, not generic bundle or credential failure.
- The full implementation MUST relax or rewrite the current hardcoded `validate_project_config` checks that reject master bundles/settings whenever `master.provider != "claude"` (`src/cli/config.rs` diagnostic text: "master uses bundle but PR-1 supports bundles only for provider claude"). Adding a separate warning is not enough; Codex master bundles/settings must be validated against Codex rules once runtime spawn actually uses Codex materialization.
- Once MPP-R1 through MPP-R4 land, the temporary warning is removed or narrowed to genuinely unsupported providers.

### MPP-R6: End-to-End Lock Test Covers Real Non-Claude Master Spawn

The implementation MUST add an end-to-end lock test that drives a non-Claude master through the real spawn-preparation path and asserts provider-specific materialization. Config-level or bundle-only tests do not satisfy this requirement.

Acceptance criteria:
- The test uses `master.provider = "codex"` and a dummy command such as `sleep` or a small shell script in `params.cmd`, following the existing pattern of testing master home layout without requiring a real provider binary or OAuth login.
- The test calls the real master spawn plan/materialization path (`prepare_master_pane_plan` or a higher-level `session.spawn_master_pane` path), not a mocked config validator only.
- Assertions prove Codex-specific home/bundle/hook materialization occurred, including `.codex` home/config structure and the whitelist-linked auth paths when source files exist.
- Assertions prove Claude-specific artifacts did not leak in: no `.claude` credential materialization as the provider home, no Claude gateway bridge env vars, and no missing-Claude-shared-credentials failure.
- The test asserts the MPP-R4 credential contract concretely: `.codex/auth.json` and `.codex/installation_id` are linked/materialized when present, and absence of those files is a logged warning/graceful path rather than a crash.
- The test would fail on current HEAD because current master spawn hardcodes Claude.

### MPP-R7: Diagnostics Must Be Provider-Named and Operator-Actionable

Failures in master spawn parameterization MUST name the selected provider, the failed materialization phase, and the missing support condition.

Acceptance criteria:
- Bundle-resolution failure reports provider + role (`master`) + bundle/extension source.
- Home-layout failure reports provider + sandbox slot (`master`) without falling through to Claude wording for non-Claude providers.
- Credential failure reports provider + credential mechanism (`claude_gateway`, Codex auth contract, inherited env, etc.).
- No accepted path emits opaque `ENVIRONMENT_NOT_SUPPORTED` for the known Codex-master false-green case without a preceding provider-specific reason.

### MPP-R8: Provider Support Matrix Is a Contract, Not Inference from `cmd`

The master provider MUST be determined from config/manifest identity, not inferred from the command string or readiness probe mode.

Acceptance criteria:
- Readiness Ack/Probe may remain provider-aware, but it is not the source of truth for home/bundle/hook/credential materialization.
- `cmd` can be overridden without silently changing provider materialization.
- The chosen provider is visible in trace/log context for `prepare_master_pane_plan` and cutover spawn.

### MPP-R9: Master Cutover Conversation Seeding Is Provider-Gated

Master cutover currently seeds handoff/history with `seed_claude_project_conversation`, which only knows Claude's `.claude/projects/...` layout. This spec MUST prevent silent history loss when the old or new master provider is Codex.

Requirement decision:
- This requirements pass chooses **explicit reject/gate of cross-provider cutover** as the hard acceptance requirement for the first implementation, rather than requiring provider-parametric conversation translation now. Reason: safe provider-parametric history translation is a separate data-model problem; a lossy or guessed converter is worse than a clear refusal in this high-risk spawn/credential spec.
- Same-provider non-Claude cutover may be supported only if design proves a provider-native seeding path exists. For Codex that means using `.codex/sessions` semantics from `src/completion/log_layout.rs`, not writing a Claude project conversation and hoping Codex reads it.

Acceptance criteria:
- Claude-to-Claude cutover preserves the existing Claude seeding behavior.
- Claude-to-Codex, Codex-to-Claude, and Codex-to-Codex cutover must either use a proven provider-native seeding path for the target provider or fail before spawn with a clear diagnostic naming old provider, new provider, and unsupported conversation seeding.
- No path may write Claude `.claude/projects` history into a Codex master home as the only seeding mechanism.
- No path may silently fall back to generic/Fallback state for Codex history without an operator-visible diagnostic.
- Provider-parametric conversation seeding is registered as backlog item B6 unless implemented in this spec.

### MPP-R10: Non-Claude Master Rules and Prompt Hooks Are Explicitly In Scope

Generic provider parameterization is not enough for master role parity because the current home layout code has master-specific bypasses for non-Claude providers. A Codex master must receive the master instruction surface required to function as PM, or validation must reject Codex master as unsupported.

Requirement decision:
- Built-in master rules/AGENTS.md composition for Codex is **in scope**. A Codex master with zero rules is not an actually-working master spawn under MPP-R1.
- `UserPromptSubmit` parity is **in scope as a design decision**, not automatically required as byte-for-byte parity. Design must determine whether Codex supports an equivalent user-prompt hook and whether PM function depends on it. If Codex cannot support it, validation must explicitly degrade/reject with the named missing capability.

Acceptance criteria:
- The current bypass `role == HomeLayoutRole::Master && provider != "claude"` must be removed, narrowed, or guarded by validation so Codex master cannot spawn without the intended master rules by accident.
- Codex master rules must be placed in the provider-native rules target, not in a Claude-only file path.
- The design must state the `UserPromptSubmit` requirement for master providers: required hook, optional hook, or unsupported provider capability. It must include the failure/degradation behavior.
- Tests must assert that a Codex master spawn receives provider-native master rules and that the hook decision is reflected in materialized config or a validation diagnostic.

## Non-Goals and Registered Deferrals

Per the project rule "defer = registered backlog entry", every non-goal here has an owner and a registration target. These are registered in this requirements file until `tasks.md` exists; `tasks.md` must copy them into a Backlog section before implementation planning.

| ID | Deferred item | Owner | Registration target | Reason |
| --- | --- | --- | --- | --- |
| B1 | Add new providers beyond current `VALID_PROVIDER_NAMES` | provider subsystem owner | `.kiro/specs/ah-master-provider-parameterization/tasks.md#backlog` | This spec parameterizes existing providers; adding providers is a separate manifest/home-layout contract. |
| B2 | Redesign Claude OAuth gateway token semantics | per-worker credentials / Claude gateway owner | `.kiro/specs/ah-per-worker-credentials/` or this spec's tasks backlog if a master-only gap is found | This spec only prevents non-Claude paths from being forced through Claude gateway. |
| B3 | General provider-auth security hardening unrelated to master spawn | security/provider owner | this spec's tasks backlog or a dedicated auth-hardening spec | Out of scope unless required to make non-Claude master spawn correct. |
| B4 | Provider readiness protocol unification | master cutover owner | this spec's tasks backlog | Readiness is already provider-aware enough to expose the current inconsistency; this spec only requires it not be confused with materialization authority. |
| B5 | Full matrix e2e tests for every provider | test owner | this spec's tasks backlog | Hard gate is one non-Claude lock test; broader matrix coverage can follow after the core path is locked. |
| B6 | Provider-parametric conversation history translation across master providers | master cutover owner | `.kiro/specs/ah-master-provider-parameterization/tasks.md#backlog` | First implementation must prevent silent loss; full cross-provider history conversion is useful but separable. |

No required brief item is considered N/A. The prior Codex credential open question is resolved by MPP-R4 using o1's code evidence. Cross-provider conversation translation is deferred as B6, with explicit reject/gate required by MPP-R9 until implemented.

## Requirement Change Log

2026-07-13:
- Initial requirements draft created from master brief and `research/architecture-index.md` Layer 6 capability-holdout note.
- Requirement MPP-R5 narrows the "warn until parameterized" instruction into a split rule: full fix must validate spawn support; an earlier partial patch must warn instead of false-green. Reason: this preserves operator UX while allowing implementation to stage the riskier spawn refactor.
- Requirement MPP-R4 does not assume Codex needs zero credential handling. Reason: the brief explicitly flags `claude_gateway` as Claude-specific and asks for either a per-provider path or an open design question before freeze.
- o1 red-team incorporated: MPP-R4 is no longer open. Codex credential materialization is required to use existing `.codex` / `.config/gcloud` manifest auth mounts, `PROVIDER_AUTH_WHITELIST` entries `.codex/auth.json` + `.codex/installation_id`, and `prepare_codex_overrides` / `CODEX_HOME`; Claude gateway remains forbidden for Codex.
- o1 red-team incorporated: added MPP-R9 for master cutover conversation seeding. Chose fail-closed gating for cross-provider cutover unless a provider-native seeding path is implemented; registered full provider-parametric history translation as B6.
- o1 red-team incorporated: tightened MPP-R6 to use a dummy command and assert home-layout artifacts rather than requiring real Codex binary/OAuth.
- o1 red-team incorporated: added MPP-R10 for non-Claude master rules and prompt-hook parity. Codex master cannot silently bypass rules; `UserPromptSubmit` support/degradation must be designed explicitly.
- o1 red-team incorporated: MPP-R1 now rejects provider/cmd mismatch at validation rather than inferring or auto-rewriting; MPP-R5 now explicitly requires rewriting the existing `master.provider != "claude"` bundle/settings validation block.

## Gate Checklist

Before this requirements file can be called frozen:
- o1 red-team/debate is completed and incorporated in the 2026-07-13 change-log entries above.
- Each o1 critical objection is adopted or explicitly resolved: Codex credentials (MPP-R4), cutover seeding (MPP-R9), lock test feasibility (MPP-R6), non-Claude master rules/hooks (MPP-R10), provider/cmd mismatch (MPP-R1), and config validation hard block (MPP-R5).
- MPP-R4's Codex credential contract is resolved; it is no longer a freeze-blocking open question.
- Operator confirms whether MPP-R5's temporary warning quick patch is allowed before full parameterization or must be bundled with the full fix.
- Operator accepts MPP-R9's first-step policy: reject/gate unsupported cross-provider cutover unless provider-native seeding is implemented in the same change.
