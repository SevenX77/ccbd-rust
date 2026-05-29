use crate::cli::config::{find_config, load_project_config};
use crate::cli::rpc_client::{CliError, RpcClient};
use serde_json::json;
use std::path::PathBuf;

pub struct UpOptions {
    pub config_path: Option<PathBuf>,
    pub cwd: PathBuf,
    pub force: bool,
}

pub async fn run_up<C: RpcClient>(client: &C, options: UpOptions) -> Result<(), CliError> {
    let config_path = match options.config_path {
        Some(path) => path,
        None => find_config(&options.cwd)?,
    };
    let config = load_project_config(&config_path)?;
    let result = client
        .call(
            "session.realign",
            json!({
                "session_id": "default",
                "force": options.force,
                "master": {
                    "cmd": config.master.cmd,
                    "hooks": config.master.hooks,
                    "plugins": config.master.plugins,
                },
                "agents": config.agents.into_iter().map(|(agent_id, agent)| {
                    json!({
                        "agent_id": agent_id,
                        "provider": agent.provider,
                        "env": agent.env,
                        "hooks": agent.hooks,
                        "plugins": agent.plugins,
                    })
                }).collect::<Vec<_>>()
            }),
        )
        .await?;
    println!("{result}");
    Ok(())
}
