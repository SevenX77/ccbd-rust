use crate::cli::config::{LayoutConfig, ProjectConfig, find_config, load_project_config};
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
    pub layout: String,
}

#[derive(Debug, Clone)]
pub struct SpawnedAgent {
    pub agent_id: String,
    pub provider: String,
    pub pid: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GridSplitHint {
    parent_index: usize,
    direction: &'static str,
    percent: u8,
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
    let split_plan = split_plan_for_layout(config.layout, config.agents.len(), has_master)?;
    let mut pane_ids = Vec::new();
    if has_master {
        let result = client
            .call(
                "session.spawn_master_pane",
                json!({
                    "session_id": session_id,
                    "cmd": config.master.cmd.clone(),
                }),
            )
            .await?;
        let pane_id = result
            .get("pane_id")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                CliError::InvalidResponse("session.spawn_master_pane missing pane_id".into())
            })?
            .to_string();
        pane_ids.push(pane_id);
    }
    for (index, (agent_id, agent)) in config.agents.iter().enumerate() {
        let mut merged_env = config.env.clone();
        merged_env.extend(agent.env.clone());
        let mut params = json!({
            "session_id": session_id,
            "agent_id": agent_id,
            "provider": agent.provider,
            "extra_env_vars": merged_env,
        });
        let plan_index = index + usize::from(has_master);
        if let Some(hint) = &split_plan[plan_index] {
            let parent = pane_ids.get(hint.parent_index).ok_or_else(|| {
                CliError::InvalidResponse(format!(
                    "missing parent pane for split plan index {}",
                    hint.parent_index
                ))
            })?;
            params["layout_parent_pane_id"] = json!(parent);
            params["layout_direction"] = json!(hint.direction);
            params["layout_percent"] = json!(hint.percent);
        }
        let result = client.call("agent.spawn", params).await;

        match result {
            Ok(value) => {
                let pane_id = value
                    .get("pane_id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| CliError::InvalidResponse("agent.spawn missing pane_id".into()))?
                    .to_string();
                pane_ids.push(pane_id);
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

fn split_plan_for_layout(
    layout: LayoutConfig,
    agent_count: usize,
    has_master: bool,
) -> Result<Vec<Option<GridSplitHint>>, CliError> {
    if layout != LayoutConfig::Grid {
        return Ok(vec![None; agent_count + usize::from(has_master)]);
    }
    if agent_count > 4 {
        return Err(CliError::Config(
            "providers max is 4 for auto layout".into(),
        ));
    }
    if has_master {
        let mut plan = vec![None; agent_count + 1];
        if agent_count >= 1 {
            plan[1] = Some(GridSplitHint {
                parent_index: 0,
                direction: "right",
                percent: 70,
            });
        }
        if agent_count >= 2 {
            plan[2] = Some(GridSplitHint {
                parent_index: 1,
                direction: "right",
                percent: 50,
            });
        }
        if agent_count >= 3 {
            plan[3] = Some(GridSplitHint {
                parent_index: 1,
                direction: "bottom",
                percent: 50,
            });
        }
        if agent_count == 4 {
            plan[4] = Some(GridSplitHint {
                parent_index: 2,
                direction: "bottom",
                percent: 50,
            });
        }
        return Ok(plan);
    }
    let mut plan = vec![None; agent_count];
    if agent_count >= 2 {
        plan[1] = Some(GridSplitHint {
            parent_index: 0,
            direction: "right",
            percent: 50,
        });
    }
    if agent_count == 3 {
        plan[2] = Some(GridSplitHint {
            parent_index: 1,
            direction: "bottom",
            percent: 50,
        });
    }
    if agent_count == 4 {
        plan[2] = Some(GridSplitHint {
            parent_index: 0,
            direction: "bottom",
            percent: 50,
        });
        plan[3] = Some(GridSplitHint {
            parent_index: 1,
            direction: "bottom",
            percent: 50,
        });
    }
    Ok(plan)
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

#[cfg(test)]
mod tests {
    use super::{GridSplitHint, split_plan_for_layout, start_project};
    use crate::cli::config::{AgentConfig, LayoutConfig, MasterConfig, ProjectConfig};
    use crate::cli::rpc_client::{CliError, RpcClient, RpcFuture};
    use serde_json::{Value, json};
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    #[test]
    fn test_grid_split_plan_for_two_agents() {
        assert_eq!(
            split_plan_for_layout(LayoutConfig::Grid, 2, false).unwrap(),
            vec![
                None,
                Some(GridSplitHint {
                    parent_index: 0,
                    direction: "right",
                    percent: 50,
                })
            ]
        );
    }

    #[test]
    fn test_grid_split_plan_for_three_agents() {
        assert_eq!(
            split_plan_for_layout(LayoutConfig::Grid, 3, false).unwrap(),
            vec![
                None,
                Some(GridSplitHint {
                    parent_index: 0,
                    direction: "right",
                    percent: 50,
                }),
                Some(GridSplitHint {
                    parent_index: 1,
                    direction: "bottom",
                    percent: 50,
                })
            ]
        );
    }

    #[test]
    fn test_grid_split_plan_for_four_agents() {
        assert_eq!(
            split_plan_for_layout(LayoutConfig::Grid, 4, false).unwrap(),
            vec![
                None,
                Some(GridSplitHint {
                    parent_index: 0,
                    direction: "right",
                    percent: 50,
                }),
                Some(GridSplitHint {
                    parent_index: 0,
                    direction: "bottom",
                    percent: 50,
                }),
                Some(GridSplitHint {
                    parent_index: 1,
                    direction: "bottom",
                    percent: 50,
                })
            ]
        );
    }

    #[test]
    fn test_grid_split_plan_with_master_and_four_agents() {
        assert_eq!(
            split_plan_for_layout(LayoutConfig::Grid, 4, true).unwrap(),
            vec![
                None,
                Some(GridSplitHint {
                    parent_index: 0,
                    direction: "right",
                    percent: 70,
                }),
                Some(GridSplitHint {
                    parent_index: 1,
                    direction: "right",
                    percent: 50,
                }),
                Some(GridSplitHint {
                    parent_index: 1,
                    direction: "bottom",
                    percent: 50,
                }),
                Some(GridSplitHint {
                    parent_index: 2,
                    direction: "bottom",
                    percent: 50,
                })
            ]
        );
    }

    #[test]
    fn test_grid_split_plan_rejects_more_than_four_agents() {
        let err = split_plan_for_layout(LayoutConfig::Grid, 5, false).unwrap_err();
        assert!(err.to_string().contains("providers max is 4"));
    }

    #[test]
    fn test_stack_split_plan_keeps_legacy_post_spawn_layout() {
        assert_eq!(
            split_plan_for_layout(LayoutConfig::Stack, 3, false).unwrap(),
            vec![None, None, None]
        );
    }

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
                    "session.apply_layout" => Ok(json!({ "status": "ok", "layout": "grid" })),
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
        let agent_calls = calls
            .iter()
            .filter(|(method, _)| method == "agent.spawn")
            .collect::<Vec<_>>();
        assert_eq!(agent_calls.len(), 4);
        assert_eq!(agent_calls[0].1["layout_parent_pane_id"], "%0");
        assert_eq!(agent_calls[0].1["layout_direction"], "right");
        assert_eq!(agent_calls[0].1["layout_percent"], 70);
    }

    #[tokio::test]
    async fn test_start_project_skips_master_when_disabled() {
        let config = project_config(false);
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
            layout: LayoutConfig::Grid,
            master: MasterConfig {
                cmd: "claude".to_string(),
                enabled: master_enabled,
            },
            env: Default::default(),
            agents,
        }
    }
}
