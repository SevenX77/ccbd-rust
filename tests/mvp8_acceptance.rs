#![cfg(unix)]

mod common;

use ah::db;
use ah::db::agents::{insert_agent, query_agent_state};
use ah::db::events::query_events_since;
use ah::db::jobs::{insert_job, query_job};
use ah::db::sessions::insert_session;
use ah::rpc::Ctx;
use ah::rpc::handlers::{
    handle_agent_kill, handle_agent_spawn, handle_agent_watch, handle_job_submit, handle_job_wait,
};
use ah::sandbox::EnvState;
use ah::tmux::{TmuxServer, compute_socket_name};
use common::scope_policy_for_test;
use serde_json::json;
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

struct Harness {
    ctx: Ctx,
    _state_dir: tempfile::TempDir,
    _db_file: tempfile::NamedTempFile,
}

impl Harness {
    fn new() -> Self {
        which::which("tmux").expect("tmux binary required for mvp8 acceptance tests");
        let db_file = tempfile::NamedTempFile::new().unwrap();
        let state_dir = tempfile::TempDir::new().unwrap();
        let state_dir_path = state_dir.path().to_path_buf();
        let socket_name = compute_socket_name(state_dir.path());
        let policy = scope_policy_for_test(&socket_name);
        let ctx = Ctx {
            db: db::init(db_file.path()).unwrap(),
            state_dir: state_dir_path.clone(),
            env_state: EnvState {
                systemd_run_available: false,
                unsafe_no_sandbox: true,
                under_systemd: false,
            },
            daemon_unit: None,
            tmux_server: Arc::new(TmuxServer::new_with_policy(state_dir.path(), policy)),
        };

        Self {
            ctx,
            _state_dir: state_dir,
            _db_file: db_file,
        }
    }

    async fn seed_agent(&self, agent_id: &str, state: &str) {
        insert_session(
            self.ctx.db.clone(),
            "s_m8".to_string(),
            "p_m8".to_string(),
            "/tmp/m8".to_string(),
        )
        .await
        .unwrap();
        insert_agent(
            self.ctx.db.clone(),
            agent_id.to_string(),
            "s_m8".to_string(),
            "bash".to_string(),
            state.to_string(),
            Some(123),
        )
        .await
        .unwrap();
    }

    async fn insert_session(&self, session_id: &str) {
        insert_session(
            self.ctx.db.clone(),
            session_id.to_string(),
            format!("p_{session_id}"),
            format!("/tmp/{session_id}"),
        )
        .await
        .unwrap();
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

async fn sleep_ms(ms: u64) {
    tokio::time::sleep(Duration::from_millis(ms)).await;
}

async fn current_state(h: &Harness, agent_id: &str) -> Option<String> {
    query_agent_state(h.ctx.db.clone(), agent_id.to_string())
        .await
        .unwrap()
        .map(|(state, _)| state)
}

async fn wait_for_state(h: &Harness, agent_id: &str, expected: &str, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if current_state(h, agent_id).await.as_deref() == Some(expected) {
            return;
        }
        sleep_ms(100).await;
    }
    panic!("agent {agent_id} did not reach {expected} within {timeout:?}");
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
        sleep_ms(100).await;
    }
    let job = query_job(h.ctx.db.clone(), job_id.to_string())
        .await
        .unwrap()
        .unwrap();
    panic!(
        "job {job_id} did not reach {expected} within {timeout:?}; status={}",
        job.status
    );
}

async fn spawn_bash(h: &Harness, session_id: &str, agent_id: &str) {
    let result = handle_agent_spawn(
        json!({
            "session_id": session_id,
            "agent_id": agent_id,
            "provider": "bash",
        }),
        &h.ctx,
    )
    .await
    .unwrap();
    assert_eq!(result["state"], "SPAWNING");
    wait_for_state(h, agent_id, "IDLE", Duration::from_secs(15)).await;
}

