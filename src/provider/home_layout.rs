use crate::error::CcbdError;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use toml::Value as TomlValue;

const SANDBOX_HOME: &str = "/home/agent";
// mvp12 M12.6 E5 r4: sandbox cwd inherits from parent process; bwrap has no --chdir,
// so providers actually run with cwd=/home/agent (the HOME bind). Pre-trust this path
// so codex/claude/gemini skip their first-run trust dialogs.
const WORKSPACE_PATH: &str = "/home/agent";
const WHITELIST: &[&str] = &[".ssh", ".gitconfig", ".git-credentials", ".netrc"];
const PROVIDER_AUTH_WHITELIST: &[&str] = &[
    ".claude.json",
    ".claude/.credentials.json",
    ".codex/auth.json",
    ".codex/installation_id",
    ".gemini/oauth_creds.json",
    ".gemini/google_accounts.json",
    ".gemini/installation_id",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HomeOverrides {
    pub home_root: PathBuf,
    pub extra_env: HashMap<String, String>,
}

pub fn prepare_home_layout(
    provider: &str,
    project_root: &Path,
) -> Result<HomeOverrides, CcbdError> {
    let source_home = materialization_source_home()?;
    let home_root = sandbox_home_for_project_root(project_root)?;
    fs::create_dir_all(&home_root)
        .map_err(|err| home_err("create sandbox home", &home_root, err))?;

    let overrides = match provider {
        "claude" => prepare_claude_overrides(&source_home, &home_root),
        "gemini" => prepare_gemini_overrides(&source_home, &home_root),
        "codex" => prepare_codex_overrides(&home_root),
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
) -> Result<HomeOverrides, CcbdError> {
    let layout = ClaudeHomeLayout::for_home(home_root);
    fs::create_dir_all(&layout.claude_dir)
        .map_err(|err| home_err("create claude dir", &layout.claude_dir, err))?;
    fs::create_dir_all(&layout.projects_root)
        .map_err(|err| home_err("create claude projects", &layout.projects_root, err))?;
    fs::create_dir_all(&layout.session_env_root)
        .map_err(|err| home_err("create claude session env", &layout.session_env_root, err))?;
    materialize_trust(source_home, &layout)?;
    materialize_claude_settings(source_home, &layout)?;
    copy_credentials(source_home, &layout);

    Ok(HomeOverrides {
        home_root: home_root.to_path_buf(),
        extra_env: HashMap::from([
            (
                "CLAUDE_PROJECTS_ROOT".to_string(),
                sandbox_path(".claude/projects"),
            ),
            (
                "CLAUDE_PROJECT_ROOT".to_string(),
                sandbox_path(".claude/projects"),
            ),
        ]),
    })
}

fn prepare_gemini_overrides(
    source_home: &Path,
    home_root: &Path,
) -> Result<HomeOverrides, CcbdError> {
    let layout = GeminiHomeLayout::for_home(home_root);
    fs::create_dir_all(&layout.gemini_dir)
        .map_err(|err| home_err("create gemini dir", &layout.gemini_dir, err))?;
    fs::create_dir_all(&layout.tmp_root)
        .map_err(|err| home_err("create gemini tmp", &layout.tmp_root, err))?;
    ensure_json_file(&layout.settings_path)?;
    ensure_json_file(&layout.trusted_folders_path)?;
    materialize_gemini_settings(source_home, &layout)?;
    materialize_trusted_folders(source_home, &layout)?;

    Ok(HomeOverrides {
        home_root: home_root.to_path_buf(),
        extra_env: HashMap::from([("GEMINI_ROOT".to_string(), sandbox_path(".gemini/tmp"))]),
    })
}

fn prepare_codex_overrides(home_root: &Path) -> Result<HomeOverrides, CcbdError> {
    let codex_home = home_root.join(".codex");
    prepare_managed_codex_home(&codex_home)?;
    Ok(HomeOverrides {
        home_root: home_root.to_path_buf(),
        extra_env: HashMap::from([
            ("CODEX_HOME".to_string(), sandbox_path(".codex")),
            (
                "CODEX_SESSION_ROOT".to_string(),
                sandbox_path(".codex/sessions"),
            ),
        ]),
    })
}

fn materialize_sandbox_home_links(source_home: &Path, home_root: &Path) {
    for relative in WHITELIST {
        link_into_sandbox(source_home, home_root, relative);
    }
    for relative in PROVIDER_AUTH_WHITELIST {
        copy_into_sandbox(source_home, home_root, relative);
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
        let _ = std::os::unix::fs::symlink(&source, &target);
    }
}

fn copy_into_sandbox(source_home: &Path, home_root: &Path, relative: &str) {
    let source = source_home.join(relative);
    if !source.is_file() {
        return;
    }
    let target = home_root.join(relative);
    copy_auth_file_if_missing_or_symlink(&source, &target);
}

fn materialize_trust(source_home: &Path, layout: &ClaudeHomeLayout) -> Result<(), CcbdError> {
    let source_trust = source_home.join(".claude.json");
    if !layout.trust_path.exists() && source_trust.is_file() {
        copy_if_missing(&source_trust, &layout.trust_path);
    }
    ensure_trust_file(&layout.trust_path)?;
    ensure_claude_workspace_trust(&layout.trust_path)
}

fn copy_credentials(source_home: &Path, layout: &ClaudeHomeLayout) {
    let source = source_home.join(".claude/.credentials.json");
    if !source.is_file() {
        return;
    }
    let target = layout.claude_dir.join(".credentials.json");
    copy_auth_file_if_missing_or_symlink(&source, &target);
}

fn materialize_trusted_folders(
    source_home: &Path,
    layout: &GeminiHomeLayout,
) -> Result<(), CcbdError> {
    let projected = read_json_object(&source_home.join(".gemini/trustedFolders.json"));
    let existing = read_json_object(&layout.trusted_folders_path);
    let mut merged = merge_object_payload(projected, existing).unwrap_or_default();
    merged.insert(
        WORKSPACE_PATH.to_string(),
        Value::String("TRUST_FOLDER".to_string()),
    );
    write_json_object(&layout.trusted_folders_path, &merged)
}

fn materialize_gemini_settings(
    source_home: &Path,
    layout: &GeminiHomeLayout,
) -> Result<(), CcbdError> {
    let source_settings = source_home.join(".gemini/settings.json");
    if source_settings.is_file() {
        fs::copy(&source_settings, &layout.settings_path)
            .map_err(|err| home_err("copy gemini settings", &layout.settings_path, err))?;
    }
    Ok(())
}

fn materialize_claude_settings(
    _source_home: &Path,
    layout: &ClaudeHomeLayout,
) -> Result<(), CcbdError> {
    ensure_json_file(&layout.settings_path)?;
    let mut settings = read_json_object(&layout.settings_path).unwrap_or_default();
    // Claude stores the Bypass Permissions confirmation separately from
    // .claude.json project trust. Mirror only this first-run state bit; copying
    // the whole host settings file would also copy hooks that point at files not
    // materialized under the sandbox HOME.
    settings.insert(
        "skipDangerousModePermissionPrompt".to_string(),
        Value::Bool(true),
    );
    write_json_object(&layout.settings_path, &settings)
}

fn prepare_managed_codex_home(codex_home: &Path) -> Result<(), CcbdError> {
    fs::create_dir_all(codex_home).map_err(|err| home_err("create codex home", codex_home, err))?;
    let session_root = codex_home.join("sessions");
    fs::create_dir_all(&session_root)
        .map_err(|err| home_err("create codex sessions", &session_root, err))?;
    let target_config = codex_home.join("config.toml");
    if !target_config.exists() {
        fs::write(&target_config, "# ccb agent-local codex config\n")
            .map_err(|err| home_err("write codex config", &target_config, err))?;
    }
    ensure_codex_workspace_trust(&target_config)
}

fn sandbox_home_for_project_root(project_root: &Path) -> Result<PathBuf, CcbdError> {
    let project_path = project_root
        .canonicalize()
        .unwrap_or_else(|_| project_root.to_path_buf());
    let mut hasher = Sha256::new();
    hasher.update(project_path.to_string_lossy().as_bytes());
    let digest = hasher.finalize();
    let project_id_short = digest
        .iter()
        .take(6)
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    Ok(xdg_cache_root()?
        .join("ccb-rs/sandboxes")
        .join(project_id_short))
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

fn resolve_materialization_source_home(
    env_home: PathBuf,
    passwd_home: Option<PathBuf>,
) -> PathBuf {
    if is_ccb_sandbox_home(&env_home) {
        if let Some(passwd_home) = passwd_home {
            return passwd_home;
        }
    }
    env_home
}

fn is_ccb_sandbox_home(path: &Path) -> bool {
    let path = path.to_string_lossy();
    path.contains("/.cache/ccb/sandboxes/") || path.contains("/.cache/ccb-rs/sandboxes/")
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

fn sandbox_path(relative: &str) -> String {
    Path::new(SANDBOX_HOME).join(relative).display().to_string()
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

fn ensure_claude_workspace_trust(path: &Path) -> Result<(), CcbdError> {
    let mut root = read_json_object(path).unwrap_or_default();
    let projects = object_entry(&mut root, "projects");
    let workspace = object_entry(projects, WORKSPACE_PATH);
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

fn ensure_codex_workspace_trust(path: &Path) -> Result<(), CcbdError> {
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
        return ensure_codex_workspace_trust(path);
    };
    let projects = table_entry(root_table, "projects");
    let workspace = table_entry(projects, WORKSPACE_PATH);
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

fn copy_auth_file_if_missing_or_symlink(source: &Path, target: &Path) {
    let Some(parent) = target.parent() else {
        return;
    };
    if fs::create_dir_all(parent).is_err() {
        return;
    }
    if target.is_symlink() {
        if fs::remove_file(target).is_err() {
            return;
        }
    } else if target.exists() {
        return;
    }
    let _ = fs::copy(source, target);
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
}
