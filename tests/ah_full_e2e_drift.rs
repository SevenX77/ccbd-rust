// PR-2 Grand Tour: cargo test --test ah_full_e2e_drift -- --include-ignored --test-threads=1

#![allow(dead_code)]

mod common;

use ccbd::db;
use ccbd::provider::extensions::{HookGroup, HookItem};
use ccbd::rpc::Ctx;
use ccbd::rpc::router::dispatch;
use ccbd::sandbox::EnvState;
use common::TmuxServerGuard;
use rusqlite::params;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

const PROJECT_ID: &str = "ah_full_e2e_pr2_project";
const SESSION_ID: &str = "s_pr2";
const AGENT_ID: &str = "a1";
const NEW_AGENT_ID: &str = "a2";
const MASTER_CMD: &str = "bash --noprofile --norc -i";
const AGENT_PROVIDER: &str = "claude";

struct Harness {
    ctx: Ctx,
    _tmux_guard: TmuxServerGuard,
    _db_file: tempfile::NamedTempFile,
    _state_dir: tempfile::TempDir,
    _project_dir: tempfile::TempDir,
}

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
                unsafe_no_sandbox: false,
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
            "id": "pr2",
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

    fn project_dir(&self) -> &Path {
        self._project_dir.path()
    }

    fn sandbox_path(&self, session_id: &str, agent_id: &str, sub_path: &str) -> PathBuf {
        self.ctx
            .state_dir
            .join("sandboxes")
            .join(session_id)
            .join(agent_id)
            .join(sub_path)
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

    fn query_agent_pid(&self, agent_id: &str) -> i64 {
        self.ctx
            .db
            .conn()
            .query_row(
                "SELECT pid FROM agents WHERE id = ?",
                params![agent_id],
                |row| row.get::<_, Option<i64>>(0),
            )
            .expect("agent pid row should exist")
            .expect("agent pid should be set")
    }

    fn query_agent_config_hash(&self, agent_id: &str) -> String {
        self.ctx
            .db
            .conn()
            .query_row(
                "SELECT config_hash FROM agents WHERE id = ?",
                params![agent_id],
                |row| row.get::<_, Option<String>>(0),
            )
            .expect("agent config_hash row should exist")
            .expect("agent config_hash should be set")
    }

    fn query_session_config_hash(&self, session_id: &str) -> String {
        self.ctx
            .db
            .conn()
            .query_row(
                "SELECT config_hash FROM sessions WHERE id = ?",
                params![session_id],
                |row| row.get::<_, Option<String>>(0),
            )
            .expect("session config_hash row should exist")
            .expect("session config_hash should be set")
    }

    fn query_events_by_type(&self, event_type: &str) -> Vec<Value> {
        let conn = self.ctx.db.conn();
        let mut stmt = conn
            .prepare("SELECT payload FROM events WHERE event_type = ? ORDER BY seq_id ASC")
            .expect("events query should prepare");
        stmt.query_map(params![event_type], |row| row.get::<_, String>(0))
            .expect("events query should run")
            .map(|payload| parse_event_payload(&payload.expect("event payload should read")))
            .collect()
    }

    fn query_agent_events(&self, agent_id: &str, event_type: &str) -> Vec<Value> {
        let conn = self.ctx.db.conn();
        let mut stmt = conn
            .prepare(
                "SELECT payload FROM events \
                 WHERE agent_id = ? AND event_type = ? ORDER BY seq_id ASC",
            )
            .expect("agent events query should prepare");
        stmt.query_map(params![agent_id, event_type], |row| row.get::<_, String>(0))
            .expect("agent events query should run")
            .map(|payload| parse_event_payload(&payload.expect("event payload should read")))
            .collect()
    }

    fn assert_sandbox_file(&self, session_id: &str, agent_id: &str, sub_path: &str) -> PathBuf {
        let path = self.sandbox_path(session_id, agent_id, sub_path);
        assert!(path.exists(), "sandbox file should exist: {}", path.display());
        path
    }

    fn assert_symlink_target(
        &self,
        session_id: &str,
        agent_id: &str,
        sub_path: &str,
        expected_target: &Path,
    ) {
        let path = self.assert_sandbox_file(session_id, agent_id, sub_path);
        let target = std::fs::read_link(&path)
            .unwrap_or_else(|err| panic!("{} should be a symlink: {err}", path.display()));
        assert_eq!(
            target,
            expected_target,
            "symlink target mismatch for {}",
            path.display()
        );
    }

    fn assert_json_contains(&self, path: &Path, pointer_or_key: &str, expected: &Value) {
        let raw = std::fs::read_to_string(path)
            .unwrap_or_else(|err| panic!("read JSON {}: {err}", path.display()));
        let json: Value = serde_json::from_str(&raw)
            .unwrap_or_else(|err| panic!("parse JSON {}: {err}", path.display()));
        let actual = if pointer_or_key.starts_with('/') {
            json.pointer(pointer_or_key)
        } else {
            json.get(pointer_or_key)
        }
        .unwrap_or_else(|| panic!("missing JSON key {pointer_or_key} in {}", path.display()));
        assert_eq!(actual, expected, "JSON value mismatch at {pointer_or_key}");
    }
}

