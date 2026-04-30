use ccbd::cli::config::{LayoutConfig, ProjectConfig, load_project_config};
use ccbd::cli::rpc_client::{CliError, RpcClient, RpcFuture};
use ccbd::cli::start::start_project;
use ccbd::db;
use ccbd::db::agents::query_agent;
use ccbd::rpc::Ctx;
use ccbd::rpc::handlers::{handle_agent_spawn, handle_session_create};
use ccbd::sandbox::EnvState;
use ccbd::tmux::TmuxServer;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

struct Harness {
    ctx: Ctx,
    _state_dir: tempfile::TempDir,
    _db_file: tempfile::NamedTempFile,
}

impl Harness {
    fn new() -> Self {
        which::which("tmux").expect("tmux binary required for mvp9 acceptance tests");
        let db_file = tempfile::NamedTempFile::new().unwrap();
        let state_dir = tempfile::TempDir::new().unwrap();
        let state_dir_path = state_dir.path().to_path_buf();
        let ctx = Ctx {
            db: db::init(db_file.path()).unwrap(),
            state_dir: state_dir_path.clone(),
            env_state: EnvState {
                bwrap_available: false,
                systemd_run_available: false,
                unsafe_no_sandbox: true,
            },
            tmux_server: Arc::new(TmuxServer::new(&state_dir_path)),
        };

        Self {
            ctx,
            _state_dir: state_dir,
            _db_file: db_file,
        }
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        let _ = Command::new("tmux")
            .args(["-L", self.ctx.tmux_server.socket_name(), "kill-server"])
            .output();
    }
}

struct HandlerClient {
    ctx: Ctx,
}

impl RpcClient for HandlerClient {
    fn call<'a>(&'a self, method: &'a str, params: Value) -> RpcFuture<'a> {
        Box::pin(async move {
            match method {
                "session.create" => handle_session_create(params, &self.ctx)
                    .await
                    .map_err(map_rpc_error),
                "agent.spawn" => handle_agent_spawn(params, &self.ctx)
                    .await
                    .map_err(map_rpc_error),
                other => Err(CliError::InvalidResponse(format!(
                    "unexpected method in test client: {other}"
                ))),
            }
        })
    }
}

struct FailingSpawnClient {
    spawn_calls: AtomicUsize,
}

impl RpcClient for FailingSpawnClient {
    fn call<'a>(&'a self, method: &'a str, _params: Value) -> RpcFuture<'a> {
        Box::pin(async move {
            match method {
                "session.create" => Ok(json!({ "session_id": "sess_fail" })),
                "agent.spawn" => {
                    let call = self.spawn_calls.fetch_add(1, Ordering::SeqCst);
                    if call == 0 {
                        Ok(json!({ "state": "SPAWNING", "pid": 123 }))
                    } else {
                        Err(CliError::Rpc {
                            code: -32000,
                            message: "spawn failed".into(),
                        })
                    }
                }
                other => Err(CliError::InvalidResponse(format!(
                    "unexpected method in failing client: {other}"
                ))),
            }
        })
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_launcher_config_parse_and_batch_spawn() {
    let h = Harness::new();
    let config_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("examples")
        .join("ccb.toml");
    let mut config = load_project_config(&config_path).unwrap();
    config.agents = config
        .agents
        .into_iter()
        .take(2)
        .collect::<BTreeMap<_, _>>();
    let client = HandlerClient { ctx: h.ctx.clone() };

    let summary = start_project(
        &client,
        config,
        &config_path,
        h.ctx.state_dir.clone(),
        false,
        std::process::id() as i64,
    )
    .await
    .unwrap();

    assert!(summary.session_id.starts_with("sess_"));
    assert_eq!(summary.layout, LayoutConfig::Grid.as_str());
    assert_eq!(summary.agents.len(), 2);
    for agent in &summary.agents {
        let db_agent = query_agent(h.ctx.db.clone(), agent.agent_id.clone())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(db_agent.session_id, summary.session_id);
        assert_eq!(db_agent.provider, "bash");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_launcher_rollback_on_spawn_failure() {
    let config = toml::from_str::<ProjectConfig>(
        r#"
version = "1"

[agents.a1]
provider = "bash"

[agents.a2]
provider = "bash"
"#,
    )
    .unwrap();
    let config_path = std::path::Path::new("test-ccb.toml");
    let client = FailingSpawnClient {
        spawn_calls: AtomicUsize::new(0),
    };

    let err = start_project(
        &client,
        config,
        config_path,
        std::env::current_dir().unwrap(),
        false,
        std::process::id() as i64,
    )
    .await
    .unwrap_err();

    assert!(err.to_string().contains("rollback is pending G9.1"));
    assert_eq!(client.spawn_calls.load(Ordering::SeqCst), 2);
}

fn map_rpc_error(err: ccbd::error::CcbdError) -> CliError {
    CliError::Rpc {
        code: -32000,
        message: err.to_string(),
    }
}
