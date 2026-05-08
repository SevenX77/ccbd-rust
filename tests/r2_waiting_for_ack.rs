//! R2 WAITING_FOR_ACK 集成测试.
//! T2.1.1: 常量 + helper 的最小 sanity. 后续 task (T2.1.2 / T2.2.x) 会扩展真实 ACK e2e flow.

use ccbd::db::{
    agents::insert_agent,
    sessions::insert_session,
    state_machine::{
        STATE_BUSY, STATE_IDLE, STATE_WAITING_FOR_ACK, is_active_state, is_waiting_for_ack,
        mark_agent_waiting_for_ack,
    },
};

#[test]
fn waiting_for_ack_constant_visible_at_crate_boundary() {
    assert_eq!(STATE_WAITING_FOR_ACK, "WAITING_FOR_ACK");
    assert!(is_active_state(STATE_WAITING_FOR_ACK));
    assert!(is_waiting_for_ack(STATE_WAITING_FOR_ACK));
    assert!(!is_waiting_for_ack(STATE_IDLE));
    assert!(!is_waiting_for_ack(STATE_BUSY));
}

#[tokio::test(flavor = "multi_thread")]
async fn waiting_for_ack_cas_only_one_concurrent_caller_wins() {
    let file = tempfile::NamedTempFile::new().unwrap();
    let db = ccbd::db::init(file.path()).unwrap();
    insert_session(
        db.clone(),
        "s_r2_ack_race".to_string(),
        "p_r2_ack_race".to_string(),
        "/tmp/r2-ack-race".to_string(),
    )
    .await
    .unwrap();
    insert_agent(
        db.clone(),
        "a_r2_ack_race".to_string(),
        "s_r2_ack_race".to_string(),
        "bash".to_string(),
        STATE_IDLE.to_string(),
        Some(1),
    )
    .await
    .unwrap();

    let first = mark_agent_waiting_for_ack(db.clone(), "a_r2_ack_race".to_string(), 1);
    let second = mark_agent_waiting_for_ack(db.clone(), "a_r2_ack_race".to_string(), 1);
    let (first, second) = tokio::join!(first, second);
    let wins = [first.unwrap(), second.unwrap()]
        .into_iter()
        .filter(|ok| *ok)
        .count();
    let (state, version): (String, i64) = db
        .conn()
        .query_row(
            "SELECT state, state_version FROM agents WHERE id='a_r2_ack_race'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();

    assert_eq!(wins, 1);
    assert_eq!(state, STATE_WAITING_FOR_ACK);
    assert_eq!(version, 2);
}
