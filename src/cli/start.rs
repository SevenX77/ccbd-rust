use crate::cli::config::{ProjectConfig, find_config, load_project_config};
use crate::cli::output::string_field;
use crate::cli::rpc_client::{CliError, RpcClient};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct StartOptions {
    pub config_path: Option<PathBuf>,
    pub cwd: PathBuf,
    pub wait: bool,
}

#[derive(Debug, Clone)]
pub struct StartSummary {
    pub session_id: String,
    pub agents: Vec<SpawnedAgent>,
}

#[derive(Debug, Clone)]
pub struct SpawnedAgent {
    pub agent_id: String,
    pub provider: String,
    pub pid: Option<i64>,
}

pub async fn start_from_options(
    client: &impl RpcClient,
    options: StartOptions,
) -> Result<StartSummary, CliError> {
    let config_path = match options.config_path {
        Some(path) => path,
        None => find_config(&options.cwd)?,
    };
    let config = load_project_config(&config_path)?;
    start_project(client, config, &config_path, options.cwd, options.wait).await
}

pub async fn start_project(
    client: &impl RpcClient,
    config: ProjectConfig,
    config_path: &Path,
    project_root: PathBuf,
    wait: bool,
) -> Result<StartSummary, CliError> {
    let project_id = project_root
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("project")
        .to_string();

    let session = client
        .call(
            "session.create",
            json!({
                "project_id": project_id,
                "absolute_path": project_root.display().to_string(),
            }),
        )
        .await?;
    let session_id = string_field(&session, "session_id");
    if session_id == "-" {
        return Err(CliError::InvalidResponse(
            "session.create missing session_id".into(),
        ));
    }

    let mut agents = Vec::new();
    let has_master = config.master.enabled;
    if has_master {
        let result = client
            .call(
                "session.spawn_master_pane",
                json!({
                    "session_id": session_id,
                    "cmd": config.master.cmd.clone(),
                    "auto_shutdown_on_master_exit": config.daemon.auto_shutdown_on_master_exit,
                }),
            )
            .await?;
        result
            .get("pane_id")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                CliError::InvalidResponse("session.spawn_master_pane missing pane_id".into())
            })?;
    }
    for (agent_id, agent) in config.agents.iter() {
        let mut merged_env = config.env.clone();
        merged_env.extend(agent.env.clone());
        let sandbox_overrides = json!({
            "extra_ro_binds": config.sandbox.additional_ro_binds
                .iter()
                .map(|path| json!({
                    "host_path": path,
                    "sandbox_path": path,
                }))
                .collect::<Vec<_>>(),
        });
        let result = client
            .call(
                "agent.spawn",
                json!({
                    "session_id": session_id,
                    "agent_id": agent_id,
                    "provider": agent.provider,
                    "extra_env_vars": merged_env,
                    "sandbox_overrides": sandbox_overrides,
                }),
            )
            .await;

        match result {
            Ok(value) => {
                value
                    .get("pane_id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        CliError::InvalidResponse("agent.spawn missing pane_id".into())
                    })?;
                agents.push(SpawnedAgent {
                    agent_id: agent_id.clone(),
                    provider: agent.provider.clone(),
                    pid: value.get("pid").and_then(Value::as_i64),
                });
            }
            Err(err) => {
                let rollback = client
                    .call(
                        "session.kill",
                        json!({
                            "session_id": session_id,
                            "force": true,
                        }),
                    )
                    .await;
                if let Err(rollback_err) = rollback {
                    return Err(CliError::Config(format!(
                        "ccb start failed while spawning {agent_id} from {}; session.kill rollback failed after original error ({err}): {rollback_err}",
                        config_path.display()
                    )));
                }
                return Err(CliError::Config(format!(
                    "ccb start failed while spawning {agent_id} from {}; rolled back session {session_id}: {err}",
                    config_path.display()
                )));
            }
        }
    }

    if wait {
        wait_until_agents_idle(client, &agents).await?;
    }

    Ok(StartSummary { session_id, agents })
}

async fn wait_until_agents_idle(
    client: &impl RpcClient,
    agents: &[SpawnedAgent],
) -> Result<(), CliError> {
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut remaining = agents
        .iter()
        .map(|agent| agent.agent_id.clone())
        .collect::<Vec<_>>();

    while !remaining.is_empty() && Instant::now() < deadline {
        let mut still_waiting = Vec::new();
        for agent_id in remaining {
            let result = client
                .call(
                    "agent.watch",
                    json!({
                        "agent_id": agent_id,
                        "since_event_id": 0,
                        "timeout": 1,
                    }),
                )
                .await?;
            let is_idle = result
                .get("events")
                .and_then(Value::as_array)
                .is_some_and(|events| {
                    events.iter().any(|event| {
                        event["event_type"] == "state_change"
                            && event["payload"]
                                .as_str()
                                .is_some_and(|payload| payload.contains("\"to\":\"IDLE\""))
                    })
                });
            if !is_idle {
                still_waiting.push(agent_id);
            }
        }
        remaining = still_waiting;
    }

    if remaining.is_empty() {
        Ok(())
    } else {
        Err(CliError::Config(format!(
            "timed out waiting for agents to become IDLE: {}",
            remaining.join(", ")
        )))
    }
}

pub fn print_start_summary(summary: &StartSummary) {
    println!("session_id={}", summary.session_id);
    for agent in &summary.agents {
        let pid = agent
            .pid
            .map(|pid| pid.to_string())
            .unwrap_or_else(|| "-".into());
        println!(
            "agent_id={} provider={} pid={}",
            agent.agent_id, agent.provider, pid
        );
    }
}

