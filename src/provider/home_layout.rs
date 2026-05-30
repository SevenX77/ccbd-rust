use crate::error::CcbdError;
use crate::provider::builtin;
use crate::provider::extensions::{ExtensionConfig, HookGroup, HookItem};
use crate::provider::plugins::{ResolvedPlugin, resolve_plugins_for_provider};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use toml::Value as TomlValue;

const WHITELIST: &[&str] = &[".ssh", ".gitconfig", ".git-credentials", ".netrc"];
const PROVIDER_AUTH_WHITELIST: &[&str] = &[
    ".claude/.credentials.json",
    ".codex/auth.json",
    ".codex/installation_id",
    ".gemini/oauth_creds.json",
    ".gemini/google_accounts.json",
    ".gemini/installation_id",
    ".gemini/antigravity-cli/antigravity-oauth-token",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HomeOverrides {
    pub home_root: PathBuf,
    pub extra_env: HashMap<String, String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HomeLayoutRole {
    Master,
    Worker,
}

pub fn prepare_home_layout(
    provider: &str,
    sandbox_dir: &Path,
    workspace_path: &Path,
) -> Result<HomeOverrides, CcbdError> {
    prepare_home_layout_with_role(
        provider,
        sandbox_dir,
        workspace_path,
        HomeLayoutRole::Worker,
    )
}

pub fn prepare_home_layout_with_role(
    provider: &str,
    sandbox_dir: &Path,
    workspace_path: &Path,
    role: HomeLayoutRole,
) -> Result<HomeOverrides, CcbdError> {
    prepare_home_layout_with_extensions(
        provider,
        sandbox_dir,
        workspace_path,
        role,
        &ExtensionConfig::default(),
    )
}

pub fn prepare_home_layout_with_extensions(
    provider: &str,
    sandbox_dir: &Path,
    workspace_path: &Path,
    role: HomeLayoutRole,
    extensions: &ExtensionConfig,
) -> Result<HomeOverrides, CcbdError> {
    let source_home = materialization_source_home()?;
    let home_root = sandbox_home_for_sandbox_dir(sandbox_dir)?;
    let workspace_key = workspace_trust_key(workspace_path);
    fs::create_dir_all(&home_root)
        .map_err(|err| home_err("create sandbox home", &home_root, err))?;

    let overrides = match provider {
        "claude" => {
            prepare_claude_overrides(&source_home, &home_root, &workspace_key, role, &extensions)
        }
        "gemini" => {
            prepare_gemini_overrides(&source_home, &home_root, &workspace_key, role, &extensions)
        }
        "codex" => {
            prepare_codex_overrides(&source_home, &home_root, &workspace_key, role, &extensions)
        }
        "antigravity" => prepare_antigravity_overrides(&source_home, &home_root, &workspace_key),
        _ => Ok(HomeOverrides {
            home_root,
            extra_env: HashMap::new(),
        }),
    }?;
    materialize_sandbox_home_links(&source_home, &overrides.home_root);
    Ok(overrides)
}

fn prepare_claude_overrides(
    source_home: &Path,
    home_root: &Path,
    workspace_key: &str,
    role: HomeLayoutRole,
    extensions: &ExtensionConfig,
) -> Result<HomeOverrides, CcbdError> {
    let layout = ClaudeHomeLayout::for_home(home_root);
    fs::create_dir_all(&layout.claude_dir)
        .map_err(|err| home_err("create claude dir", &layout.claude_dir, err))?;
    fs::create_dir_all(&layout.projects_root)
        .map_err(|err| home_err("create claude projects", &layout.projects_root, err))?;
    fs::create_dir_all(&layout.session_env_root)
        .map_err(|err| home_err("create claude session env", &layout.session_env_root, err))?;
    materialize_builtin_rules(role, "claude", home_root)?;
    materialize_trust(source_home, &layout, workspace_key)?;
    let plugins = resolve_plugins_for_provider("claude", source_home, &extensions.plugins)?;
    materialize_claude_plugins(&layout, &plugins)?;
    let hook_specs = materialize_claude_hooks(source_home, &layout, &extensions.hooks)?;
    materialize_claude_settings(source_home, &layout, &hook_specs, &plugins)?;
    link_credentials(source_home, &layout);

    Ok(HomeOverrides {
        home_root: home_root.to_path_buf(),
        extra_env: home_env(home_root, [("CLAUDE_CONFIG_DIR", ".claude")]),
    })
}

fn prepare_gemini_overrides(
    source_home: &Path,
    home_root: &Path,
    workspace_key: &str,
    role: HomeLayoutRole,
    extensions: &ExtensionConfig,
) -> Result<HomeOverrides, CcbdError> {
    let layout = GeminiHomeLayout::for_home(home_root);
    fs::create_dir_all(&layout.gemini_dir)
        .map_err(|err| home_err("create gemini dir", &layout.gemini_dir, err))?;
    fs::create_dir_all(&layout.tmp_root)
        .map_err(|err| home_err("create gemini tmp", &layout.tmp_root, err))?;
    materialize_builtin_rules(role, "gemini", home_root)?;
    ensure_json_file(&layout.settings_path)?;
    ensure_json_file(&layout.trusted_folders_path)?;
    let hook_specs = materialize_gemini_hooks(source_home, &layout, &extensions.hooks)?;
    materialize_gemini_settings(source_home, &layout, &hook_specs)?;
    materialize_gemini_state(source_home, &layout)?;
    materialize_trusted_folders(source_home, &layout, workspace_key)?;

    Ok(HomeOverrides {
        home_root: home_root.to_path_buf(),
        extra_env: home_env(home_root, [("GEMINI_CLI_HOME", ".gemini")]),
    })
}

fn prepare_codex_overrides(
    source_home: &Path,
    home_root: &Path,
    workspace_key: &str,
    role: HomeLayoutRole,
    extensions: &ExtensionConfig,
) -> Result<HomeOverrides, CcbdError> {
    let codex_home = home_root.join(".codex");
    prepare_managed_codex_home(
        source_home,
        &codex_home,
        workspace_key,
        role,
        &extensions.plugins,
    )?;
    Ok(HomeOverrides {
        home_root: home_root.to_path_buf(),
        extra_env: home_env(home_root, [("CODEX_HOME", ".codex")]),
    })
}

fn prepare_antigravity_overrides(
    source_home: &Path,
    home_root: &Path,
    workspace_key: &str,
) -> Result<HomeOverrides, CcbdError> {
    let layout = AntigravityHomeLayout::for_home(home_root);
    fs::create_dir_all(&layout.antigravity_dir).map_err(|err| {
        home_err(
            "create antigravity config dir",
            &layout.antigravity_dir,
            err,
        )
    })?;
    ensure_json_file(&layout.settings_path)?;
    materialize_antigravity_settings(source_home, &layout, workspace_key)?;
    materialize_antigravity_onboarding(source_home, &layout)?;

    Ok(HomeOverrides {
        home_root: home_root.to_path_buf(),
        extra_env: home_env(home_root, []),
    })
}

fn materialize_antigravity_settings(
    source_home: &Path,
    layout: &AntigravityHomeLayout,
    workspace_key: &str,
) -> Result<(), CcbdError> {
    let source_settings = source_home.join(".gemini/antigravity-cli/settings.json");
    if source_settings.is_file() {
        fs::copy(&source_settings, &layout.settings_path)
            .map_err(|err| home_err("copy antigravity settings", &layout.settings_path, err))?;
    }

    let mut settings = read_json_object(&layout.settings_path).unwrap_or_default();
    let trusted_workspaces = settings
        .entry("trustedWorkspaces".to_string())
        .or_insert_with(|| Value::Array(vec![]));
    if !trusted_workspaces.is_array() {
        *trusted_workspaces = Value::Array(vec![]);
    }
    let Some(workspaces) = trusted_workspaces.as_array_mut() else {
        return write_json_object(&layout.settings_path, &settings);
    };
    workspaces.retain(|value| value.as_str() != Some("/home/agent"));
    if !workspaces
        .iter()
        .any(|value| value.as_str() == Some(workspace_key))
    {
        workspaces.push(Value::String(workspace_key.to_string()));
    }
    write_json_object(&layout.settings_path, &settings)
}

fn materialize_antigravity_onboarding(
    source_home: &Path,
    layout: &AntigravityHomeLayout,
) -> Result<(), CcbdError> {
    fs::create_dir_all(&layout.cache_dir).map_err(|err| {
        home_err(
            "create antigravity onboarding cache",
            &layout.cache_dir,
            err,
        )
    })?;
    let source_onboarding = source_home.join(".gemini/antigravity-cli/cache/onboarding.json");
    if source_onboarding.is_file() {
        fs::copy(&source_onboarding, &layout.onboarding_path)
            .map_err(|err| home_err("copy antigravity onboarding", &layout.onboarding_path, err))?;
        return Ok(());
    }

    let mut onboarding = Map::new();
    onboarding.insert("consumerOnboardingComplete".to_string(), Value::Bool(true));
    onboarding.insert(
        "enterpriseOnboardingComplete".to_string(),
        Value::Bool(false),
    );
    onboarding.insert("onboardingComplete".to_string(), Value::Bool(true));
    write_json_object(&layout.onboarding_path, &onboarding)
}

fn materialize_builtin_rules(
    role: HomeLayoutRole,
    provider: &str,
    home_root: &Path,
) -> Result<(), CcbdError> {
    let Some(target) = builtin_rules_target(provider, home_root) else {
        return Ok(());
    };
    if role == HomeLayoutRole::Master && provider != "claude" {
        return Ok(());
    }
    let content = match role {
        HomeLayoutRole::Master => builtin::MASTER_RULES,
        HomeLayoutRole::Worker => builtin::WORKER_RULES,
    };
    write_builtin_rules(&target, content)
}

fn builtin_rules_target(provider: &str, home_root: &Path) -> Option<PathBuf> {
    match provider {
        "claude" => Some(home_root.join(".claude/CLAUDE.md")),
        "gemini" => Some(home_root.join(".gemini/GEMINI.md")),
        "codex" => Some(home_root.join(".codex/AGENTS.md")),
        _ => None,
    }
}

fn write_builtin_rules(path: &Path, content: &str) -> Result<(), CcbdError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| home_err("create builtin rules parent", parent, err))?;
    }
    fs::write(path, content).map_err(|err| home_err("write builtin rules", path, err))
}

