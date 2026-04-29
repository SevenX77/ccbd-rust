use ccbd::db;
use ccbd::db::agents::query_agent_state;
use ccbd::db::sessions::insert_session;
use ccbd::rpc::Ctx;
use ccbd::rpc::handlers::{
    handle_agent_kill, handle_agent_read, handle_agent_send, handle_agent_spawn,
};
use ccbd::sandbox::EnvState;
use ccbd::tmux::TmuxServer;
use serde_json::{Value, json};
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
        which::which("tmux").expect("tmux binary required for mvp7 acceptance tests");
        let db_file = tempfile::NamedTempFile::new().unwrap();
        let state_dir = tempfile::TempDir::new().unwrap();
        let state_dir_path = state_dir.path().to_path_buf();
        let ctx = Ctx {
            db: db::init(db_file.path()).unwrap(),
            state_dir: state_dir_path.clone(),
            env_state: EnvState {
                bwrap_available: false,
                systemd_run_available: false,
                unsafe_no_sandbox: true,
            },
            tmux_server: Arc::new(TmuxServer::new(&state_dir_path)),
        };

        Self {
            ctx,
            _state_dir: state_dir,
            _db_file: db_file,
        }
    }

    async fn insert_session(&self, session_id: &str) {
        insert_session(
            self.ctx.db.clone(),
            session_id.to_string(),
            format!("p_{session_id}"),
            format!("/tmp/{session_id}"),
            std::process::id() as i64,
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
    }
}

async fn sleep_ms(ms: u64) {
    tokio::time::sleep(Duration::from_millis(ms)).await;
}

async fn spawn_bash(h: &Harness, session_id: &str, agent_id: &str) -> i64 {
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
    let pid = result["pid"].as_i64().unwrap();
    wait_for_state(h, agent_id, "IDLE", Duration::from_secs(10)).await;
    pid
}

async fn send_text(h: &Harness, agent_id: &str, text: &str) {
    handle_agent_send(
        json!({
            "agent_id": agent_id,
            "text": text,
            "request_id": format!("req_{}", uuid::Uuid::new_v4()),
        }),
        &h.ctx,
    )
    .await
    .unwrap();
}

async fn read_events(h: &Harness, agent_id: &str) -> Vec<Value> {
    handle_agent_read(
        json!({
            "agent_id": agent_id,
            "since_event_id": 0,
        }),
        &h.ctx,
    )
    .await
    .unwrap()["events"]
        .as_array()
        .unwrap()
        .clone()
}

async fn wait_for_output_containing(
    h: &Harness,
    agent_id: &str,
    timeout: Duration,
    predicates: &[&str],
) -> String {
    let deadline = Instant::now() + timeout;
    let mut last_combined = String::new();
    while Instant::now() < deadline {
        let combined = read_events(h, agent_id)
            .await
            .into_iter()
            .filter(|event| event["event_type"] == "output_chunk")
            .filter_map(|event| event["payload"].as_str().map(str::to_string))
            .collect::<Vec<_>>()
            .join("\n");
        if predicates.iter().all(|needle| combined.contains(needle)) {
            return combined;
        }
        last_combined = combined;
        sleep_ms(50).await;
    }
    panic!(
        "output for {agent_id} did not contain {predicates:?} within {timeout:?}; output={last_combined:?}"
    );
}

async fn wait_for_state(h: &Harness, agent_id: &str, expected: &str, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        let state = query_agent_state(h.ctx.db.clone(), agent_id.to_string())
            .await
            .unwrap()
            .map(|(state, _)| state);
        if state.as_deref() == Some(expected) {
            return;
        }
        sleep_ms(50).await;
    }
    panic!("agent {agent_id} did not reach {expected} within {timeout:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_multiline_paste_preserves_newlines() {
    let h = Harness::new();
    h.insert_session("s_m7_paste").await;
    let agent_id = format!("ag_m7_paste_{}", uuid::Uuid::new_v4());
    spawn_bash(&h, "s_m7_paste", &agent_id).await;

    send_text(
        &h,
        &agent_id,
        "for i in 1 2; do\n  echo \"line $i\"\ndone\n",
    )
    .await;

    let output =
        wait_for_output_containing(&h, &agent_id, Duration::from_secs(5), &["line 1", "line 2"])
            .await;
    assert!(
        !output.contains("syntax error near unexpected token"),
        "multiline paste caused shell syntax error; output={output:?}"
    );
    wait_for_state(&h, &agent_id, "IDLE", Duration::from_secs(5)).await;
    let _ = handle_agent_kill(json!({ "agent_id": agent_id }), &h.ctx).await;
}
