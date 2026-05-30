use ah::db::{
    agents::{insert_agent, query_agent_state},
    sessions::insert_session,
    state_machine::{STATE_CRASHED, STATE_IDLE, STATE_STUCK, STATE_WAITING_FOR_ACK},
};
use ah::rpc::handlers::{fallback_ack_to_crashed, fallback_ack_to_stuck};

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
