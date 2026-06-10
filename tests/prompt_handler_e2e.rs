//! Prompt-handler Phase 1 integration tests using a tmux-backed mock provider.
//!
//! The fixture is an independent shell script that prints provider-like prompts
//! and waits for keys, so these tests exercise tmux capture/send behavior
//! without depending on a real Codex CLI version.

mod common;

use ah::db;
use ah::db::agents::{insert_agent, query_agent};
use ah::db::events::UNKNOWN_PATTERN_STABLE;
use ah::db::learned_rules::{
    CursorAnchor, LearnedRule, LearnedRuleCategory, RuleFingerprint, insert_learned_rule_sync,
};
use ah::db::sessions::insert_session;
use ah::db::state_machine::{
    STATE_FAILED, STATE_IDLE, STATE_PROMPT_PENDING, STATE_SPAWNING, STATE_SPAWNING_INTERVENTION,
    STATE_UNKNOWN, STATE_WAITING_FOR_ACK,
};
use ah::marker::MarkerMatcher;
use ah::monitor::{pidfd_open, register as register_pidfd, remove as remove_pidfd};
use ah::prompt_handler::{
    PromptScanDisposition, PromptScanPurpose, PromptScanRequest, load_or_bootstrap_kb,
    scan_prompt_and_apply_outcome,
};
use ah::provider::init_probe_task::STABLE_UNKNOWN_STARTUP_GRACE;
use ah::provider::manifest::InitProbeKind;
use ah::provider::manifest::get_manifest;
use ah::rpc::Ctx;
use ah::rpc::handlers::{handle_agent_learn_rule, handle_agent_resolve_prompt};
use ah::sandbox::EnvState;
use ah::tmux::{TmuxPaneId, TmuxServer, compute_socket_name};
use common::scope_policy_for_test;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
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
                systemd_run_available: false,
                unsafe_no_sandbox: true,
                under_systemd: false,
            },
            daemon_unit: None,
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
        self.spawn_prompt_agent_with_state(prompt, STATE_IDLE).await
    }

    async fn spawn_prompt_agent_with_state(
        &self,
        prompt: &str,
        state: &str,
    ) -> (String, TmuxPaneId) {
        self.spawn_prompt_agent_with_provider_and_state(prompt, "codex", state)
            .await
    }

    async fn spawn_prompt_agent_with_provider_and_state(
        &self,
        prompt: &str,
        provider: &str,
        state: &str,
    ) -> (String, TmuxPaneId) {
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
            provider.to_string(),
            state.to_string(),
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
        scan_purpose: PromptScanPurpose::Direct,
    })
    .await
    .unwrap()
}