fn parse_event_payload(payload: &str) -> Value {
    serde_json::from_str(payload).unwrap_or_else(|_| Value::String(payload.to_string()))
}

/// PR-2 grep notes:
/// - `fn drift_reason` returns "plugins changed", "hooks changed", "env changed".
/// - `compute_config_hash` hashes role(provider/env for agent, cmd for master), hooks, and sorted plugins.
#[derive(Clone, Default)]
struct DriftSpec {
    env: HashMap<String, String>,
    hooks: HashMap<String, Vec<HookGroup>>,
    plugins: Vec<String>,
}

impl DriftSpec {
    fn base() -> Self {
        Self::default()
    }

    fn agent_spec(&self, agent_id: &str) -> AgentSpec {
        AgentSpec {
            agent_id: agent_id.to_string(),
            provider: AGENT_PROVIDER.to_string(),
            env: self.env.clone(),
            hooks: self.hooks.clone(),
            plugins: self.plugins.clone(),
        }
    }
}

#[derive(Clone)]
struct NewAgentSpec {
    agent_id: String,
    drift: DriftSpec,
}

impl NewAgentSpec {
    fn a2(drift: DriftSpec) -> Self {
        Self {
            agent_id: NEW_AGENT_ID.to_string(),
            drift,
        }
    }

    fn agent_spec(&self) -> AgentSpec {
        self.drift.agent_spec(&self.agent_id)
    }
}

#[derive(Clone)]
struct MasterSpec {
    cmd: String,
    hooks: HashMap<String, Vec<HookGroup>>,
    plugins: Vec<String>,
}

impl Default for MasterSpec {
    fn default() -> Self {
        Self {
            cmd: MASTER_CMD.to_string(),
            hooks: HashMap::new(),
            plugins: Vec::new(),
        }
    }
}

impl MasterSpec {
    fn rpc_value(&self) -> Value {
        json!({
            "cmd": self.cmd,
            "hooks": self.hooks,
            "plugins": self.plugins,
        })
    }
}

#[derive(Clone)]
struct AgentSpec {
    agent_id: String,
    provider: String,
    env: HashMap<String, String>,
    hooks: HashMap<String, Vec<HookGroup>>,
    plugins: Vec<String>,
}

impl AgentSpec {
    fn rpc_value(&self) -> Value {
        json!({
            "agent_id": self.agent_id,
            "provider": self.provider,
            "env": self.env,
            "hooks": self.hooks,
            "plugins": self.plugins,
        })
    }
}

/// PR-2 grep notes:
/// - `HookGroup` is defined in `src/provider/extensions.rs:13`.
/// - Claude hooks materialize to `.claude/hooks/<script-name>` via `materialize_claude_hooks`.
fn build_drift_ah_toml(project_dir: &Path, drift: &DriftSpec) -> PathBuf {
    let path = project_dir.join("ah.toml");
    let config = render_ah_toml(&[(AGENT_ID, drift)]);
    std::fs::write(&path, config).expect("write drift ah.toml");
    path
}

