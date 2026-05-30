use ah::db;
use ah::rpc::{
    Ctx,
    handlers::{handle_agent_spawn, handle_session_create, handle_session_spawn_master_pane},
};
use ah::sandbox::EnvState;
use ah::tmux::{TmuxServer, agent_session_name, master_session_name};
use serde_json::json;
use std::process::Command;
use std::sync::Arc;

struct TmuxServerGuard {
    server: Arc<TmuxServer>,
}

impl TmuxServerGuard {
    fn new(server: Arc<TmuxServer>) -> Self {
        Self { server }
    }
}

impl Drop for TmuxServerGuard {
    fn drop(&mut self) {
        let _ = Command::new("tmux")
            .args(["-L", self.server.socket_name(), "kill-server"])
            .output();
    }
}

fn tmux_output(server: &TmuxServer, args: &[&str]) -> std::process::Output {
    let mut command = Command::new("tmux");
    command.args(["-L", server.socket_name()]);
    command.args(args);
    command.output().expect("tmux command should run")
}

fn tmux_has_session(server: &TmuxServer, session_name: &str) -> bool {
    tmux_output(server, &["has-session", "-t", session_name])
        .status
        .success()
}

fn tmux_session_names(server: &TmuxServer) -> String {
    let output = tmux_output(server, &["list-sessions", "-F", "#{session_name}"]);
    if output.status.success() {
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    } else {
        format!(
            "<list-sessions failed: {}>",
            String::from_utf8_lossy(&output.stderr).trim()
        )
    }
}

fn pane_current_path(server: &TmuxServer, pane_id: &str) -> String {
    let output = tmux_output(
        server,
        &[
            "display-message",
            "-p",
            "-t",
            pane_id,
            "#{pane_current_path}",
        ],
    );
    assert!(
        output.status.success(),
        "tmux display-message failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

#[tokio::test(flavor = "multi_thread")]
async fn r1_r3_tmux_session_names_and_cwd_follow_absolute_path() {
    which::which("tmux").expect("tmux binary required for r1/r3 joint tmux test");

    let db_file = tempfile::NamedTempFile::new().unwrap();
    let state_dir = tempfile::TempDir::new().unwrap();
    let project_dir = tempfile::TempDir::new().unwrap();
    std::fs::write(project_dir.path().join("r1-r3-marker.txt"), "joint").unwrap();

    let tmux_server = Arc::new(TmuxServer::new(state_dir.path()));
    let _guard = TmuxServerGuard::new(tmux_server.clone());
    let ctx = Ctx {
        db: db::init(db_file.path()).unwrap(),
        state_dir: state_dir.path().to_path_buf(),
        env_state: EnvState {
            systemd_run_available: false,
            unsafe_no_sandbox: true,
            under_systemd: false,
        },
        daemon_unit: None,
        tmux_server: tmux_server.clone(),
    };

    let project_id = "r1r3_joint_project";
    let agent_id = "r1r3_joint_agent";
    let create = handle_session_create(
        json!({
            "project_id": project_id,
            "absolute_path": project_dir.path(),
        }),
        &ctx,
    )
    .await
    .unwrap();
    let session_id = create["session_id"].as_str().unwrap();

    let master = handle_session_spawn_master_pane(
        json!({
            "session_id": session_id,
            "cmd": "sleep 60",
        }),
        &ctx,
    )
    .await
    .unwrap();
    let master_pane_id = master["pane_id"].as_str().unwrap();

    let agent = handle_agent_spawn(
        json!({
            "session_id": session_id,
            "agent_id": agent_id,
            "provider": "bash",
        }),
        &ctx,
    )
    .await
    .unwrap();
    let agent_pane_id = agent["pane_id"].as_str().unwrap();

    assert!(
        tmux_has_session(&tmux_server, &master_session_name(project_id)),
        "master tmux session should use the R1 name master_<project_id>; sessions={}",
        tmux_session_names(&tmux_server)
    );
    assert!(
        tmux_has_session(&tmux_server, &agent_session_name(agent_id)),
        "agent tmux session should use the R1 name agent_<agent_id>; sessions={}",
        tmux_session_names(&tmux_server)
    );
    assert_eq!(
        pane_current_path(&tmux_server, master_pane_id),
        project_dir.path().to_string_lossy()
    );
    assert_eq!(
        pane_current_path(&tmux_server, agent_pane_id),
        project_dir.path().to_string_lossy()
    );
}
