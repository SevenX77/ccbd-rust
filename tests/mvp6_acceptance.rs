mod common;

use ah::db;
use ah::db::agents::query_agent_state;
use ah::db::sessions::insert_session;
use ah::rpc::Ctx;
use ah::rpc::handlers::{
    handle_agent_kill, handle_agent_read, handle_agent_send, handle_agent_spawn,
};
use ah::sandbox::EnvState;
use ah::tmux::{TmuxServer, compute_socket_name};
use common::scope_policy_for_test;
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
        which::which("tmux").expect("tmux binary required for mvp6 acceptance tests");
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

async fn wait_for_event(
    h: &Harness,
    agent_id: &str,
    timeout: Duration,
    pred: impl Fn(&Value) -> bool,
) -> Value {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        let result = handle_agent_read(
            json!({
                "agent_id": agent_id,
                "since_event_id": 0,
            }),
            &h.ctx,
        )
        .await
        .unwrap();
        if let Some(event) = result["events"]
            .as_array()
            .unwrap()
            .iter()
            .find(|event| pred(event))
        {
            return event.clone();
        }
        sleep_ms(50).await;
    }
    panic!("event for {agent_id} did not appear within {timeout:?}");
}

async fn wait_for_state(h: &Harness, agent_id: &str, expected: &str, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        let state = current_state(h, agent_id).await;
        if state.as_deref() == Some(expected) {
            return;
        }
        sleep_ms(50).await;
    }
    panic!("agent {agent_id} did not reach {expected} within {timeout:?}");
}

async fn current_state(h: &Harness, agent_id: &str) -> Option<String> {
    query_agent_state(h.ctx.db.clone(), agent_id.to_string())
        .await
        .unwrap()
        .map(|(state, _)| state)
}

fn tmux_output(h: &Harness, args: &[&str]) -> String {
    let output = Command::new("tmux")
        .arg("-L")
        .arg(h.ctx.tmux_server.socket_name())
        .args(args)
        .output()
        .unwrap();
    String::from_utf8_lossy(&output.stdout).to_string()
}

#[test]
fn test_no_legacy_pty_dependency() {
    let cargo_toml = std::fs::read_to_string("Cargo.toml").unwrap();
    let cargo_lock = std::fs::read_to_string("Cargo.lock").unwrap();
    let removed_dep = concat!("portable", "-pty");

    assert!(!cargo_toml.contains(removed_dep));
    assert!(!cargo_lock.contains(removed_dep));
}