fn build_new_agent_ah_toml(
    project_dir: &Path,
    drift: &DriftSpec,
    new_agent: &NewAgentSpec,
) -> PathBuf {
    let new_agent_drift = &new_agent.drift;
    let config = render_ah_toml(&[(AGENT_ID, drift), (&new_agent.agent_id, new_agent_drift)]);
    let path = project_dir.join("ah.toml");
    std::fs::write(&path, config).expect("write new-agent ah.toml");
    path
}

fn render_ah_toml(agents: &[(&str, &DriftSpec)]) -> String {
    let mut config = format!(
        r#"version = "1"

[master]
cmd = "{MASTER_CMD}"
"#
    );
    for (agent_id, drift) in agents {
        config.push_str(&format!(
            r#"
[agents.{agent_id}]
provider = "{AGENT_PROVIDER}"
"#
        ));
        if !drift.plugins.is_empty() {
            let plugins = drift
                .plugins
                .iter()
                .map(|plugin| format!("{plugin:?}"))
                .collect::<Vec<_>>()
                .join(", ");
            config.push_str(&format!("plugins = [{plugins}]\n"));
        }
        if !drift.env.is_empty() {
            config.push_str(&format!("\n[agents.{agent_id}.env]\n"));
            for (key, value) in &drift.env {
                config.push_str(&format!("{key} = {value:?}\n"));
            }
        }
        if !drift.hooks.is_empty() {
            config.push_str(&format!("\n[agents.{agent_id}.hooks]\n"));
            config.push_str(
                &toml::to_string(&drift.hooks).expect("hooks should serialize to toml"),
            );
        }
    }
    config
}

fn write_hook_script(project_dir: &Path, name: &str, body: &str) -> PathBuf {
    let path = project_dir.join(name);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create hook parent dir");
    }
    std::fs::write(&path, body).expect("write hook script");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = std::fs::metadata(&path)
            .expect("hook metadata should exist")
            .permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&path, permissions).expect("chmod hook script");
    }
    path
}

/// PR-2 grep notes:
/// - `materialize_claude_plugins` symlinks `.claude/plugins/cache/<name>`.
/// - It also symlinks `.claude/plugins/<name>` (non-cache enabled plugin path).
fn write_claude_plugin_cache(project_dir: &Path, name: &str) -> PathBuf {
    let path = project_dir.join(".claude").join("plugins").join("cache").join(name);
    std::fs::create_dir_all(&path).expect("create claude plugin cache");
    std::fs::write(path.join("plugin.json"), "{}\n").expect("write plugin manifest");
    path
}

fn realign_payload(session_id: &str, master: &MasterSpec, agents: &[AgentSpec]) -> Value {
    json!({
        "session_id": session_id,
        "force": false,
        "master": master.rpc_value(),
        "agents": agents.iter().map(AgentSpec::rpc_value).collect::<Vec<_>>(),
    })
}

async fn run_realign(h: &Harness, mut payload: Value, force: bool) -> Value {
    payload["force"] = json!(force);
    let result = h.rpc("session.realign", payload).await;
    result
        .get("statuses")
        .cloned()
        .expect("session.realign result should include statuses")
}

fn command_hooks(command: &Path) -> HashMap<String, Vec<HookGroup>> {
    HashMap::from([(
        "PreToolUse".to_string(),
        vec![HookGroup {
            matcher: "*".to_string(),
            hooks: vec![HookItem {
                hook_type: "command".to_string(),
                command: command.display().to_string(),
                timeout: None,
            }],
        }],
    )])
}

// TODO PR-2 T4: case_01_env_drift
// TODO PR-2 T5: case_02_hooks_drift
// TODO PR-2 T6: case_03_plugins_drift
// TODO PR-2 T7: case_04_no_change
// TODO PR-2 T8: case_05_new_agent

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn grand_tour_drift_new_matrix() {
    panic!("red: PR-2 DRIFT + NEW matrix not implemented");
}