async fn scan_codex_prompt(ctx: &Ctx, agent_id: &str, pane: &TmuxPaneId) -> PromptScanDisposition {
    scan_prompt_and_apply_outcome(PromptScanRequest {
        db: ctx.db.clone(),
        agent_id: agent_id.to_string(),
        provider: "codex".to_string(),
        pane_id: pane.clone(),
        tmux: ctx.tmux_server.clone(),
        state_dir: ctx.state_dir.clone(),
        marker_matcher: Arc::new(MarkerMatcher::from_manifest(&get_manifest("codex"))),
        max_depth: 3,
        scan_purpose: PromptScanPurpose::Direct,
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

async fn wait_for_agent_state(ctx: &Ctx, agent_id: &str, expected: &str, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    let mut last_state = String::new();
    while Instant::now() < deadline {
        last_state = agent_state(ctx, agent_id).await;
        if last_state == expected {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("agent {agent_id} did not reach {expected}; last state {last_state}");
}

async fn wait_for_agent_state_with_pane(
    ctx: &Ctx,
    agent_id: &str,
    pane: &TmuxPaneId,
    expected: &str,
    timeout: Duration,
) {
    let deadline = Instant::now() + timeout;
    let mut last_state = String::new();
    let mut last_capture = String::new();
    while Instant::now() < deadline {
        last_state = agent_state(ctx, agent_id).await;
        last_capture = ctx.tmux_server.capture_pane(pane.clone()).await.unwrap();
        if last_state == expected {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!(
        "agent {agent_id} did not reach {expected}; last state {last_state}; capture:\n{last_capture}"
    );
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

fn unknown_prompt_event_count(ctx: &Ctx, agent_id: &str) -> usize {
    ctx.db
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM events WHERE agent_id = ? AND event_type = 'UNKNOWN_PROMPT_DETECTED'",
            [agent_id],
            |row| row.get::<_, i64>(0),
        )
        .unwrap() as usize
}

fn unknown_pattern_event(ctx: &Ctx, agent_id: &str) -> Value {
    let payload: String = ctx
        .db
        .conn()
        .query_row(
            "SELECT payload FROM events WHERE agent_id = ? AND event_type = ? ORDER BY seq_id DESC LIMIT 1",
            (agent_id, UNKNOWN_PATTERN_STABLE),
            |row| row.get(0),
        )
        .unwrap();
    serde_json::from_str(&payload).unwrap()
}

fn unknown_pattern_event_count(ctx: &Ctx, agent_id: &str) -> usize {
    ctx.db
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM events WHERE agent_id = ? AND event_type = ?",
            (agent_id, UNKNOWN_PATTERN_STABLE),
            |row| row.get::<_, i64>(0),
        )
        .unwrap() as usize
}

fn rule_learned_event_count(ctx: &Ctx, agent_id: &str) -> usize {
    ctx.db
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM events WHERE agent_id = ? AND event_type = 'rule_learned'",
            [agent_id],
            |row| row.get::<_, i64>(0),
        )
        .unwrap() as usize
}

fn register_pane_for_resolve(ctx: &Ctx, agent_id: &str, pane: TmuxPaneId) -> Arc<AtomicBool> {
    let fifo_path = ctx.state_dir.join("pipes").join(format!("{agent_id}.fifo"));
    std::fs::create_dir_all(fifo_path.parent().unwrap()).unwrap();
    std::fs::write(&fifo_path, b"").unwrap();
    let reader_handle = tokio::spawn(async {
        std::future::pending::<()>().await;
    });
    let idle_scan_enabled = Arc::new(AtomicBool::new(false));
    ah::agent_io::register(
        agent_id.to_string(),
        ah::agent_io::AgentIoEntry {
            session_id: format!("s_{agent_id}"),
            pane_id: pane,
            reader_handle,
            fifo_path,
            socket_name: ctx.tmux_server.socket_name().to_string(),
            idle_scan_enabled: idle_scan_enabled.clone(),
        },
    );
    idle_scan_enabled
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
        STATE_IDLE.to_string(),
        None,
    )
    .await
    .unwrap();
    ah::db::state_machine::mark_agent_prompt_pending(
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
    ah::monitor::agent_watch::spawn_agent_pidfd_watch_task(
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

    let disposition = scan_codex_prompt(&h.ctx, &agent_id, &pane).await;
    let post_scan_capture = h.ctx.tmux_server.capture_pane(pane.clone()).await.unwrap();
    assert!(
        matches!(
            disposition,
            PromptScanDisposition::Pending {
                depth: 1,
                block_reason: _
            }
        ),
        "known prompt without can-input confirmation must become pending, got {disposition:?}; capture:\n{post_scan_capture}"
    );
    let capture = wait_for_pane_contains(&h.ctx, &pane, "mock_prompt_provider: selected=2").await;
    assert!(
        capture.contains("$"),
        "expected fixture to return to shell-like idle marker; capture:\n{capture}"
    );
    let state = agent_state(&h.ctx, &agent_id).await;
    assert_eq!(state, STATE_PROMPT_PENDING);
}

#[tokio::test(flavor = "multi_thread")]
async fn known_startup_prompt_is_auto_handled_before_idle() {
    let h = Harness::new();
    let (agent_id, pane) = h
        .spawn_prompt_agent_with_state("codex_update", STATE_SPAWNING)
        .await;
    wait_for_pane_contains(&h.ctx, &pane, "Update available!").await;

    let disposition = scan_codex_prompt(&h.ctx, &agent_id, &pane).await;

    assert!(
        matches!(
            disposition,
            PromptScanDisposition::Deferred {
                depth: 1,
                block_reason: _
            }
        ),
        "known prompt followed by non-confirmed startup input must defer, got {disposition:?}"
    );
    wait_for_pane_contains(&h.ctx, &pane, "mock_prompt_provider: selected=2").await;
    assert_eq!(agent_state(&h.ctx, &agent_id).await, STATE_SPAWNING);
    assert_eq!(unknown_prompt_event_count(&h.ctx, &agent_id), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn unknown_startup_prompt_defers_while_spawning() {
    let h = Harness::new();
    let (agent_id, pane) = h
        .spawn_prompt_agent_with_state("unknown_eula", STATE_SPAWNING)
        .await;
    wait_for_pane_contains(&h.ctx, &pane, "New provider EULA").await;

    let disposition = scan_prompt(&h.ctx, &agent_id, &pane).await;

    assert!(
        matches!(
            disposition,
            PromptScanDisposition::Deferred {
                depth: 0,
                block_reason: _
            }
        ),
        "expected SPAWNING unknown prompt to defer, got {disposition:?}"
    );
    assert_eq!(agent_state(&h.ctx, &agent_id).await, STATE_SPAWNING);
    assert_eq!(unknown_prompt_event_count(&h.ctx, &agent_id), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn unknown_prompt_defers_while_waiting_for_ack() {
    let h = Harness::new();
    let (agent_id, pane) = h
        .spawn_prompt_agent_with_state("unknown_eula", STATE_WAITING_FOR_ACK)
        .await;
    wait_for_pane_contains(&h.ctx, &pane, "New provider EULA").await;

    let disposition = scan_prompt(&h.ctx, &agent_id, &pane).await;

    assert!(
        matches!(
            disposition,
            PromptScanDisposition::Deferred {
                depth: 0,
                ref block_reason
            } if block_reason == "active_dispatch_prompt_scan"
        ),
        "expected WAITING_FOR_ACK unknown prompt to defer, got {disposition:?}"
    );
    assert_eq!(agent_state(&h.ctx, &agent_id).await, STATE_WAITING_FOR_ACK);
    assert_eq!(unknown_prompt_event_count(&h.ctx, &agent_id), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn init_probe_task_handles_known_prompt_without_prompt_pending_demote() {
    let h = Harness::new();
    let (agent_id, pane) = h
        .spawn_prompt_agent_with_state("codex_update_ready", STATE_SPAWNING)
        .await;
    wait_for_pane_contains(&h.ctx, &pane, "Update available!").await;
    let idle_scan_enabled = Arc::new(AtomicBool::new(false));

    let handle = ah::provider::init_probe_task::spawn_init_probe_task(
        agent_id.clone(),
        h.ctx.tmux_server.clone(),
        pane.clone(),
        Arc::new(h.ctx.db.clone()),
        "codex".to_string(),
        h.ctx.state_dir.clone(),
        Arc::new(MarkerMatcher::from_manifest(&get_manifest("codex"))),
        InitProbeKind::Bash,
        Duration::from_secs(5),
        STABLE_UNKNOWN_STARTUP_GRACE,
        idle_scan_enabled.clone(),
    );

    wait_for_pane_contains(&h.ctx, &pane, "mock_prompt_provider: selected=2").await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_ne!(agent_state(&h.ctx, &agent_id).await, STATE_PROMPT_PENDING);
    assert_eq!(unknown_prompt_event_count(&h.ctx, &agent_id), 0);
    handle.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn init_probe_task_retries_non_empty_transient_unknown_until_ready() {
    let h = Harness::new();
    let (agent_id, pane) = h
        .spawn_prompt_agent_with_state("transient_ready", STATE_SPAWNING)
        .await;
    wait_for_pane_contains(&h.ctx, &pane, "transient startup panel").await;
    let idle_scan_enabled = Arc::new(AtomicBool::new(false));

    let handle = ah::provider::init_probe_task::spawn_init_probe_task(
        agent_id.clone(),
        h.ctx.tmux_server.clone(),
        pane.clone(),
        Arc::new(h.ctx.db.clone()),
        "codex".to_string(),
        h.ctx.state_dir.clone(),
        Arc::new(MarkerMatcher::from_manifest(&get_manifest("codex"))),
        InitProbeKind::Codex,
        Duration::from_secs(5),
        STABLE_UNKNOWN_STARTUP_GRACE,
        idle_scan_enabled.clone(),
    );

    wait_for_agent_state_with_pane(&h.ctx, &agent_id, &pane, STATE_IDLE, Duration::from_secs(5))
        .await;
    assert!(idle_scan_enabled.load(Ordering::SeqCst));
    assert_eq!(unknown_prompt_event_count(&h.ctx, &agent_id), 0);
    handle.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn init_probe_task_does_not_mark_idle_when_probe_echo_is_missing() {
    let h = Harness::new();
    let (agent_id, pane) = h
        .spawn_prompt_agent_with_state("codex_update", STATE_SPAWNING)
        .await;
    wait_for_pane_contains(&h.ctx, &pane, "Update available!").await;
    let idle_scan_enabled = Arc::new(AtomicBool::new(false));

    let handle = ah::provider::init_probe_task::spawn_init_probe_task(
        agent_id.clone(),
        h.ctx.tmux_server.clone(),
        pane.clone(),
        Arc::new(h.ctx.db.clone()),
        "codex".to_string(),
        h.ctx.state_dir.clone(),
        Arc::new(MarkerMatcher::default()),
        InitProbeKind::Bash,
        Duration::from_secs(1),
        STABLE_UNKNOWN_STARTUP_GRACE,
        idle_scan_enabled.clone(),
    );

    wait_for_pane_contains(&h.ctx, &pane, "mock_prompt_provider: selected=2").await;
    wait_for_agent_state(&h.ctx, &agent_id, STATE_UNKNOWN, Duration::from_secs(3)).await;
    assert!(!idle_scan_enabled.load(Ordering::SeqCst));
    assert_eq!(unknown_prompt_event_count(&h.ctx, &agent_id), 0);
    handle.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn init_probe_task_consumes_learned_startup_readiness_rule_to_idle() {
    let h = Harness::new();
    let (agent_id, pane) = h
        .spawn_prompt_agent_with_provider_and_state("claude_try_ready", "claude", STATE_SPAWNING)
        .await;
    wait_for_pane_contains(&h.ctx, &pane, r#"❯ Try "fix lint errors""#).await;
    insert_learned_rule_sync(
        &h.ctx.db,
        &LearnedRule {
            id: format!("lr_{}", uuid::Uuid::new_v4().simple()),
            provider: "claude".to_string(),
            category: LearnedRuleCategory::StartupReadiness,
            fingerprint: RuleFingerprint::Regex {
                pattern: r#"(?m)^\s*❯\s+Try "fix lint errors""#.to_string(),
            },
            regex_flags: Vec::new(),
            positive_examples: vec![r#"❯ Try "fix lint errors""#.to_string()],
            action: None,
            extraction: None,
            cursor_anchor: Some(CursorAnchor {
                cursor_row_delta_from_match: 0,
                cursor_col_delta_from_match_end: 1,
            }),
            source_event_seq_id: None,
            enabled: true,
        },
    )
    .unwrap();
    let idle_scan_enabled = Arc::new(AtomicBool::new(false));

    let handle = ah::provider::init_probe_task::spawn_init_probe_task(
        agent_id.clone(),
        h.ctx.tmux_server.clone(),
        pane.clone(),
        Arc::new(h.ctx.db.clone()),
        "claude".to_string(),
        h.ctx.state_dir.clone(),
        Arc::new(MarkerMatcher::from_manifest(&get_manifest("claude"))),
        InitProbeKind::Claude,
        Duration::from_secs(5),
        STABLE_UNKNOWN_STARTUP_GRACE,
        idle_scan_enabled.clone(),
    );

    wait_for_agent_state_with_pane(&h.ctx, &agent_id, &pane, STATE_IDLE, Duration::from_secs(5))
        .await;
    assert!(idle_scan_enabled.load(Ordering::SeqCst));
    assert_eq!(unknown_pattern_event_count(&h.ctx, &agent_id), 0);
    handle.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn init_probe_task_stable_unknown_enters_intervention_and_emits_event() {
    let h = Harness::new();
    let (agent_id, pane) = h
        .spawn_prompt_agent_with_provider_and_state("stable_unknown", "bash", STATE_SPAWNING)
        .await;
    wait_for_pane_contains(&h.ctx, &pane, "Mystery provider startup panel").await;
    let idle_scan_enabled = Arc::new(AtomicBool::new(false));

    let handle = ah::provider::init_probe_task::spawn_init_probe_task(
        agent_id.clone(),
        h.ctx.tmux_server.clone(),
        pane.clone(),
        Arc::new(h.ctx.db.clone()),
        "bash".to_string(),
        h.ctx.state_dir.clone(),
        Arc::new(MarkerMatcher::default()),
        InitProbeKind::Codex,
        Duration::from_secs(5),
        Duration::ZERO,
        idle_scan_enabled.clone(),
    );

    wait_for_agent_state_with_pane(
        &h.ctx,
        &agent_id,
        &pane,
        STATE_SPAWNING_INTERVENTION,
        Duration::from_secs(5),
    )
    .await;
    let payload = unknown_pattern_event(&h.ctx, &agent_id);
    assert_eq!(payload["category_hint"], "StartupReadiness");
    assert_eq!(payload["provider"], "bash");
    assert_eq!(payload["stable_scans"], 3);
    assert_eq!(payload["source_state"], STATE_SPAWNING);
    assert!(!idle_scan_enabled.load(Ordering::SeqCst));
    handle.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn learn_rule_rescans_spawning_intervention_and_restores_idle() {
    let h = Harness::new();
    let (agent_id, pane) = h
        .spawn_prompt_agent_with_provider_and_state("stable_unknown", "bash", STATE_SPAWNING)
        .await;
    wait_for_pane_contains(&h.ctx, &pane, "Mystery provider startup panel").await;
    let idle_scan_enabled = register_pane_for_resolve(&h.ctx, &agent_id, pane.clone());

    let handle = ah::provider::init_probe_task::spawn_init_probe_task(
        agent_id.clone(),
        h.ctx.tmux_server.clone(),
        pane.clone(),
        Arc::new(h.ctx.db.clone()),
        "bash".to_string(),
        h.ctx.state_dir.clone(),
        Arc::new(MarkerMatcher::default()),
        InitProbeKind::Codex,
        Duration::from_secs(5),
        Duration::ZERO,
        idle_scan_enabled.clone(),
    );
    wait_for_agent_state_with_pane(
        &h.ctx,
        &agent_id,
        &pane,
        STATE_SPAWNING_INTERVENTION,
        Duration::from_secs(5),
    )
    .await;

    let response = handle_agent_learn_rule(
        json!({
            "agent_id": agent_id.clone(),
            "provider": "bash",
            "category": "StartupReadiness",
            "fingerprint": {
                "type": "regex",
                "pattern": "Mystery provider startup panel"
            },
            "positive_examples": ["Mystery provider startup panel"]
        }),
        &h.ctx,
    )
    .await
    .unwrap();

    assert_eq!(response["status"], "ok");
    assert_eq!(response["init_probe_rescan_started"], true);
    wait_for_agent_state_with_pane(&h.ctx, &agent_id, &pane, STATE_IDLE, Duration::from_secs(5))
        .await;
    assert!(idle_scan_enabled.load(Ordering::SeqCst));
    assert_eq!(rule_learned_event_count(&h.ctx, &agent_id), 1);
    handle.abort();
    if let Some(entry) = ah::agent_io::remove(&agent_id) {
        entry.reader_handle.abort();
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn spawning_intervention_deadline_marks_failed() {
    let h = Harness::new();
    let (agent_id, pane) = h
        .spawn_prompt_agent_with_provider_and_state("stable_unknown", "bash", STATE_SPAWNING)
        .await;
    wait_for_pane_contains(&h.ctx, &pane, "Mystery provider startup panel").await;
    let idle_scan_enabled = Arc::new(AtomicBool::new(false));

    let handle = ah::provider::init_probe_task::spawn_init_probe_task(
        agent_id.clone(),
        h.ctx.tmux_server.clone(),
        pane.clone(),
        Arc::new(h.ctx.db.clone()),
        "bash".to_string(),
        h.ctx.state_dir.clone(),
        Arc::new(MarkerMatcher::default()),
        InitProbeKind::Codex,
        Duration::from_secs(2),
        Duration::ZERO,
        idle_scan_enabled.clone(),
    );

    wait_for_agent_state_with_pane(
        &h.ctx,
        &agent_id,
        &pane,
        STATE_FAILED,
        Duration::from_secs(5),
    )
    .await;
    let error_code = query_agent(h.ctx.db.clone(), agent_id.clone())
        .await
        .unwrap()
        .unwrap()
        .error_code;
    assert_eq!(
        error_code.as_deref(),
        Some("MASTER_OFFLINE_INTERVENTION_TIMEOUT")
    );
    assert!(!idle_scan_enabled.load(Ordering::SeqCst));
    handle.abort();
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

    if let Some(entry) = ah::agent_io::remove(&agent_id) {
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
