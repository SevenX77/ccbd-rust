use ah::db;
use ah::db::state_machine::{STATE_BUSY, STATE_IDLE};
use ah::rpc::Ctx;
use ah::sandbox::EnvState;
use ah::tmux::TmuxServer;
use serde_json::json;
use std::sync::Arc;
use std::time::{Duration, Instant};

fn test_ctx() -> Ctx {
    let state_dir = tempfile::TempDir::new().unwrap().keep();
    let db_path = state_dir.join("dispatch_atomicity.sqlite");
    Ctx {
        db: db::init(&db_path).unwrap(),
        state_dir: state_dir.clone(),
        env_state: EnvState {
            systemd_run_available: false,
            unsafe_no_sandbox: true,
            under_systemd: false,
        },
        daemon_unit: None,
        tmux_server: Arc::new(TmuxServer::new(&state_dir)),
        claude_gateway: std::sync::Arc::new(ah::claude_gateway::ClaudeGatewayService::new()),
    }
}

async fn seed_idle_agent_with_job(ctx: &Ctx, agent_id: &str, job_id: &str) {
    db::sessions::insert_session(
        ctx.db.clone(),
        format!("s_{agent_id}"),
        format!("p_{agent_id}"),
        format!("/tmp/{agent_id}"),
    )
    .await
    .unwrap();
    db::agents::insert_agent(
        ctx.db.clone(),
        agent_id.to_string(),
        format!("s_{agent_id}"),
        "bash".to_string(),
        STATE_IDLE.to_string(),
        Some(123),
    )
    .await
    .unwrap();
    db::jobs::insert_job(
        ctx.db.clone(),
        job_id.to_string(),
        agent_id.to_string(),
        None,
        "echo dispatch\n".to_string(),
    )
    .await
    .unwrap();
}

async fn wait_for_job_status(ctx: &Ctx, job_id: &str, expected: &str) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        let status = db::jobs::query_job(ctx.db.clone(), job_id.to_string())
            .await
            .unwrap()
            .map(|job| job.status);
        if status.as_deref() == Some(expected) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("job {job_id} did not reach {expected}");
}

#[tokio::test(flavor = "multi_thread")]
async fn dispatch_io_failure_compensates_agent_to_stuck() {
    let ctx = test_ctx();
    seed_idle_agent_with_job(&ctx, "ag_dispatch_no_pane", "job_dispatch_no_pane").await;

    ah::orchestrator::spawn_orchestrator_task(ctx.clone());
    wait_for_job_status(&ctx, "job_dispatch_no_pane", "FAILED").await;

    let agent = db::agents::query_agent(ctx.db.clone(), "ag_dispatch_no_pane".to_string())
        .await
        .unwrap()
        .unwrap();
    let job = db::jobs::query_job(ctx.db.clone(), "job_dispatch_no_pane".to_string())
        .await
        .unwrap()
        .unwrap();
    let state_change_count: i64 = ctx
        .db
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM events WHERE agent_id = 'ag_dispatch_no_pane' AND event_type = 'state_change'",
            [],
            |row| row.get(0),
        )
        .unwrap();

    assert_eq!(agent.state, "STUCK");
    assert_eq!(
        job.error_reason.as_deref(),
        Some("tmux pane not registered")
    );
    assert!(job.dispatched_at_seq_id.is_some());
    assert_eq!(state_change_count, 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn dispatch_atomicity_preserves_seq_id_through_completion() {
    let ctx = test_ctx();
    seed_idle_agent_with_job(&ctx, "ag_dispatch_chain", "job_dispatch_chain").await;

    let dispatched = db::jobs::dispatch_job_to_agent(
        ctx.db.clone(),
        "ag_dispatch_chain".to_string(),
        vec![STATE_IDLE.to_string()],
        STATE_BUSY.to_string(),
        "command_received".to_string(),
        json!({ "status": "SENT" }),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(dispatched.job.dispatched_at_seq_id, Some(dispatched.seq_id));

    db::events::insert_event(
        ctx.db.clone(),
        "ag_dispatch_chain".to_string(),
        None,
        "output_chunk".to_string(),
        json!({ "text": "dispatch reply\n" }).to_string(),
    )
    .await
    .unwrap();
    db::state_machine::mark_agent_idle_matched(ctx.db.clone(), "ag_dispatch_chain".to_string())
        .await
        .unwrap();

    let agent = db::agents::query_agent(ctx.db.clone(), "ag_dispatch_chain".to_string())
        .await
        .unwrap()
        .unwrap();
    let job = db::jobs::query_job(ctx.db.clone(), "job_dispatch_chain".to_string())
        .await
        .unwrap()
        .unwrap();

    assert_eq!(agent.state, STATE_IDLE);
    assert_eq!(job.status, "COMPLETED");
    assert_eq!(job.dispatched_at_seq_id, Some(dispatched.seq_id));
    assert_eq!(job.reply_text.as_deref(), Some("dispatch reply"));
}