fn materialize_sandbox_home_links(source_home: &Path, home_root: &Path) {
    for relative in WHITELIST {
        link_into_sandbox(source_home, home_root, relative);
    }
    for relative in PROVIDER_AUTH_WHITELIST {
        link_auth_file_into_sandbox(source_home, home_root, relative);
    }
}

fn link_into_sandbox(source_home: &Path, home_root: &Path, relative: &str) {
    let source = source_home.join(relative);
    if !source.exists() {
        return;
    }
    let target = home_root.join(relative);
    let Some(parent) = target.parent() else {
        return;
    };
    if fs::create_dir_all(parent).is_err() {
        return;
    }
    if target.is_symlink() {
        if same_resolved_path(&target, &source) {
            return;
        }
        if fs::remove_file(&target).is_err() {
            return;
        }
    } else if target.exists() {
        return;
    }
    #[cfg(unix)]
    {
        if let Err(err) = std::os::unix::fs::symlink(&source, &target) {
            tracing::warn!(
                source = %source.display(),
                target = %target.display(),
                %err,
                "failed to symlink sandbox home whitelist entry"
            );
        }
    }
}

fn link_auth_file_into_sandbox(source_home: &Path, home_root: &Path, relative: &str) {
    let source = source_home.join(relative);
    if !source.is_file() {
        return;
    }
    let target = home_root.join(relative);
    if is_dynamic_oauth_auth_file(relative) {
        copy_dynamic_auth_file(&source, &target);
    } else {
        symlink_auth_file(&source, &target);
    }
}

