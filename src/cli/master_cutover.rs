use crate::cli::config::ProjectConfig;
use crate::cli::output::string_field;
use crate::cli::rpc_client::{CliError, RpcClient};
use crate::master_cutover::{HandoffBundleInput, write_handoff_bundle};
use crate::tmux::master_session_name;
use serde_json::json;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct MasterCutoverOptions {
    pub config: ProjectConfig,
    pub project_root: PathBuf,
    pub state_dir: PathBuf,
    pub socket_path: PathBuf,
    pub wait: bool,
    pub print_attach: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MasterCutoverSummary {
    pub cutover_id: String,
    pub session_id: String,
    pub pane_id: String,
    pub attach_command: String,
    pub tmux_attach_command: String,
    pub handoff_path: PathBuf,
}

pub async fn run_master_cutover(
    client: &impl RpcClient,
    options: MasterCutoverOptions,
) -> Result<MasterCutoverSummary, CliError> {
    let project_id = options
        .project_root
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("project")
        .to_string();
    tracing::info!(project_id = %project_id, "starting master cutover orchestration");
    let session = client
        .call(
            "session.create",
            json!({
                "project_id": project_id,
                "absolute_path": options.project_root.display().to_string(),
            }),
        )
        .await?;
    let session_id = string_field(&session, "session_id");
    if session_id == "-" {
        tracing::warn!("master cutover session.create missing session_id");
        return Err(CliError::InvalidResponse(
            "session.create missing session_id".into(),
        ));
    }
    let cutover_id = format!("cutover-{session_id}");
    let handoff_path = write_handoff_bundle(
        &options.state_dir,
        &HandoffBundleInput {
            cutover_id: &cutover_id,
            session_id: &session_id,
            socket_path: &options.socket_path,
            state_dir: &options.state_dir,
        },
    )
    .map_err(|err| CliError::Config(format!("write handoff bundle: {err}")))?;
    tracing::info!(
        cutover_id = %cutover_id,
        session_id = %session_id,
        handoff_path = %handoff_path.display(),
        "master cutover handoff bundle prepared"
    );

    client
        .call(
            "session.realign",
            build_realign_payload(&session_id, &options.config, false),
        )
        .await?;

    let extra_env = json!({
        "AH_STATE_DIR": options.state_dir.display().to_string(),
        "CCB_SOCKET": options.socket_path.display().to_string(),
        "AH_CUTOVER_ID": cutover_id,
        "AH_MASTER_HANDOFF": handoff_path.display().to_string(),
        "AH_MASTER_ROLE": "managed",
    });
    let spawn = client
        .call(
            "session.spawn_master_pane",
            json!({
                "session_id": session_id,
                "cmd": options.config.master.cmd,
                "hooks": options.config.master.hooks,
                "plugins": options.config.master.plugins,
                "extra_env": extra_env,
            }),
        )
        .await
        .map_err(|err| {
            tracing::warn!(
                cutover_id = %cutover_id,
                session_id = %session_id,
                error = %err,
                "master cutover spawn failed; old master is left running"
            );
            err
        })?;
    let pane_id = string_field(&spawn, "pane_id");
    if pane_id == "-" {
        tracing::warn!(cutover_id = %cutover_id, session_id = %session_id, "master cutover spawn missing pane_id");
        return Err(CliError::InvalidResponse(
            "session.spawn_master_pane missing pane_id".into(),
        ));
    }
    tracing::info!(
        cutover_id = %cutover_id,
        session_id = %session_id,
        pane_id = %pane_id,
        "master cutover spawned managed master"
    );

    Ok(MasterCutoverSummary {
        cutover_id,
        session_id: session_id.clone(),
        pane_id,
        attach_command: format!("ah attach master --session {session_id}"),
        tmux_attach_command: format!(
            "tmux -S {} attach -t {}",
            options.socket_path.display(),
            master_session_name(&project_id)
        ),
        handoff_path,
    })
}

fn build_realign_payload(
    session_id: &str,
    config: &ProjectConfig,
    force: bool,
) -> serde_json::Value {
    json!({
        "session_id": session_id,
        "force": force,
        "master": {
            "cmd": config.master.cmd,
            "hooks": config.master.hooks,
            "plugins": config.master.plugins,
        },
        "agents": config.agents.iter().map(|(agent_id, agent)| {
            json!({
                "agent_id": agent_id,
                "provider": agent.provider,
                "env": agent.env,
                "hooks": agent.hooks,
                "plugins": agent.plugins,
            })
        }).collect::<Vec<_>>()
    })
}

pub fn print_master_cutover_summary(summary: &MasterCutoverSummary) {
    println!("session_id={}", summary.session_id);
    println!("pane_id={}", summary.pane_id);
    println!("{}", summary.tmux_attach_command);
    println!("{}", summary.attach_command);
}

#[cfg(test)]
mod tests {
    use super::{MasterCutoverOptions, run_master_cutover};
    use crate::cli::config::{AgentConfig, MasterConfig, ProjectConfig, SandboxConfig};
    use crate::cli::rpc_client::{CliError, RpcClient, RpcFuture};
    use crate::provider::extensions::HookGroup;
    use serde_json::{Value, json};
    use std::cell::RefCell;
    use std::collections::{BTreeMap, HashMap};
    use std::path::PathBuf;

    struct FakeRpcClient {
        calls: RefCell<Vec<(String, Value)>>,
    }

    impl FakeRpcClient {
        fn new() -> Self {
            Self {
                calls: RefCell::new(Vec::new()),
            }
        }

        fn calls(&self) -> Vec<(String, Value)> {
            self.calls.borrow().clone()
        }
    }

    impl RpcClient for FakeRpcClient {
        fn call<'a>(&'a self, method: &'a str, params: Value) -> RpcFuture<'a> {
            self.calls
                .borrow_mut()
                .push((method.to_string(), params.clone()));
            Box::pin(async move {
                match method {
                    "session.create" => Ok(json!({ "session_id": "sess_cutover" })),
                    "session.realign" => Ok(json!({ "results": [] })),
                    "session.spawn_master_pane" => Ok(json!({ "pane_id": "%42" })),
                    other => Err(CliError::InvalidResponse(format!(
                        "unexpected fake rpc method: {other}"
                    ))),
                }
            })
        }
    }

    fn config() -> ProjectConfig {
        let mut agents = BTreeMap::new();
        agents.insert(
            "a1".to_string(),
            AgentConfig {
                provider: "bash".to_string(),
                env: HashMap::new(),
                hooks: HashMap::<String, Vec<HookGroup>>::new(),
                plugins: Vec::new(),
            },
        );
        ProjectConfig {
            version: "1".to_string(),
            master: MasterConfig::default(),
            daemon: Default::default(),
            env: HashMap::new(),
            sandbox: SandboxConfig::default(),
            agents,
        }
    }

    fn options(tmp: &tempfile::TempDir) -> MasterCutoverOptions {
        MasterCutoverOptions {
            config: config(),
            project_root: PathBuf::from("/home/sevenx/coding/ccbd-rust"),
            state_dir: tmp.path().join("state"),
            socket_path: tmp.path().join("state").join("ahd.sock"),
            wait: true,
            print_attach: true,
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn cutover_uses_start_session_then_spawn_master_with_env() {
        let tmp = tempfile::tempdir().unwrap();
        let client = FakeRpcClient::new();

        let summary = run_master_cutover(&client, options(&tmp)).await.unwrap();

        let calls = client.calls();
        let methods = calls
            .iter()
            .map(|(method, _)| method.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            methods,
            vec![
                "session.create",
                "session.realign",
                "session.spawn_master_pane"
            ]
        );
        let spawn = calls
            .iter()
            .find(|(method, _)| method == "session.spawn_master_pane")
            .expect("spawn master call");
        let extra_env = spawn.1.get("extra_env").expect("spawn extra_env");
        for key in [
            "AH_STATE_DIR",
            "CCB_SOCKET",
            "AH_CUTOVER_ID",
            "AH_MASTER_HANDOFF",
            "AH_MASTER_ROLE",
        ] {
            assert!(extra_env.get(key).is_some(), "missing extra env {key}");
        }
        assert_eq!(summary.pane_id, "%42");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn cutover_does_not_call_session_kill() {
        let tmp = tempfile::tempdir().unwrap();
        let client = FakeRpcClient::new();

        let _summary = run_master_cutover(&client, options(&tmp)).await.unwrap();

        assert!(
            client
                .calls()
                .iter()
                .all(|(method, _)| method != "session.kill")
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn cutover_prints_attach_master_command() {
        let tmp = tempfile::tempdir().unwrap();
        let client = FakeRpcClient::new();

        let summary = run_master_cutover(&client, options(&tmp)).await.unwrap();

        assert!(
            summary
                .attach_command
                .contains("ah attach master --session sess_cutover")
        );
        assert!(summary.tmux_attach_command.contains("tmux -S"));
        assert!(summary.tmux_attach_command.contains("master_ccbd-rust"));
    }
}
