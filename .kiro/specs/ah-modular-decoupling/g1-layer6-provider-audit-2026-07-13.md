# Layer 6 Provider/Gateway/Credential Audit

Date: 2026-07-13
Auditor: g1-codex
Scope: `src/provider/`, `src/process_identity.rs`, and bin ownership evidence from `src/bin/ah.rs` / `src/bin/ahd.rs`.

## Source Reality Check

- `src/provider/mod.rs` exports the provider subsystem from the library crate.
- Top-level `src/claude_gateway.rs` does not exist in this worktree. The gateway implementation is `src/provider/claude_gateway.rs` and is exported as `crate::provider::claude_gateway`.
- `src/lib.rs` declares `pub mod provider;` and `pub(crate) mod process_identity;`; both `src/bin/ah.rs` and `src/bin/ahd.rs` use the same `ah` library crate, so these modules are compiled/typechecked into the shared crate for both bins. Runtime ownership differs by caller, listed per module below.

## Bin Ownership Evidence

- `ah` CLI process: `src/bin/ah.rs` directly wires CLI subcommands. Provider-facing direct paths include `ah bundle` via `ah::cli::bundle::{run_bundle_list, run_bundle_validate}`, `ah doctor` via `ah::cli::doctor`, `ah start/up` via `ah::cli::{start,up}`, config validation via `ah::cli::config_cmd`, and hook delivery via `ah agent notify`.
- `ahd` daemon process: `src/bin/ahd.rs` initializes DB/tmux/RPC/orchestrator. Provider runtime is reached through RPC handlers and monitors: agent/master spawn, realign, prompt handling, health watcher, and master revival.
- Gateway bridge pitfall: `src/provider/home_layout.rs::build_ah_hook_command` runs inside whichever process materializes hooks, commonly `ahd`; it uses `std::env::current_exe()` and then `resolve_ah_binary()` to choose a sibling `ah` binary, not the current `ahd` executable path.

## Module Inventory

### `provider::mod`

- Name: Provider module index
- One-line responsibility: Declares and exports every provider subsystem module.
- Source path: `src/provider/mod.rs` exists, 13 lines.
- Key public entry symbols: `pub mod builtin`, `bundles`, `claude_gateway`, `extensions`, `fingerprint`, `health_check`, `home_layout`, `init_probe`, `init_probe_task`, `manifest`, `plugins`, `skills`.
- Ownership axis: both. Compiled through `src/lib.rs` for both `ah` and `ahd`; no runtime behavior itself.

### `provider::builtin`

- Name: Built-in provider rule/skill assets
- One-line responsibility: Embeds ah-owned kernels, default role rules, and built-in skill metadata.
- Source path: `src/provider/builtin.rs` exists, 42 lines.
- Key public entry symbols: `MASTER_KERNEL`, `WORKER_KERNEL`, `DEFAULT_MASTER`, `DEFAULT_WORKER`, `BuiltinSkillScope`, `BuiltinSkill`, `BUILTIN_SKILLS`.
- Ownership axis: both. `ahd` uses it during master/worker home materialization through `home_layout`; `ah` can reach the same materialization logic indirectly through config/bundle validation paths and library tests.

### `provider::bundles`

- Name: Bundle resolver and validator
- One-line responsibility: Parses `.ah/bundles/<name>/bundle.toml`, validates capability support by provider/role, merges bundle contributions into `ExtensionConfig`, and computes bundle digests.
- Source path: `src/provider/bundles.rs` exists, 874 lines.
- Key public entry symbols: `BundleRole`, `ResolvedBundles`, `BundleInspection`, `resolve_bundles_for_provider`, `digest_for_bundles`, `list_bundle_names`, `inspect_bundle`.
- Ownership axis: both. `ah` CLI uses it through `src/cli/bundle.rs` for `ah bundle list/validate`; `ahd` uses it in `rpc/handlers/{agent,sessions,realign}.rs` before spawn/realign so running panes receive resolved skills/hooks/rules/MCP and stable bundle digests.

### `provider::claude_gateway`

- Name: Claude per-worker credential gateway
- One-line responsibility: Loads host Claude OAuth seed credentials, creates a process-local production gateway, registers per-worker UDS gateways, injects worker JWT-facing env, refreshes tokens, and proxies worker requests upstream.
- Source path: `src/provider/claude_gateway.rs` exists, 920 lines. `src/claude_gateway.rs` is absent.
- Key public entry symbols: `FakeClaims`, `build_fake_worker_jwt_for_test`, `decode_fake_worker_jwt_claims`, `CredentialFailureCode`, `GatewayBind`, `SeedCredential`, `WorkerGatewayEnv`, `port_from_slot_id`, `ClaudeGatewayConfig`, `TestWorkerGateway`, `ClaudeGateway`, `WorkerGateway`, `get_or_init_production_gateway`, `load_seed_credential`, `ClaudeGateway::spawn_for_test`, `ClaudeGateway::worker_gateway_for_test`, `ClaudeGateway::register_worker`.
- Ownership axis: primarily `ahd` runtime. `rpc/handlers/agent.rs` starts/gets the production gateway and calls `register_worker()` during Claude worker spawn, then mounts the per-worker UDS into the sandbox. The module is compiled in the shared library for both bins, but production gateway runtime is daemon-side.