fn is_dynamic_oauth_auth_file(relative: &str) -> bool {
    matches!(
        relative,
        ".gemini/oauth_creds.json"
            | ".gemini/google_accounts.json"
            | ".gemini/antigravity-cli/antigravity-oauth-token"
    )
}

fn materialize_trust(
    source_home: &Path,
    layout: &ClaudeHomeLayout,
    workspace_key: &str,
) -> Result<(), CcbdError> {
    let source_trust = source_home.join(".claude.json");
    if !layout.trust_path.exists() && source_trust.is_file() {
        copy_if_missing(&source_trust, &layout.trust_path);
    }
    if !layout.config_dir_state_path.exists() && source_trust.is_file() {
        copy_if_missing(&source_trust, &layout.config_dir_state_path);
    }
    ensure_trust_file(&layout.trust_path)?;
    ensure_trust_file(&layout.config_dir_state_path)?;
    ensure_claude_workspace_trust(&layout.trust_path, workspace_key)?;
    ensure_claude_workspace_trust(&layout.config_dir_state_path, workspace_key)
}

fn link_credentials(source_home: &Path, layout: &ClaudeHomeLayout) {
    let source = source_home.join(".claude/.credentials.json");
    if !source.is_file() {
        return;
    }
    let target = layout.claude_dir.join(".credentials.json");
    symlink_auth_file(&source, &target);
}

fn materialize_trusted_folders(
    source_home: &Path,
    layout: &GeminiHomeLayout,
    workspace_key: &str,
) -> Result<(), CcbdError> {
    let projected = read_json_object(&source_home.join(".gemini/trustedFolders.json"));
    let existing = read_json_object(&layout.trusted_folders_path);
    let mut merged = merge_object_payload(projected, existing).unwrap_or_default();
    remove_legacy_workspace_json_key(&mut merged, workspace_key);
    merged.insert(
        workspace_key.to_string(),
        Value::String("TRUST_FOLDER".to_string()),
    );
    write_json_object(&layout.trusted_folders_path, &merged)
}

#[derive(Debug, Clone)]
struct MaterializedHook {
    event: String,
    matcher: String,
    item: HookItem,
}

fn materialize_claude_hooks(
    source_home: &Path,
    layout: &ClaudeHomeLayout,
    hooks: &HashMap<String, Vec<HookGroup>>,
) -> Result<Vec<MaterializedHook>, CcbdError> {
    materialize_hooks(source_home, &layout.claude_dir.join("hooks"), hooks)
}

fn materialize_gemini_hooks(
    source_home: &Path,
    layout: &GeminiHomeLayout,
    hooks: &HashMap<String, Vec<HookGroup>>,
) -> Result<Vec<MaterializedHook>, CcbdError> {
    materialize_hooks(source_home, &layout.gemini_dir.join("hooks"), hooks)
}

fn materialize_hooks(
    source_home: &Path,
    target_dir: &Path,
    hooks: &HashMap<String, Vec<HookGroup>>,
) -> Result<Vec<MaterializedHook>, CcbdError> {
    let mut materialized = Vec::new();
    for (event, groups) in hooks {
        for group in groups {
            for item in &group.hooks {
                let source = resolve_extension_source(source_home, &item.command);
                if !source.is_file() {
                    return Err(CcbdError::EnvironmentNotSupported {
                        details: format!("hook script not found: {}", source.display()),
                    });
                }
                let file_name =
                    source
                        .file_name()
                        .ok_or_else(|| CcbdError::EnvironmentNotSupported {
                            details: format!("hook script has no filename: {}", source.display()),
                        })?;
                let target = target_dir.join(file_name);
                force_symlink(&source, &target)?;
                let mut item = item.clone();
                item.command = target.display().to_string();
                materialized.push(MaterializedHook {
                    event: event.clone(),
                    matcher: group.matcher.clone(),
                    item,
                });
            }
        }
    }
    Ok(materialized)
}