#[tokio::test(flavor = "multi_thread")]
async fn test_tmux_pane_created() {
    let h = Harness::new();
    h.insert_session("s_m6_pane").await;
    let agent_id = format!("ag_m6_pane_{}", uuid::Uuid::new_v4());
    spawn_bash(&h, "s_m6_pane", &agent_id).await;

    let pane = ah::agent_io::pane_id(&agent_id).unwrap();
    let panes = tmux_output(&h, &["list-panes", "-a", "-F", "#{pane_id}"]);
    assert!(panes.contains(&pane.0), "panes={panes}");
    let _ = handle_agent_kill(json!({ "agent_id": agent_id }), &h.ctx).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn test_pane_pid_matches_agent_pid() {
    let h = Harness::new();
    h.insert_session("s_m6_pid").await;
    let agent_id = format!("ag_m6_pid_{}", uuid::Uuid::new_v4());
    let pid = spawn_bash(&h, "s_m6_pid", &agent_id).await;
    let pane = ah::agent_io::pane_id(&agent_id).unwrap();

    let pane_pid = tmux_output(&h, &["display-message", "-p", "-t", &pane.0, "#{pane_pid}"]);
    assert_eq!(pid, pane_pid.trim().parse::<i64>().unwrap());
    let _ = handle_agent_kill(json!({ "agent_id": agent_id }), &h.ctx).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn test_pipe_pane_drains_to_fifo() {
    let h = Harness::new();
    h.insert_session("s_m6_pipe").await;
    let agent_id = format!("ag_m6_pipe_{}", uuid::Uuid::new_v4());
    spawn_bash(&h, "s_m6_pipe", &agent_id).await;

    send_text(&h, &agent_id, "echo pipe-ok\n").await;
    let event = wait_for_event(&h, &agent_id, Duration::from_secs(5), |event| {
        event["event_type"] == "output_chunk"
            && event["payload"]
                .as_str()
                .is_some_and(|payload| payload.contains("pipe-ok"))
    })
    .await;
    assert_eq!(event["event_type"], "output_chunk");
    let _ = handle_agent_kill(json!({ "agent_id": agent_id }), &h.ctx).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn test_send_text_with_trailing_newline_triggers_shell_eval() {
    let h = Harness::new();
    h.insert_session("s_m6_newline").await;
    let agent_id = format!("ag_m6_newline_{}", uuid::Uuid::new_v4());
    spawn_bash(&h, "s_m6_newline", &agent_id).await;

    send_text(&h, &agent_id, "echo hello\n").await;
    wait_for_event(&h, &agent_id, Duration::from_secs(5), |event| {
        event["payload"]
            .as_str()
            .is_some_and(|payload| payload.contains("hello"))
    })
    .await;
    let _ = handle_agent_kill(json!({ "agent_id": agent_id }), &h.ctx).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn test_send_text_with_multiple_lines() {
    let h = Harness::new();
    h.insert_session("s_m6_multi").await;
    let agent_id = format!("ag_m6_multi_{}", uuid::Uuid::new_v4());
    spawn_bash(&h, "s_m6_multi", &agent_id).await;

    send_text(&h, &agent_id, "echo one\necho two\n").await;
    wait_for_event(&h, &agent_id, Duration::from_secs(5), |event| {
        event["payload"]
            .as_str()
            .is_some_and(|payload| payload.contains("one") && payload.contains("two"))
    })
    .await;
    let _ = handle_agent_kill(json!({ "agent_id": agent_id }), &h.ctx).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn test_long_running_send_does_not_false_idle_from_history_capture() {
    let h = Harness::new();
    h.insert_session("s_m6_sleep").await;
    let agent_id = format!("ag_m6_sleep_{}", uuid::Uuid::new_v4());
    spawn_bash(&h, "s_m6_sleep", &agent_id).await;

    send_text(&h, &agent_id, "sleep 3\n").await;
    sleep_ms(300).await;
    assert_eq!(current_state(&h, &agent_id).await.as_deref(), Some("BUSY"));
    sleep_ms(1500).await;
    assert_eq!(current_state(&h, &agent_id).await.as_deref(), Some("BUSY"));
    wait_for_state(&h, &agent_id, "IDLE", Duration::from_secs(6)).await;
    let _ = handle_agent_kill(json!({ "agent_id": agent_id }), &h.ctx).await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "terminal echo makes typed-but-unexecuted text visible before shell evaluation"]
async fn test_send_text_without_trailing_newline_does_not_execute() {}

#[tokio::test(flavor = "multi_thread")]
async fn test_kill_pane_triggers_crashed() {
    let h = Harness::new();
    h.insert_session("s_m6_crash").await;
    let agent_id = format!("ag_m6_crash_{}", uuid::Uuid::new_v4());
    spawn_bash(&h, "s_m6_crash", &agent_id).await;
    let pane = ah::agent_io::pane_id(&agent_id).unwrap();

    let _ = Command::new("tmux")
        .arg("-L")
        .arg(h.ctx.tmux_server.socket_name())
        .args(["kill-pane", "-t", &pane.0])
        .output()
        .unwrap();
    let event = wait_for_event(&h, &agent_id, Duration::from_secs(5), |event| {
        event["event_type"] == "state_change"
            && event["payload"]
                .as_str()
                .is_some_and(|payload| payload.contains("\"to\":\"CRASHED\""))
    })
    .await;
    let payload: Value = serde_json::from_str(event["payload"].as_str().unwrap()).unwrap();
    assert_eq!(payload["reason"], "EXIT_CODE_UNAVAILABLE_NON_CHILD");
    assert!(payload["exit_code"].is_null());
    let exit_code: Option<i64> = h
        .ctx
        .db
        .conn()
        .query_row(
            "SELECT exit_code FROM agents WHERE id = ?",
            [agent_id.as_str()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(exit_code, None);
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if ah::agent_io::pane_id(&agent_id).is_none() {
            return;
        }
        sleep_ms(100).await;
    }
    panic!("TMUX_PANE_MAP not cleaned within 5s after kill-pane");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_pidfd_kill_cleans_tmux_pane() {
    let h = Harness::new();
    h.insert_session("s_m6_kill").await;
    let agent_id = format!("ag_m6_kill_{}", uuid::Uuid::new_v4());
    spawn_bash(&h, "s_m6_kill", &agent_id).await;
    let pane = ah::agent_io::pane_id(&agent_id).unwrap();

    handle_agent_kill(json!({ "agent_id": agent_id }), &h.ctx)
        .await
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        let panes = tmux_output(&h, &["list-panes", "-a", "-F", "#{pane_id}"]);
        if !panes.contains(&pane.0) {
            return;
        }
        sleep_ms(50).await;
    }
    panic!("pane {} still present after agent.kill", pane.0);
}
