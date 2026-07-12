#![cfg(target_os = "linux")]
// PR-2 Grand Tour: cargo test --test ah_full_e2e_drift -- --include-ignored --test-threads=1
#![allow(dead_code)]

mod common;

use ah::db;
use ah::provider::extensions::{HookGroup, HookItem};
use ah::rpc::Ctx;
use ah::rpc::router::dispatch;
use ah::sandbox::EnvState;
use ah::tmux::agent_session_name;
use common::TmuxServerGuard;
use rusqlite::params;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

const PROJECT_ID: &str = "ah_full_e2e_pr2_project";
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
            claude_gateway: std::sync::Arc::new(ah::claude_gateway::ClaudeGatewayService::new()),
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
        let response: Value =
            serde_json::from_str(&dispatch(&request.to_string(), &self.ctx).await)
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

    fn query_agent_pane_id(&self, agent_id: &str) -> String {
        ah::agent_io::pane_id(agent_id)
            .unwrap_or_else(|| panic!("agent pane_id should be registered for {agent_id}"))
            .0
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
        assert!(
            path.exists(),
            "sandbox file should exist: {}",
            path.display()
        );
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
        assert!(
            output.status.success(),
            "tmux list-panes failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(str::to_string)
            .collect()
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
            config
                .push_str(&toml::to_string(&drift.hooks).expect("hooks should serialize to toml"));
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
    let path = project_dir
        .join(".claude")
        .join("plugins")
        .join("cache")
        .join(name);
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

struct MatrixState {
    session_id: String,
    master: MasterSpec,
    agent: AgentSpec,
    drift: DriftSpec,
}

async fn baseline_setup(h: &Harness) -> MatrixState {
    install_fake_claude(h.project_dir());
    let baseline_drift = DriftSpec::base();
    build_drift_ah_toml(h.project_dir(), &baseline_drift);
    let master = MasterSpec::default();
    let agent = baseline_drift.agent_spec(AGENT_ID);

    let created = h
        .rpc(
            "session.create",
            json!({
                "project_id": PROJECT_ID,
                "absolute_path": h.project_dir(),
            }),
        )
        .await;
    let session_id = created["session_id"]
        .as_str()
        .expect("session.create should return session_id")
        .to_string();

    h.rpc(
        "session.spawn_master_pane",
        json!({
            "session_id": session_id,
            "cmd": master.cmd,
            "hooks": master.hooks,
            "plugins": master.plugins,
        }),
    )
    .await;
    h.rpc(
        "agent.spawn",
        json!({
            "session_id": session_id,
            "agent_id": agent.agent_id,
            "provider": agent.provider,
            "extra_env_vars": agent.env,
            "hooks": agent.hooks,
            "plugins": agent.plugins,
        }),
    )
    .await;
    wait_for_agent_state(h, AGENT_ID, "IDLE", Duration::from_secs(10)).await;
    assert!(
        h.has_tmux_session(&ah::tmux::agent_session_name(AGENT_ID)),
        "baseline agent tmux session should exist"
    );

    MatrixState {
        session_id,
        master,
        agent,
        drift: baseline_drift,
    }
}

fn install_fake_claude(project_dir: &Path) {
    let bin_dir = project_dir.join("bin");
    std::fs::create_dir_all(&bin_dir).expect("create fake claude bin dir");
    let fake_claude = bin_dir.join("claude");
    std::fs::write(
        &fake_claude,
        r#"#!/usr/bin/env bash
printf 'status Sonnet\n────────\n  ❯ '
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
    }
}

async fn wait_for_agent_state(h: &Harness, agent_id: &str, expected: &str, timeout: Duration) {
    wait_until("agent state", timeout, || {
        h.query_agent_state(agent_id) == expected
    })
    .await;
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

fn process_exists(pid: i64) -> bool {
    Path::new("/proc").join(pid.to_string()).exists()
}

async fn wait_for_pid_gone(pid: i64) {
    wait_until("old agent pid gone", Duration::from_secs(3), || {
        !process_exists(pid)
    })
    .await;
}

fn agent_status<'a>(statuses: &'a Value, agent_id: &str) -> &'a Value {
    statuses
        .as_array()
        .expect("session.realign statuses should be an array")
        .iter()
        .find(|status| status["agent_id"] == agent_id)
        .unwrap_or_else(|| panic!("missing per-agent status for {agent_id}: {statuses}"))
}

async fn case_01_env_drift(h: &Harness, state: &mut MatrixState) {
    let old_pid = h.query_agent_pid(AGENT_ID);
    let old_hash = h.query_agent_config_hash(AGENT_ID);
    let old_event_count = h.query_agent_events(AGENT_ID, "drift_realigned").len();

    let mut drift = DriftSpec::base();
    drift
        .env
        .insert("GRAND_TOUR_DRIFT_ENV".to_string(), "v2".to_string());
    build_drift_ah_toml(h.project_dir(), &drift);
    state.agent = drift.agent_spec(AGENT_ID);
    state.drift = drift;

    let payload = realign_payload(
        &state.session_id,
        &state.master,
        std::slice::from_ref(&state.agent),
    );
    assert_eq!(
        payload["agents"][0]["env"]["GRAND_TOUR_DRIFT_ENV"], "v2",
        "session.realign payload must carry the env drift"
    );

    let statuses = run_realign(h, payload, false).await;
    let a1_status = agent_status(&statuses, AGENT_ID);
    assert_eq!(
        a1_status["status"], "REALIGNED",
        "assert per-agent statuses[] entry, not master/session aggregate status"
    );
    assert_eq!(a1_status["event"], "drift_realigned");

    wait_for_agent_state(h, AGENT_ID, "IDLE", Duration::from_secs(10)).await;
    let new_pid = h.query_agent_pid(AGENT_ID);
    assert_ne!(
        new_pid, old_pid,
        "env drift should respawn a1 with a new pid"
    );
    wait_for_pid_gone(old_pid).await;

    let new_hash = h.query_agent_config_hash(AGENT_ID);
    assert_ne!(
        new_hash, old_hash,
        "env drift should update agent config_hash"
    );
    let new_event_count = h.query_agent_events(AGENT_ID, "drift_realigned").len();
    assert_eq!(
        new_event_count,
        old_event_count + 1,
        "env drift should add exactly one drift_realigned event"
    );

    println!(
        "case_01 PASS old_pid={old_pid} new_pid={new_pid} old_hash={old_hash} new_hash={new_hash} old_events={old_event_count} new_events={new_event_count} pid_retry=30x100ms"
    );
}

async fn case_02_hooks_drift(h: &Harness, state: &mut MatrixState) {
    let old_pid = h.query_agent_pid(AGENT_ID);
    let old_hash = h.query_agent_config_hash(AGENT_ID);
    let _old_event_count = h.query_agent_events(AGENT_ID, "drift_realigned").len();
    let hook_path = write_hook_script(
        h.project_dir(),
        "hooks/pr2-audit-v2.sh",
        "#!/bin/sh\nexit 0\n",
    );

    let mut drift = state.drift.clone();
    drift.hooks = command_hooks(&hook_path);
    build_drift_ah_toml(h.project_dir(), &drift);
    state.agent = drift.agent_spec(AGENT_ID);
    state.drift = drift;

    let payload = realign_payload(
        &state.session_id,
        &state.master,
        std::slice::from_ref(&state.agent),
    );
    assert!(
        payload["agents"][0]["hooks"]
            .as_object()
            .is_some_and(|hooks| !hooks.is_empty()),
        "session.realign payload must carry hooks drift"
    );

    let statuses = run_realign(h, payload, false).await;
    let a1_status = agent_status(&statuses, AGENT_ID);
    assert_eq!(
        a1_status["status"], "REALIGNED",
        "assert per-agent statuses[] entry, not master/session aggregate status"
    );
    assert_eq!(a1_status["event"], "drift_realigned");
    assert!(
        a1_status["reason"]
            .as_str()
            .is_some_and(|reason| reason.contains("hooks changed")),
        "hooks drift reason should mention hooks changed: {a1_status}"
    );

    wait_for_agent_state(h, AGENT_ID, "IDLE", Duration::from_secs(10)).await;
    let new_pid = h.query_agent_pid(AGENT_ID);
    assert_ne!(
        new_pid, old_pid,
        "hooks drift should respawn a1 with a new pid"
    );
    wait_for_pid_gone(old_pid).await;

    let new_hash = h.query_agent_config_hash(AGENT_ID);
    assert_ne!(
        new_hash, old_hash,
        "hooks drift should update agent config_hash"
    );
    let events = h.query_agent_events(AGENT_ID, "drift_realigned");
    assert!(
        events
            .last()
            .and_then(|event| event["reason"].as_str())
            .is_some_and(|reason| reason.contains("hooks changed")),
        "hooks drift should leave latest drift_realigned reason=hooks changed: {events:?}"
    );

    h.assert_sandbox_file(&state.session_id, AGENT_ID, ".claude/settings.json");
    h.assert_symlink_target(
        &state.session_id,
        AGENT_ID,
        ".claude/hooks/pr2-audit-v2.sh",
        &hook_path,
    );

    println!(
        "case_02 PASS old_pid={old_pid} new_pid={new_pid} old_hash={old_hash} new_hash={new_hash} hook_symlink=.claude/hooks/pr2-audit-v2.sh->{} reason=hooks changed pid_retry=30x100ms",
        hook_path.display()
    );
}

async fn case_03_plugins_drift(h: &Harness, state: &mut MatrixState) {
    let old_pid = h.query_agent_pid(AGENT_ID);
    let old_hash = h.query_agent_config_hash(AGENT_ID);
    let _old_event_count = h.query_agent_events(AGENT_ID, "drift_realigned").len();
    let plugin_cache = write_claude_plugin_cache(h.project_dir(), "pr2-claude-audit");

    let mut drift = state.drift.clone();
    drift.plugins = vec!["pr2-claude-audit".to_string()];
    build_drift_ah_toml(h.project_dir(), &drift);
    state.agent = drift.agent_spec(AGENT_ID);
    state.drift = drift;

    let payload = realign_payload(
        &state.session_id,
        &state.master,
        std::slice::from_ref(&state.agent),
    );
    assert!(
        payload["agents"][0]["plugins"]
            .as_array()
            .is_some_and(|plugins| plugins.iter().any(|plugin| plugin == "pr2-claude-audit")),
        "session.realign payload must carry plugins drift"
    );

    let statuses = run_realign(h, payload, false).await;
    let a1_status = agent_status(&statuses, AGENT_ID);
    assert_eq!(
        a1_status["status"], "REALIGNED",
        "assert per-agent statuses[] entry, not master/session aggregate status"
    );
    assert_eq!(a1_status["event"], "drift_realigned");
    assert!(
        a1_status["reason"]
            .as_str()
            .is_some_and(|reason| reason.contains("plugins changed")),
        "plugins drift reason should mention plugins changed: {a1_status}"
    );

    wait_for_agent_state(h, AGENT_ID, "IDLE", Duration::from_secs(10)).await;
    let new_pid = h.query_agent_pid(AGENT_ID);
    assert_ne!(
        new_pid, old_pid,
        "plugins drift should respawn a1 with a new pid"
    );
    wait_for_pid_gone(old_pid).await;

    let new_hash = h.query_agent_config_hash(AGENT_ID);
    assert_ne!(
        new_hash, old_hash,
        "plugins drift should update agent config_hash"
    );
    let events = h.query_agent_events(AGENT_ID, "drift_realigned");
    assert!(
        events
            .last()
            .and_then(|event| event["reason"].as_str())
            .is_some_and(|reason| reason.contains("plugins changed")),
        "plugins drift should leave latest drift_realigned reason=plugins changed: {events:?}"
    );

    h.assert_symlink_target(
        &state.session_id,
        AGENT_ID,
        ".claude/plugins/cache/pr2-claude-audit",
        &plugin_cache,
    );
    h.assert_symlink_target(
        &state.session_id,
        AGENT_ID,
        ".claude/plugins/pr2-claude-audit",
        &plugin_cache,
    );
    h.assert_json_contains(
        &h.sandbox_path(&state.session_id, AGENT_ID, ".claude/settings.json"),
        "/enabledPlugins/pr2-claude-audit",
        &json!(true),
    );

    println!(
        "case_03 PASS old_pid={old_pid} new_pid={new_pid} old_hash={old_hash} new_hash={new_hash} cache_symlink=.claude/plugins/cache/pr2-claude-audit->{} enabled_symlink=.claude/plugins/pr2-claude-audit->{} reason=plugins changed pid_retry=30x100ms",
        plugin_cache.display(),
        plugin_cache.display()
    );
}

async fn case_04_no_change(h: &Harness, state: &MatrixState) {
    let old_pid = h.query_agent_pid(AGENT_ID);
    let old_hash = h.query_agent_config_hash(AGENT_ID);
    let old_drift_events = h.query_agent_events(AGENT_ID, "drift_realigned").len();
    let old_spawn_events = h.query_agent_events(AGENT_ID, "agent_spawned").len();

    let payload = realign_payload(
        &state.session_id,
        &state.master,
        std::slice::from_ref(&state.agent),
    );
    let statuses = run_realign(h, payload, false).await;
    let a1_status = agent_status(&statuses, AGENT_ID);
    assert_eq!(
        a1_status["status"], "NO_CHANGE",
        "assert per-agent handlers.rs:470 NO_CHANGE, not master/session aggregate status"
    );

    let new_pid = h.query_agent_pid(AGENT_ID);
    let new_hash = h.query_agent_config_hash(AGENT_ID);
    let new_drift_events = h.query_agent_events(AGENT_ID, "drift_realigned").len();
    let new_spawn_events = h.query_agent_events(AGENT_ID, "agent_spawned").len();
    assert_eq!(new_pid, old_pid, "NO_CHANGE must not respawn a1");
    assert_eq!(
        new_hash, old_hash,
        "NO_CHANGE must not alter a1 config_hash"
    );
    assert_eq!(
        new_drift_events, old_drift_events,
        "NO_CHANGE must not add drift_realigned events"
    );
    assert_eq!(
        new_spawn_events, old_spawn_events,
        "NO_CHANGE must not add agent_spawned events"
    );

    println!(
        "case_04 PASS plugins+hooks stacked idempotency pid={new_pid} hash={new_hash} drift_events={new_drift_events} spawn_events={new_spawn_events}"
    );
}

async fn case_05_new_agent(h: &Harness, state: &mut MatrixState) {
    let old_a1_pid = h.query_agent_pid(AGENT_ID);
    let old_a1_pane = h.query_agent_pane_id(AGENT_ID);
    let new_agent = NewAgentSpec::a2(DriftSpec::base());
    build_new_agent_ah_toml(h.project_dir(), &state.drift, &new_agent);

    let a2 = new_agent.agent_spec();
    let payload = realign_payload(
        &state.session_id,
        &state.master,
        &[state.agent.clone(), a2.clone()],
    );
    assert_eq!(
        payload["agents"]
            .as_array()
            .expect("agents should be array")
            .len(),
        2,
        "NEW payload must include both a1 and a2 to avoid orphan classification"
    );

    let statuses = run_realign(h, payload, false).await;
    let a2_status = agent_status(&statuses, NEW_AGENT_ID);
    assert_eq!(a2_status["status"], "NEW");
    assert_eq!(a2_status["action"], "spawned");

    wait_for_agent_state(h, NEW_AGENT_ID, "IDLE", Duration::from_secs(10)).await;
    assert!(
        h.has_tmux_session(&agent_session_name(NEW_AGENT_ID)),
        "agent_a2 tmux session should exist"
    );
    let a2_pane = h.query_agent_pane_id(NEW_AGENT_ID);
    assert_ne!(
        a2_pane, old_a1_pane,
        "a2 pane must be distinct from a1 pane"
    );
    assert!(
        h.list_tmux_panes(&agent_session_name(AGENT_ID))
            .iter()
            .any(|pane| pane == &old_a1_pane),
        "a1 pane should remain in agent_a1 session"
    );
    assert!(
        h.list_tmux_panes(&agent_session_name(NEW_AGENT_ID))
            .iter()
            .any(|pane| pane == &a2_pane),
        "a2 pane should exist in agent_a2 session"
    );

    let spawned_events = h.query_agent_events(NEW_AGENT_ID, "agent_spawned");
    assert!(
        spawned_events
            .iter()
            .any(|event| event["reason"].as_str() == Some("NEW")),
        "a2 should record agent_spawned reason=NEW: {spawned_events:?}"
    );
    assert_eq!(
        h.query_agent_state(AGENT_ID),
        "IDLE",
        "a1 should remain IDLE"
    );
    assert_eq!(
        h.query_agent_pid(AGENT_ID),
        old_a1_pid,
        "adding a2 must not respawn a1"
    );
    h.assert_sandbox_file(&state.session_id, NEW_AGENT_ID, "");
    h.assert_sandbox_file(&state.session_id, NEW_AGENT_ID, ".claude/CLAUDE.md");

    println!(
        "case_05 PASS a2_status=NEW a2_pane={a2_pane} a1_pid={old_a1_pid} a2_claude_rules=exists"
    );
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
// PR-2 cases: ENV Drift / HOOKS Drift / PLUGINS Drift / NO_CHANGE / NEW Agent
// PR-2 scope: DRIFT + NEW only
// ORPHAN / BUSY / ERROR -> PR-3 future scope

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn grand_tour_drift_new_matrix() {
    let h = Harness::new();
    let mut state = baseline_setup(&h).await;
    case_01_env_drift(&h, &mut state).await;
    case_02_hooks_drift(&h, &mut state).await;
    case_03_plugins_drift(&h, &mut state).await;
    case_04_no_change(&h, &state).await;
    case_05_new_agent(&h, &mut state).await;
}
