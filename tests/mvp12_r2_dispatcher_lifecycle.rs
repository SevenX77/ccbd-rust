use ah::db;
use ah::orchestrator::pubsub;
use tokio::time::{Duration, timeout};

#[tokio::test(flavor = "multi_thread")]
async fn test_r2_idle_match_caller_notifies_waiters_and_completes_job() {
    let file = tempfile::NamedTempFile::new().unwrap();
    let db = db::init(file.path()).unwrap();

    db::sessions::insert_session(
        db.clone(),
        "s_r2".to_string(),
        "p_r2".to_string(),
        "/tmp/r2".to_string(),
    )
    .await
    .unwrap();
    db::agents::insert_agent(
        db.clone(),
        "ag_r2".to_string(),
        "s_r2".to_string(),
        "bash".to_string(),
        "IDLE".to_string(),
        Some(123),
    )
    .await
    .unwrap();
    db::jobs::insert_job(
        db.clone(),
        "job_r2".to_string(),
        "ag_r2".to_string(),
        Some("req_r2".to_string()),
        "echo 1".to_string(),
    )
    .await
    .unwrap();
    let dispatched = db::jobs::dispatch_job_to_agent(
        db.clone(),
        "ag_r2".to_string(),
        vec!["IDLE".to_string()],
        "BUSY".to_string(),
        "command_received".to_string(),
        serde_json::json!({ "cmd": "echo 1", "status": "SENT", "job_id": "job_r2" }),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(dispatched.job.id, "job_r2");
    db::events::insert_event(
        db.clone(),
        "ag_r2".to_string(),
        None,
        "output_chunk".to_string(),
        serde_json::json!({ "text": "reply 1\n" }).to_string(),
    )
    .await
    .unwrap();

    let mut rx = pubsub::subscribe_job_updates();
    let (changes, affected_job) =
        db::state_machine::mark_agent_idle_matched(db.clone(), "ag_r2".to_string())
            .await
            .unwrap();
    assert_eq!(changes, 1);
    if let Some(job_id) = affected_job {
        // R-2 (mvp12): notify hoisted from state_machine wrapper for clearer dispatcher boundary.
        pubsub::notify_job_update(&job_id);
    }

    let notified_job = timeout(Duration::from_secs(5), rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(notified_job, "job_r2");

    let agent = db::agents::query_agent(db.clone(), "ag_r2".to_string())
        .await
        .unwrap()
        .unwrap();
    let job = db::jobs::query_job(db.clone(), "job_r2".to_string())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(agent.state, "IDLE");
    assert_eq!(job.status, "COMPLETED");
    assert_eq!(job.reply_text.as_deref(), Some("reply 1"));
}
