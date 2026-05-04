mod common;

use ccbd::agent_io::{ReaderMarkerConfig, spawn_agent_io_reader_task_with_config};
use ccbd::db;
use ccbd::db::agents::insert_agent;
use ccbd::db::agents::query_agent_state;
use ccbd::db::sessions::insert_session;
use ccbd::marker::MarkerMatcher;
use ccbd::provider::manifest::{IdleDetectionMode, ProviderManifest};
use ccbd::rpc::Ctx;
use ccbd::rpc::handlers::{
    handle_agent_kill, handle_agent_read, handle_agent_send, handle_agent_spawn,
};
use ccbd::sandbox::EnvState;
use ccbd::sandbox::bwrap;
use ccbd::tmux::{TmuxServer, compute_socket_name};
use common::scope_policy_for_test;
use nix::sys::stat::Mode;
use serde_json::{Value, json};
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::process::Command;
use std::sync::{Arc, Mutex};
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
        let socket_name = compute_socket_name(state_dir.path());
        let policy = scope_policy_for_test(&socket_name);
        let ctx = Ctx {
            db: db::init(db_file.path()).unwrap(),
            state_dir: state_dir_path.clone(),
            env_state: EnvState {
                bwrap_available: false,
                systemd_run_available: false,
                unsafe_no_sandbox: true,
            },
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

async fn current_state(db: db::Db, agent_id: &str) -> Option<String> {
    query_agent_state(db, agent_id.to_string())
        .await
        .unwrap()
        .map(|(state, _)| state)
}

#[test]
fn test_codex_auth_mount_passthrough() {
    let home = tempfile::tempdir().unwrap();
    let codex_dir = home.path().join(".codex");
    std::fs::create_dir_all(&codex_dir).unwrap();
    std::fs::write(codex_dir.join("mock_token"), "token").unwrap();
    let old_home = std::env::var_os("HOME");
    unsafe {
        std::env::set_var("HOME", home.path());
    }

    let sandbox = tempfile::tempdir().unwrap();
    let manifest = ccbd::provider::manifest::get_manifest("codex");
    let args = bwrap::build_args(
        sandbox.path(),
        &bwrap::SandboxOverrides::default(),
        Some(&manifest),
    )
    .unwrap();

    unsafe {
        if let Some(old_home) = old_home {
            std::env::set_var("HOME", old_home);
        } else {
            std::env::remove_var("HOME");
        }
    }
    let codex_dir = codex_dir.display().to_string();
    assert!(
        args.windows(3).any(|window| window
            == [
                "--ro-bind".to_string(),
                codex_dir.clone(),
                codex_dir.clone()
            ]),
        "codex auth mount missing from bwrap args: {args:?}"
    );
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

#[tokio::test(flavor = "multi_thread")]
async fn test_stability_timer_cancels_on_noise() {
    let db_file = tempfile::NamedTempFile::new().unwrap();
    let db = db::init(db_file.path()).unwrap();
    insert_session(
        db.clone(),
        "s_m7_stability".to_string(),
        "p_m7_stability".to_string(),
        "/tmp/s_m7_stability".to_string(),
    )
    .await
    .unwrap();
    let agent_id = format!("ag_m7_stability_{}", uuid::Uuid::new_v4());
    insert_agent(
        db.clone(),
        agent_id.clone(),
        "s_m7_stability".to_string(),
        "gemini".to_string(),
        "BUSY".to_string(),
        Some(1),
    )
    .await
    .unwrap();

    let tmp = tempfile::tempdir().unwrap();
    let fifo_path = tmp.path().join("agent.fifo");
    nix::unistd::mkfifo(&fifo_path, Mode::S_IRUSR | Mode::S_IWUSR).unwrap();
    let reader_file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(&fifo_path)
        .unwrap();
    let mut writer = std::fs::OpenOptions::new()
        .write(true)
        .open(&fifo_path)
        .unwrap();
    let parser = Arc::new(Mutex::new(vt100::Parser::new(200, 200, 0)));
    let manifest = ProviderManifest {
        provider_name: "fake-gemini",
        auth_mount_paths: vec![],
        command: &["fake-gemini"],
        env_passthrough: &[],
        injected_env_vars: &[],
        readiness_timeout_s: 1,
        startup_sequence: &[],
        interactive_prompt_handlers: &[],
        idle_detection_mode: IdleDetectionMode::ObservedStability,
        marker_pattern: r"✦",
        stability_ms: 200,
    };
    let reader_handle = spawn_agent_io_reader_task_with_config(
        agent_id.clone(),
        reader_file,
        db.clone(),
        parser,
        ReaderMarkerConfig {
            matcher: Arc::new(MarkerMatcher::from_manifest(&manifest)),
            stability_ms: manifest.stability_ms,
        },
    );

    writer.write_all("✦".as_bytes()).unwrap();
    writer.flush().unwrap();
    sleep_ms(50).await;
    writer.write_all("still rendering\n".as_bytes()).unwrap();
    writer.flush().unwrap();
    sleep_ms(300).await;
    assert_eq!(
        current_state(db.clone(), &agent_id).await.as_deref(),
        Some("BUSY")
    );

    writer.write_all("\x1b[2J✦".as_bytes()).unwrap();
    writer.flush().unwrap();
    wait_for_state_by_db(&db, &agent_id, "IDLE", Duration::from_secs(2)).await;
    reader_handle.abort();
    let _ = reader_handle.await;
}

async fn wait_for_state_by_db(db: &db::Db, agent_id: &str, expected: &str, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if current_state(db.clone(), agent_id).await.as_deref() == Some(expected) {
            return;
        }
        sleep_ms(50).await;
    }
    panic!("agent {agent_id} did not reach {expected} within {timeout:?}");
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires playground codex login and provider availability"]
async fn test_true_codex_smoke_idle_roundtrip() {
    panic!("manual playground smoke hook for MVP7 final validation");
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires playground gemini login and provider availability"]
async fn test_true_gemini_smoke_idle_roundtrip() {
    panic!("manual playground smoke hook for MVP7 final validation");
}
