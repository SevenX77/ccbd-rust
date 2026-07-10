use ah::db::{
    agents::{insert_agent, query_agent_state},
    jobs::{insert_job, query_job},
    sessions::insert_session,
    state_machine::{
        self, STATE_BUSY, STATE_CRASHED, STATE_IDLE, STATE_STUCK, STATE_WAITING_FOR_ACK,
    },
};
use ah::rpc::handlers::{
    AckBusyOutcome, ack_mark_busy_or_resolve, fallback_ack_to_crashed, fallback_ack_to_stuck,
};

async fn seed_agent(db: ah::db::Db, agent_id: &str, state: &str) {
    insert_session(
        db.clone(),
        format!("s_{agent_id}"),
        format!("p_{agent_id}"),
        format!("/tmp/{agent_id}"),
    )
    .await
    .unwrap();
    insert_agent(
        db,
        agent_id.to_string(),
        format!("s_{agent_id}"),
        "bash".to_string(),
        state.to_string(),
        Some(123),
    )
    .await
    .unwrap();
}

async fn seed_agent_with_dispatched_job(db: ah::db::Db, agent_id: &str, state: &str, job_id: &str) {
    seed_agent(db.clone(), agent_id, state).await;
    insert_job(
        db.clone(),
        job_id.to_string(),
        agent_id.to_string(),
        None,
        "run task\n".to_string(),
    )
    .await
    .unwrap();
    db.conn()
        .execute(
            "UPDATE jobs SET status = 'DISPATCHED', dispatched_at = unixepoch(), dispatched_at_seq_id = 1 WHERE id = ?",
            [job_id],
        )
        .unwrap();
}

fn latest_state_change_reason(db: &ah::db::Db, agent_id: &str) -> String {
    let payload: String = db
        .conn()
        .query_row(
            "SELECT payload FROM events \
             WHERE agent_id = ? AND event_type = 'state_change' \
             ORDER BY seq_id DESC LIMIT 1",
            [agent_id],
            |row| row.get(0),
        )
        .unwrap();
    let payload: serde_json::Value = serde_json::from_str(&payload).unwrap();
    payload["reason"].as_str().unwrap().to_string()
}

