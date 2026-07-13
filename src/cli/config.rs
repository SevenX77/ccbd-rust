use crate::cli::rpc_client::CliError;
pub use crate::provider::extensions::{ExtensionConfig, HookGroup, HookItem};
use crate::provider::manifest::{
    canonicalize_provider_name, is_valid_provider, unknown_provider_message,
};
use crate::provider::skills::parse_skill_refs;
use crate::tmux::TmuxWindowSize;
use serde::Deserialize;
use std::collections::{BTreeMap, HashMap};
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Deserialize)]
pub struct ProjectConfig {
    pub version: String,
    #[serde(default)]
    pub master: MasterConfig,
    #[serde(default)]
    pub completion: CompletionConfig,
    #[serde(default)]
    pub daemon: DaemonConfig,
    #[serde(default)]
    pub providers: ProviderConfigs,
    #[serde(default)]
    pub env: HashMap<String, String>,
    #[serde(default)]
    pub sandbox: SandboxConfig,
    pub agents: BTreeMap<String, AgentConfig>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct CompletionConfig {
    #[serde(default)]
    pub hook_push_enabled: bool,
}

impl Default for CompletionConfig {
    fn default() -> Self {
        Self {
            hook_push_enabled: false,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct MasterConfig {
    #[serde(
        default = "default_master_cmd",
        deserialize_with = "deserialize_master_cmd"
    )]
    pub cmd: String,
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default = "default_master_readiness_timeout_s")]
    pub readiness_timeout_s: u64,
    #[serde(default = "default_master_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub window_size: TmuxWindowSize,
    #[serde(default)]
    pub hooks: HashMap<String, Vec<HookGroup>>,
    #[serde(default)]
    pub plugins: Vec<String>,
    #[serde(default)]
    pub skills: Vec<String>,
    #[serde(default, deserialize_with = "deserialize_bundle_refs")]
    pub bundle: Vec<String>,
    #[serde(default)]
    pub settings: serde_json::Map<String, serde_json::Value>,
}

