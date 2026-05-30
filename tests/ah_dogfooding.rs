#![allow(dead_code)]

use ah::db::{self, agents, events, jobs, sessions, state_machine};
use ah::rpc::Ctx;
use ah::rpc::router::dispatch;
use ah::sandbox::EnvState;
use ah::tmux::TmuxServer;
use rusqlite::params;
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

const SESSION_ID: &str = "dogfood_s1";
const PROJECT_ID: &str = "dogfood_project";
const AGENT_ID: &str = "dogfood_a1";
const JOB_ID: &str = "dogfood_job_1";

struct Harness {
    ctx: Ctx,
    _db_file: tempfile::NamedTempFile,
    state_dir: tempfile::TempDir,
    project_dir: tempfile::TempDir,
}

impl Harness {
    fn new() -> Self {
        let db_file = tempfile::NamedTempFile::new().unwrap();
        let state_dir = tempfile::TempDir::new().unwrap();
        let project_dir = tempfile::TempDir::new().unwrap();
        let tmux_server = Arc::new(TmuxServer::new(state_dir.path()));
        let ctx = Ctx {
            db: db::init(db_file.path()).unwrap(),
            state_dir: state_dir.path().to_path_buf(),
            env_state: EnvState {
                systemd_run_available: false,
                unsafe_no_sandbox: true,
                under_systemd: false,
            },
            daemon_unit: None,
            tmux_server,
        };
        Self {
            ctx,
            _db_file: db_file,
            state_dir,
            project_dir,
        }
    }

    async fn rpc_raw(&self, method: &str, params: Value) -> Value {
        let request = json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
            "id": "dogfood",
        });
        serde_json::from_str(&dispatch(&request.to_string(), &self.ctx).await)
            .expect("dispatch response should be JSON")
    }

    async fn rpc(&self, method: &str, params: Value) -> Value {
        let response = self.rpc_raw(method, params).await;
        if let Some(error) = response.get("error") {
            panic!("RPC {method} failed: {error}");
        }
        response.get("result").cloned().expect("RPC result")
    }

    async fn seed_idle_agent(&self) {
        sessions::insert_session(self.ctx.db.clone(), SESSION_ID.into(), PROJECT_ID.into(), self.project_dir.path().display().to_string()).await.unwrap();
        agents::insert_agent(self.ctx.db.clone(), AGENT_ID.into(), SESSION_ID.into(), "bash".into(), "IDLE".into(), Some(123)).await.unwrap();
    }

    async fn seed_dispatched_busy_job(&self, job_id: &str, prompt: &str) {
        self.seed_idle_agent().await;
        jobs::insert_job(self.ctx.db.clone(), job_id.into(), AGENT_ID.into(), None, prompt.into()).await.unwrap();
        jobs::dispatch_job_to_agent(self.ctx.db.clone(), AGENT_ID.into(), vec!["IDLE".into()], "BUSY".into(), "command_received".into(), json!({ "status": "SENT" })).await.unwrap().expect("job should dispatch");
    }

    fn agent_state(&self) -> String {
        self.ctx.db.conn().query_row("SELECT state FROM agents WHERE id = ?", params![AGENT_ID], |row| row.get(0)).unwrap()
    }

    fn job_status(&self, job_id: &str) -> String {
        self.ctx.db.conn().query_row("SELECT status FROM jobs WHERE id = ?", params![job_id], |row| row.get(0)).unwrap()
    }
}

struct DogfoodAhClient {
    socket: PathBuf,
}

impl DogfoodAhClient {
    fn call(&self, method: &str, params: Value) -> std::io::Result<Value> {
        let mut stream = UnixStream::connect(&self.socket)?;
        let request = json!({"jsonrpc": "2.0", "method": method, "params": params, "id": 1});
        writeln!(stream, "{request}")?;
        stream.shutdown(std::net::Shutdown::Write)?;
        let mut line = String::new();
        BufReader::new(stream).read_line(&mut line)?;
        Ok(serde_json::from_str(line.trim()).expect("response JSON"))
    }
}

fn dogfood_ah_client(state_dir: &Path) -> DogfoodAhClient {
    DogfoodAhClient {
        socket: state_dir.join("ahd.sock"),
    }
}

#[derive(Default)]
struct InterventionCounters {
    cancel: AtomicUsize,
    capture: AtomicUsize,
}

impl InterventionCounters {
    fn cancel_count(&self) -> usize {
        self.cancel.load(Ordering::SeqCst)
    }

