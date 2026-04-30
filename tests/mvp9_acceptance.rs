use ccbd::cli::config::{LayoutConfig, ProjectConfig, load_project_config};
use ccbd::cli::rpc_client::{CliError, RpcClient, RpcFuture};
use ccbd::cli::start::start_project;
use ccbd::db;
use ccbd::db::agents::query_agent;
use ccbd::db::jobs::{insert_job, query_job};
use ccbd::rpc::Ctx;
use ccbd::rpc::handlers::{handle_agent_spawn, handle_session_create, handle_session_kill};
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
                "session.kill" => handle_session_kill(params, &self.ctx)
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
                "session.kill" => {
                    Ok(json!({ "session_id": "sess_fail", "state": "KILLED", "killed_agents": 1 }))
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

    assert!(err.to_string().contains("rolled back session sess_fail"));
    assert_eq!(client.spawn_calls.load(Ordering::SeqCst), 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_session_kill_cleans_resources() {
    let h = Harness::new();
    let session = handle_session_create(
        json!({
            "project_id": "p_kill",
            "absolute_path": h.ctx.state_dir.display().to_string(),
            "master_pid": std::process::id() as i64,
        }),
        &h.ctx,
    )
    .await
    .unwrap();
    let session_id = session["session_id"].as_str().unwrap().to_string();
    for agent_id in ["ag_kill_1", "ag_kill_2"] {
        handle_agent_spawn(
            json!({
                "session_id": session_id,
                "agent_id": agent_id,
                "provider": "bash",
            }),
            &h.ctx,
        )
        .await
        .unwrap();
    }

    let result = handle_session_kill(
        json!({
            "session_id": session_id,
            "force": true,
        }),
        &h.ctx,
    )
    .await
    .unwrap();

    assert_eq!(result["state"], "KILLED");
    assert_eq!(result["killed_agents"], 2);
    for agent_id in ["ag_kill_1", "ag_kill_2"] {
        let agent = query_agent(h.ctx.db.clone(), agent_id.to_string())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(agent.state, "KILLED");
        assert!(!ccbd::agent_io::contains(agent_id));
        assert!(
            !h.ctx
                .state_dir
                .join("pipes")
                .join(format!("{agent_id}.fifo"))
                .exists()
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_reconcile_cleans_dead_pids_on_boot() {
    let h = Harness::new();
    ccbd::db::sessions::insert_session(
        h.ctx.db.clone(),
        "s_reconcile".to_string(),
        "p_reconcile".to_string(),
        h.ctx.state_dir.display().to_string(),
        std::process::id() as i64,
    )
    .await
    .unwrap();
    ccbd::db::agents::insert_agent(
        h.ctx.db.clone(),
        "ag_reconcile".to_string(),
        "s_reconcile".to_string(),
        "bash".to_string(),
        "BUSY".to_string(),
        Some(999_999_999),
    )
    .await
    .unwrap();
    insert_job(
        h.ctx.db.clone(),
        "job_reconcile".to_string(),
        "ag_reconcile".to_string(),
        None,
        "echo lost\n".to_string(),
    )
    .await
    .unwrap();
    h.ctx
        .db
        .conn()
        .execute(
            "UPDATE jobs SET status = 'DISPATCHED', dispatched_at = unixepoch() WHERE id = 'job_reconcile'",
            [],
        )
        .unwrap();

    let count = ccbd::db::system::reconcile_startup(h.ctx.db.clone())
        .await
        .unwrap();
    let agent = query_agent(h.ctx.db.clone(), "ag_reconcile".to_string())
        .await
        .unwrap()
        .unwrap();
    let job = query_job(h.ctx.db.clone(), "job_reconcile".to_string())
        .await
        .unwrap()
        .unwrap();

    assert_eq!(count, 1);
    assert_eq!(agent.state, "CRASHED");
    assert_eq!(
        agent.error_code.as_deref(),
        Some("STARTUP_RECONCILE_DEAD_PID")
    );
    assert_eq!(job.status, "FAILED");
    assert_eq!(
        job.error_reason.as_deref(),
        Some("STARTUP_RECONCILE_DEAD_PID")
    );
}

fn map_rpc_error(err: ccbd::error::CcbdError) -> CliError {
    CliError::Rpc {
        code: -32000,
        message: err.to_string(),
    }
}
