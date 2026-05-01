mod common;

use ccbd::cli::config::{AgentConfig, LayoutConfig, ProjectConfig, load_project_config};
use ccbd::cli::rpc_client::{CliError, RpcClient, RpcFuture};
use ccbd::cli::start::start_project;
use ccbd::db;
use ccbd::db::agents::query_agent;
use ccbd::db::jobs::{insert_job, query_job};
use ccbd::rpc::Ctx;
use ccbd::rpc::handlers::{
    handle_agent_send, handle_agent_spawn, handle_job_cancel, handle_session_apply_layout,
    handle_session_create, handle_session_kill,
};
use ccbd::sandbox::EnvState;
use ccbd::tmux::{TmuxServer, compute_socket_name};
use common::scope_policy_for_test;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

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
        let socket_name = compute_socket_name(state_dir.path());
        let policy = scope_policy_for_test(&socket_name);
        let ctx = Ctx {
            db: db::init(db_file.path()).unwrap(),
            state_dir: state_dir_path.clone(),
            env_state: EnvState {
                bwrap_available: false,
                systemd_run_available: false,
                unsafe_no_sandbox: true,
            },
            tmux_server: Arc::new(TmuxServer::new_with_policy(state_dir.path(), policy)),
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
        let socket_path = format!(
            "/tmp/tmux-{}/{}",
            unsafe { libc::geteuid() },
            self.ctx.tmux_server.socket_name()
        );
        let _ = std::fs::remove_file(socket_path);
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
                "session.apply_layout" => handle_session_apply_layout(params, &self.ctx)
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

struct RecordingStartClient {
    calls: Mutex<Vec<(String, Value)>>,
}

impl RpcClient for RecordingStartClient {
    fn call<'a>(&'a self, method: &'a str, params: Value) -> RpcFuture<'a> {
        Box::pin(async move {
            self.calls
                .lock()
                .unwrap()
                .push((method.to_string(), params.clone()));
            match method {
                "session.create" => Ok(json!({ "session_id": "sess_env" })),
                "agent.spawn" => Ok(json!({ "state": "SPAWNING", "pid": 123 })),
                "session.apply_layout" => Ok(json!({ "status": "ok", "layout": "grid" })),
                other => Err(CliError::InvalidResponse(format!(
                    "unexpected method in recording client: {other}"
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
async fn test_launcher_passes_merged_env_to_agent_spawn() {
    let mut global_env = std::collections::HashMap::new();
    global_env.insert("GLOBAL_ONLY".to_string(), "1".to_string());
    global_env.insert("OVERRIDE_ME".to_string(), "global".to_string());
    let mut agent_env = std::collections::HashMap::new();
    agent_env.insert("OVERRIDE_ME".to_string(), "agent".to_string());
    agent_env.insert("AGENT_ONLY".to_string(), "2".to_string());
    let mut agents = BTreeMap::new();
    agents.insert(
        "ag_env".to_string(),
        AgentConfig {
            provider: "bash".to_string(),
            env: agent_env,
        },
    );
    let config = ProjectConfig {
        version: "1".to_string(),
        layout: LayoutConfig::Grid,
        env: global_env,
        agents,
    };
    let client = RecordingStartClient {
        calls: Mutex::new(Vec::new()),
    };

    start_project(
        &client,
        config,
        std::path::Path::new("test-ccb.toml"),
        std::env::current_dir().unwrap(),
        false,
        std::process::id() as i64,
    )
    .await
    .unwrap();

    let calls = client.calls.lock().unwrap();
    let (_, spawn_params) = calls
        .iter()
        .find(|(method, _)| method == "agent.spawn")
        .unwrap();
    assert_eq!(spawn_params["extra_env_vars"]["GLOBAL_ONLY"], "1");
    assert_eq!(spawn_params["extra_env_vars"]["AGENT_ONLY"], "2");
    assert_eq!(spawn_params["extra_env_vars"]["OVERRIDE_ME"], "agent");
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

#[tokio::test(flavor = "multi_thread")]
async fn test_reconcile_crashes_alive_agent_when_fifo_missing() {
    let h = Harness::new();
    ccbd::db::sessions::insert_session(
        h.ctx.db.clone(),
        "s_reconcile_fifo".to_string(),
        "p_reconcile_fifo".to_string(),
        h.ctx.state_dir.display().to_string(),
        std::process::id() as i64,
    )
    .await
    .unwrap();
    ccbd::db::agents::insert_agent(
        h.ctx.db.clone(),
        "ag_reconcile_fifo".to_string(),
        "s_reconcile_fifo".to_string(),
        "bash".to_string(),
        "BUSY".to_string(),
        Some(std::process::id() as i64),
    )
    .await
    .unwrap();
    insert_job(
        h.ctx.db.clone(),
        "job_reconcile_fifo".to_string(),
        "ag_reconcile_fifo".to_string(),
        None,
        "echo lost\n".to_string(),
    )
    .await
    .unwrap();
    h.ctx
        .db
        .conn()
        .execute(
            "UPDATE jobs SET status = 'DISPATCHED', dispatched_at = unixepoch() WHERE id = 'job_reconcile_fifo'",
            [],
        )
        .unwrap();

    let count = ccbd::db::system::reconcile_startup_with_state_dir(
        h.ctx.db.clone(),
        h.ctx.state_dir.clone(),
    )
    .await
    .unwrap();
    let agent = query_agent(h.ctx.db.clone(), "ag_reconcile_fifo".to_string())
        .await
        .unwrap()
        .unwrap();
    let job = query_job(h.ctx.db.clone(), "job_reconcile_fifo".to_string())
        .await
        .unwrap()
        .unwrap();

    assert_eq!(count, 1);
    assert_eq!(agent.state, "CRASHED");
    assert_eq!(
        agent.error_code.as_deref(),
        Some("STARTUP_RECONCILE_FIFO_REATTACH_FAILED")
    );
    assert_eq!(job.status, "FAILED");
    assert_eq!(
        job.error_reason.as_deref(),
        Some("STARTUP_RECONCILE_FIFO_REATTACH_FAILED")
    );
    let _ = ccbd::monitor::remove("master:s_reconcile_fifo");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_launcher_batch_spawn_and_layout() {
    let h = Harness::new();
    let config = toml::from_str::<ProjectConfig>(
        r#"
version = "1"
layout = "grid"

[agents.a1]
provider = "bash"

[agents.a2]
provider = "bash"

[agents.a3]
provider = "bash"
"#,
    )
    .unwrap();
    let client = HandlerClient { ctx: h.ctx.clone() };
    let project_root = h.ctx.state_dir.join("ccb-project");
    std::fs::create_dir_all(&project_root).unwrap();

    let summary = start_project(
        &client,
        config,
        std::path::Path::new("test-ccb.toml"),
        project_root,
        false,
        std::process::id() as i64,
    )
    .await
    .unwrap();

    let panes = ccbd::tmux::layout::list_panes(
        &h.ctx.tmux_server,
        format!("{}:{}", ccbd::tmux::SESSION_NAME, summary.session_id),
    )
    .await
    .unwrap();
    assert_eq!(summary.agents.len(), 3);
    assert_eq!(panes.len(), 3);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_concurrent_agent_spawn_serializes_window_creation() {
    let h = Harness::new();
    let session = handle_session_create(
        json!({
            "project_id": "p_concurrent",
            "absolute_path": h.ctx.state_dir.join("concurrent").display().to_string(),
            "master_pid": std::process::id() as i64,
        }),
        &h.ctx,
    )
    .await
    .unwrap();
    let session_id = session["session_id"].as_str().unwrap().to_string();

    let f1 = handle_agent_spawn(
        json!({ "session_id": session_id, "agent_id": "ag_c1", "provider": "bash" }),
        &h.ctx,
    );
    let f2 = handle_agent_spawn(
        json!({ "session_id": session_id, "agent_id": "ag_c2", "provider": "bash" }),
        &h.ctx,
    );
    let f3 = handle_agent_spawn(
        json!({ "session_id": session_id, "agent_id": "ag_c3", "provider": "bash" }),
        &h.ctx,
    );
    let (r1, r2, r3) = tokio::join!(f1, f2, f3);
    r1.unwrap();
    r2.unwrap();
    r3.unwrap();

    let panes = ccbd::tmux::layout::list_panes(
        &h.ctx.tmux_server,
        format!("{}:{}", ccbd::tmux::SESSION_NAME, session_id),
    )
    .await
    .unwrap();
    assert_eq!(panes.len(), 3);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_job_cancel_queued() {
    let h = Harness::new();
    ccbd::db::sessions::insert_session(
        h.ctx.db.clone(),
        "s_cancel_q".to_string(),
        "p_cancel_q".to_string(),
        h.ctx.state_dir.join("cancel-q").display().to_string(),
        std::process::id() as i64,
    )
    .await
    .unwrap();
    ccbd::db::agents::insert_agent(
        h.ctx.db.clone(),
        "ag_cancel_q".to_string(),
        "s_cancel_q".to_string(),
        "bash".to_string(),
        "IDLE".to_string(),
        Some(std::process::id() as i64),
    )
    .await
    .unwrap();
    insert_job(
        h.ctx.db.clone(),
        "job_cancel_q".to_string(),
        "ag_cancel_q".to_string(),
        None,
        "echo queued\n".to_string(),
    )
    .await
    .unwrap();

    let result = handle_job_cancel(json!({ "job_id": "job_cancel_q" }), &h.ctx)
        .await
        .unwrap();
    let job = query_job(h.ctx.db.clone(), "job_cancel_q".to_string())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(result["status"], "CANCELLED");
    assert_eq!(job.status, "CANCELLED");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_job_cancel_dispatched_sends_sigint() {
    let h = Harness::new();
    let session = handle_session_create(
        json!({
            "project_id": "p_cancel_d",
            "absolute_path": h.ctx.state_dir.join("cancel-d").display().to_string(),
            "master_pid": std::process::id() as i64,
        }),
        &h.ctx,
    )
    .await
    .unwrap();
    let session_id = session["session_id"].as_str().unwrap().to_string();
    handle_agent_spawn(
        json!({ "session_id": session_id, "agent_id": "ag_cancel_d", "provider": "bash" }),
        &h.ctx,
    )
    .await
    .unwrap();
    wait_for_agent_state(&h, "ag_cancel_d", "IDLE", Duration::from_secs(10)).await;

    let sent = handle_agent_send(
        json!({
            "agent_id": "ag_cancel_d",
            "text": "sleep 10; echo should_not_finish\n",
            "request_id": "req_cancel_d"
        }),
        &h.ctx,
    )
    .await
    .unwrap();
    let seq_id = sent["seq_id"].as_i64().unwrap();
    insert_job(
        h.ctx.db.clone(),
        "job_cancel_d".to_string(),
        "ag_cancel_d".to_string(),
        None,
        "sleep 10; echo should_not_finish\n".to_string(),
    )
    .await
    .unwrap();
    h.ctx
        .db
        .conn()
        .execute(
            "UPDATE jobs SET status = 'DISPATCHED', dispatched_at = unixepoch(), dispatched_at_seq_id = ? WHERE id = 'job_cancel_d'",
            [seq_id],
        )
        .unwrap();
    let start = Instant::now();

    let result = handle_job_cancel(json!({ "job_id": "job_cancel_d" }), &h.ctx)
        .await
        .unwrap();
    assert_eq!(result["status"], "CANCEL_REQUESTED");
    wait_for_job_status(
        &h,
        result["job_id"].as_str().unwrap(),
        "CANCELLED",
        Duration::from_secs(20),
    )
    .await;
    assert!(start.elapsed() < Duration::from_secs(20));
}

#[tokio::test(flavor = "multi_thread")]
async fn test_cancel_requested_skips_prompt_only_swallow() {
    let h = Harness::new();
    ccbd::db::sessions::insert_session(
        h.ctx.db.clone(),
        "s_prompt_cancel".to_string(),
        "p_prompt_cancel".to_string(),
        h.ctx.state_dir.join("prompt-cancel").display().to_string(),
        std::process::id() as i64,
    )
    .await
    .unwrap();
    ccbd::db::agents::insert_agent(
        h.ctx.db.clone(),
        "ag_prompt_cancel".to_string(),
        "s_prompt_cancel".to_string(),
        "bash".to_string(),
        "BUSY".to_string(),
        Some(std::process::id() as i64),
    )
    .await
    .unwrap();
    let before = ccbd::db::events::insert_event(
        h.ctx.db.clone(),
        "ag_prompt_cancel".to_string(),
        None,
        "command_received".to_string(),
        r#"{"cmd":"sleep 10","status":"SENT"}"#.to_string(),
    )
    .await
    .unwrap();
    ccbd::db::events::insert_event(
        h.ctx.db.clone(),
        "ag_prompt_cancel".to_string(),
        None,
        "output_chunk".to_string(),
        r#"{"text":"$ "}"#.to_string(),
    )
    .await
    .unwrap();
    insert_job(
        h.ctx.db.clone(),
        "job_prompt_cancel".to_string(),
        "ag_prompt_cancel".to_string(),
        None,
        "sleep 10\n".to_string(),
    )
    .await
    .unwrap();
    ccbd::db::jobs::claim_next_job(h.ctx.db.clone(), "ag_prompt_cancel".to_string())
        .await
        .unwrap();
    ccbd::db::jobs::update_dispatched_seq_id(
        h.ctx.db.clone(),
        "job_prompt_cancel".to_string(),
        before,
    )
    .await
    .unwrap();
    ccbd::db::jobs::request_dispatched_job_cancel(
        h.ctx.db.clone(),
        "job_prompt_cancel".to_string(),
    )
    .await
    .unwrap();

    ccbd::db::state_machine::mark_agent_idle_matched(
        h.ctx.db.clone(),
        "ag_prompt_cancel".to_string(),
    )
    .await
    .unwrap();
    let job = query_job(h.ctx.db.clone(), "job_prompt_cancel".to_string())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(job.status, "CANCELLED");
}

async fn wait_for_agent_state(h: &Harness, agent_id: &str, expected: &str, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        let agent = query_agent(h.ctx.db.clone(), agent_id.to_string())
            .await
            .unwrap()
            .unwrap();
        if agent.state == expected {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("agent {agent_id} did not reach {expected}");
}

async fn wait_for_job_status(h: &Harness, job_id: &str, expected: &str, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        let job = query_job(h.ctx.db.clone(), job_id.to_string())
            .await
            .unwrap()
            .unwrap();
        if job.status == expected {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("job {job_id} did not reach {expected}");
}

fn map_rpc_error(err: ccbd::error::CcbdError) -> CliError {
    CliError::Rpc {
        code: -32000,
        message: err.to_string(),
    }
}
