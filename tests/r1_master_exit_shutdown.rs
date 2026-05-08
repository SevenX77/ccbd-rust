use ccbd::db;
use ccbd::tmux::scope::unit_name_for_socket;
use ccbd::tmux::TmuxServer;
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

static DEV_STATE_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

fn require_tmux() {
    which::which("tmux").expect("tmux binary required for r1 master exit shutdown tests");
}

fn dev_state_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("dev_state")
}

fn cleanup_dev_state(state_dir: &Path) {
    let server = TmuxServer::new(state_dir);
    let _ = Command::new("systemctl")
        .args([
            "--user",
            "stop",
            &format!("{}.scope", unit_name_for_socket(server.socket_name())),
        ])
        .output();
    for suffix in ["", "-shm", "-wal"] {
        let _ = std::fs::remove_file(state_dir.join(format!("ccbd.sqlite{suffix}")));
    }
    let _ = std::fs::remove_file(state_dir.join("ccbd.sock"));
    let _ = Command::new("tmux")
        .args(["-L", server.socket_name(), "kill-server"])
        .output();
}

fn spawn_daemon(state_dir: &Path) -> Child {
    let child = Command::new(env!("CARGO_BIN_EXE_ccbd"))
        .env("CCB_ENV", "dev")
        .env("CCBD_UNSAFE_NO_SANDBOX", "1")
        .spawn()
        .expect("spawn ccbd");
    wait_for_socket(&state_dir.join("ccbd.sock"), Duration::from_secs(5));
    child
}

fn wait_for_socket(path: &Path, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if path.exists() {
            return;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    panic!("ccbd socket did not appear at {}", path.display());
}

fn rpc_call(socket_path: &Path, id: u64, method: &str, params: Value) -> Value {
    let mut stream = UnixStream::connect(socket_path).expect("connect ccbd socket");
    let request = json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": method,
        "params": params,
    });
    writeln!(stream, "{request}").expect("write rpc request");

    let mut reader = BufReader::new(stream);
    let mut response = String::new();
    reader.read_line(&mut response).expect("read rpc response");
    let value: Value = serde_json::from_str(response.trim()).expect("parse rpc response");
    assert!(
        value.get("error").is_none(),
        "rpc {method} returned error: {value}"
    );
    value["result"].clone()
}

fn create_session(socket_path: &Path, id: u64, project_id: &str, project_dir: &Path) -> String {
    let result = rpc_call(
        socket_path,
        id,
        "session.create",
        json!({
            "project_id": project_id,
            "absolute_path": project_dir,
        }),
    );
    result["session_id"].as_str().unwrap().to_string()
}

fn spawn_master(socket_path: &Path, id: u64, session_id: &str) -> String {
    let result = rpc_call(
        socket_path,
        id,
        "session.spawn_master_pane",
        json!({
            "session_id": session_id,
            "cmd": "sleep 60",
        }),
    );
    result["pane_id"].as_str().unwrap().to_string()
}

fn spawn_agent(socket_path: &Path, id: u64, session_id: &str, agent_id: &str) {
    let _ = rpc_call(
        socket_path,
        id,
        "agent.spawn",
        json!({
            "session_id": session_id,
            "agent_id": agent_id,
            "provider": "bash",
        }),
    );
}

fn pane_pid(state_dir: &Path, pane_id: &str) -> i32 {
    let server = TmuxServer::new(state_dir);
    let output = Command::new("tmux")
        .args([
            "-L",
            server.socket_name(),
            "display-message",
            "-p",
            "-t",
            pane_id,
            "#{pane_pid}",
        ])
        .output()
        .expect("tmux display-message");
    assert!(
        output.status.success(),
        "tmux display-message failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse()
        .unwrap()
}

fn kill_pid(pid: i32) {
    // SAFETY: pid comes from tmux's #{pane_pid}; SIGTERM is a constant signal.
    unsafe {
        libc::kill(pid, libc::SIGTERM);
    }
}

fn wait_for_daemon_exit(child: &mut Child, timeout: Duration) -> Option<std::process::ExitStatus> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if let Some(status) = child.try_wait().expect("poll ccbd child") {
            return Some(status);
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    None
}

fn terminate_daemon(mut child: Child) {
    // SAFETY: child.id is the ccbd process spawned by this test.
    unsafe {
        libc::kill(child.id() as i32, libc::SIGTERM);
    }
    let _ = wait_for_daemon_exit(&mut child, Duration::from_secs(5));
    let _ = child.kill();
}

fn active_agent_count(state_dir: &Path) -> i64 {
    let db = db::init(&state_dir.join("ccbd.sqlite")).unwrap();
    db.conn()
        .query_row(
            "SELECT COUNT(*) FROM agents WHERE state NOT IN ('CRASHED', 'KILLED')",
            [],
            |row| row.get(0),
        )
        .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn master_exit_with_zero_active_agents_shuts_down_daemon_after_grace() {
    let _guard = DEV_STATE_LOCK.lock().unwrap();
    require_tmux();
    let state_dir = dev_state_dir();
    std::fs::create_dir_all(&state_dir).unwrap();
    cleanup_dev_state(&state_dir);
    let socket_path = state_dir.join("ccbd.sock");
    let project_dir = tempfile::TempDir::new().unwrap();

    let mut child = spawn_daemon(&state_dir);
    let session_id = create_session(&socket_path, 1, "p_master_exit_shutdown", project_dir.path());
    let master_pane = spawn_master(&socket_path, 2, &session_id);
    spawn_agent(&socket_path, 3, &session_id, "ag_master_exit_shutdown");

    kill_pid(pane_pid(&state_dir, &master_pane));

    let status = wait_for_daemon_exit(&mut child, Duration::from_secs(10));
    assert!(
        status.is_some_and(|status| status.success()),
        "ccbd did not exit successfully within 10s after master exit"
    );
    assert_eq!(active_agent_count(&state_dir), 0);

    cleanup_dev_state(&state_dir);
}

#[tokio::test(flavor = "multi_thread")]
async fn master_exit_does_not_shutdown_daemon_when_other_session_has_active_agent() {
    let _guard = DEV_STATE_LOCK.lock().unwrap();
    require_tmux();
    let state_dir = dev_state_dir();
    std::fs::create_dir_all(&state_dir).unwrap();
    cleanup_dev_state(&state_dir);
    let socket_path = state_dir.join("ccbd.sock");
    let first_project = tempfile::TempDir::new().unwrap();
    let second_project = tempfile::TempDir::new().unwrap();

    let mut child = spawn_daemon(&state_dir);
    let first_session = create_session(
        &socket_path,
        10,
        "p_master_exit_has_peer",
        first_project.path(),
    );
    let first_master = spawn_master(&socket_path, 11, &first_session);
    spawn_agent(&socket_path, 12, &first_session, "ag_master_exit_has_peer");

    let second_session = create_session(
        &socket_path,
        13,
        "p_master_exit_peer_alive",
        second_project.path(),
    );
    let _second_master = spawn_master(&socket_path, 14, &second_session);
    spawn_agent(&socket_path, 15, &second_session, "ag_master_exit_peer_alive");

    kill_pid(pane_pid(&state_dir, &first_master));

    let status = wait_for_daemon_exit(&mut child, Duration::from_secs(7));
    assert!(
        status.is_none(),
        "ccbd exited despite another daemon-wide active agent"
    );
    assert_eq!(active_agent_count(&state_dir), 1);

    terminate_daemon(child);
    cleanup_dev_state(&state_dir);
}
