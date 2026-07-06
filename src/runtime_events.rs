use crate::db::common::{map_db_error, spawn_db};
use crate::error::CcbdError;
use crate::rpc::Ctx;
use crate::tmux::{TmuxPaneId, agent_session_name, master_session_name};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeSnapshotReason {
    Initial,
    InventoryChanged,
    TmuxChanged,
    AgentChanged,
    Shutdown,
    DaemonAbsent,
    DaemonLost,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeState {
    Active,
    Inactive,
    Starting,
    Degraded,
}

const STARTING_WINDOW_SECS: i64 = 120;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeSnapshotRequest {
    pub reason: RuntimeSnapshotReason,
    pub config_path: Option<String>,
    pub workspace_path: Option<String>,
    pub sequence: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeInactiveInput {
    pub reason: RuntimeSnapshotReason,
    pub config_path: Option<String>,
    pub workspace_path: Option<String>,
    pub state_dir: Option<String>,
    pub sequence: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeSnapshot {
    pub schema_version: u16,
    pub event: String,
    pub sequence: u64,
    pub reason: RuntimeSnapshotReason,
    pub runtime_state: RuntimeState,
    pub config_path: Option<String>,
    pub workspace_path: Option<String>,
    pub state_dir: Option<String>,
    pub tmux_socket: Option<String>,
    pub ahd_alive: bool,
    pub active: bool,
    pub ahd_has_inventory: bool,
    pub tmux_server_alive: bool,
    pub master_tmux_alive: bool,
    pub worker_tmux_alive: bool,
    pub worker_tmux_expected_count: usize,
    pub sessions: Vec<RuntimeSessionSnapshot>,
    pub agents: Vec<RuntimeAgentSnapshot>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeSessionSnapshot {
    pub session_id: String,
    pub project_id: String,
    pub path: String,
    pub status: String,
    pub master_state: String,
    pub master_tmux_session: String,
    pub master_tmux_alive: bool,
    pub master_pane_id: Option<String>,
    pub master_pid: Option<i64>,
    pub active_agents: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeAgentSnapshot {
    pub agent_id: String,
    pub session_id: String,
    pub provider: String,
    pub state: String,
    pub sub_state: Option<String>,
    pub pid: Option<i64>,
    pub tmux_session: String,
    pub tmux_alive: bool,
}

#[derive(Debug, Clone)]
struct InventorySession {
    id: String,
    project_id: String,
    absolute_path: String,
    status: String,
    master_state: String,
    master_pane_id: Option<String>,
    master_pid: i64,
    active_agents: i64,
    created_at: i64,
}

#[derive(Debug, Clone)]
struct InventoryAgent {
    id: String,
    session_id: String,
    provider: String,
    state: String,
    sub_state: Option<String>,
    pid: Option<i64>,
    created_at: i64,
}

pub fn inactive_runtime_snapshot(input: RuntimeInactiveInput) -> RuntimeSnapshot {
    RuntimeSnapshot {
        schema_version: 1,
        event: "snapshot".to_string(),
        sequence: input.sequence,
        reason: input.reason,
        runtime_state: RuntimeState::Inactive,
        config_path: input.config_path,
        workspace_path: input.workspace_path,
        state_dir: input.state_dir,
        tmux_socket: None,
        ahd_alive: false,
        active: false,
        ahd_has_inventory: false,
        tmux_server_alive: false,
        master_tmux_alive: false,
        worker_tmux_alive: false,
        worker_tmux_expected_count: 0,
        sessions: Vec::new(),
        agents: Vec::new(),
    }
}

pub async fn build_runtime_snapshot(
    ctx: &Ctx,
    request: RuntimeSnapshotRequest,
) -> Result<RuntimeSnapshot, CcbdError> {
    let workspace_path = request.workspace_path.clone();
    let (sessions, agents) =
        query_runtime_inventory(ctx.db.clone(), workspace_path.clone()).await?;
    let ahd_has_inventory = sessions.iter().any(|session| session.status == "ACTIVE");
    let tmux_server_alive = if ahd_has_inventory {
        ctx.tmux_server.server_running().await.unwrap_or(false)
    } else {
        false
    };

    let mut session_snapshots = Vec::with_capacity(sessions.len());
    let mut all_active_masters_alive = true;
    for session in &sessions {
        let master_tmux_session = master_session_name(&session.project_id);
        let master_tmux_alive = if session.status == "ACTIVE" {
            master_runtime_alive(ctx, session, &master_tmux_session).await
        } else {
            false
        };
        if session.status == "ACTIVE" {
            all_active_masters_alive &= master_tmux_alive;
        }
        session_snapshots.push(RuntimeSessionSnapshot {
            session_id: session.id.clone(),
            project_id: session.project_id.clone(),
            path: session.absolute_path.clone(),
            status: session.status.clone(),
            master_state: session.master_state.clone(),
            master_tmux_session,
            master_tmux_alive,
            master_pane_id: session.master_pane_id.clone(),
            master_pid: positive_pid(session.master_pid),
            active_agents: session.active_agents,
        });
    }

    let active_agents = agents
        .iter()
        .filter(|agent| !is_terminal_agent_state(&agent.state))
        .collect::<Vec<_>>();
    let worker_tmux_expected_count = active_agents.len();
    let mut all_active_workers_alive = true;
    let mut agent_snapshots = Vec::with_capacity(agents.len());
    for agent in &agents {
        let tmux_session = agent_session_name(&agent.id);
        let tmux_alive = if is_terminal_agent_state(&agent.state) {
            false
        } else {
            ctx.tmux_server
                .session_exists(tmux_session.clone())
                .await
                .unwrap_or(false)
        };
        if !is_terminal_agent_state(&agent.state) {
            all_active_workers_alive &= tmux_alive;
        }
        agent_snapshots.push(RuntimeAgentSnapshot {
            agent_id: agent.id.clone(),
            session_id: agent.session_id.clone(),
            provider: agent.provider.clone(),
            state: agent.state.clone(),
            sub_state: agent.sub_state.clone(),
            pid: agent.pid,
            tmux_session,
            tmux_alive,
        });
    }

    let master_tmux_alive = ahd_has_inventory && all_active_masters_alive;
    let worker_tmux_alive = all_active_workers_alive;
    let active = ahd_has_inventory && master_tmux_alive && worker_tmux_alive;
    let now = now_epoch_seconds();
    let degraded = ahd_has_inventory
        && (sessions
            .iter()
            .zip(session_snapshots.iter())
            .any(|(session, snapshot)| session_is_degraded(session, snapshot, now))
            || agents
                .iter()
                .zip(agent_snapshots.iter())
                .any(|(agent, snapshot)| agent_is_degraded(agent, snapshot, now)));
    let runtime_state = if active {
        RuntimeState::Active
    } else if degraded {
        RuntimeState::Degraded
    } else if ahd_has_inventory {
        RuntimeState::Starting
    } else {
        RuntimeState::Inactive
    };

    Ok(RuntimeSnapshot {
        schema_version: 1,
        event: "snapshot".to_string(),
        sequence: request.sequence,
        reason: request.reason,
        runtime_state,
        config_path: request.config_path,
        workspace_path,
        state_dir: Some(ctx.state_dir.display().to_string()),
        tmux_socket: Some(ctx.tmux_server.socket_name().to_string()),
        ahd_alive: true,
        active,
        ahd_has_inventory,
        tmux_server_alive,
        master_tmux_alive,
        worker_tmux_alive,
        worker_tmux_expected_count,
        sessions: session_snapshots,
        agents: agent_snapshots,
    })
}

pub fn runtime_snapshot_fingerprint(snapshot: &RuntimeSnapshot) -> Result<String, CcbdError> {
    let mut value = serde_json::to_value(snapshot).map_err(|err| {
        CcbdError::IpcInvalidRequest(format!("serialize runtime snapshot: {err}"))
    })?;
    if let Value::Object(map) = &mut value {
        map.remove("sequence");
        map.remove("reason");
    }
    serde_json::to_string(&value)
        .map_err(|err| CcbdError::IpcInvalidRequest(format!("fingerprint runtime snapshot: {err}")))
}

async fn master_runtime_alive(
    ctx: &Ctx,
    session: &InventorySession,
    master_tmux_session: &str,
) -> bool {
    if !ctx
        .tmux_server
        .session_exists(master_tmux_session.to_string())
        .await
        .unwrap_or(false)
    {
        return false;
    }
    let Some(pane_id) = session.master_pane_id.as_deref() else {
        return false;
    };
    let Ok(pane_id) = TmuxPaneId::parse(pane_id) else {
        return false;
    };
    match ctx.tmux_server.get_pane_pid(pane_id).await {
        Ok(pid) => session.master_pid <= 0 || i64::from(pid) == session.master_pid,
        Err(_) => false,
    }
}

async fn query_runtime_inventory(
    db: crate::db::Db,
    workspace_path: Option<String>,
) -> Result<(Vec<InventorySession>, Vec<InventoryAgent>), CcbdError> {
    spawn_db("runtime_events::query_inventory", move || {
        let conn = db.conn();
        query_runtime_inventory_sync(&conn, workspace_path.as_deref())
    })
    .await
}

fn query_runtime_inventory_sync(
    conn: &Connection,
    workspace_path: Option<&str>,
) -> Result<(Vec<InventorySession>, Vec<InventoryAgent>), CcbdError> {
    let sessions = {
        let mut stmt = conn
            .prepare(
                "SELECT sessions.id, sessions.project_id, projects.absolute_path, sessions.status,
                        sessions.master_state, sessions.master_pane_id, sessions.master_pid,
                        COALESCE(SUM(CASE WHEN agents.state NOT IN ('CRASHED', 'KILLED') THEN 1 ELSE 0 END), 0) AS active_agents,
                        sessions.created_at
                 FROM sessions
                 JOIN projects ON projects.id = sessions.project_id
                 LEFT JOIN agents ON agents.session_id = sessions.id
                 WHERE (?1 IS NULL OR projects.absolute_path = ?1)
                 GROUP BY sessions.id, sessions.project_id, projects.absolute_path, sessions.status,
                          sessions.master_state, sessions.master_pane_id, sessions.master_pid, sessions.created_at
                 ORDER BY sessions.created_at ASC, sessions.id ASC",
            )
            .map_err(|err| map_db_error("prepare runtime sessions query", err))?;
        let rows = stmt
            .query_map(params![workspace_path], |row| {
                Ok(InventorySession {
                    id: row.get(0)?,
                    project_id: row.get(1)?,
                    absolute_path: row.get(2)?,
                    status: row.get(3)?,
                    master_state: row.get(4)?,
                    master_pane_id: row.get(5)?,
                    master_pid: row.get(6)?,
                    active_agents: row.get(7)?,
                    created_at: row.get(8)?,
                })
            })
            .map_err(|err| map_db_error("query runtime sessions", err))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|err| map_db_error("collect runtime sessions", err))?
    };
    let agents = {
        let mut stmt = conn
            .prepare(
                "SELECT agents.id, agents.session_id, agents.provider, agents.state,
                        agents.sub_state, agents.pid, agents.created_at
                 FROM agents
                 JOIN sessions ON sessions.id = agents.session_id
                 JOIN projects ON projects.id = sessions.project_id
                 WHERE (?1 IS NULL OR projects.absolute_path = ?1)
                 ORDER BY agents.created_at ASC, agents.id ASC",
            )
            .map_err(|err| map_db_error("prepare runtime agents query", err))?;
        let rows = stmt
            .query_map(params![workspace_path], |row| {
                Ok(InventoryAgent {
                    id: row.get(0)?,
                    session_id: row.get(1)?,
                    provider: row.get(2)?,
                    state: row.get(3)?,
                    sub_state: row.get(4)?,
                    pid: row.get(5)?,
                    created_at: row.get(6)?,
                })
            })
            .map_err(|err| map_db_error("query runtime agents", err))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|err| map_db_error("collect runtime agents", err))?
    };
    Ok((sessions, agents))
}

fn is_terminal_agent_state(state: &str) -> bool {
    matches!(state, "CRASHED" | "KILLED")
}

fn session_is_degraded(
    session: &InventorySession,
    snapshot: &RuntimeSessionSnapshot,
    now: i64,
) -> bool {
    if session.status != "ACTIVE" || snapshot.master_tmux_alive {
        return false;
    }
    if session.master_pane_id.is_some() {
        return true;
    }
    !within_starting_window(session.created_at, now)
}

fn agent_is_degraded(
    agent: &InventoryAgent,
    snapshot: &RuntimeAgentSnapshot,
    now: i64,
) -> bool {
    if is_terminal_agent_state(&agent.state) || snapshot.tmux_alive {
        return false;
    }
    if agent.state != "SPAWNING" {
        return true;
    }
    !within_starting_window(agent.created_at, now)
}

fn within_starting_window(created_at: i64, now: i64) -> bool {
    now.saturating_sub(created_at) < STARTING_WINDOW_SECS
}

fn now_epoch_seconds() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

fn positive_pid(pid: i64) -> Option<i64> {
    (pid > 0).then_some(pid)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use crate::db::agents::insert_agent_sync;
    use crate::db::sessions::insert_session_sync;
    use crate::rpc::Ctx;
    use crate::sandbox::EnvState;
    use crate::tmux::TmuxServer;
    use serde_json::json;
    use std::sync::Arc;

    fn test_ctx() -> Ctx {
        let file = tempfile::NamedTempFile::new().unwrap();
        let state_dir = tempfile::TempDir::new().unwrap().keep();
        Ctx {
            db: db::init(file.path()).unwrap(),
            state_dir: state_dir.clone(),
            env_state: EnvState {
                systemd_run_available: false,
                unsafe_no_sandbox: true,
                under_systemd: false,
            },
            daemon_unit: None,
            tmux_server: Arc::new(TmuxServer::new(&state_dir)),
        }
    }

    #[tokio::test]
    async fn empty_inventory_snapshot_is_inactive() {
        let ctx = test_ctx();

        let snapshot = build_runtime_snapshot(
            &ctx,
            RuntimeSnapshotRequest {
                reason: RuntimeSnapshotReason::Initial,
                config_path: Some("/tmp/project/ah.toml".into()),
                workspace_path: Some("/tmp/project".into()),
                sequence: 1,
            },
        )
        .await
        .unwrap();

        assert_eq!(snapshot.schema_version, 1);
        assert_eq!(snapshot.event, "snapshot");
        assert_eq!(snapshot.reason, RuntimeSnapshotReason::Initial);
        assert!(!snapshot.active);
        assert!(!snapshot.ahd_has_inventory);
        assert_eq!(snapshot.runtime_state, RuntimeState::Inactive);
        assert!(snapshot.sessions.is_empty());
        assert!(snapshot.agents.is_empty());
    }

    #[tokio::test]
    async fn missing_tmux_for_active_inventory_is_degraded_not_active() {
        let ctx = test_ctx();
        {
            let conn = ctx.db.conn();
            insert_session_sync(&conn, "sess_1", "proj", "/tmp/project").unwrap();
            conn.execute(
                "UPDATE sessions SET master_pid = 123, master_pane_id = '%404' WHERE id = 'sess_1'",
                [],
            )
            .unwrap();
            insert_agent_sync(&conn, "a1", "sess_1", "codex", "IDLE", Some(456)).unwrap();
        }

        let snapshot = build_runtime_snapshot(
            &ctx,
            RuntimeSnapshotRequest {
                reason: RuntimeSnapshotReason::Initial,
                config_path: Some("/tmp/project/ah.toml".into()),
                workspace_path: Some("/tmp/project".into()),
                sequence: 1,
            },
        )
        .await
        .unwrap();

        assert!(snapshot.ahd_has_inventory);
        assert!(!snapshot.active);
        assert_eq!(snapshot.runtime_state, RuntimeState::Degraded);
        assert!(!snapshot.master_tmux_alive);
        assert!(!snapshot.worker_tmux_alive);
        assert_eq!(snapshot.worker_tmux_expected_count, 1);
        assert_eq!(snapshot.sessions[0].master_tmux_session, "master_proj");
        assert_eq!(snapshot.agents[0].tmux_session, "agent_a1");
    }

    #[tokio::test]
    async fn starting_snapshot_is_not_degraded() {
        let ctx = test_ctx();
        {
            let conn = ctx.db.conn();
            insert_session_sync(&conn, "sess_starting", "proj", "/tmp/project").unwrap();
            insert_agent_sync(
                &conn,
                "a_starting",
                "sess_starting",
                "codex",
                "SPAWNING",
                Some(456),
            )
            .unwrap();
        }

        let snapshot = build_runtime_snapshot(
            &ctx,
            RuntimeSnapshotRequest {
                reason: RuntimeSnapshotReason::Initial,
                config_path: Some("/tmp/project/ah.toml".into()),
                workspace_path: Some("/tmp/project".into()),
                sequence: 1,
            },
        )
        .await
        .unwrap();

        assert!(snapshot.ahd_has_inventory);
        assert!(!snapshot.active);
        assert_eq!(snapshot.runtime_state, RuntimeState::Starting);
        assert_ne!(snapshot.runtime_state, RuntimeState::Degraded);
        assert_eq!(
            serde_json::to_value(snapshot.runtime_state).unwrap(),
            serde_json::json!("starting")
        );
    }

    #[tokio::test]
    async fn in_session_dispatch_spawning_worker_is_starting_not_degraded() {
        let ctx = test_ctx();
        let (master_pane, master_pid) = {
            let session_name = master_session_name("proj");
            ctx.tmux_server
                .ensure_session(session_name.clone(), ctx.state_dir.clone())
                .await
                .unwrap();
            let pane = ctx
                .tmux_server
                .spawn_window(
                    session_name,
                    "master".to_string(),
                    ctx.state_dir.clone(),
                    vec!["sh".into(), "-lc".into(), "sleep 30".into()],
                )
                .await
                .unwrap();
            let pid = ctx.tmux_server.get_pane_pid(pane.clone()).await.unwrap();
            (pane, i64::from(pid))
        };
        {
            let conn = ctx.db.conn();
            insert_session_sync(&conn, "sess_old", "proj", "/tmp/project").unwrap();
            conn.execute(
                "UPDATE sessions
                    SET created_at = unixepoch() - 10000,
                        master_pid = ?1,
                        master_pane_id = ?2
                  WHERE id = 'sess_old'",
                params![master_pid, master_pane.0],
            )
            .unwrap();
            insert_agent_sync(&conn, "a_new", "sess_old", "codex", "SPAWNING", Some(456))
                .unwrap();
        }

        let snapshot = build_runtime_snapshot(
            &ctx,
            RuntimeSnapshotRequest {
                reason: RuntimeSnapshotReason::Initial,
                config_path: Some("/tmp/project/ah.toml".into()),
                workspace_path: Some("/tmp/project".into()),
                sequence: 1,
            },
        )
        .await
        .unwrap();

        assert!(snapshot.ahd_has_inventory);
        assert!(!snapshot.active);
        assert_eq!(snapshot.runtime_state, RuntimeState::Starting);
        assert_ne!(snapshot.runtime_state, RuntimeState::Degraded);
    }

    #[tokio::test]
    async fn cold_start_no_agents_is_starting() {
        let ctx = test_ctx();
        {
            let conn = ctx.db.conn();
            insert_session_sync(&conn, "sess_no_agents", "proj", "/tmp/project").unwrap();
        }

        let snapshot = build_runtime_snapshot(
            &ctx,
            RuntimeSnapshotRequest {
                reason: RuntimeSnapshotReason::Initial,
                config_path: Some("/tmp/project/ah.toml".into()),
                workspace_path: Some("/tmp/project".into()),
                sequence: 1,
            },
        )
        .await
        .unwrap();

        assert!(snapshot.ahd_has_inventory);
        assert!(!snapshot.active);
        assert_eq!(snapshot.runtime_state, RuntimeState::Starting);
        assert_ne!(snapshot.runtime_state, RuntimeState::Degraded);
        assert_eq!(snapshot.worker_tmux_expected_count, 0);
    }

    #[tokio::test]
    async fn starting_past_timeout_is_degraded() {
        let ctx = test_ctx();
        {
            let conn = ctx.db.conn();
            insert_session_sync(&conn, "sess_stale_starting", "proj", "/tmp/project").unwrap();
            conn.execute(
                "UPDATE sessions SET created_at = unixepoch() - 10000 WHERE id = 'sess_stale_starting'",
                [],
            )
            .unwrap();
            insert_agent_sync(
                &conn,
                "a_stale_starting",
                "sess_stale_starting",
                "codex",
                "SPAWNING",
                Some(456),
            )
            .unwrap();
        }

        let snapshot = build_runtime_snapshot(
            &ctx,
            RuntimeSnapshotRequest {
                reason: RuntimeSnapshotReason::Initial,
                config_path: Some("/tmp/project/ah.toml".into()),
                workspace_path: Some("/tmp/project".into()),
                sequence: 1,
            },
        )
        .await
        .unwrap();

        assert!(snapshot.ahd_has_inventory);
        assert!(!snapshot.active);
        assert_eq!(snapshot.runtime_state, RuntimeState::Degraded);
    }

    #[test]
    fn inactive_client_snapshot_is_jsonl_safe() {
        let snapshot = inactive_runtime_snapshot(RuntimeInactiveInput {
            reason: RuntimeSnapshotReason::DaemonAbsent,
            config_path: Some("/tmp/project/ah.toml".into()),
            workspace_path: Some("/tmp/project".into()),
            state_dir: Some("/tmp/ah-state".into()),
            sequence: 1,
        });

        let line = serde_json::to_string(&snapshot).unwrap();
        let value: serde_json::Value = serde_json::from_str(&line).unwrap();

        assert!(!line.contains('\n'));
        assert_eq!(value["event"], "snapshot");
        assert_eq!(value["reason"], "daemon_absent");
        assert_eq!(value["active"], false);
        assert_eq!(value["runtime_state"], "inactive");
    }

    #[tokio::test]
    async fn runtime_snapshot_rpc_method_is_registered() {
        let ctx = test_ctx();
        let response = crate::rpc::router::dispatch(
            &json!({
                "jsonrpc": "2.0",
                "method": "runtime.snapshot",
                "params": {
                    "config_path": "/tmp/project/ah.toml",
                    "workspace_path": "/tmp/project"
                },
                "id": 99
            })
            .to_string(),
            &ctx,
        )
        .await;
        let obj: serde_json::Value = serde_json::from_str(&response).unwrap();

        assert_eq!(obj["id"], 99);
        assert_eq!(obj["result"]["schema_version"], 1);
        assert_eq!(obj["result"]["event"], "snapshot");
        assert_eq!(obj["result"]["reason"], "initial");
    }
}
