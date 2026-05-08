use ccbd::tmux::TmuxServer;
use std::process::Command;

fn require_tmux() {
    which::which("tmux").expect("tmux binary required for r1 session lifecycle tests");
}

fn cleanup_server(server: &TmuxServer) {
    let _ = Command::new("tmux")
        .args(["-L", server.socket_name(), "kill-server"])
        .output();
}

#[tokio::test(flavor = "multi_thread")]
async fn kill_session_lifecycle_removes_session() {
    require_tmux();
    let tmp = tempfile::tempdir().unwrap();
    let server = TmuxServer::new(tmp.path());
    let session_name = "r1-session-lifecycle";

    let result = async {
        server
            .ensure_session(session_name.to_string(), tmp.path().to_path_buf())
            .await?;
        server.kill_session(session_name.to_string()).await?;
        let has_session = Command::new("tmux")
            .args(["-L", server.socket_name(), "has-session", "-t", session_name])
            .output()
            .expect("tmux has-session should run");
        assert!(
            !has_session.status.success(),
            "tmux session should be gone after kill-session"
        );
        Ok::<(), ccbd::error::CcbdError>(())
    }
    .await;

    cleanup_server(&server);
    result.unwrap();
}