### `provider::extensions`

- Name: Provider extension schema
- One-line responsibility: Defines the serialized extension surface for hooks, plugins, skills, bundles, settings, rules, and MCP servers.
- Source path: `src/provider/extensions.rs` exists, 151 lines.
- Key public entry symbols: `ExtensionConfig`, `McpServerConfig`, `McpTransport`, `HookGroup`, `HookItem`, `default_matcher`.
- Ownership axis: both. `ah` parses/validates project config and bundle CLI inputs through this schema; `ahd` consumes the same schema during spawn, home materialization, hook injection, and realign hashing.

### `provider::fingerprint`

- Name: Provider config fingerprinting
- One-line responsibility: Produces deterministic hashes for master/agent provider configuration, including hooks/plugins/skills/settings and non-empty bundle digests.
- Source path: `src/provider/fingerprint.rs` exists, 198 lines.
- Key public entry symbols: `ConfigRole`, `ConfigFingerprintInput`, `BundleDigest`, `BundleDigest::is_empty`, `BundleDigestEntry`, `compute_config_hash`, `deterministic_json`.
- Ownership axis: primarily `ahd` runtime, with shared compile. `rpc/handlers/{agent,sessions,realign}.rs` use it to store and compare drift hashes; CLI code may parse the same structures but does not own live drift decisions.

### `provider::health_check`

- Name: Provider health watcher
- One-line responsibility: Observes active agents for tmux/predicate/completion failures and queued starvation, then emits alerts or marks agents STUCK.
- Source path: `src/provider/health_check.rs` exists, 915 lines.
- Key public entry symbols: `QUEUED_STARVATION_THRESHOLD_SECS`, `HealthCheckResult`, `HealthCheckObservation`, `health_check_observe`, `escalate_health_stuck`, `health_check_watcher_loop`.
- Ownership axis: `ahd` daemon runtime. `src/orchestrator/mod.rs` imports `health_check_watcher_loop`; it needs daemon DB, tmux, pubsub, and state-machine context. It is not a CLI command path except through shared library compilation/tests.

### `provider::home_layout`

- Name: Provider sandbox home materializer
- One-line responsibility: Builds provider-specific sandbox homes, auth materialization, trust files, rules, skills, plugins, hooks, MCP settings, Claude gateway env, and hook push commands.
- Source path: `src/provider/home_layout.rs` exists, 2907 lines.
- Key public entry symbols: `HomeOverrides`, `HookPushContext`, `HomeLayoutRole`, `AuthMaterializationErrorCode`, `materialize_auth_file_with_ladder`, `prepare_home_layout`, `prepare_home_layout_with_role`, `prepare_home_layout_with_extensions`, `prepare_home_layout_with_extensions_for_slot`, `prepare_claude_home_layout_with_gateway`, `compose_rules`, `compose_rules_with_layers`, `build_ah_hook_command`, `sandbox_home_for_sandbox_dir`.
- Ownership axis: both, with daemon-critical runtime. `ahd` calls `prepare_home_layout_with_extensions_for_slot` from `rpc/handlers/{agent,sessions}.rs` and monitor revival paths before spawning panes. `ah` reaches related config/bundle/schema validation and the generated hook command is executed as `ah agent notify` from provider hooks. The `current_exe()` call in `build_ah_hook_command` must be read as "current materializer process"; when that is `ahd`, `resolve_ah_binary()` deliberately resolves sibling `ah`.

### `provider::init_probe`

- Name: Provider TUI readiness predicates
- One-line responsibility: Implements deterministic pane-capture predicates for bash, Codex, Claude, and Antigravity startup readiness.
- Source path: `src/provider/init_probe.rs` exists, 279 lines.
- Key public entry symbols: `InitGateProbe`, `ClaudeInitProbe`, `AntigravityInitProbe`, `CodexInitProbe`, `BashInitProbe`.
- Ownership axis: `ahd` daemon runtime. `manifest::InitProbeKind::build`, `health_check_observe`, and `init_probe_task` use these predicates against tmux captures. CLI does not directly run readiness probes.

### `provider::init_probe_task`

- Name: Async InitGate task driver
- One-line responsibility: Polls spawned provider panes until readiness, learned readiness, prompt intervention, timeout, or unknown stable startup state is detected.
- Source path: `src/provider/init_probe_task.rs` exists, 1139 lines.
- Key public entry symbols: `STABLE_UNKNOWN_STARTUP_GRACE`, `spawn_init_probe_task`, `respawn_init_probe_for_agent`.
- Ownership axis: `ahd` daemon runtime. `rpc/handlers/agent.rs` spawns the task after worker tmux pane creation; `rpc/handlers/prompt.rs` can respawn it after startup prompt resolution. It depends on daemon DB, tmux, prompt handler, and state transitions.

### `provider::manifest`

