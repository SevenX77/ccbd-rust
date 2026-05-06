use ccbd::tmux::{SESSION_NAME, SplitDirection, SplitSpec, TmuxServer};
use std::process::Command;

fn require_tmux() {
    which::which("tmux").expect("tmux binary required for mvp12 grid layout test");
}

fn cleanup_server(server: &TmuxServer) {
    let _ = Command::new("tmux")
        .args(["-L", server.socket_name(), "kill-server"])
        .output();
    let socket_path = format!(
        "/tmp/tmux-{}/{}",
        unsafe { libc::geteuid() },
        server.socket_name()
    );
    let _ = std::fs::remove_file(socket_path);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_grid_split_hints_create_three_deterministic_panes() {
    require_tmux();
    let tmp = tempfile::tempdir().unwrap();
    let server = TmuxServer::new(tmp.path());

    let result = async {
        server
            .ensure_session(SESSION_NAME.into(), tmp.path().to_path_buf())
            .await?;
        let target = format!("{SESSION_NAME}:mvp12-grid");
        let first = server
            .spawn_window(
                SESSION_NAME.into(),
                "mvp12-grid".into(),
                tmp.path().to_path_buf(),
                vec!["bash".into()],
            )
            .await?;
        let second = server
            .split_window_with_spec(
                target.clone(),
                tmp.path().to_path_buf(),
                vec!["bash".into()],
                SplitSpec {
                    parent: Some(first.clone()),
                    direction: Some(SplitDirection::Right),
                    percent: Some(50),
                },
            )
            .await?;
        let third = server
            .split_window_with_spec(
                target.clone(),
                tmp.path().to_path_buf(),
                vec!["bash".into()],
                SplitSpec {
                    parent: Some(second.clone()),
                    direction: Some(SplitDirection::Bottom),
                    percent: Some(50),
                },
            )
            .await?;

        let panes = ccbd::tmux::layout::list_panes(&server, target).await?;
        assert_eq!(panes.len(), 3);
        assert!(panes.contains(&first));
        assert!(panes.contains(&second));
        assert!(panes.contains(&third));

        let all_panes = Command::new("tmux")
            .args([
                "-L",
                server.socket_name(),
                "list-panes",
                "-a",
                "-F",
                "#{pane_id}",
            ])
            .output()
            .unwrap();
        assert!(all_panes.status.success());
        assert_eq!(
            String::from_utf8_lossy(&all_panes.stdout)
                .lines()
                .filter(|line| !line.trim().is_empty())
                .count(),
            3
        );
        Ok::<(), ccbd::error::CcbdError>(())
    }
    .await;

    cleanup_server(&server);
    result.unwrap();
}
