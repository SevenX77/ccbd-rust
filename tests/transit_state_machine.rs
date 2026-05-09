use ccbd::db;
use ccbd::db::state_machine::{STATE_BUSY, STATE_IDLE, STATE_SPAWNING, STATE_WAITING_FOR_ACK};

#[tokio::test]
async fn transit_state_machine_emits_state_change_chain() {
    let file = tempfile::NamedTempFile::new().unwrap();
    let db = db::init(file.path()).unwrap();

    db::sessions::create_session(
        db.clone(),
        "s_transit_chain".to_string(),
        "p_transit_chain".to_string(),
        "/tmp/transit-chain".to_string(),
    )
    .await
    .unwrap();
    db::agents::insert_agent(
        db.clone(),
        "a_transit_chain".to_string(),
        "s_transit_chain".to_string(),
        "bash".to_string(),
        STATE_SPAWNING.to_string(),
        Some(1),
    )
    .await
    .unwrap();

    db::state_machine::transit_agent_state(
        db.clone(),
        "a_transit_chain".to_string(),
        vec![STATE_SPAWNING.to_string()],
        STATE_IDLE.to_string(),
        Some("INIT_READY".to_string()),
    )
    .await
    .unwrap();
    assert!(
        db::state_machine::mark_agent_waiting_for_ack(
            db.clone(),
            "a_transit_chain".to_string(),
            2,
        )
        .await
        .unwrap()
    );
    db::state_machine::transit_agent_state(
        db.clone(),
        "a_transit_chain".to_string(),
        vec![STATE_WAITING_FOR_ACK.to_string()],
        STATE_BUSY.to_string(),
        Some("ACK_VISUAL_DIFF".to_string()),
    )
    .await
    .unwrap();
    db::state_machine::mark_agent_idle_matched(db.clone(), "a_transit_chain".to_string())
        .await
        .unwrap();

    let events = db::events::query_events_since(db.clone(), "a_transit_chain".to_string(), 0)
        .await
        .unwrap();
    let state_changes = events
        .iter()
        .filter(|event| event.event_type == "state_change")
        .collect::<Vec<_>>();

    assert_eq!(state_changes.len(), 4);
    let transitions = state_changes
        .iter()
        .map(|event| {
            let payload: serde_json::Value = serde_json::from_str(&event.payload).unwrap();
            (
                payload["from"].as_str().unwrap().to_string(),
                payload["to"].as_str().unwrap().to_string(),
                payload["reason"].as_str().unwrap().to_string(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        transitions,
        vec![
            (
                STATE_SPAWNING.to_string(),
                STATE_IDLE.to_string(),
                "INIT_READY".to_string()
            ),
            (
                STATE_IDLE.to_string(),
                STATE_WAITING_FOR_ACK.to_string(),
                "ack_pending".to_string()
            ),
            (
                STATE_WAITING_FOR_ACK.to_string(),
                STATE_BUSY.to_string(),
                "ACK_VISUAL_DIFF".to_string()
            ),
            (
                STATE_BUSY.to_string(),
                STATE_IDLE.to_string(),
                "MARKER_MATCHED".to_string()
            ),
        ]
    );

    let agent = db::agents::query_agent(db, "a_transit_chain".to_string())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(agent.state, STATE_IDLE);
}