fn materialize_gemini_settings(
    source_home: &Path,
    layout: &GeminiHomeLayout,
    hooks: &[MaterializedHook],
) -> Result<(), CcbdError> {
    let source_settings = source_home.join(".gemini/settings.json");
    if source_settings.is_file() {
        fs::copy(&source_settings, &layout.settings_path)
            .map_err(|err| home_err("copy gemini settings", &layout.settings_path, err))?;
    }
    let mut settings = read_json_object(&layout.settings_path).unwrap_or_default();
    let security = settings
        .entry("security".to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    if let Value::Object(security_map) = security {
        let auth = security_map
            .entry("auth".to_string())
            .or_insert_with(|| Value::Object(Map::new()));
        if let Value::Object(auth_map) = auth {
            auth_map
                .entry("selectedType".to_string())
                .or_insert_with(|| Value::String("oauth-personal".to_string()));
        }
    }
    inject_gemini_hooks(&mut settings, hooks);
    write_json_object(&layout.settings_path, &settings)
}

fn materialize_gemini_state(
    source_home: &Path,
    layout: &GeminiHomeLayout,
) -> Result<(), CcbdError> {
    let source_state = source_home.join(".gemini/state.json");
    let target_state = layout.gemini_dir.join("state.json");
    if source_state.is_file() {
        fs::copy(&source_state, &target_state)
            .map_err(|err| home_err("copy gemini state", &target_state, err))?;
    } else if !target_state.exists() {
        let default_state = serde_json::json!({
            "tipsShown": 10,
            "startupWarningCounts": {},
            "defaultBannerShownCount": {}
        });
        let data = serde_json::to_string_pretty(&default_state).unwrap() + "\n";
        fs::write(&target_state, data)
            .map_err(|err| home_err("write gemini state", &target_state, err))?;
    }
    Ok(())
}

fn materialize_claude_settings(
    _source_home: &Path,
    layout: &ClaudeHomeLayout,
    hooks: &[MaterializedHook],
    plugins: &[ResolvedPlugin],
) -> Result<(), CcbdError> {
    ensure_json_file(&layout.settings_path)?;
    let mut settings = read_json_object(&layout.settings_path).unwrap_or_default();
    settings.insert(
        "skipDangerousModePermissionPrompt".to_string(),
        Value::Bool(true),
    );
    let permissions = settings
        .entry("permissions".to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    if let Value::Object(perms) = permissions {
        perms
            .entry("defaultMode".to_string())
            .or_insert_with(|| Value::String("bypassPermissions".to_string()));
    }
    inject_claude_hooks(&mut settings, hooks);
    let enabled_plugins = object_entry(&mut settings, "enabledPlugins");
    for plugin in plugins {
        enabled_plugins.insert(plugin.name.clone(), Value::Bool(true));
    }
    write_json_object(&layout.settings_path, &settings)
}

fn inject_claude_hooks(settings: &mut Map<String, Value>, hooks: &[MaterializedHook]) {
    let hooks_root = object_entry(settings, "hooks");
    for hook in hooks {
        let event = hooks_root
            .entry(hook.event.clone())
            .or_insert_with(|| Value::Array(vec![]));
        if !event.is_array() {
            *event = Value::Array(vec![]);
        }
        let Some(event_hooks) = event.as_array_mut() else {
            continue;
        };
        let mut hook_item = Map::new();
        hook_item.insert(
            "type".to_string(),
            Value::String(hook.item.hook_type.clone()),
        );
        hook_item.insert(
            "command".to_string(),
            Value::String(hook.item.command.clone()),
        );
        if let Some(timeout) = hook.item.timeout {
            hook_item.insert("timeout".to_string(), Value::from(timeout));
        }
        event_hooks.push(serde_json::json!({
            "matcher": hook.matcher,
            "hooks": [Value::Object(hook_item)],
        }));
    }
}

fn inject_gemini_hooks(settings: &mut Map<String, Value>, hooks: &[MaterializedHook]) {
    let hooks_root = object_entry(settings, "hooks");
    for hook in hooks {
        let event = hooks_root
            .entry(hook.event.clone())
            .or_insert_with(|| Value::Array(vec![]));
        if !event.is_array() {
            *event = Value::Array(vec![]);
        }
        let Some(event_hooks) = event.as_array_mut() else {
            continue;
        };
        let mut hook_obj = Map::new();
        hook_obj.insert(
            "type".to_string(),
            Value::String(hook.item.hook_type.clone()),
        );
        hook_obj.insert(
            "command".to_string(),
            Value::String(hook.item.command.clone()),
        );
        hook_obj.insert("matcher".to_string(), Value::String(hook.matcher.clone()));
        if let Some(timeout) = hook.item.timeout {
            hook_obj.insert("timeout".to_string(), Value::from(timeout));
        }
        event_hooks.push(Value::Object(hook_obj));
    }
}

fn materialize_claude_plugins(
    layout: &ClaudeHomeLayout,
    plugins: &[ResolvedPlugin],
) -> Result<(), CcbdError> {
    for plugin in plugins {
        if !plugin.cache_dir.is_dir() {
            return Err(CcbdError::EnvironmentNotSupported {
                details: format!(
                    "claude plugin cache not found for {}: {}",
                    plugin.name,
                    plugin.cache_dir.display()
                ),
            });
        }
        force_symlink(
            &plugin.cache_dir,
            &layout.claude_dir.join("plugins/cache").join(&plugin.name),
        )?;
        force_symlink(
            &plugin.cache_dir,
            &layout.claude_dir.join("plugins").join(&plugin.name),
        )?;
    }
    Ok(())
}

fn prepare_managed_codex_home(
    source_home: &Path,
    codex_home: &Path,
    workspace_key: &str,
    role: HomeLayoutRole,
    plugins: &[String],
) -> Result<(), CcbdError> {
    fs::create_dir_all(codex_home).map_err(|err| home_err("create codex home", codex_home, err))?;
    let Some(home_root) = codex_home.parent() else {
        return Err(CcbdError::EnvironmentNotSupported {
            details: format!("codex home has no parent: {}", codex_home.display()),
        });
    };
    materialize_builtin_rules(role, "codex", home_root)?;
    let session_root = codex_home.join("sessions");
    fs::create_dir_all(&session_root)
        .map_err(|err| home_err("create codex sessions", &session_root, err))?;
    let target_config = codex_home.join("config.toml");
    if !target_config.exists() {
        fs::write(&target_config, "# ccb agent-local codex config\n")
            .map_err(|err| home_err("write codex config", &target_config, err))?;
    }
    ensure_codex_workspace_trust(&target_config, workspace_key)?;
    let plugins = resolve_plugins_for_provider("codex", source_home, plugins)?;
    materialize_codex_plugins(codex_home, &plugins)?;
    enable_codex_plugins(&target_config, &plugins)?;
    let source_version = source_home.join(".codex/version.json");
    let target_version = codex_home.join("version.json");
    if source_version.is_file() && !target_version.exists() {
        let _ = fs::copy(&source_version, &target_version);
    }
    let target_migration = codex_home.join(".personality_migration");
    if !target_migration.exists() {
        let _ = fs::write(&target_migration, "ok\n");
    }
    Ok(())
}

fn materialize_codex_plugins(
    codex_home: &Path,
    plugins: &[ResolvedPlugin],
) -> Result<(), CcbdError> {
    for plugin in plugins {
        if !plugin.cache_dir.is_dir() {
            return Err(CcbdError::EnvironmentNotSupported {
                details: format!(
                    "codex plugin cache not found for {}: {}",
                    plugin.name,
                    plugin.cache_dir.display()
                ),
            });
        }
        force_symlink(
            &plugin.cache_dir,
            &codex_home.join("plugins/cache").join(&plugin.name),
        )?;
    }
    Ok(())
}

fn enable_codex_plugins(path: &Path, plugins: &[ResolvedPlugin]) -> Result<(), CcbdError> {
    if plugins.is_empty() {
        return Ok(());
    }
    let data = fs::read_to_string(path).unwrap_or_default();
    let mut root = data
        .parse::<TomlValue>()
        .unwrap_or_else(|_| TomlValue::Table(toml::map::Map::new()));
    if !root.is_table() {
        root = TomlValue::Table(toml::map::Map::new());
    }
    let root_table = root.as_table_mut().expect("root was normalized to table");
    let plugins_table = table_entry(root_table, "plugins");
    for plugin in plugins {
        let plugin_table = table_entry(plugins_table, &plugin.name);
        plugin_table.insert("enabled".to_string(), TomlValue::Boolean(true));
    }
    write_codex_config(path, &root)
}

fn sandbox_home_for_sandbox_dir(sandbox_dir: &Path) -> Result<PathBuf, CcbdError> {
    let sandbox_path = sandbox_dir
        .canonicalize()
        .unwrap_or_else(|_| sandbox_dir.to_path_buf());
    let mut hasher = Sha256::new();
    hasher.update(sandbox_path.to_string_lossy().as_bytes());
    let digest = hasher.finalize();
    let project_id_short = digest
        .iter()
        .take(6)
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    Ok(xdg_cache_root()?
        .join("ah/sandboxes")
        .join(project_id_short))
}

fn resolve_extension_source(source_home: &Path, path: &str) -> PathBuf {
    let path = PathBuf::from(path);
    if path.is_absolute() {
        path
    } else {
        source_home.join(path)
    }
}

fn workspace_trust_key(workspace_path: &Path) -> String {
    workspace_path
        .canonicalize()
        .unwrap_or_else(|_| workspace_path.to_path_buf())
        .display()
        .to_string()
}

fn xdg_cache_root() -> Result<PathBuf, CcbdError> {
    if let Some(cache) = std::env::var_os("XDG_CACHE_HOME").filter(|value| !value.is_empty()) {
        return Ok(PathBuf::from(cache));
    }
    Ok(env_home()?.join(".cache"))
}

fn materialization_source_home() -> Result<PathBuf, CcbdError> {
    let env_home = env_home()?;
    let passwd_home = std::env::var("USER")
        .ok()
        .and_then(|user| passwd_home_for_user(&user));
    Ok(resolve_materialization_source_home(env_home, passwd_home))
}

fn resolve_materialization_source_home(env_home: PathBuf, passwd_home: Option<PathBuf>) -> PathBuf {
    if is_ccb_sandbox_home(&env_home)
        && let Some(passwd_home) = passwd_home
    {
        return passwd_home;
    }
    env_home
}

fn is_ccb_sandbox_home(path: &Path) -> bool {
    let path = path.to_string_lossy();
    path.contains("/.cache/ccb/sandboxes/") || path.contains("/.cache/ah/sandboxes/")
}

fn passwd_home_for_user(user: &str) -> Option<PathBuf> {
    let passwd = fs::read_to_string("/etc/passwd").ok()?;
    passwd.lines().find_map(|line| {
        let mut fields = line.split(':');
        let name = fields.next()?;
        if name != user {
            return None;
        }
        let _password = fields.next()?;
        let _uid = fields.next()?;
        let _gid = fields.next()?;
        let _gecos = fields.next()?;
        let home = fields.next()?;
        if home.is_empty() {
            None
        } else {
            Some(PathBuf::from(home))
        }
    })
}

fn env_home() -> Result<PathBuf, CcbdError> {
    std::env::var_os("HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| CcbdError::EnvironmentNotSupported {
            details: "HOME is not set for provider home materialization".into(),
        })
}

fn home_env<const N: usize>(
    home_root: &Path,
    entries: [(&str, &str); N],
) -> HashMap<String, String> {
    let mut env = HashMap::from([("HOME".to_string(), home_root.display().to_string())]);
    for (key, relative) in entries {
        env.insert(
            key.to_string(),
            home_root.join(relative).display().to_string(),
        );
    }
    env
}

fn ensure_trust_file(path: &Path) -> Result<(), CcbdError> {
    if path.exists() {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| home_err("create trust parent", parent, err))?;
    }
    fs::write(path, "{}\n").map_err(|err| home_err("write trust file", path, err))
}

