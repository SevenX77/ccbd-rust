#[cfg(any())]
mod common;

#[allow(dead_code)]
mod common {
    use ccbd::tmux::TmuxServer;
    use std::ops::Deref;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::sync::Arc;

    pub struct TmuxServerGuard {
        server: Arc<TmuxServer>,
        socket_path: PathBuf,
    }

    impl TmuxServerGuard {
        pub fn new(state_dir: &Path) -> Self {
            let server = Arc::new(TmuxServer::new(state_dir));
            let socket_path = tmux_socket_path(server.socket_name());
            Self {
                server,
                socket_path,
            }
        }

        pub fn server(&self) -> Arc<TmuxServer> {
            self.server.clone()
        }

        pub fn socket_name(&self) -> &str {
            self.server.socket_name()
        }
    }

    impl Drop for TmuxServerGuard {
        fn drop(&mut self) {
            let _ = Command::new("tmux")
                .args(["-L", self.server.socket_name(), "kill-server"])
                .output();
            let _ = std::fs::remove_file(&self.socket_path);
        }
    }

    impl Deref for TmuxServerGuard {
        type Target = TmuxServer;

        fn deref(&self) -> &Self::Target {
            &self.server
        }
    }

    fn tmux_socket_path(socket_name: &str) -> PathBuf {
        let uid = unsafe { libc::geteuid() };
        PathBuf::from(format!("/tmp/tmux-{uid}")).join(socket_name)
    }
}

use ccbd::db;
use ccbd::rpc::Ctx;
use ccbd::rpc::router::dispatch;
use ccbd::sandbox::EnvState;
use common::TmuxServerGuard;
use rusqlite::params;
use serde_json::{Value, json};
use std::process::Command;

#[allow(dead_code)]
struct Harness {
    ctx: Ctx,
    _tmux_guard: TmuxServerGuard,
    _db_file: tempfile::NamedTempFile,
    _state_dir: tempfile::TempDir,
    _project_dir: tempfile::TempDir,
}

#[allow(dead_code)]
impl Harness {
    fn new() -> Self {
        let db_file = tempfile::NamedTempFile::new().unwrap();
        let state_dir = tempfile::TempDir::new().unwrap();
        let project_dir = tempfile::TempDir::new().unwrap();
        let tmux_guard = TmuxServerGuard::new(state_dir.path());
        let ctx = Ctx {
            db: db::init(db_file.path()).unwrap(),
            state_dir: state_dir.path().to_path_buf(),
            env_state: EnvState {
                systemd_run_available: false,
                unsafe_no_sandbox: true,
                under_systemd: false,
            },
            daemon_unit: None,
            tmux_server: tmux_guard.server(),
        };

        Self {
            ctx,
            _tmux_guard: tmux_guard,
            _db_file: db_file,
            _state_dir: state_dir,
            _project_dir: project_dir,
        }
    }

    async fn rpc(&self, method: &str, params: Value) -> Value {
        let request = json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
            "id": 1,
        });
        let response: Value = serde_json::from_str(&dispatch(&request.to_string(), &self.ctx).await)
            .expect("dispatch response should be valid JSON");
        if let Some(error) = response.get("error") {
            panic!("RPC {method} failed: {error}");
        }
        response
            .get("result")
            .cloned()
            .expect("dispatch response should include result")
    }

    fn query_agent_state(&self, agent_id: &str) -> String {
        self.ctx
            .db
            .conn()
            .query_row(
                "SELECT state FROM agents WHERE id = ?",
                params![agent_id],
                |row| row.get(0),
            )
            .expect("agent state should exist")
    }

    fn query_events_by_type(&self, event_type: &str) -> Vec<Value> {
        let conn = self.ctx.db.conn();
        let mut stmt = conn
            .prepare(
                "SELECT payload FROM events WHERE event_type = ? ORDER BY seq_id ASC",
            )
            .expect("prepare events query");
        let rows = stmt
            .query_map(params![event_type], |row| row.get::<_, String>(0))
            .expect("query events by type");

        rows.map(|payload| {
            let payload = payload.expect("event payload row should be readable");
            serde_json::from_str(&payload).unwrap_or_else(|_| Value::String(payload))
        })
        .collect()
    }

    fn has_tmux_session(&self, session_name: &str) -> bool {
        Command::new("tmux")
            .args([
                "-L",
                self._tmux_guard.socket_name(),
                "has-session",
                "-t",
                session_name,
            ])
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false)
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn grand_tour_mainline_13_ah_commands_plus_config_drift() {
    panic!("red: grand tour mainline not implemented");
}
