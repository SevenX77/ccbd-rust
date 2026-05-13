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
use ccbd::rpc::Ctx;
use ccbd::rpc::handlers::{handle_agent_kill, handle_agent_send, handle_agent_spawn};
use ccbd::sandbox::EnvState;
use ccbd::tmux::TmuxServer;
use serde_json::json;
use std::sync::Arc;
use std::time::{Duration, Instant};

fn test_ctx(db_file: &tempfile::NamedTempFile, state_dir: &tempfile::TempDir) -> Ctx {
    Ctx {
        db: ccbd::db::init(db_file.path()).unwrap(),
        state_dir: state_dir.path().to_path_buf(),
        env_state: EnvState {
            bwrap_available: false,
            systemd_run_available: false,
            unsafe_no_sandbox: true,
            under_systemd: false,
        },
        tmux_server: Arc::new(TmuxServer::new(state_dir.path())),
    }
}

async fn wait_for_agent_state(ctx: &Ctx, agent_id: &str, expected: &str) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        let state = ccbd::db::agents::query_agent_state(ctx.db.clone(), agent_id.to_string())
            .await
            .unwrap()
            .map(|(state, _)| state);
        if state.as_deref() == Some(expected) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("agent {agent_id} did not reach {expected}");
}

async fn spawn_idle_bash(ctx: &Ctx, session_id: &str, agent_id: &str) {
    insert_session(
        ctx.db.clone(),
        session_id.to_string(),
        format!("p_{session_id}"),
        format!("/tmp/{session_id}"),
    )
    .await
    .unwrap();
    let spawn = handle_agent_spawn(
        json!({
            "session_id": session_id,
            "agent_id": agent_id,
            "provider": "bash",
        }),
        ctx,
    )
    .await
    .unwrap();
    assert_eq!(spawn["state"], "SPAWNING");
    wait_for_agent_state(ctx, agent_id, STATE_IDLE).await;
}

fn command_count(ctx: &Ctx, agent_id: &str) -> i64 {
    ctx.db
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM events WHERE agent_id = ? AND event_type = 'command_received'",
            [agent_id],
            |row| row.get(0),
        )
        .unwrap()
}

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

#[tokio::test(flavor = "multi_thread")]
async fn agent_send_reply_returns_waiting_for_ack_state() {
    let db_file = tempfile::NamedTempFile::new().unwrap();
    let state_dir = tempfile::TempDir::new().unwrap();
    let ctx = test_ctx(&db_file, &state_dir);
    let agent_id = format!("ag_r2_reply_{}", uuid::Uuid::new_v4());
    spawn_idle_bash(&ctx, "s_r2_reply", &agent_id).await;

    let sent = handle_agent_send(
        json!({
            "agent_id": agent_id,
            "text": "sleep 1; echo r2-reply\n",
            "request_id": "r2-reply-1",
        }),
        &ctx,
    )
    .await
    .unwrap();

    assert_eq!(sent["state"], STATE_WAITING_FOR_ACK);
    let _ = handle_agent_kill(json!({ "agent_id": agent_id }), &ctx).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn agent_send_concurrent_only_one_proceeds_to_send_text() {
    let db_file = tempfile::NamedTempFile::new().unwrap();
    let state_dir = tempfile::TempDir::new().unwrap();
    let ctx = test_ctx(&db_file, &state_dir);
    let agent_id = format!("ag_r2_mutex_{}", uuid::Uuid::new_v4());
    spawn_idle_bash(&ctx, "s_r2_mutex", &agent_id).await;

    let first = handle_agent_send(
        json!({
            "agent_id": agent_id,
            "text": "sleep 1; echo first\n",
            "request_id": "r2-mutex-1",
        }),
        &ctx,
    );
    let second = handle_agent_send(
        json!({
            "agent_id": agent_id,
            "text": "echo second\n",
            "request_id": "r2-mutex-2",
        }),
        &ctx,
    );
    let (first, second) = tokio::join!(first, second);
    let ok_count = [&first, &second]
        .into_iter()
        .filter(|result| result.is_ok())
        .count();
    let err_count = [&first, &second]
        .into_iter()
        .filter(|result| result.is_err())
        .count();
    let ok_state = [&first, &second]
        .into_iter()
        .find_map(|result| result.as_ref().ok().map(|value| value["state"].clone()))
        .unwrap();
    let saw_wrong_state = [&first, &second]
        .into_iter()
        .any(|result| matches!(result, Err(ccbd::error::CcbdError::AgentWrongState { .. })));

    assert_eq!(ok_count, 1);
    assert_eq!(err_count, 1);
    assert_eq!(ok_state, STATE_WAITING_FOR_ACK);
    assert!(saw_wrong_state);
    assert_eq!(command_count(&ctx, &agent_id), 1);
    let _ = handle_agent_kill(json!({ "agent_id": agent_id }), &ctx).await;
}