fn ensure_claude_workspace_trust(path: &Path, workspace_key: &str) -> Result<(), CcbdError> {
    let mut root = read_json_object(path).unwrap_or_default();
    root.insert("trusted".to_string(), Value::Bool(true));
    let projects = object_entry(&mut root, "projects");
    remove_legacy_workspace_json_key(projects, workspace_key);
    let workspace = object_entry(projects, workspace_key);
    workspace.insert("hasTrustDialogAccepted".to_string(), Value::Bool(true));
    workspace
        .entry("allowedTools".to_string())
        .or_insert_with(|| Value::Array(vec![]));
    workspace
        .entry("mcpContextUris".to_string())
        .or_insert_with(|| Value::Array(vec![]));
    workspace
        .entry("mcpServers".to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    workspace
        .entry("enabledMcpjsonServers".to_string())
        .or_insert_with(|| Value::Array(vec![]));
    workspace
        .entry("disabledMcpjsonServers".to_string())
        .or_insert_with(|| Value::Array(vec![]));
    workspace
        .entry("projectOnboardingSeenCount".to_string())
        .or_insert_with(|| Value::from(1));
    write_json_object(path, &root)
}

fn object_entry<'a>(map: &'a mut Map<String, Value>, key: &str) -> &'a mut Map<String, Value> {
    let value = map
        .entry(key.to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    if !value.is_object() {
        *value = Value::Object(Map::new());
    }
    value
        .as_object_mut()
        .expect("value was normalized to object")
}