#[cfg(test)]
mod tests {
    use super::start_project;
    use crate::cli::config::{AgentConfig, MasterConfig, ProjectConfig, SandboxConfig};
    use crate::cli::rpc_client::{CliError, RpcClient, RpcFuture};
    use serde_json::{Value, json};
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    struct RecordingClient {
        calls: Mutex<Vec<(String, Value)>>,
    }

    impl RpcClient for RecordingClient {
        fn call<'a>(&'a self, method: &'a str, params: Value) -> RpcFuture<'a> {
            Box::pin(async move {
                let mut calls = self.calls.lock().unwrap();
                calls.push((method.to_string(), params));
                let call_index = calls.len();
                drop(calls);
                match method {
                    "session.create" => Ok(json!({ "session_id": "sess_start" })),
                    "session.spawn_master_pane" => Ok(json!({ "pane_id": "%0" })),
                    "agent.spawn" => Ok(json!({
                        "state": "SPAWNING",
                        "pid": 123,
                        "pane_id": format!("%{call_index}")
                    })),
                    other => Err(CliError::InvalidResponse(format!(
                        "unexpected method in recording client: {other}"
                    ))),
                }
            })
        }
    }

    #[tokio::test]
    async fn test_start_project_spawns_master_when_enabled() {
        let config = project_config(true);
        let client = RecordingClient {
            calls: Mutex::new(Vec::new()),
        };

        start_project(
            &client,
            config,
            std::path::Path::new("test-ccb.toml"),
            std::env::current_dir().unwrap(),
            false,
        )
        .await
        .unwrap();

        let calls = client.calls.lock().unwrap();
        assert_eq!(calls[0].0, "session.create");
        assert_eq!(calls[1].0, "session.spawn_master_pane");
        assert_eq!(calls[1].1["cmd"], "claude");
        assert_eq!(calls[1].1["auto_shutdown_on_master_exit"], true);
        let agent_calls = calls
            .iter()
            .filter(|(method, _)| method == "agent.spawn")
            .collect::<Vec<_>>();
        assert_eq!(agent_calls.len(), 4);
        assert!(agent_calls[0].1.get("layout_parent_pane_id").is_none());
        assert!(agent_calls[0].1.get("layout_direction").is_none());
        assert!(agent_calls[0].1.get("layout_percent").is_none());
    }

    #[tokio::test]
    async fn test_start_project_passes_auto_shutdown_false_to_master_spawn() {
        let mut config = project_config(true);
        config.daemon.auto_shutdown_on_master_exit = false;
        let client = RecordingClient {
            calls: Mutex::new(Vec::new()),
        };

        start_project(
            &client,
            config,
            std::path::Path::new("test-ccb.toml"),
            std::env::current_dir().unwrap(),
            false,
        )
        .await
        .unwrap();

        let calls = client.calls.lock().unwrap();
        let master_spawn = calls
            .iter()
            .find(|(method, _)| method == "session.spawn_master_pane")
            .unwrap();
        assert_eq!(
            master_spawn.1["auto_shutdown_on_master_exit"], false,
            "daemon auto-shutdown config must be forwarded only through the master watcher RPC"
        );
    }

    #[tokio::test]
    async fn test_start_project_passes_additional_ro_binds_to_agent_spawn() {
        let mut config = project_config(false);
        config.sandbox.additional_ro_binds = vec!["/opt/tools".to_string()];
        let client = RecordingClient {
            calls: Mutex::new(Vec::new()),
        };

        start_project(
            &client,
            config,
            std::path::Path::new("test-ccb.toml"),
            std::env::current_dir().unwrap(),
            false,
        )
        .await
        .unwrap();

        let calls = client.calls.lock().unwrap();
        let first_agent = calls
            .iter()
            .find(|(method, _)| method == "agent.spawn")
            .unwrap();
        assert_eq!(
            first_agent.1["sandbox_overrides"]["extra_ro_binds"][0]["host_path"],
            "/opt/tools"
        );
        assert_eq!(
            first_agent.1["sandbox_overrides"]["extra_ro_binds"][0]["sandbox_path"],
            "/opt/tools"
        );
    }

    #[tokio::test]
    async fn test_start_project_skips_master_when_disabled() {
        let mut config = project_config(false);
        config.daemon.auto_shutdown_on_master_exit = false;
        let client = RecordingClient {
            calls: Mutex::new(Vec::new()),
        };

        start_project(
            &client,
            config,
            std::path::Path::new("test-ccb.toml"),
            std::env::current_dir().unwrap(),
            false,
        )
        .await
        .unwrap();

        let calls = client.calls.lock().unwrap();
        assert!(
            !calls
                .iter()
                .any(|(method, _)| method == "session.spawn_master_pane")
        );
        let first_agent = calls
            .iter()
            .find(|(method, _)| method == "agent.spawn")
            .unwrap();
        assert!(first_agent.1.get("layout_parent_pane_id").is_none());
    }

    fn project_config(master_enabled: bool) -> ProjectConfig {
        let mut agents = BTreeMap::new();
        for (id, provider) in [
            ("a1", "codex"),
            ("a2", "codex"),
            ("a3", "gemini"),
            ("a4", "claude"),
        ] {
            agents.insert(
                id.to_string(),
                AgentConfig {
                    provider: provider.to_string(),
                    env: Default::default(),
                },
            );
        }
        ProjectConfig {
            version: "1".to_string(),
            master: MasterConfig {
                cmd: "claude".to_string(),
                enabled: master_enabled,
            },
            daemon: Default::default(),
            env: Default::default(),
            sandbox: SandboxConfig::default(),
            agents,
        }
    }
}
