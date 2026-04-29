use ccbd::db;
use ccbd::db::agents::{insert_agent, query_agent_state};
use ccbd::db::sessions::insert_session;
use ccbd::db::system::reconcile_startup;
use ccbd::error::CcbdError;
use ccbd::rpc::Ctx;
use ccbd::rpc::handlers::{
    handle_agent_kill, handle_agent_read, handle_agent_send, handle_agent_spawn,
    handle_session_create,
};
use ccbd::sandbox::{EnvState, bwrap};
use ccbd::tmux::TmuxServer;
use serde_json::{Value, json};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

struct Harness {
    ctx: Ctx,
    _state_dir: tempfile::TempDir,
    _db_file: tempfile::NamedTempFile,
}

impl Harness {
    fn new(unsafe_no_sandbox: bool) -> Self {
        let db_file = tempfile::NamedTempFile::new().unwrap();
        let state_dir = tempfile::TempDir::new().unwrap();
        let state_dir_path = state_dir.path().to_path_buf();
        let ctx = Ctx {
            db: db::init(db_file.path()).unwrap(),
            state_dir: state_dir_path.clone(),
            env_state: EnvState {
                bwrap_available: !unsafe_no_sandbox,
                systemd_run_available: !unsafe_no_sandbox,
                unsafe_no_sandbox,
            },
            tmux_server: Arc::new(TmuxServer::new(&state_dir_path)),
        };

        Self {
            ctx,
            _state_dir: state_dir,
            _db_file: db_file,
        }
    }

    async fn insert_session(&self, session_id: &str, master_pid: i64) {
        insert_session(
            self.ctx.db.clone(),
            session_id.to_string(),
            format!("p_{session_id}"),
            format!("/tmp/{session_id}"),
            master_pid,
        )
        .await
        .unwrap();
    }
}

async fn sleep_ms(ms: u64) {
    tokio::task::spawn_blocking(move || std::thread::sleep(Duration::from_millis(ms)))
        .await
        .unwrap();
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

async fn wait_for_event(
    h: &Harness,
    agent_id: &str,
    timeout: Duration,
    pred: impl Fn(&Value) -> bool,
) -> Value {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        let result = handle_agent_read(
            json!({
                "agent_id": agent_id,
                "since_event_id": 0,
            }),
            &h.ctx,
        )
        .await
        .unwrap();
        if let Some(event) = result["events"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| pred(e))
        {
            return event.clone();
        }
        sleep_ms(50).await;
    }
    panic!("event for {agent_id} did not appear within {timeout:?}");
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

async fn wait_for_proc_absent(pid: i64, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if !std::path::Path::new(&format!("/proc/{pid}")).exists() {
            return true;
        }
        sleep_ms(50).await;
    }
    false
}

fn proc_comm(pid: i64) -> String {
    std::fs::read_to_string(format!("/proc/{pid}/comm"))
        .unwrap()
        .trim()
        .to_string()
}

async fn wait_for_proc_comm(pid: i64, timeout: Duration, pred: impl Fn(&str) -> bool) -> String {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if let Ok(comm) = std::fs::read_to_string(format!("/proc/{pid}/comm")) {
            let comm = comm.trim().to_string();
            if pred(&comm) {
                return comm;
            }
        }
        sleep_ms(50).await;
    }
    proc_comm(pid)
}

fn kill_pid(pid: i64, signal: i32) {
    // SAFETY: kill is invoked with a pid returned by the OS and a constant
    // signal. It does not dereference Rust memory.
    let result = unsafe { libc::kill(pid as libc::pid_t, signal) };
    assert_eq!(result, 0, "kill({pid}, {signal}) failed");
}

#[tokio::test(flavor = "multi_thread")]
async fn ac1_sandbox_boundary_builds_private_home_and_etc_policy() {
    let tmp = tempfile::TempDir::new().unwrap();
    let args = bwrap::build_args(tmp.path(), &bwrap::SandboxOverrides::default()).unwrap();

    assert!(args.windows(2).any(|w| w == ["--dir", "/home/agent"]));
    assert!(
        args.windows(3)
            .any(|w| w == ["--ro-bind", "/etc/resolv.conf", "/etc/resolv.conf"])
    );
    assert!(!args.windows(3).any(|w| w == ["--ro-bind", "/etc", "/etc"]));
}

#[tokio::test(flavor = "multi_thread")]
async fn ac2_master_death_cascades_agents_to_killed() {
    let h = Harness::new(true);
    let mut master = Command::new("sh")
        .arg("-c")
        .arg("sleep 60")
        .spawn()
        .unwrap();
    let session = handle_session_create(
        json!({
            "project_id": "p_master",
            "absolute_path": "/tmp/master-ac2",
            "master_pid": master.id(),
        }),
        &h.ctx,
    )
    .await
    .unwrap()["session_id"]
        .as_str()
        .unwrap()
        .to_string();
    let agent_id = format!("ag_ac2_{}", uuid::Uuid::new_v4());
    let _pid = spawn_bash(&h, &session, &agent_id).await;

    master.kill().unwrap();
    let _ = master.wait();

    let event = wait_for_event(&h, &agent_id, Duration::from_secs(2), |event| {
        event["payload"]
            .as_str()
            .is_some_and(|p| p.contains("\"to\":\"KILLED\"") && p.contains("MASTER_DEATH"))
    })
    .await;
    assert_eq!(event["event_type"], "state_change");
}