fn ensure_codex_workspace_trust(path: &Path, workspace_key: &str) -> Result<(), CcbdError> {
    let data = fs::read_to_string(path).unwrap_or_default();
    let mut root = data
        .parse::<TomlValue>()
        .unwrap_or_else(|_| TomlValue::Table(toml::map::Map::new()));
    let Some(root_table) = root.as_table_mut() else {
        root = TomlValue::Table(toml::map::Map::new());
        root.as_table_mut()
            .expect("root was normalized to table")
            .insert(
                "projects".to_string(),
                TomlValue::Table(toml::map::Map::new()),
            );
        write_codex_config(path, &root)?;
        return ensure_codex_workspace_trust(path, workspace_key);
    };
    let projects = table_entry(root_table, "projects");
    remove_legacy_workspace_toml_key(projects, workspace_key);
    let workspace = table_entry(projects, workspace_key);
    workspace.insert(
        "trust_level".to_string(),
        TomlValue::String("trusted".to_string()),
    );
    let tui = table_entry(root_table, "tui");
    let model_availability_nux = table_entry(tui, "model_availability_nux");
    model_availability_nux.insert("gpt-5.5".to_string(), TomlValue::Integer(4));
    write_codex_config(path, &root)
}

fn table_entry<'a>(
    table: &'a mut toml::map::Map<String, TomlValue>,
    key: &str,
) -> &'a mut toml::map::Map<String, TomlValue> {
    let value = table
        .entry(key.to_string())
        .or_insert_with(|| TomlValue::Table(toml::map::Map::new()));
    if !value.is_table() {
        *value = TomlValue::Table(toml::map::Map::new());
    }
    value.as_table_mut().expect("value was normalized to table")
}

fn remove_legacy_workspace_json_key(map: &mut Map<String, Value>, workspace_key: &str) {
    if workspace_key != "/home/agent" {
        map.remove("/home/agent");
    }
}

fn remove_legacy_workspace_toml_key(
    table: &mut toml::map::Map<String, TomlValue>,
    workspace_key: &str,
) {
    if workspace_key != "/home/agent" {
        table.remove("/home/agent");
    }
}

fn write_codex_config(path: &Path, root: &TomlValue) -> Result<(), CcbdError> {
    let data = toml::to_string_pretty(root).map_err(|err| CcbdError::EnvironmentNotSupported {
        details: format!("serialize codex config {}: {err}", path.display()),
    })?;
    fs::write(path, data).map_err(|err| home_err("write codex config", path, err))
}

fn ensure_json_file(path: &Path) -> Result<(), CcbdError> {
    if path.exists() {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| home_err("create json parent", parent, err))?;
    }
    fs::write(path, "{}\n").map_err(|err| home_err("write json file", path, err))
}

fn copy_if_missing(source: &Path, target: &Path) {
    if target.exists() || !source.is_file() {
        return;
    }
    let Some(parent) = target.parent() else {
        return;
    };
    if fs::create_dir_all(parent).is_err() {
        return;
    }
    let _ = fs::copy(source, target);
}

