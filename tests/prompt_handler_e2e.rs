//! Prompt-handler Phase 1 integration tests using a tmux-backed mock provider.
//!
//! The fixture is an independent shell script that prints provider-like prompts
//! and waits for keys, so these tests exercise tmux capture/send behavior
//! without depending on a real Codex CLI version.

mod common;

use ccbd::db;
use ccbd::db::agents::{insert_agent, query_agent};
use ccbd::db::sessions::insert_session;
use ccbd::db::state_machine::{STATE_BUSY, STATE_IDLE, STATE_PROMPT_PENDING};
use ccbd::marker::MarkerMatcher;
use ccbd::monitor::{pidfd_open, register as register_pidfd, remove as remove_pidfd};
use ccbd::prompt_handler::{
    PromptScanDisposition, PromptScanRequest, load_or_bootstrap_kb, scan_prompt_and_apply_outcome,
};
use ccbd::rpc::Ctx;
use ccbd::rpc::handlers::handle_agent_resolve_prompt;
use ccbd::sandbox::EnvState;
use ccbd::tmux::{TmuxPaneId, TmuxServer, compute_socket_name};
use common::scope_policy_for_test;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

struct Harness {
    ctx: Ctx,
    project_dir: tempfile::TempDir,
    _state_dir: tempfile::TempDir,
    _db_file: tempfile::NamedTempFile,
}

impl Harness {
    fn new() -> Self {
        which::which("tmux").expect("tmux binary required for prompt-handler e2e tests");
        let db_file = tempfile::NamedTempFile::new().unwrap();
        let state_dir = tempfile::TempDir::new().unwrap();
        let project_dir = tempfile::TempDir::new().unwrap();
        let socket_name = compute_socket_name(state_dir.path());
        let ctx = Ctx {
            db: db::init(db_file.path()).unwrap(),
            state_dir: state_dir.path().to_path_buf(),
            env_state: EnvState {
                bwrap_available: false,
                systemd_run_available: false,
                unsafe_no_sandbox: true,
                under_systemd: false,
            },
            tmux_server: Arc::new(TmuxServer::new_with_policy(
                state_dir.path(),
                scope_policy_for_test(&socket_name),
            )),
        };

        Self {
            ctx,
            project_dir,
            _state_dir: state_dir,
            _db_file: db_file,
        }
    }

