use ccbd::cli::rpc_client::{RpcClient, UnixRpcClient};
use ccbd::db;
use ccbd::tmux::compute_socket_name;
use serde_json::json;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{Duration, Instant};

struct ChildGuard {
    child: Child,
}

impl ChildGuard {
    fn new(child: Child) -> Self {
        Self { child }
    }

    fn id(&self) -> u32 {
        self.child.id()
    }

    fn wait_timeout(&mut self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if self.child.try_wait().unwrap().is_some() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        false
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_main_sigterm_cleans_resources() {
    which::which("tmux").expect("tmux binary required for mvp10 acceptance tests");
    let xdg_state = tempfile::tempdir().unwrap();
    let state_dir = xdg_state.path().join("ccbd");
    let socket_name = compute_socket_name(&state_dir);
    let daemon_socket = state_dir.join("ccbd.sock");
    let tmux_socket = tmux_socket_path(&socket_name);

    let child = Command::new(env!("CARGO_BIN_EXE_ccbd"))
        .env("XDG_STATE_HOME", xdg_state.path())
        .env("CCBD_UNSAFE_NO_SANDBOX", "1")
        .env_remove("CCB_ENV")
        .spawn()
        .expect("spawn ccbd daemon");
    let mut child = ChildGuard::new(child);
    wait_for(
        || daemon_socket.exists(),
        Duration::from_secs(5),
        "daemon socket",
    );

    let client = UnixRpcClient::new(daemon_socket);
    let session = client
        .call(
            "session.create",
            json!({
                "project_id": "mvp10_sigterm",
                "absolute_path": state_dir.display().to_string(),
                "master_pid": std::process::id() as i64,
            }),
        )
        .await
        .expect("create session");
    let session_id = session["session_id"].as_str().unwrap().to_string();
    client
        .call(
            "agent.spawn",
            json!({
                "session_id": session_id,
                "agent_id": "ag_mvp10_sigterm",
                "provider": "bash",
            }),
        )
        .await
        .expect("spawn agent");
    wait_for(
        || tmux_socket_alive(&socket_name),
        Duration::from_secs(5),
        "tmux socket alive",
    );

    unsafe {
        libc::kill(child.id() as i32, libc::SIGTERM);
    }
    assert!(
        child.wait_timeout(Duration::from_secs(5)),
        "ccbd daemon did not exit after SIGTERM"
    );
    wait_for(
        || !tmux_socket_alive(&socket_name) && !tmux_socket.exists(),
        Duration::from_secs(5),
        "tmux cleanup after SIGTERM",
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_startup_reconcile_cleans_stale_sockets() {
    which::which("tmux").expect("tmux binary required for mvp10 acceptance tests");
    let socket_dir = PathBuf::from(format!("/tmp/tmux-{}", unsafe { libc::geteuid() }));
    std::fs::create_dir_all(&socket_dir).unwrap();
    let fake_socket = socket_dir.join("ccbd-fakedeadbeef0000");
    std::fs::write(&fake_socket, b"stale").unwrap();

    let db_file = tempfile::NamedTempFile::new().unwrap();
    let state_dir = tempfile::tempdir().unwrap();
    let db = db::init(db_file.path()).unwrap();
    db::system::reconcile_startup_with_tmux_socket(
        db,
        state_dir.path().to_path_buf(),
        Some("ccbd-live-current".to_string()),
    )
    .await
    .unwrap();

    assert!(
        !fake_socket.exists(),
        "startup reconcile did not remove stale socket {}",
        fake_socket.display()
    );
}

#[ignore = "T2.4 rewrites this as wrapper scope SIGKILL coverage"]
#[test]
fn test_main_sigkill_systemd_cleans() {
    unimplemented!("T2.4 wrapper scope model");
}

fn wait_for(mut pred: impl FnMut() -> bool, timeout: Duration, label: &str) {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if pred() {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("timed out waiting for {label}");
}

fn tmux_socket_alive(socket_name: &str) -> bool {
    match Command::new("tmux")
        .args(["-L", socket_name, "ls-sessions"])
        .output()
    {
        Ok(output) if output.status.success() => true,
        Ok(output) if String::from_utf8_lossy(&output.stderr).contains("unknown command") => {
            Command::new("tmux")
                .args(["-L", socket_name, "list-sessions"])
                .output()
                .map(|output| output.status.success())
                .unwrap_or(false)
        }
        _ => false,
    }
}

fn tmux_socket_path(socket_name: &str) -> PathBuf {
    Path::new("/tmp")
        .join(format!("tmux-{}", unsafe { libc::geteuid() }))
        .join(socket_name)
}