fn symlink_auth_file(source: &Path, target: &Path) {
    let Some(parent) = target.parent() else {
        return;
    };
    if fs::create_dir_all(parent).is_err() {
        return;
    }
    if target.is_symlink() || target.is_file() {
        let _ = fs::remove_file(target);
    } else if target.exists() {
        return;
    }
    #[cfg(unix)]
    {
        if let Err(err) = std::os::unix::fs::symlink(source, target) {
            tracing::warn!(
                source = %source.display(),
                target = %target.display(),
                %err,
                "failed to symlink provider auth file"
            );
        }
    }
}

fn copy_dynamic_auth_file(source: &Path, target: &Path) {
    let Some(parent) = target.parent() else {
        return;
    };
    if fs::create_dir_all(parent).is_err() {
        return;
    }
    if target.is_symlink() || target.is_file() {
        let _ = fs::remove_file(target);
    } else if target.exists() {
        return;
    }
    if let Err(err) = fs::copy(source, target) {
        tracing::warn!(
            source = %source.display(),
            target = %target.display(),
            %err,
            "failed to copy dynamic provider auth file"
        );
        return;
    }
    if let Err(err) = fs::set_permissions(target, fs::Permissions::from_mode(0o600)) {
        tracing::warn!(
            target = %target.display(),
            %err,
            "failed to set dynamic provider auth file permissions"
        );
    }
}

fn force_symlink(source: &Path, target: &Path) -> Result<(), CcbdError> {
    let Some(parent) = target.parent() else {
        return Err(CcbdError::EnvironmentNotSupported {
            details: format!("symlink target has no parent: {}", target.display()),
        });
    };
    fs::create_dir_all(parent).map_err(|err| home_err("create symlink parent", parent, err))?;
    if target.is_symlink() || target.is_file() {
        fs::remove_file(target)
            .map_err(|err| home_err("remove existing symlink target", target, err))?;
    } else if target.is_dir() {
        fs::remove_dir_all(target)
            .map_err(|err| home_err("remove existing symlink directory", target, err))?;
    }
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(source, target)
            .map_err(|err| home_err("create symlink", target, err))
    }
    #[cfg(not(unix))]
    {
        let _ = source;
        Err(CcbdError::EnvironmentNotSupported {
            details: "provider extension symlinks require unix".into(),
        })
    }
}

fn read_json_object(path: &Path) -> Option<Map<String, Value>> {
    let data = fs::read_to_string(path).ok()?;
    match serde_json::from_str::<Value>(&data).ok()? {
        Value::Object(map) => Some(map),
        _ => None,
    }
}

fn merge_object_payload(
    projected: Option<Map<String, Value>>,
    existing: Option<Map<String, Value>>,
) -> Option<Map<String, Value>> {
    let mut merged = projected.unwrap_or_default();
    if let Some(existing) = existing {
        merged.extend(existing);
    }
    if merged.is_empty() {
        None
    } else {
        Some(merged)
    }
}

fn write_json_object(path: &Path, payload: &Map<String, Value>) -> Result<(), CcbdError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| home_err("create json parent", parent, err))?;
    }
    let data = serde_json::to_string_pretty(payload).map_err(|err| {
        CcbdError::EnvironmentNotSupported {
            details: format!("serialize json {}: {err}", path.display()),
        }
    })? + "\n";
    fs::write(path, data).map_err(|err| home_err("write json object", path, err))
}

fn same_resolved_path(left: &Path, right: &Path) -> bool {
    match (left.canonicalize(), right.canonicalize()) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
}

fn home_err(action: &str, path: &Path, err: std::io::Error) -> CcbdError {
    CcbdError::EnvironmentNotSupported {
        details: format!("{action} {}: {err}", path.display()),
    }
}

struct ClaudeHomeLayout {
    claude_dir: PathBuf,
    projects_root: PathBuf,
    session_env_root: PathBuf,
    settings_path: PathBuf,
    trust_path: PathBuf,
    config_dir_state_path: PathBuf,
}

impl ClaudeHomeLayout {
    fn for_home(home_root: &Path) -> Self {
        let claude_dir = home_root.join(".claude");
        Self {
            claude_dir: claude_dir.clone(),
            projects_root: claude_dir.join("projects"),
            session_env_root: claude_dir.join("session-env"),
            settings_path: claude_dir.join("settings.json"),
            trust_path: home_root.join(".claude.json"),
            config_dir_state_path: claude_dir.join(".claude.json"),
        }
    }
}

struct GeminiHomeLayout {
    gemini_dir: PathBuf,
    settings_path: PathBuf,
    trusted_folders_path: PathBuf,
    tmp_root: PathBuf,
}

impl GeminiHomeLayout {
    fn for_home(home_root: &Path) -> Self {
        let gemini_dir = home_root.join(".gemini");
        Self {
            gemini_dir: gemini_dir.clone(),
            settings_path: gemini_dir.join("settings.json"),
            trusted_folders_path: gemini_dir.join("trustedFolders.json"),
            tmp_root: gemini_dir.join("tmp"),
        }
    }
}

struct AntigravityHomeLayout {
    antigravity_dir: PathBuf,
    cache_dir: PathBuf,
    settings_path: PathBuf,
    onboarding_path: PathBuf,
}

