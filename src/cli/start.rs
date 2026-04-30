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
    pub master_pid: i64,
}

#[derive(Debug, Clone)]
pub struct StartSummary {
    pub session_id: String,
    pub agents: Vec<SpawnedAgent>,
    pub layout: String,
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
    start_project(
        client,
        config,
        &config_path,
        options.cwd,
        options.wait,
        options.master_pid,
    )
    .await
}

pub async fn start_project(
    client: &impl RpcClient,
    config: ProjectConfig,
    config_path: &Path,
    project_root: PathBuf,
    wait: bool,
    master_pid: i64,
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
                "master_pid": master_pid,
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
    for (agent_id, agent) in &config.agents {
        let mut merged_env = config.env.clone();
        merged_env.extend(agent.env.clone());
        let result = client
            .call(
                "agent.spawn",
                json!({
                    "session_id": session_id,
                    "agent_id": agent_id,
                    "provider": agent.provider,
                    "extra_env_vars": merged_env,
                }),
            )
            .await;

        match result {
            Ok(value) => {
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

    client
        .call(
            "session.apply_layout",
            json!({
                "session_id": session_id,
                "layout": config.layout.as_str(),
            }),
        )
        .await?;

    if wait {
        wait_until_agents_idle(client, &agents).await?;
    }

    Ok(StartSummary {
        session_id,
        agents,
        layout: config.layout.as_str().to_string(),
    })
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
    println!(
        "session_id={} layout={}",
        summary.session_id, summary.layout
    );
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