#[tokio::test(flavor = "multi_thread")]
async fn ack_capture_failure_fallback_marks_waiting_agent_stuck() {
    let file = tempfile::NamedTempFile::new().unwrap();
    let db = ah::db::init(file.path()).unwrap();
    seed_agent(db.clone(), "ag_ack_stuck", STATE_WAITING_FOR_ACK).await;

    let changes =
        fallback_ack_to_stuck(db.clone(), "ag_ack_stuck", "tmux_capture_failed_during_ack")
            .await
            .unwrap();
    let state = query_agent_state(db.clone(), "ag_ack_stuck".to_string())
        .await
        .unwrap()
        .map(|(state, _)| state);

    assert_eq!(changes, 1);
    assert_eq!(state.as_deref(), Some(STATE_STUCK));
    assert_eq!(
        latest_state_change_reason(&db, "ag_ack_stuck"),
        "tmux_capture_failed_during_ack"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn ack_pane_or_reader_loss_fallback_marks_waiting_agent_crashed() {
    let file = tempfile::NamedTempFile::new().unwrap();
    let db = ah::db::init(file.path()).unwrap();
    seed_agent(db.clone(), "ag_ack_crashed", STATE_WAITING_FOR_ACK).await;

    let changes =
        fallback_ack_to_crashed(db.clone(), "ag_ack_crashed", "pane_unregistered_during_ack")
            .await
            .unwrap();
    let state = query_agent_state(db.clone(), "ag_ack_crashed".to_string())
        .await
        .unwrap()
        .map(|(state, _)| state);

    assert_eq!(changes, 1);
    assert_eq!(state.as_deref(), Some(STATE_CRASHED));
    assert_eq!(
        latest_state_change_reason(&db, "ag_ack_crashed"),
        "pane_unregistered_during_ack"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn ack_fallback_ignores_non_waiting_agent() {
    let file = tempfile::NamedTempFile::new().unwrap();
    let db = ah::db::init(file.path()).unwrap();
    seed_agent(db.clone(), "ag_ack_idle", STATE_IDLE).await;

    let stuck = fallback_ack_to_stuck(db.clone(), "ag_ack_idle", "ack_deadline_timeout")
        .await
        .unwrap();
    let crashed =
        fallback_ack_to_crashed(db.clone(), "ag_ack_idle", "pane_unregistered_during_ack")
            .await
            .unwrap();
    let state = query_agent_state(db, "ag_ack_idle".to_string())
        .await
        .unwrap()
        .map(|(state, _)| state);

    assert_eq!(stuck, 0);
    assert_eq!(crashed, 0);
    assert_eq!(state.as_deref(), Some(STATE_IDLE));
}

#[tokio::test(flavor = "multi_thread")]
async fn ack_stability_treats_already_busy_as_success() {
    let file = tempfile::NamedTempFile::new().unwrap();
    let db = ah::db::init(file.path()).unwrap();
    seed_agent(db.clone(), "ag_ack_already_busy", STATE_BUSY).await;

    let outcome =
        ack_mark_busy_or_resolve(db.clone(), "ag_ack_already_busy", "ACK_STABILITY_WINDOW")
            .await
            .unwrap();
    let state = query_agent_state(db, "ag_ack_already_busy".to_string())
        .await
        .unwrap()
        .map(|(state, _)| state);

    assert_eq!(outcome, AckBusyOutcome::AlreadyBusy);
    assert_eq!(state.as_deref(), Some(STATE_BUSY));
}

#[tokio::test(flavor = "multi_thread")]
async fn ack_stability_does_not_set_busy_marked_after_wrong_state() {
    let file = tempfile::NamedTempFile::new().unwrap();
    let db = ah::db::init(file.path()).unwrap();
    seed_agent(db.clone(), "ag_ack_wrong_state", STATE_IDLE).await;

    let outcome =
        ack_mark_busy_or_resolve(db.clone(), "ag_ack_wrong_state", "ACK_STABILITY_WINDOW")
            .await
            .unwrap();
    let state = query_agent_state(db, "ag_ack_wrong_state".to_string())
        .await
        .unwrap()
        .map(|(state, _)| state);

    assert_eq!(outcome, AckBusyOutcome::AlreadyIdle);
    assert_eq!(state.as_deref(), Some(STATE_IDLE));
}

#[tokio::test(flavor = "multi_thread")]
async fn mark_agent_stuck_from_waiting_requeues_unacknowledged_job() {
    let file = tempfile::NamedTempFile::new().unwrap();
    let db = ah::db::init(file.path()).unwrap();
    seed_agent_with_dispatched_job(
        db.clone(),
        "ag_waiting_requeue",
        STATE_WAITING_FOR_ACK,
        "job_waiting_requeue",
    )
    .await;

    let changes = fallback_ack_to_stuck(
        db.clone(),
        "ag_waiting_requeue",
        "tmux_capture_failed_during_ack",
    )
    .await
    .unwrap();
    let state = query_agent_state(db.clone(), "ag_waiting_requeue".to_string())
        .await
        .unwrap()
        .map(|(state, _)| state);
    let job = query_job(db.clone(), "job_waiting_requeue".to_string())
        .await
        .unwrap()
        .unwrap();
    let resolution: String = db
        .conn()
        .query_row(
            "SELECT json_extract(payload, '$.job_resolution') FROM events \
             WHERE agent_id = 'ag_waiting_requeue' AND event_type = 'job_resolution' \
             ORDER BY seq_id DESC LIMIT 1",
            [],
            |row| row.get(0),
        )
        .unwrap();

    assert_eq!(changes, 1);
    assert_eq!(state.as_deref(), Some(STATE_IDLE));
    assert_eq!(job.status, "QUEUED");
    assert!(job.dispatched_at.is_none());
    assert_eq!(resolution, "REQUEUED");
}

#[tokio::test(flavor = "multi_thread")]
async fn mark_agent_stuck_from_busy_fails_dispatched_job() {
    let file = tempfile::NamedTempFile::new().unwrap();
    let db = ah::db::init(file.path()).unwrap();
    seed_agent_with_dispatched_job(db.clone(), "ag_busy_fail", STATE_BUSY, "job_busy_fail").await;

    let changes = state_machine::mark_agent_stuck(db.clone(), "ag_busy_fail".to_string(), "PANE_DIFF_STUCK".to_string())
        .await
        .unwrap();
    let state = query_agent_state(db.clone(), "ag_busy_fail".to_string())
        .await
        .unwrap()
        .map(|(state, _)| state);
    let job = query_job(db.clone(), "job_busy_fail".to_string())
        .await
        .unwrap()
        .unwrap();
    let resolution: String = db
        .conn()
        .query_row(
            "SELECT json_extract(payload, '$.job_resolution') FROM events \
             WHERE agent_id = 'ag_busy_fail' AND event_type = 'job_resolution' \
             ORDER BY seq_id DESC LIMIT 1",
            [],
            |row| row.get(0),
        )
        .unwrap();

    assert_eq!(changes, 1);
    assert_eq!(state.as_deref(), Some(STATE_STUCK));
    assert_eq!(job.status, "FAILED");
    assert_eq!(job.error_reason.as_deref(), Some("PANE_DIFF_STUCK"));
    assert_eq!(resolution, "FAILED");
}