    async fn spawn_prompt_agent(&self, prompt: &str) -> (String, TmuxPaneId) {
        let agent_id = format!("ag_prompt_{}", uuid::Uuid::new_v4().simple());
        let session_id = format!("s_{agent_id}");
        let session_name = format!("tmux_{agent_id}");
        let fixture = fixture_path();

        insert_session(
            self.ctx.db.clone(),
            session_id.clone(),
            "prompt-e2e".to_string(),
            self.project_dir.path().display().to_string(),
        )
        .await
        .unwrap();
        insert_agent(
            self.ctx.db.clone(),
            agent_id.clone(),
            session_id,
            "codex".to_string(),
            STATE_IDLE.to_string(),
            Some(std::process::id() as i64),
        )
        .await
        .unwrap();
        self.ctx
            .tmux_server
            .ensure_session(session_name.clone(), self.project_dir.path().to_path_buf())
            .await
            .unwrap();
        let pane = self
            .ctx
            .tmux_server
            .spawn_window(
                session_name,
                agent_id.clone(),
                self.project_dir.path().to_path_buf(),
                vec![
                    "bash".to_string(),
                    fixture.display().to_string(),
                    "--prompt".to_string(),
                    prompt.to_string(),
                ],
            )
            .await
            .unwrap();
        (agent_id, pane)
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

fn fixture_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("mock_prompt_provider.sh")
}

async fn scan_prompt(ctx: &Ctx, agent_id: &str, pane: &TmuxPaneId) -> PromptScanDisposition {
    scan_prompt_and_apply_outcome(PromptScanRequest {
        db: ctx.db.clone(),
        agent_id: agent_id.to_string(),
        provider: "codex".to_string(),
        pane_id: pane.clone(),
        tmux: ctx.tmux_server.clone(),
        state_dir: ctx.state_dir.clone(),
        marker_matcher: Arc::new(MarkerMatcher::default()),
        max_depth: 3,
    })
    .await
    .unwrap()
}

async fn wait_for_pane_contains(ctx: &Ctx, pane: &TmuxPaneId, needle: &str) -> String {
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut last_capture = String::new();
    while Instant::now() < deadline {
        last_capture = ctx.tmux_server.capture_pane(pane.clone()).await.unwrap();
        if last_capture.contains(needle) {
            return last_capture;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!(
        "pane {} did not contain {needle:?}; last capture:\n{last_capture}",
        pane.0
    );
}

async fn agent_state(ctx: &Ctx, agent_id: &str) -> String {
    query_agent(ctx.db.clone(), agent_id.to_string())
        .await
        .unwrap()
        .unwrap()
        .state
}

fn unknown_prompt_event(ctx: &Ctx, agent_id: &str) -> Value {
    let payload: String = ctx
        .db
        .conn()
        .query_row(
            "SELECT payload FROM events WHERE agent_id = ? AND event_type = 'UNKNOWN_PROMPT_DETECTED' ORDER BY seq_id DESC LIMIT 1",
            [agent_id],
            |row| row.get(0),
        )
        .unwrap();
    serde_json::from_str(&payload).unwrap()
}

fn register_pane_for_resolve(ctx: &Ctx, agent_id: &str, pane: TmuxPaneId) {
    let fifo_path = ctx.state_dir.join("pipes").join(format!("{agent_id}.fifo"));
    std::fs::create_dir_all(fifo_path.parent().unwrap()).unwrap();
    std::fs::write(&fifo_path, b"").unwrap();
    let reader_handle = tokio::spawn(async {
        std::future::pending::<()>().await;
    });
    ccbd::agent_io::register(
        agent_id.to_string(),
        ccbd::agent_io::AgentIoEntry {
            pane_id: pane,
            reader_handle,
            fifo_path,
            idle_scan_enabled: Arc::new(AtomicBool::new(true)),
        },
    );
}

async fn child_pidfd_prompt_pending_case(ctx: &Ctx, agent_id: &str) -> Child {
    insert_session(
        ctx.db.clone(),
        format!("s_{agent_id}"),
        "prompt-pidfd".to_string(),
        "/tmp/prompt-pidfd".to_string(),
    )
    .await
    .unwrap();
    insert_agent(
        ctx.db.clone(),
        agent_id.to_string(),
        format!("s_{agent_id}"),
        "bash".to_string(),
        STATE_BUSY.to_string(),
        None,
    )
    .await
    .unwrap();
    ccbd::db::state_machine::mark_agent_prompt_pending(
        ctx.db.clone(),
        agent_id.to_string(),
        "UNKNOWN_PROMPT".to_string(),
    )
    .await
    .unwrap();

    let child = Command::new("sh")
        .arg("-c")
        .arg("sleep 30")
        .spawn()
        .unwrap();
    let pidfd = pidfd_open(child.id() as i32).unwrap();
    let task_fd = pidfd.try_clone().unwrap();
    register_pidfd(agent_id.to_string(), pidfd);
    ccbd::monitor::agent_watch::spawn_agent_pidfd_watch_task(
        agent_id.to_string(),
        child.id() as i32,
        task_fd,
        Arc::new(ctx.db.clone()),
    );
    child
}

#[tokio::test(flavor = "multi_thread")]
async fn known_codex_update_prompt_is_auto_skipped_in_tmux() {
    let h = Harness::new();
    let (agent_id, pane) = h.spawn_prompt_agent("codex_update").await;
    wait_for_pane_contains(&h.ctx, &pane, "Update available!").await;

    let disposition = scan_prompt(&h.ctx, &agent_id, &pane).await;
    let post_scan_capture = h.ctx.tmux_server.capture_pane(pane.clone()).await.unwrap();
    assert!(
        matches!(disposition, PromptScanDisposition::Handled { depth: 1 }),
        "expected known prompt to be auto-handled, got {disposition:?}; capture:\n{post_scan_capture}"
    );
    let capture = wait_for_pane_contains(&h.ctx, &pane, "mock_prompt_provider: selected=2").await;
    assert!(
        capture.contains("$"),
        "expected fixture to return to shell-like idle marker; capture:\n{capture}"
    );
    let state = agent_state(&h.ctx, &agent_id).await;
    assert!(
        matches!(state.as_str(), STATE_IDLE | STATE_BUSY),
        "expected agent to remain active after auto-skip, got {state}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn unknown_prompt_enters_pending_emits_event_and_resolves_to_kb() {
    let h = Harness::new();
    let (agent_id, pane) = h.spawn_prompt_agent("unknown_eula").await;
    wait_for_pane_contains(&h.ctx, &pane, "New provider EULA").await;

    let disposition = scan_prompt(&h.ctx, &agent_id, &pane).await;
    assert!(
        matches!(
            disposition,
            PromptScanDisposition::Pending {
                depth: 0,
                block_reason: _
            }
        ),
        "expected unknown prompt to become pending, got {disposition:?}"
    );
    assert_eq!(agent_state(&h.ctx, &agent_id).await, STATE_PROMPT_PENDING);
    let payload = unknown_prompt_event(&h.ctx, &agent_id);
    assert!(
        payload["pane_screenshot"]
            .as_str()
            .unwrap()
            .contains("New provider EULA"),
        "payload must include pane_screenshot, got {payload}"
    );
    assert!(payload["capture_hash"].as_str().is_some());
    assert_eq!(payload["depth"], 0);
    assert_eq!(payload["provider"], "codex");

    register_pane_for_resolve(&h.ctx, &agent_id, pane.clone());
    let response = handle_agent_resolve_prompt(
        json!({
            "agent_id": agent_id.clone(),
            "action": [
                { "type": "key", "value": "1" },
                { "type": "key", "value": "Enter" }
            ],
            "save_to_kb": true
        }),
        &h.ctx,
    )
    .await
    .unwrap();
    assert_eq!(response["status"], "ok");
    assert_eq!(response["resolved_state"], STATE_IDLE);
    assert_eq!(response["saved_to_kb"], true);
    let case_id = response["case_id"]
        .as_str()
        .expect("resolve response must include saved case_id");
    wait_for_pane_contains(&h.ctx, &pane, "mock_prompt_provider: selected=1").await;
    assert_eq!(agent_state(&h.ctx, &agent_id).await, STATE_IDLE);

    let kb = load_or_bootstrap_kb(&h.ctx.state_dir.join("prompt-cases.json")).unwrap();
    assert!(
        kb.cases.iter().any(|case| case.id == case_id),
        "saved KB should include resolved case {case_id}"
    );

    if let Some(entry) = ccbd::agent_io::remove(&agent_id) {
        entry.reader_handle.abort();
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn pidfd_exit_does_not_crash_prompt_pending_agent() {
    let h = Harness::new();
    let agent_id = format!("ag_prompt_pidfd_{}", uuid::Uuid::new_v4().simple());
    let mut child = child_pidfd_prompt_pending_case(&h.ctx, &agent_id).await;

    child.kill().unwrap();
    let _ = child.wait();
    tokio::time::sleep(Duration::from_millis(300)).await;

    let agent = query_agent(h.ctx.db.clone(), agent_id.clone())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(agent.state, STATE_PROMPT_PENDING);
    assert_eq!(agent.exit_code, None);
    let _ = remove_pidfd(&agent_id);
}
