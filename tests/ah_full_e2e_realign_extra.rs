#![allow(dead_code)]

// PR-3 Grand Tour: cargo test --test ah_full_e2e_realign_extra -- --include-ignored --test-threads=1

mod common;

use ccbd::db;
use ccbd::provider::extensions::HookGroup;
use ccbd::rpc::Ctx;
use ccbd::rpc::router::dispatch;
use ccbd::sandbox::EnvState;
use ccbd::tmux::agent_session_name;
use common::TmuxServerGuard;
use rusqlite::params;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

const PROJECT_ID: &str = "ah_full_e2e_pr3_project";
const MASTER_CMD: &str = "bash --noprofile --norc -i";
const AGENT_PROVIDER: &str = "claude";
const MOCK_ECHO: &str = "ECHO";
const MOCK_BUSY: &str = "BUSY";
const MOCK_CRASH: &str = "CRASH";

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
        let response = self.rpc_raw(method, params).await;
        if let Some(error) = response.get("error") {
            panic!("RPC {method} failed: {error}");
        }
        response
            .get("result")
            .cloned()
            .expect("dispatch response should include result")
    }

    async fn rpc_raw(&self, method: &str, params: Value) -> Value {
        let request = json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
            "id": "pr3",
        });
        serde_json::from_str(&dispatch(&request.to_string(), &self.ctx).await)
            .expect("dispatch response should be valid JSON")
    }

    fn project_dir(&self) -> &Path {
        self._project_dir.path()
    }

    fn sandbox_path(&self, session_id: &str, agent_id: &str, sub_path: &str) -> PathBuf {
        if sub_path.starts_with(".claude/")
            || sub_path.starts_with(".gemini/")
            || sub_path.starts_with(".codex/")
        {
            return self.provider_home_path(session_id, agent_id).join(sub_path);
        }
        self.ctx
            .state_dir
            .join("sandboxes")
            .join(session_id)
            .join(agent_id)
            .join(sub_path)
    }

    fn provider_home_path(&self, session_id: &str, agent_id: &str) -> PathBuf {
        let sandbox_dir = self
            .ctx
            .state_dir
            .join("sandboxes")
            .join(session_id)
            .join(agent_id);
        let sandbox_path = sandbox_dir
            .canonicalize()
            .unwrap_or_else(|_| sandbox_dir.to_path_buf());
        let mut hasher = Sha256::new();
        hasher.update(sandbox_path.to_string_lossy().as_bytes());
        let digest = hasher.finalize();
        let project_id_short = digest
            .iter()
            .take(6)
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let cache_root = std::env::var_os("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                std::env::var_os("HOME")
                    .map(PathBuf::from)
                    .unwrap_or_else(std::env::temp_dir)
                    .join(".cache")
            });
        cache_root.join("ah/sandboxes").join(project_id_short)
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

    fn query_agent_pane_id(&self, agent_id: &str) -> Option<String> {
        ccbd::agent_io::pane_id(agent_id).map(|pane_id| pane_id.0)
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

    fn query_agent_last_error(&self, agent_id: &str) -> (Option<String>, Option<i64>) {
        self.ctx
            .db
            .conn()
            .query_row(
                "SELECT error_code, exit_code FROM agents WHERE id = ?",
                params![agent_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("agent error row should exist")
    }

    fn update_agent_state_direct(&self, agent_id: &str, state: &str) {
        self.ctx
            .db
            .conn()
            .execute(
                "UPDATE agents SET state = ?, state_version = state_version + 1, updated_at = unixepoch() WHERE id = ?",
                params![state, agent_id],
            )
            .expect("direct agent state update should succeed");
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

    fn has_tmux_session(&self, session_name: &str) -> bool {
        Command::new("tmux")
            .args([
                "-L",
                self.ctx.tmux_server.socket_name(),
                "has-session",
                "-t",
                session_name,
            ])
            .status()
            .is_ok_and(|status| status.success())
    }

    fn list_tmux_panes(&self, session_name: &str) -> Vec<String> {
        let output = Command::new("tmux")
            .args([
                "-L",
                self.ctx.tmux_server.socket_name(),
                "list-panes",
                "-t",
                session_name,
                "-F",
                "#{pane_id}",
            ])
            .output()
            .expect("tmux list-panes should run");
        if !output.status.success() {
            return Vec::new();
        }
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(str::to_string)
            .collect()
    }

    async fn wait_for_tmux_pane_gone(&self, agent_id: &str, timeout: Duration) {
        let session_name = agent_session_name(agent_id);
        let old_pane = self.query_agent_pane_id(agent_id);
        wait_until("agent tmux pane gone", timeout, || {
            if let Some(pane) = old_pane.as_ref()
                && self.list_tmux_panes(&session_name).iter().any(|item| item == pane)
            {
                return false;
            }
            !self.has_tmux_session(&session_name) || self.list_tmux_panes(&session_name).is_empty()
        })
        .await;
    }
}

fn parse_event_payload(payload: &str) -> Value {
    serde_json::from_str(payload).unwrap_or_else(|_| Value::String(payload.to_string()))
}

#[derive(Clone, Default)]
struct AgentFixture {
    env: HashMap<String, String>,
    hooks: HashMap<String, Vec<HookGroup>>,
    plugins: Vec<String>,
}

impl AgentFixture {
    fn spec(&self, agent_id: &str) -> AgentSpec {
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

fn build_realign_extra_ah_toml(project_dir: &Path, agents: &[(&str, &AgentFixture)]) -> PathBuf {
    let path = project_dir.join("ah.toml");
    let mut config = format!(
        r#"version = "1"

[master]
cmd = "{MASTER_CMD}"
"#
    );
    for (agent_id, fixture) in agents {
        config.push_str(&format!(
            r#"
[agents.{agent_id}]
provider = "{AGENT_PROVIDER}"
"#
        ));
        if !fixture.plugins.is_empty() {
            let plugins = fixture
                .plugins
                .iter()
                .map(|plugin| format!("{plugin:?}"))
                .collect::<Vec<_>>()
                .join(", ");
            config.push_str(&format!("plugins = [{plugins}]\n"));
        }
        if !fixture.env.is_empty() {
            config.push_str(&format!("\n[agents.{agent_id}.env]\n"));
            for (key, value) in &fixture.env {
                config.push_str(&format!("{key} = {value:?}\n"));
            }
        }
        if !fixture.hooks.is_empty() {
            config.push_str(&format!("\n[agents.{agent_id}.hooks]\n"));
            config.push_str(
                &toml::to_string(&fixture.hooks).expect("hooks should serialize to toml"),
            );
        }
    }
    std::fs::write(&path, config).expect("write PR-3 ah.toml");
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

fn install_fake_claude(project_dir: &Path) -> PathBuf {
    install_fake_claude_with_behavior(project_dir, MOCK_ECHO)
}

fn install_fake_claude_with_behavior(project_dir: &Path, behavior: &str) -> PathBuf {
    assert!(
        [MOCK_ECHO, MOCK_BUSY, MOCK_CRASH].contains(&behavior),
        "unsupported fake claude behavior: {behavior}"
    );
    let bin_dir = project_dir.join("bin");
    std::fs::create_dir_all(&bin_dir).expect("create fake claude bin dir");
    let fake_claude = bin_dir.join("claude");
    std::fs::write(
        &fake_claude,
        r#"#!/usr/bin/env bash
behavior="${GRAND_TOUR_MOCK_BEHAVIOR:-ECHO}"
if [[ "$behavior" == "CRASH" ]]; then
  exit 1
fi

printf 'status Sonnet\n────────\n  ❯ '
if [[ "$behavior" == "BUSY" ]]; then
  if IFS= read -r line; then
    printf '\nmock claude busy: %s\n' "$line"
    while true; do sleep 3600; done
  fi
  exit 0
fi

while IFS= read -r line; do
  printf '\nmock claude: %s\n  ❯ ' "$line"
done
"#,
    )
    .expect("write fake claude");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = std::fs::metadata(&fake_claude)
            .expect("fake claude metadata should exist")
            .permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&fake_claude, permissions).expect("chmod fake claude");
    }
    let old_path = std::env::var("PATH").unwrap_or_default();
    let new_path = format!("{}:{old_path}", bin_dir.display());
    unsafe {
        std::env::set_var("HOME", project_dir);
        std::env::set_var("XDG_CACHE_HOME", project_dir.join(".cache"));
        std::env::set_var("PATH", new_path);
        std::env::set_var("GRAND_TOUR_MOCK_BEHAVIOR", behavior);
    }
    fake_claude
}

async fn wait_for_agent_state(h: &Harness, agent_id: &str, expected: &str, timeout: Duration) {
    wait_until("agent state", timeout, || h.query_agent_state(agent_id) == expected).await;
}

async fn wait_until<F>(label: &str, timeout: Duration, mut predicate: F)
where
    F: FnMut() -> bool,
{
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if predicate() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("timed out waiting for {label}");
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn grand_tour_realign_extra_matrix() {
    panic!("red: PR-3 ORPHAN + BUSY + ERROR matrix not implemented");
}

// TODO PR-3 T4: case_06_orphan_audit_only
// TODO PR-3 T5: case_07_orphan_force_cleanup
// TODO PR-3 T6: case_08_busy_skip
// TODO PR-3 T7: case_09_busy_force_realign
// TODO PR-3 T8: case_10_error_crash_detection
// TODO PR-3 T9: case_11_error_recovery_known_gap
