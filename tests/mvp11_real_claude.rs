use ccbd::db;
use ccbd::rpc::Ctx;
use ccbd::rpc::handlers::{
    handle_agent_spawn, handle_job_submit, handle_job_wait, handle_session_create,
    handle_session_kill,
};
use ccbd::sandbox::EnvState;
use ccbd::tmux::{TmuxServer, compute_socket_name};
use serde_json::json;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

mod common;

struct RealHarness {
    ctx: Ctx,
    sessions: Arc<Mutex<Vec<String>>>,
    _state_dir: tempfile::TempDir,
    _db_file: tempfile::NamedTempFile,
}

impl RealHarness {
    fn new() -> Self {
        hard_gate("claude", &["ANTHROPIC_API_KEY"]);
        let db_file = tempfile::NamedTempFile::new().unwrap();
        let state_dir = tempfile::TempDir::new().unwrap();
        let socket_name = compute_socket_name(state_dir.path());
        let ctx = Ctx {
            db: db::init(db_file.path()).unwrap(),
            state_dir: state_dir.path().to_path_buf(),
            env_state: EnvState {
                bwrap_available: true,
                systemd_run_available: true,
                unsafe_no_sandbox: false,
            },
            tmux_server: Arc::new(TmuxServer::new_with_policy(
                state_dir.path(),
                common::scope_policy_for_test(&socket_name),
            )),
        };
        ccbd::orchestrator::spawn_orchestrator_task(ctx.clone());
        Self {
            ctx,
            sessions: Arc::new(Mutex::new(Vec::new())),
            _state_dir: state_dir,
            _db_file: db_file,
        }
    }

    async fn create_session(&self) -> String {
        let result = handle_session_create(
            json!({
                "project_id": "mvp11-real-claude",
                "absolute_path": self.ctx.state_dir.display().to_string(),
            }),
            &self.ctx,
        )
        .await
        .unwrap();
        let session_id = result["session_id"].as_str().unwrap().to_string();
        self.sessions.lock().unwrap().push(session_id.clone());
        session_id
    }
}

impl Drop for RealHarness {
    fn drop(&mut self) {
        for session_id in self.sessions.lock().unwrap().iter() {
            stop_anchor(session_id);
        }
        let _ = Command::new("tmux")
            .args(["-L", self.ctx.tmux_server.socket_name(), "kill-server"])
            .output();
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_claude_spawn_ask_flow() {
    if std::env::var("CCB_TEST_SKIP_REAL_PROVIDER").as_deref() == Ok("1") {
        return;
    }
    let h = RealHarness::new();
    let session_id = h.create_session().await;
    let agent_id = "ag_mvp11_real_claude";
    spawn_provider(&h, &session_id, agent_id, "claude").await;

    let env_job = submit_job(
        &h,
        agent_id,
        "Run `env | grep '^CCB_'` and reply with the values for CCB_CLAUDE_MD_MODE, CCB_REPLY_LANG, and CCB_CTX_TRANSFER_LAST_N.\n",
    )
    .await;
    let env_reply = wait_job(&h, &env_job).await;
    assert!(env_reply.contains("CCB_CLAUDE_MD_MODE"));
    assert!(env_reply.contains("CCB_REPLY_LANG"));
    assert!(env_reply.contains("CCB_CTX_TRANSFER_LAST_N"));

    let first = submit_job(&h, agent_id, "Reply with exactly: echo 1\n").await;
    assert!(wait_job(&h, &first).await.contains('1'));
    let second = submit_job(&h, agent_id, "Reply with exactly: echo 2\n").await;
    assert!(wait_job(&h, &second).await.contains('2'));
    wait_for_agent_state(&h.ctx, agent_id, "IDLE", Duration::from_secs(30)).await;
    let _ = handle_session_kill(json!({ "session_id": session_id, "force": true }), &h.ctx).await;
}

async fn spawn_provider(h: &RealHarness, session_id: &str, agent_id: &str, provider: &str) {
    handle_agent_spawn(
        json!({
            "session_id": session_id,
            "agent_id": agent_id,
            "provider": provider,
        }),
        &h.ctx,
    )
    .await
    .unwrap();
    wait_for_agent_state(&h.ctx, agent_id, "IDLE", Duration::from_secs(90)).await;
}

async fn submit_job(h: &RealHarness, agent_id: &str, text: &str) -> String {
    let result = handle_job_submit(
        json!({
            "agent_id": agent_id,
            "text": text,
            "request_id": format!("req_{}", uuid::Uuid::new_v4()),
        }),
        &h.ctx,
    )
    .await
    .unwrap();
    result["job_id"].as_str().unwrap().to_string()
}

async fn wait_job(h: &RealHarness, job_id: &str) -> String {
    let result = handle_job_wait(json!({ "job_id": job_id, "timeout": 120 }), &h.ctx)
        .await
        .unwrap();
    assert_eq!(result["status"], "COMPLETED");
    result["reply_text"].as_str().unwrap().to_string()
}

fn hard_gate(binary: &str, env_keys: &[&str]) {
    assert!(
        which::which(binary).is_ok(),
        "missing {binary} binary, set CCB_TEST_SKIP_REAL_PROVIDER=1 to opt-out"
    );
    assert!(
        env_keys.iter().any(|key| std::env::var(key).is_ok()),
        "missing one of {env_keys:?}, set CCB_TEST_SKIP_REAL_PROVIDER=1 to opt-out"
    );
    assert!(
        which::which("tmux").is_ok()
            && which::which("bwrap").is_ok()
            && common::can_use_systemd_run(),
        "real provider tests require tmux, bwrap, and user systemd; set CCB_TEST_SKIP_REAL_PROVIDER=1 to opt-out"
    );
}

async fn wait_for_agent_state(ctx: &Ctx, agent_id: &str, expected: &str, timeout: Duration) {
    wait_for(
        || agent_state(ctx, agent_id).as_deref() == Some(expected),
        timeout,
        &format!("agent {agent_id} state {expected}"),
    )
    .await;
}

async fn wait_for(mut condition: impl FnMut() -> bool, timeout: Duration, label: &str) {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if condition() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    panic!("timed out waiting for {label}");
}

fn agent_state(ctx: &Ctx, agent_id: &str) -> Option<String> {
    ctx.db
        .conn()
        .query_row(
            "SELECT state FROM agents WHERE id = ?1",
            [agent_id],
            |row| row.get(0),
        )
        .ok()
}

fn stop_anchor(session_id: &str) {
    let _ = Command::new("systemctl")
        .args([
            "--user",
            "stop",
            &format!("ccbd-session-{session_id}.service"),
        ])
        .output();
}