impl Default for MasterConfig {
    fn default() -> Self {
        Self {
            cmd: default_master_cmd(),
            provider: None,
            readiness_timeout_s: default_master_readiness_timeout_s(),
            enabled: default_master_enabled(),
            window_size: TmuxWindowSize::Fixed,
            hooks: HashMap::new(),
            plugins: Vec::new(),
            skills: Vec::new(),
            bundle: Vec::new(),
            settings: serde_json::Map::new(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct DaemonConfig {}

impl Default for DaemonConfig {
    fn default() -> Self {
        Self {}
    }
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

#[derive(Debug, Clone, Deserialize)]
pub struct AgentConfig {
    pub provider: String,
    #[serde(default)]
    pub env: HashMap<String, String>,
    #[serde(default)]
    pub hooks: HashMap<String, Vec<HookGroup>>,
    #[serde(default)]
    pub plugins: Vec<String>,
    #[serde(default)]
    pub skills: Vec<String>,
    #[serde(default, deserialize_with = "deserialize_bundle_refs")]
    pub bundle: Vec<String>,
    #[serde(default)]
    pub settings: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct SandboxConfig {
    #[serde(default)]
    pub additional_ro_binds: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiagnosticSeverity {
    Error,
    Warning,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub severity: DiagnosticSeverity,
    pub message: String,
}

pub fn load_project_config(path: &Path) -> Result<ProjectConfig, CliError> {
    let raw = fs::read_to_string(path).map_err(|err| {
        CliError::Config(format!("failed to read config {}: {err}", path.display()))
    })?;
    reject_removed_layout_field(&raw)?;
    let mut config: ProjectConfig = toml::from_str(&raw)?;
    canonicalize_project_config_providers(&mut config);
    let diagnostics = validate_project_config(&config);
    if let Some(diagnostic) = diagnostics
        .iter()
        .find(|diagnostic| diagnostic.severity == DiagnosticSeverity::Error)
    {
        return Err(CliError::Config(diagnostic.message.clone()));
    }
    Ok(config)
}

pub fn find_config(start_dir: &Path) -> Result<PathBuf, CliError> {
    find_config_with_env(start_dir, std::env::var_os("CCB_CONFIG_PATH"))
}

pub fn validate_project_config(config: &ProjectConfig) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    if config.version != "1" {
        diagnostics.push(error("ah.toml version must be \"1\""));
    }
    if !config.sandbox.additional_ro_binds.is_empty() {
        diagnostics.push(error(
            "additional_ro_binds is not supported because systemd-run --scope does not accept BindReadOnlyPaths (which is a service-unit-only property)"
        ));
    }
    if config.agents.is_empty() {
        diagnostics.push(error("ah.toml must define at least one [agents.<id>]"));
    }
    if let Err(err) = parse_skill_refs(&config.master.skills) {
        diagnostics.push(error(format!("invalid master skills: {err}")));
    }
    if let Err(err) = validate_bundle_refs(&config.master.bundle) {
        diagnostics.push(error(format!("invalid master bundle: {err}")));
    }
    if !config.master.bundle.is_empty()
        && config
            .master
            .provider
            .as_deref()
            .is_some_and(|provider| provider != "claude")
    {
        diagnostics.push(error(
            "master uses bundle but PR-1 supports bundles only for provider claude",
        ));
    }
    if !config.master.settings.is_empty()
        && config
            .master
            .provider
            .as_deref()
            .is_some_and(|provider| provider != "claude")
    {
        diagnostics.push(error(format!(
            "provider settings are only supported for the 'claude' provider today; master uses '{}'",
            config.master.provider.as_deref().unwrap_or_default()
        )));
    }
    validate_claude_provider_config(config, &mut diagnostics);
    for (agent_id, agent) in &config.agents {
        if !is_valid_agent_id(agent_id) {
            diagnostics.push(error(format!(
                "invalid agent id {agent_id:?}; use ASCII alphanumeric, '_' or '-'"
            )));
        }
        if agent.provider.trim().is_empty() {
            diagnostics.push(error(format!(
                "agent {agent_id:?} must define a non-empty provider"
            )));
        } else if !is_valid_provider(&agent.provider) {
            diagnostics.push(error(format!(
                "agent {agent_id:?} has {}; fix provider spelling",
                unknown_provider_message(&agent.provider)
            )));
        }
        if let Err(err) = parse_skill_refs(&agent.skills) {
            diagnostics.push(error(format!(
                "agent {agent_id:?} has invalid skills: {err}"
            )));
        }
        if let Err(err) = validate_bundle_refs(&agent.bundle) {
            diagnostics.push(error(format!(
                "agent {agent_id:?} has invalid bundle: {err}"
            )));
        }
        if !agent.settings.is_empty() && agent.provider != "claude" {
            diagnostics.push(error(format!(
                "provider settings are only supported for the 'claude' provider today; agent '{agent_id}' uses '{}'",
                agent.provider
            )));
        }
    }
    diagnostics
}

fn validate_claude_provider_config(config: &ProjectConfig, diagnostics: &mut Vec<Diagnostic>) {
    if let Some(path) = config.providers.claude.shared_credentials_dir.as_ref() {
        if path.as_os_str().is_empty() || path.as_os_str().to_string_lossy().trim().is_empty() {
            diagnostics.push(error(
                "providers.claude.shared_credentials_dir must be a non-empty absolute path",
            ));
        } else if !path.is_absolute() {
            diagnostics.push(error(
                "providers.claude.shared_credentials_dir must be an absolute path",
            ));
        }
    }

    if config_uses_claude_provider(config)
        && config.providers.claude.shared_credentials_dir.is_none()
    {
        diagnostics.push(error(
            "providers.claude.shared_credentials_dir is required when master or agents use provider claude",
        ));
    }
}

fn config_uses_claude_provider(config: &ProjectConfig) -> bool {
    let master_uses_claude = config.master.enabled
        && config
            .master
            .provider
            .as_deref()
            .map(|provider| provider == "claude")
            .unwrap_or_else(|| {
                config
                    .master
                    .cmd
                    .split_whitespace()
                    .next()
                    .is_some_and(|cmd| cmd == "claude")
            });
    master_uses_claude
        || config
            .agents
            .values()
            .any(|agent| agent.provider == "claude")
}

fn deserialize_bundle_refs<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum BundleInput {
        Single(String),
        Many(Vec<String>),
    }

    match Option::<BundleInput>::deserialize(deserializer)? {
        Some(BundleInput::Single(value)) => Ok(vec![value]),
        Some(BundleInput::Many(values)) => Ok(values),
        None => Ok(Vec::new()),
    }
}

fn validate_bundle_refs(bundle: &[String]) -> Result<(), String> {
    for name in bundle {
        if name.is_empty() {
            return Err("bundle name must not be empty".to_string());
        }
        let path = Path::new(name);
        if path.is_absolute()
            || name.contains('\\')
            || path.components().count() != 1
            || !path
                .components()
                .all(|component| matches!(component, std::path::Component::Normal(_)))
        {
            return Err(format!(
                "invalid bundle name {name:?}; use a single directory name"
            ));
        }
    }
    Ok(())
}

pub(crate) fn find_config_with_env(
    start_dir: &Path,
    env_path: Option<OsString>,
) -> Result<PathBuf, CliError> {
    if let Some(path) = env_path {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Ok(path);
        }
        return Err(CliError::Config(format!(
            "CCB_CONFIG_PATH points to missing config: {}",
            path.display()
        )));
    }

    let mut current = if start_dir.is_file() {
        start_dir.parent()
    } else {
        Some(start_dir)
    }
    .ok_or_else(|| CliError::Config(format!("invalid start dir: {}", start_dir.display())))?
    .to_path_buf();

    loop {
        let candidate = current.join("ah.toml");
        if candidate.is_file() {
            return Ok(candidate);
        }
        if !current.pop() {
            break;
        }
    }

    Err(CliError::Config(format!(
        "could not find ah.toml from {}; create one or set CCB_CONFIG_PATH",
        start_dir.display()
    )))
}

fn default_master_cmd() -> String {
    "claude".into()
}

fn default_master_readiness_timeout_s() -> u64 {
    120
}

fn deserialize_master_cmd<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let cmd = String::deserialize(deserializer)?;
    if cmd.trim().is_empty() {
        Ok("claude".to_string())
    } else {
        Ok(cmd)
    }
}

fn default_master_enabled() -> bool {
    true
}

fn reject_removed_layout_field(raw: &str) -> Result<(), CliError> {
    let value = raw.parse::<toml::Value>()?;
    if value
        .as_table()
        .is_some_and(|table| table.contains_key("layout"))
    {
        return Err(CliError::Config(
            "layout config was removed; omit the top-level layout field".into(),
        ));
    }
    Ok(())
}

fn canonicalize_project_config_providers(config: &mut ProjectConfig) {
    if let Some(provider) = config.master.provider.as_mut() {
        let canonical = canonicalize_provider_name(provider);
        if canonical != provider {
            *provider = canonical.to_string();
        }
    }
    for agent in config.agents.values_mut() {
        let canonical = canonicalize_provider_name(&agent.provider);
        if canonical != agent.provider {
            agent.provider = canonical.to_string();
        }
    }
}

fn is_valid_agent_id(agent_id: &str) -> bool {
    !agent_id.is_empty()
        && agent_id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
}

fn error(message: impl Into<String>) -> Diagnostic {
    Diagnostic {
        severity: DiagnosticSeverity::Error,
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CompletionConfig, DaemonConfig, DiagnosticSeverity, MasterConfig, find_config_with_env,
        load_project_config,
    };
    use crate::tmux::TmuxWindowSize;
    use std::ffi::OsString;

    #[test]
    fn test_load_valid_config_without_layout() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("ah.toml");
        std::fs::write(
            &path,
            r#"
version = "1"

[master]
enabled = false

[agents.a1]
provider = "bash"
"#,
        )
        .unwrap();

        let config = load_project_config(&path).unwrap();

        assert_eq!(config.agents["a1"].provider, "bash");
        assert!(config.sandbox.additional_ro_binds.is_empty());
        assert!(!config.completion.hook_push_enabled);
    }

    #[test]
    fn test_load_project_config_canonicalizes_gemini_provider_alias() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("ah.toml");
        std::fs::write(
            &path,
            r#"
version = "1"

[master]
provider = "gemini"

[agents.a1]
provider = "gemini"
"#,
        )
        .unwrap();

        let config = load_project_config(&path).unwrap();

        assert_eq!(config.master.provider.as_deref(), Some("antigravity"));
        assert_eq!(config.agents["a1"].provider, "antigravity");
    }

    #[test]
    fn parses_claude_shared_credentials_dir_config() {
        let config = toml::from_str::<super::ProjectConfig>(
            r#"
version = "1"

[master]
enabled = false

[providers.claude]
shared_credentials_dir = "/tmp/user/.claude"

[agents.a1]
provider = "bash"
"#,
        )
        .unwrap();

        assert_eq!(
            config.providers.claude.shared_credentials_dir.as_deref(),
            Some(std::path::Path::new("/tmp/user/.claude"))
        );
        assert!(super::validate_project_config(&config).is_empty());
    }

    #[test]
    fn rejects_empty_claude_shared_credentials_dir_config() {
        let config = toml::from_str::<super::ProjectConfig>(
            r#"
version = "1"

[master]
enabled = false

[providers.claude]
shared_credentials_dir = ""

[agents.a1]
provider = "bash"
"#,
        )
        .unwrap();

        let diagnostics = super::validate_project_config(&config);
        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic.message.contains("shared_credentials_dir")
                && diagnostic.message.contains("non-empty")
        }));
    }

    #[test]
    fn rejects_relative_claude_shared_credentials_dir_config() {
        let config = toml::from_str::<super::ProjectConfig>(
            r#"
version = "1"

[master]
enabled = false

[providers.claude]
shared_credentials_dir = ".claude"

[agents.a1]
provider = "bash"
"#,
        )
        .unwrap();

        let diagnostics = super::validate_project_config(&config);
        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic.message.contains("shared_credentials_dir")
                && diagnostic.message.contains("absolute")
        }));
    }

    #[test]
    fn rejects_claude_provider_without_shared_credentials_dir_config() {
        let config = toml::from_str::<super::ProjectConfig>(
            r#"
version = "1"

[master]
enabled = false

[agents.a1]
provider = "claude"
"#,
        )
        .unwrap();

        let diagnostics = super::validate_project_config(&config);
        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic.message.contains("shared_credentials_dir")
                && diagnostic.message.contains("required")
        }));
    }

    #[test]
    fn non_claude_only_config_does_not_require_shared_credentials_dir() {
        let config = toml::from_str::<super::ProjectConfig>(
            r#"
version = "1"

[master]
enabled = false

[agents.a1]
provider = "bash"
"#,
        )
        .unwrap();

        assert!(super::validate_project_config(&config).is_empty());
    }

    #[test]
    fn completion_hook_push_enabled_defaults_false() {
        let config = toml::from_str::<super::ProjectConfig>(
            r#"
version = "1"

[agents.a1]
provider = "bash"
"#,
        )
        .unwrap();

        assert_eq!(config.completion, CompletionConfig::default());
        assert!(!config.completion.hook_push_enabled);
    }

    #[test]
    fn completion_hook_push_enabled_reads_true() {
        let config = toml::from_str::<super::ProjectConfig>(
            r#"
version = "1"

[completion]
hook_push_enabled = true

[agents.a1]
provider = "bash"
"#,
        )
        .unwrap();

        assert!(config.completion.hook_push_enabled);
    }

    #[test]
    fn test_load_project_config_with_sandbox_additional_ro_binds() {
        let config = toml::from_str::<super::ProjectConfig>(
            r#"
version = "1"

[sandbox]
additional_ro_binds = ["/opt/tools", "/var/cache/models"]

[agents.a1]
provider = "bash"
"#,
        )
        .unwrap();

        assert_eq!(
            config.sandbox.additional_ro_binds,
            vec!["/opt/tools", "/var/cache/models"]
        );
    }

    #[test]
    fn test_validate_project_config_rejects_scope_incompatible_ro_binds() {
        let config = toml::from_str::<super::ProjectConfig>(
            r#"
version = "1"

[sandbox]
additional_ro_binds = ["/opt/tools"]

[agents.a1]
provider = "bash"
"#,
        )
        .unwrap();

        let diagnostics = super::validate_project_config(&config);

        assert!(
            diagnostics.iter().any(|diagnostic| {
                diagnostic.severity == DiagnosticSeverity::Error && {
                    let message = diagnostic.message.to_lowercase();
                    message.contains("additional_ro_binds") && message.contains("scope")
                }
            }),
            "{diagnostics:?}"
        );
    }

    #[test]
    fn test_load_project_config_rejects_scope_incompatible_ro_binds() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("ah.toml");
        std::fs::write(
            &path,
            r#"
version = "1"

[sandbox]
additional_ro_binds = ["/opt/tools"]

[agents.a1]
provider = "bash"
"#,
        )
        .unwrap();

        let err = load_project_config(&path).unwrap_err();
        let message = err.to_string().to_lowercase();

        assert!(message.contains("additional_ro_binds"), "{message}");
        assert!(message.contains("scope"), "{message}");
    }

    #[test]
    fn test_load_project_config_reads_provider_settings() {
        let config = toml::from_str::<super::ProjectConfig>(
            r#"
version = "1"

[master.settings]
model = "claude-opus-4-20250514"
autoCompact = false

[master.settings.statusLine]
type = "command"
command = "ah ps --format compact"

[agents.a1]
provider = "claude"

[agents.a1.settings]
model = "claude-sonnet-4-20250514"
autoCompact = true
"#,
        )
        .unwrap();

        assert_eq!(
            config.master.settings["model"],
            serde_json::json!("claude-opus-4-20250514")
        );
        assert_eq!(
            config.master.settings["autoCompact"],
            serde_json::json!(false)
        );
        assert_eq!(
            config.master.settings["statusLine"]["command"],
            serde_json::json!("ah ps --format compact")
        );
        assert_eq!(
            config.agents["a1"].settings["model"],
            serde_json::json!("claude-sonnet-4-20250514")
        );
    }

    #[test]
    fn test_load_project_config_rejects_non_claude_provider_settings() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("ah.toml");
        std::fs::write(
            &path,
            r#"
version = "1"

[master]
enabled = false

[agents.a1]
provider = "codex"

[agents.a1.settings]
model = "claude-sonnet-4-20250514"
"#,
        )
        .unwrap();

        let err = load_project_config(&path).unwrap_err().to_string();

        assert!(
            err.contains("provider settings are only supported for the 'claude' provider today")
        );
        assert!(err.contains("agent 'a1' uses 'codex'"));
    }

    #[test]
    fn test_load_project_config_accepts_claude_and_default_master_settings() {
        let dir = tempfile::TempDir::new().unwrap();
        let shared_credentials_dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("ah.toml");
        std::fs::write(
            &path,
            format!(
                r#"
version = "1"

[providers.claude]
shared_credentials_dir = "{}"

[master.settings]
model = "claude-opus-4-20250514"

[agents.a1]
provider = "claude"

[agents.a1.settings]
model = "claude-sonnet-4-20250514"
"#,
                shared_credentials_dir.path().display()
            ),
        )
        .unwrap();

        let config = load_project_config(&path).unwrap();

        assert_eq!(
            config.master.settings["model"],
            serde_json::json!("claude-opus-4-20250514")
        );
        assert_eq!(
            config.agents["a1"].settings["model"],
            serde_json::json!("claude-sonnet-4-20250514")
        );
    }

    #[test]
    fn test_master_config_default() {
        let master = MasterConfig::default();

        assert!(master.enabled);
        assert_eq!(master.cmd, "claude");
        assert_eq!(master.window_size, TmuxWindowSize::Fixed);
    }

    #[test]
    fn test_daemon_config_default() {
        let daemon = DaemonConfig::default();

        assert_eq!(daemon, DaemonConfig {});
    }

    #[test]
    fn test_load_project_config_default_daemon_when_missing() {
        let config = toml::from_str::<super::ProjectConfig>(
            r#"
version = "1"

[agents.a1]
provider = "bash"
"#,
        )
        .unwrap();

        assert_eq!(config.daemon, DaemonConfig {});
    }

    #[test]
    fn test_load_project_config_with_master_section() {
        let config = toml::from_str::<super::ProjectConfig>(
            r#"
version = "1"

[master]
cmd = "opencode"
enabled = false

[agents.a1]
provider = "bash"
"#,
        )
        .unwrap();

        assert!(!config.master.enabled);
        assert_eq!(config.master.cmd, "opencode");
    }

    #[test]
    fn test_load_project_config_reads_master_follow_window_size() {
        let config = toml::from_str::<super::ProjectConfig>(
            r#"
version = "1"

[master]
window_size = "follow"

[agents.a1]
provider = "bash"
"#,
        )
        .unwrap();

        assert_eq!(config.master.window_size, TmuxWindowSize::Follow);
    }

    #[test]
    fn test_load_project_config_reads_master_and_agent_skills() {
        let shared_credentials_dir = tempfile::TempDir::new().unwrap();
        let config = toml::from_str::<super::ProjectConfig>(&format!(
            r#"
version = "1"

[providers.claude]
shared_credentials_dir = "{}"

[master]
skills = ["master-domain"]

[agents.a1]
provider = "claude"
skills = ["worker-domain"]
"#,
            shared_credentials_dir.path().display()
        ))
        .unwrap();

        assert_eq!(config.master.skills, vec!["master-domain"]);
        assert_eq!(config.agents["a1"].skills, vec!["worker-domain"]);
        assert!(super::validate_project_config(&config).is_empty());
    }

    #[test]
    fn test_load_project_config_reads_bundle_string_and_list() {
        let shared_credentials_dir = tempfile::TempDir::new().unwrap();
        let config = toml::from_str::<super::ProjectConfig>(&format!(
            r#"
version = "1"

[providers.claude]
shared_credentials_dir = "{}"

[master]
bundle = "domain"

[agents.a1]
provider = "claude"
bundle = ["domain", "team"]
"#,
            shared_credentials_dir.path().display()
        ))
        .unwrap();

        assert_eq!(config.master.bundle, vec!["domain"]);
        assert_eq!(config.agents["a1"].bundle, vec!["domain", "team"]);
        assert!(super::validate_project_config(&config).is_empty());
    }

    #[test]
    fn test_allows_bundle_refs_for_non_claude_provider() {
        let config = toml::from_str::<super::ProjectConfig>(
            r#"
version = "1"

[master]
enabled = false

[agents.a1]
provider = "codex"
bundle = "domain"
"#,
        )
        .unwrap();

        let diagnostics = super::validate_project_config(&config);

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn test_load_project_config_default_master_when_missing() {
        let config = toml::from_str::<super::ProjectConfig>(
            r#"
version = "1"

[agents.a1]
provider = "bash"
"#,
        )
        .unwrap();

        assert!(config.master.enabled);
        assert_eq!(config.master.cmd, "claude");
    }

    #[test]
    fn test_load_project_config_empty_master_cmd_normalizes_to_claude() {
        let config = toml::from_str::<super::ProjectConfig>(
            r#"
version = "1"

[master]
cmd = "   "

[agents.a1]
provider = "bash"
"#,
        )
        .unwrap();

        assert_eq!(config.master.cmd, "claude");
    }

    #[test]
    fn test_rejects_removed_layout_field() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("ah.toml");
        std::fs::write(
            &path,
            r#"
version = "1"
layout = "diagonal"

[agents.a1]
provider = "bash"
"#,
        )
        .unwrap();

        let err = load_project_config(&path).unwrap_err();

        assert!(err.to_string().contains("layout config was removed"));
    }

    #[test]
    fn test_rejects_removed_grid_layout() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("ah.toml");
        std::fs::write(
            &path,
            r#"
version = "1"
layout = "grid"

[agents.a1]
provider = "bash"
"#,
        )
        .unwrap();

        let err = load_project_config(&path).unwrap_err();

        assert!(err.to_string().contains("layout config was removed"));
    }

    #[test]
    fn test_rejects_empty_agents() {
        let config = toml::from_str::<super::ProjectConfig>(
            r#"
version = "1"
[agents]
"#,
        )
        .unwrap();

        let diagnostics = super::validate_project_config(&config);

        assert_eq!(diagnostics[0].severity, DiagnosticSeverity::Error);
        assert!(diagnostics[0].message.contains("at least one"));
    }

    #[test]
    fn test_rejects_bad_agent_id() {
        let config = toml::from_str::<super::ProjectConfig>(
            r#"
version = "1"

[agents."bad/id"]
provider = "bash"
"#,
        )
        .unwrap();

        let diagnostics = super::validate_project_config(&config);

        assert!(diagnostics.iter().any(|d| d.message.contains("bad/id")));
    }

    #[test]
    fn test_rejects_unknown_provider_with_valid_values() {
        let config = toml::from_str::<super::ProjectConfig>(
            r#"
version = "1"

[master]
enabled = false

[agents.a1]
provider = "claud"
"#,
        )
        .unwrap();

        let diagnostics = super::validate_project_config(&config);
        let message = diagnostics
            .iter()
            .find(|diagnostic| diagnostic.severity == DiagnosticSeverity::Error)
            .map(|diagnostic| diagnostic.message.as_str())
            .unwrap_or("");

        assert!(message.contains("claud"), "{message}");
        for provider in ["bash", "codex", "claude", "antigravity"] {
            assert!(message.contains(provider), "{message}");
        }
    }

    #[test]
    fn test_accepts_skills_for_codex_provider() {
        let config = toml::from_str::<super::ProjectConfig>(
            r#"
version = "1"

[master]
enabled = false

[agents.a1]
provider = "codex"
skills = ["domain"]
"#,
        )
        .unwrap();

        let diagnostics = super::validate_project_config(&config);
        assert!(
            !diagnostics
                .iter()
                .any(|diagnostic| diagnostic.severity == DiagnosticSeverity::Error),
            "{diagnostics:?}"
        );
    }

    #[test]
    fn test_load_project_config_rejects_unknown_provider() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("ah.toml");
        std::fs::write(
            &path,
            r#"
version = "1"

[master]
enabled = false

[agents.a1]
provider = "coddex"
"#,
        )
        .unwrap();

        let err = load_project_config(&path).unwrap_err();
        let message = err.to_string();

        assert!(message.contains("coddex"), "{message}");
        assert!(message.contains("codex"), "{message}");
        assert!(message.contains("claude"), "{message}");
        assert!(message.contains("antigravity"), "{message}");
        assert!(message.contains("bash"), "{message}");
    }

    #[test]
    fn test_find_config_walks_up_from_cwd() {
        let root = tempfile::TempDir::new().unwrap();
        let nested = root.path().join("a/b");
        std::fs::create_dir_all(&nested).unwrap();
        let config = root.path().join("ah.toml");
        std::fs::write(
            &config,
            "version = \"1\"\n[agents.a1]\nprovider = \"bash\"\n",
        )
        .unwrap();

        let found = find_config_with_env(&nested, None).unwrap();

        assert_eq!(found, config);
    }

    #[test]
    fn test_find_config_prefers_env_path() {
        let root = tempfile::TempDir::new().unwrap();
        let env_config = root.path().join("custom.toml");
        std::fs::write(
            &env_config,
            "version = \"1\"\n[agents.env]\nprovider = \"bash\"\n",
        )
        .unwrap();
        std::fs::write(
            root.path().join("ah.toml"),
            "version = \"1\"\n[agents.local]\nprovider = \"bash\"\n",
        )
        .unwrap();

        let found = find_config_with_env(root.path(), Some(OsString::from(env_config.as_os_str())))
            .unwrap();

        assert_eq!(found, env_config);
    }
}
