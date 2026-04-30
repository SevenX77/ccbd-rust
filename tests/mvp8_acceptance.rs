use ccbd::db;
use ccbd::db::agents::insert_agent;
use ccbd::db::jobs::query_job;
use ccbd::db::sessions::insert_session;
use ccbd::rpc::Ctx;
use ccbd::rpc::handlers::handle_job_submit;
use ccbd::sandbox::EnvState;
use ccbd::tmux::TmuxServer;
use serde_json::json;
use std::sync::Arc;

struct Harness {
    ctx: Ctx,
    _state_dir: tempfile::TempDir,
    _db_file: tempfile::NamedTempFile,
}

impl Harness {
    fn new() -> Self {
        let db_file = tempfile::NamedTempFile::new().unwrap();
        let state_dir = tempfile::TempDir::new().unwrap();
        let state_dir_path = state_dir.path().to_path_buf();
        let ctx = Ctx {
            db: db::init(db_file.path()).unwrap(),
            state_dir: state_dir_path.clone(),
            env_state: EnvState {
                bwrap_available: false,
                systemd_run_available: false,
                unsafe_no_sandbox: true,
            },
            tmux_server: Arc::new(TmuxServer::new(&state_dir_path)),
        };

        Self {
            ctx,
            _state_dir: state_dir,
            _db_file: db_file,
        }
    }

    async fn seed_agent(&self, agent_id: &str, state: &str) {
        insert_session(
            self.ctx.db.clone(),
            "s_m8".to_string(),
            "p_m8".to_string(),
            "/tmp/m8".to_string(),
            std::process::id() as i64,
        )
        .await
        .unwrap();
        insert_agent(
            self.ctx.db.clone(),
            agent_id.to_string(),
            "s_m8".to_string(),
            "bash".to_string(),
            state.to_string(),
            Some(123),
        )
        .await
        .unwrap();
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_job_submit_returns_id_and_queues_when_idle_or_busy() {
    let h = Harness::new();
    h.seed_agent("ag_m8_submit", "IDLE").await;

    let result = handle_job_submit(
        json!({
            "agent_id": "ag_m8_submit",
            "text": "echo mvp8\n",
        }),
        &h.ctx,
    )
    .await
    .unwrap();
    let job_id = result["job_id"].as_str().unwrap();

    assert!(job_id.starts_with("job_"));
    assert_eq!(result["status"], "QUEUED");
    let job = query_job(h.ctx.db.clone(), job_id.to_string())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(job.agent_id, "ag_m8_submit");
    assert_eq!(job.prompt_text, "echo mvp8\n");
    assert_eq!(job.status, "QUEUED");
    assert_eq!(job.dispatched_at_seq_id, None);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_job_submit_is_idempotent_by_request_id() {
    let h = Harness::new();
    h.seed_agent("ag_m8_idem", "BUSY").await;

    let first = handle_job_submit(
        json!({
            "agent_id": "ag_m8_idem",
            "text": "echo first\n",
            "request_id": "m8-req-1",
        }),
        &h.ctx,
    )
    .await
    .unwrap();
    let second = handle_job_submit(
        json!({
            "agent_id": "ag_m8_idem",
            "text": "echo second\n",
            "request_id": "m8-req-1",
        }),
        &h.ctx,
    )
    .await
    .unwrap();

    assert_eq!(first["job_id"], second["job_id"]);
    let job = query_job(
        h.ctx.db.clone(),
        first["job_id"].as_str().unwrap().to_string(),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(job.prompt_text, "echo first\n");
    assert_eq!(job.request_id.as_deref(), Some("m8-req-1"));
}
