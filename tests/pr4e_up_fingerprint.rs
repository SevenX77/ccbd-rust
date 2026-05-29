use ccbd::db;
use ccbd::provider::extensions::{HookGroup, HookItem};
use ccbd::provider::fingerprint::{ConfigFingerprintInput, ConfigRole, compute_config_hash};
use ccbd::rpc::Ctx;
use ccbd::rpc::router::dispatch;
use ccbd::sandbox::EnvState;
use ccbd::tmux::TmuxServer;
use rusqlite::{Connection, params};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Arc;

const MASTER_CMD: &str = "ccb master";
const AGENT_PROVIDER: &str = "claude";

struct Harness {
    ctx: Ctx,
    _db_file: tempfile::NamedTempFile,
    _state_dir: tempfile::TempDir,
}

impl Harness {
    fn new() -> Self {
        let db_file = tempfile::NamedTempFile::new().unwrap();
        let state_dir = tempfile::TempDir::new().unwrap();
        let ctx = Ctx {
            db: db::init(db_file.path()).unwrap(),
            state_dir: state_dir.path().to_path_buf(),
            env_state: EnvState {
                systemd_run_available: false,
                unsafe_no_sandbox: true,
                under_systemd: false,
            },
            daemon_unit: None,
            tmux_server: Arc::new(TmuxServer::new(state_dir.path())),
        };
        seed_base_rows(&ctx.db.conn());
        Self {
            ctx,
            _db_file: db_file,
            _state_dir: state_dir,
        }
    }
}

#[derive(Clone)]
struct MasterSpec {
    cmd: String,
    hooks: HashMap<String, Vec<HookGroup>>,
    plugins: Vec<String>,
}

impl MasterSpec {
    fn new(plugins: &[&str], hooks: HashMap<String, Vec<HookGroup>>) -> Self {
        Self {
            cmd: MASTER_CMD.to_string(),
            hooks,
            plugins: plugins.iter().map(|plugin| plugin.to_string()).collect(),
        }
    }

    fn hash(&self) -> String {
        compute_config_hash(&ConfigFingerprintInput {
            role: ConfigRole::Master { cmd: &self.cmd },
            hooks: &self.hooks,
            plugins: &self.plugins,
        })
        .expect("PR4e fingerprint API should compute a deterministic master hash")
    }