- Name: Provider manifest registry
- One-line responsibility: Defines provider commands, auth mounts, env passthrough/injection, readiness modes, recovery args, valid names, and spawn env collection.
- Source path: `src/provider/manifest.rs` exists, 795 lines.
- Key public entry symbols: `ProviderManifest`, `CompletionSignalKind`, `is_recovery_eligible_provider`, `compute_recovery_args`, `IdleDetectionMode`, `InitProbeKind`, `InitProbeKind::build`, `ENV_PASSTHROUGH`, `CLAUDE_INJECTED_ENV`, `CODEX_INJECTED_ENV`, `ANTIGRAVITY_INJECTED_ENV`, `OPENCODE_INJECTED_ENV`, `PANE_LOG_INJECTED_ENV`, `VALID_PROVIDER_NAMES`, `canonicalize_provider_name`, `MANIFESTS`, `get_manifest`, `try_get_manifest`, `is_valid_provider`, `valid_provider_names`, `valid_provider_names_csv`, `unknown_provider_message`, `known_provider_manifests`, `cancel_keysyms_for_provider`, `collect_spawn_env`.
- Ownership axis: both. `ah` uses it in config validation, doctor provider checks, setup/service env passthrough, and CLI-side provider normalization. `ahd` uses it for spawn command construction, readiness probes, recovery args, cancellation keys, and platform scope env collection.

### `provider::plugins`

- Name: Provider plugin resolver
- One-line responsibility: Parses id-only or git plugin specs, resolves provider cache paths, and clones/caches git plugins.
- Source path: `src/provider/plugins.rs` exists, 285 lines.
- Key public entry symbols: `PluginSpec`, `GitUrlSpec`, `ResolvedPlugin`, `parse_plugin_spec`, `resolve_plugins_for_provider`.
- Ownership axis: both. `ahd` uses it through `home_layout` while materializing Claude/Codex plugin directories. `ah` can exercise the same resolver through bundle/config validation paths that prepare extension plans; git clone side effects are in the shared library helper.

### `provider::skills`

- Name: Provider skill resolver/materialization planner
- One-line responsibility: Validates project skill references, resolves `.ah/skills/<name>/SKILL.md`, and plans provider-specific symlink targets.
- Source path: `src/provider/skills.rs` exists, 230 lines.
- Key public entry symbols: `SkillRef`, `ResolvedSkill`, `SkillMaterialization`, `parse_skill_refs`, `resolve_project_skills`, `plan_claude_skill_materialization`, `plan_codex_skill_materialization`.
- Ownership axis: both. `ah` parses config skill references and bundle validation inputs; `ahd` materializes resolved project/bundle skills into sandbox homes before master/worker spawn.

### `process_identity`

- Name: Spawned process identity injector
- One-line responsibility: Injects per-pane `AH_ROLE`, `AH_SESSION_ID`, and optional `AH_AGENT_ID` into master/worker process environments without conflating them with daemon socket/state identity.
- Source path: `src/process_identity.rs` exists, 73 lines.
- Key public entry symbols: crate-private `AH_AGENT_ID`, `AH_ROLE`, `AH_SESSION_ID`, `AH_ROLE_MASTER`, `AH_ROLE_WORKER`, `inject_worker_identity`, `inject_master_identity`.
- Ownership axis: `ahd` daemon runtime. The module is crate-private and used from `rpc/handlers/agent.rs`, `rpc/handlers/sessions.rs`, `monitor/master_watch.rs`, and platform scope tests to prepare spawned provider process env; the `ah` CLI does not inject live pane identity.

## Capability To Owner Map

- Credential/bundle parsing: bundle parsing is owned by `src/provider/bundles.rs::{resolve_bundles_for_provider,digest_for_bundles,inspect_bundle,list_bundle_names}` with schema types from `src/provider/extensions.rs`; provider OAuth/auth file materialization is owned by `src/provider/home_layout.rs::materialize_auth_file_with_ladder` and Claude seed loading by `src/provider/claude_gateway.rs::load_seed_credential`.
- Provider health check: observation logic is owned by `src/provider/health_check.rs::{health_check_observe,escalate_health_stuck,health_check_watcher_loop}` and is daemon-started through `src/orchestrator/mod.rs`.
- Gateway bridging: production gateway lifecycle is owned by `src/provider/claude_gateway.rs::{get_or_init_production_gateway,ClaudeGateway::register_worker}`; sandbox env/home wiring is owned by `src/provider/home_layout.rs::{prepare_home_layout_with_extensions_for_slot,prepare_claude_home_layout_with_gateway}`; hook bridge command resolution is owned by `src/provider/home_layout.rs::build_ah_hook_command` plus private `resolve_ah_binary`.
- Process identity recognition: env keys and injection are owned by `src/process_identity.rs::{inject_master_identity,inject_worker_identity}`; daemon callers are `rpc/handlers/sessions.rs`, `rpc/handlers/agent.rs`, and `monitor/master_watch.rs`.
- Provider command/env registry: provider canonicalization, spawn env collection, recovery args, readiness kind, and cancel keys are owned by `src/provider/manifest.rs`.
- Startup readiness: pure predicates are owned by `src/provider/init_probe.rs`; async polling, learned startup readiness, prompt intervention, and respawn are owned by `src/provider/init_probe_task.rs`.