async fn submit_job(h: &Harness, agent_id: &str, text: &str) -> String {
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

async fn latest_event_id(h: &Harness, agent_id: &str) -> i64 {
    query_events_since(h.ctx.db.clone(), agent_id.to_string(), 0)
        .await
        .unwrap()
        .into_iter()
        .map(|event| event.seq_id)
        .max()
        .unwrap_or(0)
}

#[tokio::test(flavor = "multi_thread")]
async fn test_job_submit_returns_id_and_queues_when_idle_or_busy() {
    let h = Harness::new();
    h.seed_agent("ag_m8_submit", "IDLE").await;

    let result = handle_job_submit(
        json!({
            "agent_id": "ag_m8_submit",
            "text": "echo mvp8\n",
        }),
        &h.ctx,
    )
    .await
    .unwrap();
    let job_id = result["job_id"].as_str().unwrap();

    assert!(job_id.starts_with("job_"));
    assert_eq!(result["status"], "QUEUED");
    let job = query_job(h.ctx.db.clone(), job_id.to_string())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(job.agent_id, "ag_m8_submit");
    assert_eq!(job.prompt_text, "echo mvp8\n");
    assert_eq!(job.status, "QUEUED");
    assert_eq!(job.dispatched_at_seq_id, None);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_job_submit_is_idempotent_by_request_id() {
    let h = Harness::new();
    h.seed_agent("ag_m8_idem", "BUSY").await;

    let first = handle_job_submit(
        json!({
            "agent_id": "ag_m8_idem",
            "text": "echo first\n",
            "request_id": "m8-req-1",
        }),
        &h.ctx,
    )
    .await
    .unwrap();
    let second = handle_job_submit(
        json!({
            "agent_id": "ag_m8_idem",
            "text": "echo second\n",
            "request_id": "m8-req-1",
        }),
        &h.ctx,
    )
    .await
    .unwrap();

    assert_eq!(first["job_id"], second["job_id"]);
    let job = query_job(
        h.ctx.db.clone(),
        first["job_id"].as_str().unwrap().to_string(),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(job.prompt_text, "echo first\n");
    assert_eq!(job.request_id.as_deref(), Some("m8-req-1"));
}

#[tokio::test(flavor = "multi_thread")]
async fn test_job_submit_returns_id_then_dispatched_when_idle() {
    let h = Harness::new();
    h.insert_session("s_m8_dispatch").await;
    let agent_id = format!("ag_m8_dispatch_{}", uuid::Uuid::new_v4());
    spawn_bash(&h, "s_m8_dispatch", &agent_id).await;
    ah::orchestrator::spawn_orchestrator_task(h.ctx.clone());

    let job_id = submit_job(&h, &agent_id, "sleep 1; echo m8-dispatch\n").await;

    wait_for_job_status(&h, &job_id, "DISPATCHED", Duration::from_secs(10)).await;
    wait_for_state(&h, &agent_id, "BUSY", Duration::from_secs(10)).await;
    let job = query_job(h.ctx.db.clone(), job_id).await.unwrap().unwrap();
    assert!(job.dispatched_at.is_some());
    assert!(job.dispatched_at_seq_id.is_some());
    let _ = handle_agent_kill(json!({ "agent_id": agent_id }), &h.ctx).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn test_serial_per_agent_two_concurrent_asks() {
    let h = Harness::new();
    h.insert_session("s_m8_serial").await;
    let agent_id = format!("ag_m8_serial_{}", uuid::Uuid::new_v4());
    spawn_bash(&h, "s_m8_serial", &agent_id).await;
    ah::orchestrator::spawn_orchestrator_task(h.ctx.clone());

    let first_job = submit_job(&h, &agent_id, "sleep 1; echo m8-first\n").await;
    let second_job = submit_job(&h, &agent_id, "sleep 1; echo m8-second\n").await;

    wait_for_job_status(&h, &first_job, "DISPATCHED", Duration::from_secs(10)).await;
    let second = query_job(h.ctx.db.clone(), second_job.clone())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(second.status, "QUEUED");

    wait_for_job_status(&h, &first_job, "COMPLETED", Duration::from_secs(15)).await;
    wait_for_job_status(&h, &second_job, "DISPATCHED", Duration::from_secs(10)).await;
    let _ = handle_agent_kill(json!({ "agent_id": agent_id }), &h.ctx).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn test_pend_blocks_until_completed() {
    let h = Harness::new();
    h.insert_session("s_m8_pend").await;
    let agent_id = format!("ag_m8_pend_{}", uuid::Uuid::new_v4());
    spawn_bash(&h, "s_m8_pend", &agent_id).await;
    ah::orchestrator::spawn_orchestrator_task(h.ctx.clone());

    let job_id = submit_job(&h, &agent_id, "echo mvp8_done\n").await;
    let result = handle_job_wait(
        json!({
            "job_id": job_id,
            "timeout": 10,
        }),
        &h.ctx,
    )
    .await
    .unwrap();

    assert_eq!(result["status"], "COMPLETED");
    assert!(result["reply_text"].as_str().unwrap().contains("mvp8_done"));
    let _ = handle_agent_kill(json!({ "agent_id": agent_id }), &h.ctx).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn test_agent_watch_receives_output_chunk() {
    let h = Harness::new();
    h.insert_session("s_m8_watch").await;
    let agent_id = format!("ag_m8_watch_{}", uuid::Uuid::new_v4());
    spawn_bash(&h, "s_m8_watch", &agent_id).await;
    let since_event_id = latest_event_id(&h, &agent_id).await;
    ah::orchestrator::spawn_orchestrator_task(h.ctx.clone());

    let watch_ctx = h.ctx.clone();
    let watch_agent_id = agent_id.clone();
    let watch_task = tokio::spawn(async move {
        handle_agent_watch(
            json!({
                "agent_id": watch_agent_id,
                "since_event_id": since_event_id,
                "timeout": 10,
            }),
            &watch_ctx,
        )
        .await
    });
    sleep_ms(100).await;
    let job_id = submit_job(&h, &agent_id, "echo mvp8_watch_done\n").await;
    let result = watch_task.await.unwrap().unwrap();

    let events = result["events"].as_array().unwrap();
    assert!(
        events.iter().any(|event| {
            event["event_type"] == "output_chunk"
                && event["payload"]
                    .as_str()
                    .is_some_and(|payload| payload.contains("mvp8_watch_done"))
        }),
        "events={events:?}"
    );
    wait_for_job_status(&h, &job_id, "COMPLETED", Duration::from_secs(10)).await;
    let _ = handle_agent_kill(json!({ "agent_id": agent_id }), &h.ctx).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn test_persistence_across_daemon_restart() {
    let db_file = tempfile::NamedTempFile::new().unwrap();
    let db_path = db_file.path().to_path_buf();

    {
        let db = db::init(&db_path).unwrap();
        insert_session(
            db.clone(),
            "s_m8_restart".to_string(),
            "p_m8_restart".to_string(),
            "/tmp/m8_restart".to_string(),
        )
        .await
        .unwrap();
        insert_agent(
            db.clone(),
            "ag_m8_restart".to_string(),
            "s_m8_restart".to_string(),
            "bash".to_string(),
            "IDLE".to_string(),
            Some(123),
        )
        .await
        .unwrap();
        insert_job(
            db,
            "job_m8_restart".to_string(),
            "ag_m8_restart".to_string(),
            Some("req-m8-restart".to_string()),
            "echo persisted\n".to_string(),
        )
        .await
        .unwrap();
    }

    let reopened = db::init(&db_path).unwrap();
    let job = query_job(reopened, "job_m8_restart".to_string())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(job.status, "QUEUED");
    assert_eq!(job.prompt_text, "echo persisted\n");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_startup_reconcile_fails_dispatched_job() {
    let db_file = tempfile::NamedTempFile::new().unwrap();
    let db_path = db_file.path().to_path_buf();

    {
        let db = db::init(&db_path).unwrap();
        insert_session(
            db.clone(),
            "s_m8_reconcile".to_string(),
            "p_m8_reconcile".to_string(),
            "/tmp/m8_reconcile".to_string(),
        )
        .await
        .unwrap();
        insert_agent(
            db.clone(),
            "ag_m8_reconcile".to_string(),
            "s_m8_reconcile".to_string(),
            "bash".to_string(),
            "BUSY".to_string(),
            Some(999_999_999),
        )
        .await
        .unwrap();
        insert_job(
            db.clone(),
            "job_m8_reconcile".to_string(),
            "ag_m8_reconcile".to_string(),
            Some("req-m8-reconcile".to_string()),
            "echo lost\n".to_string(),
        )
        .await
        .unwrap();
        db.conn()
            .execute(
                "UPDATE jobs SET status = 'DISPATCHED', dispatched_at = unixepoch(), dispatched_at_seq_id = 42 WHERE id = 'job_m8_reconcile'",
                [],
            )
            .unwrap();
    }

    let reopened = db::init(&db_path).unwrap();
    let count = db::system::reconcile_startup(reopened.clone())
        .await
        .unwrap();
    let job = query_job(reopened.clone(), "job_m8_reconcile".to_string())
        .await
        .unwrap()
        .unwrap();
    let state = query_agent_state(reopened, "ag_m8_reconcile".to_string())
        .await
        .unwrap()
        .unwrap()
        .0;

    assert_eq!(count, 1);
    assert_eq!(state, "CRASHED");
    assert_eq!(job.status, "FAILED");
    assert_eq!(
        job.error_reason.as_deref(),
        Some("STARTUP_RECONCILE_DEAD_PID")
    );
}

#[ignore]
#[test]
fn test_true_codex_ask_pend_roundtrip() {
    eprintln!(
        "manual smoke only: requires a running daemon, a spawned codex agent, and logged-in Codex auth"
    );
}