    fn rpc_value(&self) -> Value {
        json!({
            "cmd": self.cmd,
            "hooks": self.hooks,
            "plugins": self.plugins
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
    fn new(agent_id: &str, plugins: &[&str], hooks: HashMap<String, Vec<HookGroup>>) -> Self {
        Self {
            agent_id: agent_id.to_string(),
            provider: AGENT_PROVIDER.to_string(),
            env: HashMap::new(),
            hooks,
            plugins: plugins.iter().map(|plugin| plugin.to_string()).collect(),
        }
    }

    fn hash(&self) -> String {
        compute_config_hash(&ConfigFingerprintInput {
            role: ConfigRole::Agent {
                provider: &self.provider,
                env: &self.env,
            },
            hooks: &self.hooks,
            plugins: &self.plugins,
        })
        .expect("PR4e fingerprint API should compute a deterministic agent hash")
    }

    fn rpc_value(&self) -> Value {
        json!({
            "agent_id": self.agent_id,
            "provider": self.provider,
            "env": self.env,
            "hooks": self.hooks,
            "plugins": self.plugins
        })
    }
}

fn seed_base_rows(conn: &Connection) {
    conn.execute(
        "INSERT INTO projects (id, absolute_path) VALUES ('p4e', '/tmp/pr4e')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO sessions (id, project_id, master_pid, master_pane_id) VALUES ('s4e', 'p4e', 1, '%1')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO agents (id, session_id, provider, state, pid) VALUES ('a1', 's4e', 'claude', 'IDLE', 11)",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO agents (id, session_id, provider, state, pid) VALUES ('a2', 's4e', 'codex', 'IDLE', 12)",
        [],
    )
    .unwrap();
}

fn set_session_hash(conn: &Connection, hash: &str) {
    conn.execute(
        "UPDATE sessions SET config_hash = ? WHERE id = 's4e'",
        params![hash],
    )
    .expect("PR4e must add sessions.config_hash and allow storing the running master fingerprint");
}

fn set_agent_hash(conn: &Connection, agent_id: &str, hash: &str) {
    conn.execute(
        "UPDATE agents SET config_hash = ? WHERE id = ?",
        params![hash, agent_id],
    )
    .expect("PR4e must add agents.config_hash and allow storing the running agent fingerprint");
}

fn set_agent_state(conn: &Connection, agent_id: &str, state: &str) {
    conn.execute(
        "UPDATE agents SET state = ?, state_version = state_version + 1 WHERE id = ?",
        params![state, agent_id],
    )
    .unwrap();
}

async fn session_realign(
    ctx: &Ctx,
    force: bool,
    master: &MasterSpec,
    agents: &[AgentSpec],
) -> Value {
    let request = json!({
        "jsonrpc": "2.0",
        "id": "pr4e",
        "method": "session.realign",
        "params": {
            "session_id": "s4e",
            "force": force,
            "master": master.rpc_value(),
            "agents": agents.iter().map(AgentSpec::rpc_value).collect::<Vec<_>>()
        }
    });
    assert!(
        request.to_string().find("expected_hash").is_none(),
        "PR4e safety contract: RPC input must be raw config only; server recomputes expected hash"
    );
    let response = dispatch(&request.to_string(), ctx).await;
    let value: Value = serde_json::from_str(&response).unwrap();
    assert!(
        value.get("error").is_none(),
        "session.realign should be registered and return a PR4e diff/alignment result, got {value}"
    );
    value["result"].clone()
}

fn command_hooks(command: &str) -> HashMap<String, Vec<HookGroup>> {
    HashMap::from([(
        "PreToolUse".to_string(),
        vec![HookGroup {
            matcher: "*".to_string(),
            hooks: vec![HookItem {
                hook_type: "command".to_string(),
                command: command.to_string(),
                timeout: None,
            }],
        }],
    )])
}

fn no_hooks() -> HashMap<String, Vec<HookGroup>> {
    HashMap::new()
}

fn result_text(value: &Value) -> String {
    value.to_string()
}

#[tokio::test]
async fn no_change_reports_up_to_date() {
    let harness = Harness::new();
    let master = MasterSpec::new(&[], no_hooks());
    let agent = AgentSpec::new("a1", &[], no_hooks());
    {
        let conn = harness.ctx.db.conn();
        set_session_hash(&conn, &master.hash());
        set_agent_hash(&conn, "a1", &agent.hash());
    }

    let result = session_realign(&harness.ctx, false, &master, &[agent]).await;
    let text = result_text(&result);

    assert!(
        text.contains("NO_CHANGE") || text.contains("up to date"),
        "unchanged raw config must report NO_CHANGE/up-to-date after server-side hash recompute, got {text}"
    );
}

#[tokio::test]
async fn plugin_drift_realigns_agent() {
    let harness = Harness::new();
    let master = MasterSpec::new(&[], no_hooks());
    let old_agent = AgentSpec::new("a1", &[], no_hooks());
    let new_agent = AgentSpec::new("a1", &["github@openai-curated"], no_hooks());
    {
        let conn = harness.ctx.db.conn();
        set_session_hash(&conn, &master.hash());
        set_agent_hash(&conn, "a1", &old_agent.hash());
    }

    let result = session_realign(&harness.ctx, false, &master, &[new_agent]).await;
    let text = result_text(&result);

    assert!(text.contains("DRIFT"), "plugin change must report DRIFT: {text}");
    assert!(
        text.contains("plugins changed"),
        "plugin change must explain the field-level drift: {text}"
    );
    assert!(
        text.contains("drift_realigned") || text.contains("REALIGNED"),
        "non-BUSY plugin drift should trigger agent.realign and commit a new hash: {text}"
    );
}

#[tokio::test]
async fn hook_drift_realigns_agent() {
    let harness = Harness::new();
    let master = MasterSpec::new(&[], no_hooks());
    let old_agent = AgentSpec::new("a1", &[], no_hooks());
    let new_agent = AgentSpec::new("a1", &[], command_hooks("echo checked"));
    {
        let conn = harness.ctx.db.conn();
        set_session_hash(&conn, &master.hash());
        set_agent_hash(&conn, "a1", &old_agent.hash());
    }

    let result = session_realign(&harness.ctx, false, &master, &[new_agent]).await;
    let text = result_text(&result);

    assert!(text.contains("DRIFT"), "hook change must report DRIFT: {text}");
    assert!(
        text.contains("hooks changed"),
        "hook change must explain the field-level drift: {text}"
    );
    assert!(
        text.contains("drift_realigned") || text.contains("REALIGNED"),
        "non-BUSY hook drift should trigger agent.realign and commit a new hash: {text}"
    );
}

#[tokio::test]
async fn orphan_agent_is_reported() {
    let harness = Harness::new();
    let master = MasterSpec::new(&[], no_hooks());
    let agent = AgentSpec::new("a1", &[], no_hooks());
    let orphan = AgentSpec::new("a2", &[], no_hooks());
    {
        let conn = harness.ctx.db.conn();
        set_session_hash(&conn, &master.hash());
        set_agent_hash(&conn, "a1", &agent.hash());
        set_agent_hash(&conn, "a2", &orphan.hash());
    }

    let result = session_realign(&harness.ctx, false, &master, &[agent]).await;
    let text = result_text(&result);

    assert!(
        text.contains("ORPHAN"),
        "deleted config agent must be reported as ORPHAN: {text}"
    );
    assert!(
        text.contains("a2"),
        "ORPHAN report must identify the DB-only agent id: {text}"
    );
    let state: String = harness
        .ctx
        .db
        .conn()
        .query_row("SELECT state FROM agents WHERE id = 'a2'", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_ne!(state, "KILLED", "ORPHAN must be audit-only without --force");
}

#[tokio::test]
async fn busy_agent_skip_and_force_realign() {
    let harness = Harness::new();
    let master = MasterSpec::new(&[], no_hooks());
    let old_agent = AgentSpec::new("a1", &[], no_hooks());
    let new_agent = AgentSpec::new("a1", &["github@openai-curated"], no_hooks());
    {
        let conn = harness.ctx.db.conn();
        set_session_hash(&conn, &master.hash());
        set_agent_hash(&conn, "a1", &old_agent.hash());
        set_agent_state(&conn, "a1", "BUSY");
    }

    let skipped = session_realign(&harness.ctx, false, &master, std::slice::from_ref(&new_agent))
        .await;
    let skipped_text = result_text(&skipped);
    assert!(
        skipped_text.contains("SKIPPED_BUSY"),
        "BUSY drift must be skipped without --force: {skipped_text}"
    );

    let forced = session_realign(&harness.ctx, true, &master, &[new_agent]).await;
    let forced_text = result_text(&forced);
    assert!(
        forced_text.contains("drift_realigned") || forced_text.contains("REALIGNED"),
        "--force must allow BUSY agent kill + full rebuild: {forced_text}"
    );
}

#[tokio::test]
async fn master_drift_audit_only_by_default() {
    let harness = Harness::new();
    let old_master = MasterSpec::new(&[], no_hooks());
    let new_master = MasterSpec::new(&["github@openai-curated"], no_hooks());
    let agent = AgentSpec::new("a1", &[], no_hooks());
    {
        let conn = harness.ctx.db.conn();
        set_session_hash(&conn, &old_master.hash());
        set_agent_hash(&conn, "a1", &agent.hash());
    }

    let result = session_realign(&harness.ctx, false, &new_master, &[agent]).await;
    let text = result_text(&result);

    assert!(
        text.contains("DRIFT"),
        "master hash mismatch must report DRIFT: {text}"
    );
    assert!(
        text.contains("master") || text.contains("session"),
        "master drift report must identify the master/session role: {text}"
    );
    assert!(
        !text.contains("master_realign") && !text.contains("REALIGNED"),
        "master drift must be audit-only by default, got {text}"
    );
}

#[tokio::test]
async fn master_drift_force_triggers_realign() {
    let harness = Harness::new();
    let old_master = MasterSpec::new(&[], no_hooks());
    let new_master = MasterSpec::new(&["github@openai-curated"], no_hooks());
    let agent = AgentSpec::new("a1", &[], no_hooks());
    {
        let conn = harness.ctx.db.conn();
        set_session_hash(&conn, &old_master.hash());
        set_agent_hash(&conn, "a1", &agent.hash());
    }

    let result = session_realign(&harness.ctx, true, &new_master, &[agent]).await;
    let text = result_text(&result);

    assert!(
        text.contains("master_realign") || text.contains("REALIGNED"),
        "--force must trigger full master pane realign when master hash drifts: {text}"
    );
}

#[tokio::test]
async fn new_agent_is_spawned() {
    let harness = Harness::new();
    let master = MasterSpec::new(&[], no_hooks());
    let existing = AgentSpec::new("a1", &[], no_hooks());
    let new_agent = AgentSpec::new("a_new", &["github@openai-curated"], no_hooks());
    {
        let conn = harness.ctx.db.conn();
        set_session_hash(&conn, &master.hash());
        set_agent_hash(&conn, "a1", &existing.hash());
    }

    let result = session_realign(&harness.ctx, false, &master, &[existing, new_agent]).await;
    let text = result_text(&result);

    assert!(
        text.contains("NEW"),
        "config-only agent block must be classified as NEW: {text}"
    );
    assert!(
        text.contains("a_new"),
        "NEW report must identify the config-only agent id: {text}"
    );
    assert!(
        text.contains("spawned") || text.contains("SPAWNED"),
        "NEW agent should spawn through agent.spawn instead of realign/kill: {text}"
    );
}