impl AntigravityHomeLayout {
    fn for_home(home_root: &Path) -> Self {
        let antigravity_dir = home_root.join(".gemini/antigravity-cli");
        let cache_dir = antigravity_dir.join("cache");
        Self {
            antigravity_dir: antigravity_dir.clone(),
            cache_dir: cache_dir.clone(),
            settings_path: antigravity_dir.join("settings.json"),
            onboarding_path: cache_dir.join("onboarding.json"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::resolve_materialization_source_home;
    use std::path::PathBuf;

    #[test]
    fn test_materialization_source_home_keeps_normal_home() {
        let env_home = PathBuf::from("/tmp/normal-home");
        let resolved = resolve_materialization_source_home(
            env_home.clone(),
            Some(PathBuf::from("/home/user")),
        );

        assert_eq!(resolved, env_home);
    }

    #[test]
    fn test_materialization_source_home_uses_passwd_home_from_nested_ccb_sandbox() {
        let env_home = PathBuf::from("/home/user/.cache/ccb/sandboxes/abc123");
        let passwd_home = PathBuf::from("/home/user");
        let resolved = resolve_materialization_source_home(env_home, Some(passwd_home.clone()));

        assert_eq!(resolved, passwd_home);
    }

    #[test]
    fn test_gemini_overrides_creates_state_and_settings_with_auth() {
        use super::{ExtensionConfig, HomeLayoutRole, prepare_gemini_overrides, read_json_object};
        use tempfile::TempDir;

        let source = TempDir::new().unwrap();
        let target = TempDir::new().unwrap();

        let source_gemini = source.path().join(".gemini");
        std::fs::create_dir_all(&source_gemini).unwrap();
        std::fs::write(
            source_gemini.join("settings.json"),
            r#"{"security":{"auth":{"selectedType":"oauth-personal"}}}"#,
        )
        .unwrap();
        std::fs::write(source_gemini.join("state.json"), r#"{"tipsShown":5}"#).unwrap();
        std::fs::write(source_gemini.join("trustedFolders.json"), "{}").unwrap();

        let workspace = TempDir::new().unwrap();
        let overrides = prepare_gemini_overrides(
            source.path(),
            target.path(),
            &workspace.path().display().to_string(),
            HomeLayoutRole::Worker,
            &ExtensionConfig::default(),
        )
        .unwrap();
        assert_eq!(overrides.home_root, target.path());

        let settings = read_json_object(&target.path().join(".gemini/settings.json")).unwrap();
        let auth_type = settings["security"]["auth"]["selectedType"]
            .as_str()
            .unwrap();
        assert_eq!(auth_type, "oauth-personal");

        assert!(target.path().join(".gemini/state.json").exists());
    }

    #[test]
    fn test_codex_overrides_creates_version_and_migration() {
        use super::{ExtensionConfig, HomeLayoutRole, prepare_codex_overrides};
        use tempfile::TempDir;

        let source = TempDir::new().unwrap();
        let target = TempDir::new().unwrap();

        let source_codex = source.path().join(".codex");
        std::fs::create_dir_all(&source_codex).unwrap();
        std::fs::write(source_codex.join("version.json"), r#"{"v":"1.0"}"#).unwrap();

        let workspace = TempDir::new().unwrap();
        let overrides = prepare_codex_overrides(
            source.path(),
            target.path(),
            &workspace.path().display().to_string(),
            HomeLayoutRole::Worker,
            &ExtensionConfig::default(),
        )
        .unwrap();
        assert_eq!(overrides.home_root, target.path());

        assert!(target.path().join(".codex/version.json").exists());
        assert!(target.path().join(".codex/.personality_migration").exists());
        assert!(target.path().join(".codex/config.toml").exists());
    }

    #[test]
    fn test_claude_settings_has_bypass_and_permissions() {
        use super::{ExtensionConfig, HomeLayoutRole, prepare_claude_overrides, read_json_object};
        use tempfile::TempDir;

        let source = TempDir::new().unwrap();
        let target = TempDir::new().unwrap();

        std::fs::write(source.path().join(".claude.json"), "{}").unwrap();

        let workspace = TempDir::new().unwrap();
        let _ = prepare_claude_overrides(
            source.path(),
            target.path(),
            &workspace.path().display().to_string(),
            HomeLayoutRole::Worker,
            &ExtensionConfig::default(),
        )
        .unwrap();

        let settings = read_json_object(&target.path().join(".claude/settings.json")).unwrap();
        assert_eq!(settings["skipDangerousModePermissionPrompt"], true);
        assert_eq!(settings["permissions"]["defaultMode"], "bypassPermissions");
    }

    #[test]
    fn test_claude_config_dir_receives_onboarding_state() {
        use super::{ExtensionConfig, HomeLayoutRole, prepare_claude_overrides, read_json_object};
        use tempfile::TempDir;

        let source = TempDir::new().unwrap();
        let target = TempDir::new().unwrap();

        std::fs::write(
            source.path().join(".claude.json"),
            r#"{"hasCompletedOnboarding":true,"lastOnboardingVersion":"2.1.116"}"#,
        )
        .unwrap();

        let workspace = TempDir::new().unwrap();
        let _ = prepare_claude_overrides(
            source.path(),
            target.path(),
            &workspace.path().display().to_string(),
            HomeLayoutRole::Worker,
            &ExtensionConfig::default(),
        )
        .unwrap();

        let config_dir_state =
            read_json_object(&target.path().join(".claude/.claude.json")).unwrap();
        assert_eq!(config_dir_state["hasCompletedOnboarding"], true);
        assert_eq!(config_dir_state["lastOnboardingVersion"], "2.1.116");

        let root_state = read_json_object(&target.path().join(".claude.json")).unwrap();
        assert_eq!(root_state["trusted"], true);
        assert_eq!(config_dir_state["trusted"], true);
    }
}