// Run with:
// cargo test --test mvp2_acceptance ac3a_systemd_managed_cascade -- --ignored --include-ignored
#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn ac3a_systemd_managed_cascade() {
    panic!("systemd-managed cascade is reserved for the dedicated systemd CI job");
}

#[tokio::test(flavor = "multi_thread")]
async fn ac3b_startup_reconcile_marks_live_master_agents_crashed() {
    let h = Harness::new(true);
    h.insert_session("s_ac3b", std::process::id() as i64).await;
    insert_agent(
        h.ctx.db.clone(),
        "ag_ac3b".to_string(),
        "s_ac3b".to_string(),
        "bash".to_string(),
        "BUSY".to_string(),
        Some(123),
    )
    .await
    .unwrap();

    let count = reconcile_startup(h.ctx.db.clone()).await.unwrap();
    let event = wait_for_event(&h, "ag_ac3b", Duration::from_secs(1), |event| {
        event["payload"]
            .as_str()
            .is_some_and(|p| p.contains("\"to\":\"CRASHED\"") && p.contains("STARTUP_RECONCILE"))
    })
    .await;

    assert_eq!(count, 1);
    assert_eq!(event["event_type"], "state_change");
    let _ = ccbd::monitor::remove("master:s_ac3b");
}

#[tokio::test(flavor = "multi_thread")]
async fn ac4_agent_pidfd_captures_external_death() {
    let h = Harness::new(true);
    h.insert_session("s_ac4", 999).await;
    let agent_id = format!("ag_ac4_{}", uuid::Uuid::new_v4());
    let pid = spawn_bash(&h, "s_ac4", &agent_id).await;

    let comm = wait_for_proc_comm(pid, Duration::from_secs(1), |comm| comm == "bash").await;
    assert_eq!(comm, "bash");
    kill_pid(pid, libc::SIGKILL);

    let event = wait_for_event(&h, &agent_id, Duration::from_secs(2), |event| {
        event["payload"].as_str().is_some_and(|p| {
            p.contains("\"to\":\"CRASHED\"") && p.contains("EXIT_CODE_UNAVAILABLE_NON_CHILD")
        })
    })
    .await;
    let exit_code: Option<i64> = h
        .ctx
        .db
        .conn()
        .query_row(
            "SELECT exit_code FROM agents WHERE id = ?",
            [agent_id.as_str()],
            |row| row.get(0),
        )
        .unwrap();

    assert_eq!(event["event_type"], "state_change");
    assert_eq!(exit_code, None);
}

#[tokio::test(flavor = "multi_thread")]
async fn ac5_agent_kill_marks_killed_and_is_not_repeatable() {
    let h = Harness::new(true);
    h.insert_session("s_ac5", 999).await;
    let agent_id = format!("ag_ac5_{}", uuid::Uuid::new_v4());
    let pid = spawn_bash(&h, "s_ac5", &agent_id).await;

    let killed = handle_agent_kill(json!({ "agent_id": agent_id }), &h.ctx)
        .await
        .unwrap();
    let repeat = handle_agent_kill(json!({ "agent_id": agent_id }), &h.ctx)
        .await
        .unwrap_err();
    let event = wait_for_event(&h, &agent_id, Duration::from_secs(1), |event| {
        event["payload"]
            .as_str()
            .is_some_and(|p| p.contains("\"to\":\"KILLED\"") && p.contains("SIGKILL_BY_DAEMON"))
    })
    .await;

    assert_eq!(killed["state"], "KILLED");
    assert!(matches!(repeat, CcbdError::AgentNotFound(_)));
    assert_eq!(event["event_type"], "state_change");
    assert!(!ccbd::monitor::contains(&agent_id));
    assert!(wait_for_proc_absent(pid, Duration::from_secs(2)).await);
}

#[tokio::test(flavor = "multi_thread")]
async fn ac6_unsafe_bypass_spawns_provider_directly() {
    let h = Harness::new(true);
    h.insert_session("s_ac6", 999).await;
    let agent_id = format!("ag_ac6_{}", uuid::Uuid::new_v4());
    let pid = spawn_bash(&h, "s_ac6", &agent_id).await;

    let comm = wait_for_proc_comm(pid, Duration::from_secs(1), |comm| comm == "bash").await;
    assert_eq!(comm, "bash");
    send_text(&h, &agent_id, "printf 'bypass_home:%s\\n' \"$HOME\"\n").await;
    let event = wait_for_event(&h, &agent_id, Duration::from_secs(1), |event| {
        event["payload"]
            .as_str()
            .is_some_and(|p| p.contains("bypass_home:"))
    })
    .await;

    assert!(event["payload"].as_str().unwrap().contains("bypass_home:"));
    let _ = handle_agent_kill(json!({ "agent_id": agent_id }), &h.ctx).await;
}

#[test]
fn ac7_missing_bwrap_fails_closed_before_socket() {
    let state_dir = std::path::Path::new("target/dev_state");
    let socket_path = state_dir.join("ccbd.sock");
    let _ = std::fs::remove_file(&socket_path);

    let output = Command::new(env!("CARGO_BIN_EXE_ccbd"))
        .env("CCB_ENV", "dev")
        .env_remove("CCBD_UNSAFE_NO_SANDBOX")
        .env("PATH", "/nonexistent/bin")
        .stdin(Stdio::null())
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("SANDBOX_BWRAP_NOT_FOUND"),
        "stderr={stderr}"
    );
    assert!(!socket_path.exists());
}
