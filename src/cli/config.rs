use crate::cli::rpc_client::CliError;
use serde::Deserialize;
use std::collections::{BTreeMap, HashMap};
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Deserialize)]
pub struct ProjectConfig {
    pub version: String,
    #[serde(default = "default_layout")]
    pub layout: LayoutConfig,
    #[serde(default)]
    pub env: HashMap<String, String>,
    pub agents: BTreeMap<String, AgentConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AgentConfig {
    pub provider: String,
    #[serde(default)]
    pub env: HashMap<String, String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutConfig {
    Single,
    Stack,
    Grid,
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

impl<'de> Deserialize<'de> for LayoutConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        parse_layout(&value).map_err(serde::de::Error::custom)
    }
}

impl LayoutConfig {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Single => "single",
            Self::Stack => "stack",
            Self::Grid => "grid",
        }
    }
}

pub fn load_project_config(path: &Path) -> Result<ProjectConfig, CliError> {
    let raw = fs::read_to_string(path).map_err(|err| {
        CliError::Config(format!("failed to read config {}: {err}", path.display()))
    })?;
    let config: ProjectConfig = toml::from_str(&raw)?;
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
        diagnostics.push(error("ccb.toml version must be \"1\""));
    }
    if config.agents.is_empty() {
        diagnostics.push(error("ccb.toml must define at least one [agents.<id>]"));
    }
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
        }
    }
    diagnostics
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
        let candidate = current.join("ccb.toml");
        if candidate.is_file() {
            return Ok(candidate);
        }
        if !current.pop() {
            break;
        }
    }

    Err(CliError::Config(format!(
        "could not find ccb.toml from {}; create one or set CCB_CONFIG_PATH",
        start_dir.display()
    )))
}

fn default_layout() -> LayoutConfig {
    LayoutConfig::Grid
}

fn parse_layout(value: &str) -> Result<LayoutConfig, String> {
    match value {
        "single" => Ok(LayoutConfig::Single),
        "stack" => Ok(LayoutConfig::Stack),
        "grid" => Ok(LayoutConfig::Grid),
        other => Err(format!(
            "unknown layout {other:?}; expected single, stack or grid"
        )),
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
    use super::{DiagnosticSeverity, LayoutConfig, find_config_with_env, load_project_config};
    use std::ffi::OsString;

    #[test]
    fn test_load_valid_config_defaults_layout() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("ccb.toml");
        std::fs::write(
            &path,
            r#"
version = "1"

[agents.a1]
provider = "bash"
"#,
        )
        .unwrap();

        let config = load_project_config(&path).unwrap();

        assert_eq!(config.layout, LayoutConfig::Grid);
        assert_eq!(config.agents["a1"].provider, "bash");
    }

    #[test]
    fn test_rejects_unknown_layout() {
        let err = toml::from_str::<super::ProjectConfig>(
            r#"
version = "1"
layout = "diagonal"

[agents.a1]
provider = "bash"
"#,
        )
        .unwrap_err();

        assert!(err.to_string().contains("unknown layout"));
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
    fn test_find_config_walks_up_from_cwd() {
        let root = tempfile::TempDir::new().unwrap();
        let nested = root.path().join("a/b");
        std::fs::create_dir_all(&nested).unwrap();
        let config = root.path().join("ccb.toml");
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
            root.path().join("ccb.toml"),
            "version = \"1\"\n[agents.local]\nprovider = \"bash\"\n",
        )
        .unwrap();

        let found = find_config_with_env(root.path(), Some(OsString::from(env_config.as_os_str())))
            .unwrap();

        assert_eq!(found, env_config);
    }
}
