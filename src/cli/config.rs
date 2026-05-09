use crate::cli::rpc_client::CliError;
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
    pub daemon: DaemonConfig,
    #[serde(default)]
    pub env: HashMap<String, String>,
    #[serde(default)]
    pub sandbox: SandboxConfig,
    pub agents: BTreeMap<String, AgentConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MasterConfig {
    #[serde(
        default = "default_master_cmd",
        deserialize_with = "deserialize_master_cmd"
    )]
    pub cmd: String,
    #[serde(default = "default_master_enabled")]
    pub enabled: bool,
}

impl Default for MasterConfig {
    fn default() -> Self {
        Self {
            cmd: default_master_cmd(),
            enabled: default_master_enabled(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct DaemonConfig {
    #[serde(default = "default_daemon_auto_shutdown")]
    pub auto_shutdown_on_master_exit: bool,
}

impl Default for DaemonConfig {
    fn default() -> Self {
        Self {
            auto_shutdown_on_master_exit: default_daemon_auto_shutdown(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct AgentConfig {
    pub provider: String,
    #[serde(default)]
    pub env: HashMap<String, String>,
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

fn default_master_cmd() -> String {
    "claude --dangerously-skip-permissions --continue /remote-control".into()
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

fn default_daemon_auto_shutdown() -> bool {
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
        DaemonConfig, DiagnosticSeverity, MasterConfig, find_config_with_env, load_project_config,
    };
    use std::ffi::OsString;

    #[test]
    fn test_load_valid_config_without_layout() {
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

        assert_eq!(config.agents["a1"].provider, "bash");
        assert!(config.daemon.auto_shutdown_on_master_exit);
        assert!(config.sandbox.additional_ro_binds.is_empty());
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
    fn test_master_config_default() {
        let master = MasterConfig::default();

        assert!(master.enabled);
        assert_eq!(
            master.cmd,
            "claude --dangerously-skip-permissions --continue /remote-control"
        );
    }

    #[test]
    fn test_daemon_config_default() {
        let daemon = DaemonConfig::default();

        assert!(daemon.auto_shutdown_on_master_exit);
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

        assert!(config.daemon.auto_shutdown_on_master_exit);
    }

    #[test]
    fn test_load_project_config_daemon_auto_shutdown_false() {
        let config = toml::from_str::<super::ProjectConfig>(
            r#"
version = "1"

[daemon]
auto_shutdown_on_master_exit = false

[agents.a1]
provider = "bash"
"#,
        )
        .unwrap();

        assert!(!config.daemon.auto_shutdown_on_master_exit);
    }

    #[test]
    fn test_load_project_config_daemon_auto_shutdown_true() {
        let config = toml::from_str::<super::ProjectConfig>(
            r#"
version = "1"

[daemon]
auto_shutdown_on_master_exit = true

[agents.a1]
provider = "bash"
"#,
        )
        .unwrap();

        assert!(config.daemon.auto_shutdown_on_master_exit);
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
        assert_eq!(
            config.master.cmd,
            "claude --dangerously-skip-permissions --continue /remote-control"
        );
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
        let path = dir.path().join("ccb.toml");
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
        let path = dir.path().join("ccb.toml");
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