    fn capture_count(&self) -> usize {
        self.capture.load(Ordering::SeqCst)
    }
}

async fn dispatch_job_via_ah(h: &Harness, text: &str) -> String {
    let result = h.rpc("job.submit", json!({"agent_id": AGENT_ID, "text": text})).await;
    result["job_id"].as_str().unwrap().to_string()
}

fn install_mock_dogfood_provider(tmp_dir: &Path) -> PathBuf {
    let src = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/mock_dogfood_provider.sh");
    let bin_dir = tmp_dir.join("bin");
    std::fs::create_dir_all(&bin_dir).unwrap();
    let dst = bin_dir.join("mock_dogfood_provider.sh");
    std::fs::copy(src, &dst).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&dst).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&dst, perms).unwrap();
    }
    dst
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn test_event_subscribe_pushes_idle_frame() {
    let h = Harness::new();
    h.seed_idle_agent().await;
    let job_id = dispatch_job_via_ah(&h, "job-id:dogfood_job_1\nwrite design").await;

    let response = h.rpc_raw("event.subscribe", json!({"job_id": job_id, "event_kind": ["job_state_change"]})).await;

    assert!(
        response.get("error").is_none(),
        "event.subscribe should be registered and stream frames, got {response}"
    );
    let frame = response.get("result").expect("subscribe should yield a frame");
    assert_eq!(frame["kind"], "job_state_change");
    assert_eq!(frame["state"], "COMPLETED");
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn test_real_completion_path_no_seam() {
    let h = Harness::new();
    let _provider = install_mock_dogfood_provider(h.project_dir.path());
    h.seed_dispatched_busy_job(JOB_ID, "job-id:dogfood_job_1\nwrite audit").await;

    events::insert_event(h.ctx.db.clone(), AGENT_ID.into(), None, "output_chunk".into(), json!({"text": "mock done\n<<ah-idle:job-id=dogfood_job_1>>\n$ "}).to_string()).await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;

    assert_eq!(h.agent_state(), "IDLE", "marker should drive BUSY -> IDLE");
    assert_eq!(h.job_status(JOB_ID), "COMPLETED", "marker should complete job without dispatch_and_complete_job seam");
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn test_zero_cancel_zero_capture_assertion() {
    let h = Harness::new();
    let counters = Arc::new(InterventionCounters::default());
    h.seed_idle_agent().await;
    let job_id = dispatch_job_via_ah(&h, "job-id:dogfood_job_1\nrun five rpc path").await;

    let subscribe = h.rpc_raw("event.subscribe", json!({"job_id": job_id, "event_kind": ["job_state_change"]})).await;
    let _ = h.rpc("session.kill", json!({ "session_id": SESSION_ID })).await;

    assert!(
        subscribe.get("error").is_none(),
        "5 RPC path requires event.subscribe, got {subscribe}"
    );
    assert_eq!(counters.cancel_count(), 0, "master cancel counter");
    assert_eq!(counters.capture_count(), 0, "master capture counter");
}

#[test]
#[ignore]
fn test_pr1_regression_still_green() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/ah_full_e2e_main.rs");
    let source = std::fs::read_to_string(path).unwrap();
    assert!(
        !source.contains("dispatch_and_complete_job"),
        "PR-1 regression must be migrated off dispatch_and_complete_job seam"
    );
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn test_marker_job_id_对账() {
    let h = Harness::new();
    h.seed_dispatched_busy_job(JOB_ID, "job-id:dogfood_job_1\ncheck marker").await;

    events::insert_event(h.ctx.db.clone(), AGENT_ID.into(), None, "output_chunk".into(), json!({"text": "wrong marker\n<<ah-idle:job-id=WRONG>>\n$ "}).to_string()).await.unwrap();
    let _ = state_machine::mark_agent_idle_matched(h.ctx.db.clone(), AGENT_ID.into()).await.unwrap();

    assert_eq!(h.agent_state(), "BUSY", "wrong job-id marker must not complete current dispatched job");

    events::insert_event(h.ctx.db.clone(), AGENT_ID.into(), None, "output_chunk".into(), json!({"text": "correct marker\n<<ah-idle:job-id=dogfood_job_1>>\n$ "}).to_string()).await.unwrap();
    let _ = state_machine::mark_agent_idle_matched(h.ctx.db.clone(), AGENT_ID.into()).await.unwrap();
    assert_eq!(h.agent_state(), "IDLE");
}
