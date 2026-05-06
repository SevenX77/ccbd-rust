pub mod error;
pub mod layout;
pub mod pane;
pub mod scope;
pub mod session;

use sha2::{Digest, Sha256};
use std::path::Path;

pub use error::TmuxError;
pub use layout::LayoutKind;
pub use pane::TmuxPaneId;
pub use session::{SplitDirection, SplitSpec, TmuxServer};

pub const SESSION_NAME: &str = "ccbd-agents";

pub fn compute_socket_name(state_dir: &Path) -> String {
    let canonical = state_dir
        .canonicalize()
        .unwrap_or_else(|_| state_dir.to_path_buf());
    let mut hasher = Sha256::new();
    hasher.update(canonical.display().to_string().as_bytes());
    let hex = format!("{:x}", hasher.finalize());
    format!("ccbd-{}", &hex[..16])
}

#[cfg(test)]
mod tests {
    use super::{SESSION_NAME, TmuxServer, compute_socket_name};
    use crate::tmux::pane::TmuxPaneId;
    use crate::tmux::session::parse_pane_pid_for_test;
    use nix::sys::stat::Mode;
    use std::io::Read;
    use std::os::unix::fs::OpenOptionsExt;
    use std::process::Command;
    use std::sync::{Arc, Mutex, mpsc};
    use std::time::{Duration, Instant};

