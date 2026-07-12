mod common;

use ah::db;
use ah::rpc::{
    Ctx,
    handlers::{handle_agent_spawn, handle_session_spawn_master_pane},
};
use ah::sandbox::EnvState;
use common::TmuxServerGuard;
use serde_json::json;
use std::process::Command;

#[tokio::test(flavor = "multi_thread")]
async fn session_query_returns_project_absolute_path() {
    let file = tempfile::NamedTempFile::new().unwrap();
    let db = db::init(file.path()).unwrap();
    let absolute_path = "/tmp/r3-absolute-path-project";

    db::sessions::create_session(
        db.clone(),
        "s_r3_absolute".to_string(),
        "p_r3_absolute".to_string(),
        absolute_path.to_string(),
    )
    .await
    .unwrap();

    let session = db::sessions::query_session_by_id(db, "s_r3_absolute".to_string())
        .await
        .unwrap()
        .unwrap();

    assert_eq!(session.id, "s_r3_absolute");
    assert_eq!(session.project_id, "p_r3_absolute");
    assert_eq!(session.absolute_path, absolute_path);
}

#[tokio::test(flavor = "multi_thread")]
async fn master_pane_starts_in_project_absolute_path() {
    which::which("tmux").expect("tmux binary required for r3 master cwd test");
    let file = tempfile::NamedTempFile::new().unwrap();
    let state_dir = tempfile::TempDir::new().unwrap();
    let project_dir = tempfile::TempDir::new().unwrap();
    let tmux_guard = TmuxServerGuard::new(state_dir.path());
    let ctx = Ctx {
        db: db::init(file.path()).unwrap(),
        state_dir: state_dir.path().to_path_buf(),
        env_state: EnvState {
            systemd_run_available: false,
            unsafe_no_sandbox: true,
            under_systemd: false,
        },
        daemon_unit: None,
        tmux_server: tmux_guard.server(),
        claude_gateway: std::sync::Arc::new(ah::claude_gateway::ClaudeGatewayService::new()),
    };

    db::sessions::create_session(
        ctx.db.clone(),
        "s_r3_master_cwd".to_string(),
        "p_r3_master_cwd".to_string(),
        project_dir.path().to_string_lossy().to_string(),
    )
    .await
    .unwrap();

    let result = handle_session_spawn_master_pane(
        json!({
            "session_id": "s_r3_master_cwd",
            "cmd": "sleep 60",
        }),
        &ctx,
    )
    .await
    .unwrap();
    let pane_id = result["pane_id"].as_str().unwrap();

    let output = Command::new("tmux")
        .args([
            "-L",
            tmux_guard.socket_name(),
            "display-message",
            "-p",
            "-t",
            pane_id,
            "#{pane_current_path}",
        ])
        .output()
        .expect("tmux display-message should run");
    assert!(
        output.status.success(),
        "tmux display-message failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        project_dir.path().to_string_lossy().as_ref()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn agent_pane_starts_in_project_absolute_path_without_sandbox() {
    which::which("tmux").expect("tmux binary required for r3 agent cwd test");
    let file = tempfile::NamedTempFile::new().unwrap();
    let state_dir = tempfile::TempDir::new().unwrap();
    let project_dir = tempfile::TempDir::new().unwrap();
    let tmux_guard = TmuxServerGuard::new(state_dir.path());
    let ctx = Ctx {
        db: db::init(file.path()).unwrap(),
        state_dir: state_dir.path().to_path_buf(),
        env_state: EnvState {
            systemd_run_available: false,
            unsafe_no_sandbox: true,
            under_systemd: false,
        },
        daemon_unit: None,
        tmux_server: tmux_guard.server(),
        claude_gateway: std::sync::Arc::new(ah::claude_gateway::ClaudeGatewayService::new()),
    };

    db::sessions::create_session(
        ctx.db.clone(),
        "s_r3_agent_cwd".to_string(),
        "p_r3_agent_cwd".to_string(),
        project_dir.path().to_string_lossy().to_string(),
    )
    .await
    .unwrap();

    let result = handle_agent_spawn(
        json!({
            "session_id": "s_r3_agent_cwd",
            "agent_id": "ag_r3_agent_cwd",
            "provider": "bash",
        }),
        &ctx,
    )
    .await
    .unwrap();
    let pane_id = result["pane_id"].as_str().unwrap();

    let output = Command::new("tmux")
        .args([
            "-L",
            tmux_guard.socket_name(),
            "display-message",
            "-p",
            "-t",
            pane_id,
            "#{pane_current_path}",
        ])
        .output()
        .expect("tmux display-message should run");
    assert!(
        output.status.success(),
        "tmux display-message failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        project_dir.path().to_string_lossy().as_ref()
    );
}