    fn require_tmux() {
        which::which("tmux").expect("tmux binary required for tmux module tests");
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

    async fn wait_for_pane_text(
        server: &TmuxServer,
        pane: &TmuxPaneId,
        needle: &str,
        timeout: Duration,
    ) -> Result<(), crate::error::CcbdError> {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            let capture = server.capture_pane(pane.clone()).await?;
            if capture.contains(needle) {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        Err(crate::error::CcbdError::TmuxCommandFailed {
            cmd: "wait_for_pane_text".into(),
            stderr: format!("pane did not contain {needle:?} within {timeout:?}"),
            exit: -1,
        })
    }

    #[test]
    fn test_compute_socket_name_deterministic() {
        let tmp = tempfile::tempdir().unwrap();
        let first = compute_socket_name(tmp.path());
        let second = compute_socket_name(tmp.path());

        assert_eq!(first, second);
        assert!(first.starts_with("ccbd-"));
        assert_eq!(first.len(), "ccbd-".len() + 16);
    }

    #[test]
    fn test_pane_id_parse() {
        assert_eq!(TmuxPaneId::parse("%7").unwrap(), TmuxPaneId("%7".into()));
        assert!(TmuxPaneId::parse("7").is_err());
    }

    #[test]
    fn test_pid_parse_error() {
        assert!(parse_pane_pid_for_test("not-a-pid").is_err());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_full_lifecycle() {
        require_tmux();
        let tmp = tempfile::tempdir().unwrap();
        let server = TmuxServer::new(tmp.path());

        let result = async {
            server
                .ensure_session(SESSION_NAME.into(), tmp.path().to_path_buf())
                .await?;
            let pane = server
                .spawn_window(
                    SESSION_NAME.into(),
                    "test-agent".into(),
                    tmp.path().to_path_buf(),
                    vec!["bash".into()],
                )
                .await?;
            let pid = server.get_pane_pid(pane.clone()).await?;
            assert!(pid > 0);

            let fifo_path = tmp.path().join("agent.fifo");
            nix::unistd::mkfifo(&fifo_path, Mode::S_IRUSR | Mode::S_IWUSR).unwrap();
            let mut fifo = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(&fifo_path)
                .unwrap();

            server
                .pipe_pane_to_fifo(pane.clone(), fifo_path.clone())
                .await?;
            let (tx, rx) = mpsc::channel();
            std::thread::spawn(move || {
                let deadline = Instant::now() + Duration::from_secs(5);
                let mut seen = String::new();
                let mut buf = [0_u8; 256];
                while Instant::now() < deadline && !seen.contains("echo hello") {
                    match fifo.read(&mut buf) {
                        Ok(0) => continue,
                        Ok(n) => seen.push_str(&String::from_utf8_lossy(&buf[..n])),
                        Err(_) => break,
                    }
                }
                let _ = tx.send(seen);
            });

            server
                .send_keys_literal(pane.clone(), "echo hello".into())
                .await?;
            server
                .send_keys_keysym(pane.clone(), "Enter".into())
                .await?;

            let seen = rx.recv_timeout(Duration::from_secs(6)).unwrap_or_default();
            assert!(
                seen.contains("echo hello"),
                "fifo output did not contain command echo; seen={seen:?}"
            );

            server.kill_pane(pane).await?;
            Ok::<(), crate::error::CcbdError>(())
        }
        .await;

        cleanup_server(&server);
        result.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_buffer_load_paste_delete_multiline() {
        require_tmux();
        let tmp = tempfile::tempdir().unwrap();
        let server = TmuxServer::new(tmp.path());

        let result = async {
            server
                .ensure_session(SESSION_NAME.into(), tmp.path().to_path_buf())
                .await?;
            let pane = server
                .spawn_window(
                    SESSION_NAME.into(),
                    "test-buffer-agent".into(),
                    tmp.path().to_path_buf(),
                    vec![
                        "env".into(),
                        "PS1=$ ".into(),
                        "bash".into(),
                        "--noprofile".into(),
                        "--norc".into(),
                        "-i".into(),
                    ],
                )
                .await?;
            wait_for_pane_text(&server, &pane, "$", Duration::from_secs(15)).await?;

            let fifo_path = tmp.path().join("buffer-agent.fifo");
            nix::unistd::mkfifo(&fifo_path, Mode::S_IRUSR | Mode::S_IWUSR).unwrap();
            let mut fifo = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .custom_flags(libc::O_NONBLOCK)
                .open(&fifo_path)
                .unwrap();
            server
                .pipe_pane_to_fifo(pane.clone(), fifo_path.clone())
                .await?;
            tokio::time::sleep(Duration::from_secs(1)).await;

            let seen_output = Arc::new(Mutex::new(String::new()));
            let seen_for_reader = seen_output.clone();
            std::thread::spawn(move || {
                let deadline = Instant::now() + Duration::from_secs(15);
                let mut buf = [0_u8; 512];
                while Instant::now() < deadline
                    && !seen_for_reader
                        .lock()
                        .is_ok_and(|seen| seen.contains("line 1") && seen.contains("line 2"))
                {
                    match fifo.read(&mut buf) {
                        Ok(0) => std::thread::sleep(Duration::from_millis(50)),
                        Ok(n) => {
                            if let Ok(mut seen) = seen_for_reader.lock() {
                                seen.push_str(&String::from_utf8_lossy(&buf[..n]));
                            }
                        }
                        Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(Duration::from_millis(50));
                        }
                        Err(_) => break,
                    }
                }
            });

            let buffer_name = "ccbd-test-buffer";
            let text = "for i in 1 2; do\n  echo \"line $i\"\ndone\n";
            for _ in 0..5 {
                server
                    .load_buffer(buffer_name.to_string(), text.to_string())
                    .await?;
                server
                    .paste_buffer(pane.clone(), buffer_name.to_string())
                    .await?;
                server.send_enter(pane.clone()).await?;
                tokio::time::sleep(Duration::from_millis(500)).await;
                if seen_output
                    .lock()
                    .is_ok_and(|seen| seen.contains("line 1") && seen.contains("line 2"))
                {
                    break;
                }
            }
            server.delete_buffer(buffer_name.to_string()).await?;

            let deadline = Instant::now() + Duration::from_secs(15);
            let seen = loop {
                let seen = seen_output
                    .lock()
                    .map(|seen| seen.clone())
                    .unwrap_or_default();
                if seen.contains("line 1") && seen.contains("line 2") {
                    break seen;
                }
                if Instant::now() >= deadline {
                    break seen;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            };
            assert!(
                seen.contains("line 1"),
                "fifo output missing line 1; seen={seen:?}"
            );
            assert!(
                seen.contains("line 2"),
                "fifo output missing line 2; seen={seen:?}"
            );
            assert!(
                !seen.contains("syntax error near unexpected token"),
                "multiline paste produced shell syntax error; seen={seen:?}"
            );

            server.kill_pane(pane).await?;
            Ok::<(), crate::error::CcbdError>(())
        }
        .await;

        cleanup_server(&server);
        result.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_layout_grid_lists_shared_panes() {
        require_tmux();
        let tmp = tempfile::tempdir().unwrap();
        let server = TmuxServer::new(tmp.path());

        let result = async {
            server
                .ensure_session(SESSION_NAME.into(), tmp.path().to_path_buf())
                .await?;
            let target = format!("{SESSION_NAME}:layout-agent");
            let first = server
                .spawn_window(
                    SESSION_NAME.into(),
                    "layout-agent".into(),
                    tmp.path().to_path_buf(),
                    vec!["bash".into()],
                )
                .await?;
            let second = server
                .split_window(
                    target.clone(),
                    tmp.path().to_path_buf(),
                    vec!["bash".into()],
                )
                .await?;
            let third = server
                .split_window(
                    target.clone(),
                    tmp.path().to_path_buf(),
                    vec!["bash".into()],
                )
                .await?;

            crate::tmux::layout::apply_layout(
                &server,
                target.clone(),
                crate::tmux::LayoutKind::Grid,
            )
            .await?;
            let panes = crate::tmux::layout::list_panes(&server, target).await?;

            assert_eq!(panes.len(), 3);
            assert!(panes.contains(&first));
            assert!(panes.contains(&second));
            assert!(panes.contains(&third));
            Ok::<(), crate::error::CcbdError>(())
        }
        .await;

        cleanup_server(&server);
        result.unwrap();
    }
}
