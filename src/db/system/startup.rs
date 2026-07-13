use super::{
    cascade_kill_session_agents_with_runner_sync, remove_agent_sandbox_dir_preserving_home_sync,
    remove_agent_sandbox_dir_sync, sweep_stale_tmux_sockets_sync,
};
use crate::db::Db;
use crate::db::common::{map_db_error, spawn_db};
use crate::db::jobs::mark_dispatched_jobs_failed_for_agent_conn_sync;
use crate::db::master_recovery::expire_master_recovery_window_sync;
use crate::db::state_machine::STATE_PROMPT_PENDING;
use crate::error::CcbdError;
use crate::platform::sys::scope::{RealSystemctlRunner, ScopeUnit, SystemctlRunner};
use crate::tmux::TmuxPaneId;
use rusqlite::{TransactionBehavior, params};
use std::collections::HashSet;
use std::fs::{File, OpenOptions};
use std::io;
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DaemonMarkerProvenance {
    Explicit,
    Ambient,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DaemonMarker {
    value: String,
    provenance: DaemonMarkerProvenance,
}

impl DaemonMarker {
    fn explicit(value: impl Into<String>) -> Self {
        Self {
            value: value.into(),
            provenance: DaemonMarkerProvenance::Explicit,
        }
    }

    fn ambient(value: impl Into<String>) -> Self {
        Self {
            value: value.into(),
            provenance: DaemonMarkerProvenance::Ambient,
        }
    }

    fn may_stop_scopes(&self) -> bool {
        self.provenance == DaemonMarkerProvenance::Explicit
    }
}

#[derive(Debug, Clone)]
struct StartupAgentCandidate {
    id: String,
    session_id: String,
    pid: Option<i64>,
    state: String,
    provider: String,
}

#[derive(Debug, Clone)]
struct StartupCrashCandidate {
    agent: StartupAgentCandidate,
    reason: &'static str,
}

struct StartupAliveIoCandidate {
    agent: StartupAgentCandidate,
    fifo_path: PathBuf,
    fifo_file: File,
    socket_name: String,
}

#[derive(Debug, Clone)]
struct StartupClaudeWorkerSeat {
    id: String,
    session_id: String,
}

#[derive(Debug, Clone, Default)]
struct StartupClaudeSeats {
    workers: Vec<StartupClaudeWorkerSeat>,
    sessions: Vec<String>,
}

pub fn recovery_eligible_orphan_scope_should_be_preserved(
    provider: &str,
    agent_state: &str,
) -> bool {
    agent_state == "CRASHED" && crate::provider::manifest::is_recovery_eligible_provider(provider)
}

pub(crate) fn reconcile_startup_sync(db: &Db) -> Result<usize, CcbdError> {
    reconcile_active_agents_to_crashed_sync(db, None, None)
}

pub(crate) fn reconcile_startup_sync_with_state_dir(
    db: &Db,
    state_dir: Option<&Path>,
) -> Result<usize, CcbdError> {
    reconcile_startup_sync_with_state_dir_and_runner(
        db,
        state_dir,
        &RealSystemctlRunner,
        unixepoch(),
        reconcile_orphan_scopes_dry_run_enabled(),
    )
}

pub(crate) fn reconcile_startup_sync_with_state_dir_and_runner(
    db: &Db,
    state_dir: Option<&Path>,
    runner: &dyn SystemctlRunner,
    now: i64,
    orphan_dry_run: bool,
) -> Result<usize, CcbdError> {
    let socket_name = state_dir.map(crate::tmux::compute_socket_name);
    let agents_count =
        reconcile_active_agents_to_crashed_sync(db, state_dir, socket_name.as_deref())?;
    let daemon_marker = state_dir
        .map(crate::tmux::compute_socket_name)
        .map(DaemonMarker::explicit)
        .unwrap_or_else(|| DaemonMarker::ambient("ccbd-unknown"));
    // Recovery windows are authoritative after ahd restart. Reconcile them before
    // ordinary orphan-scope cleanup so live recovery windows keep their scopes.
    let recovery_count =
        reconcile_master_recovery_windows_with_marker_sync(db, now, &daemon_marker, runner)?;
    let scopes_count =
        reconcile_orphan_scopes_with_marker_sync(db, runner, &daemon_marker, orphan_dry_run)?;
    Ok(agents_count + recovery_count + scopes_count)
}

pub(crate) fn reconcile_orphan_scopes_sync(
    db: &Db,
    daemon_marker: &str,
) -> Result<usize, CcbdError> {
    let dry_run = reconcile_orphan_scopes_dry_run_enabled();
    reconcile_orphan_scopes_with_runner_sync(db, &RealSystemctlRunner, daemon_marker, dry_run)
}

pub(crate) fn reconcile_orphan_scopes_dry_run_enabled() -> bool {
    std::env::var("CCBD_RECONCILE_DRY_RUN").as_deref() == Ok("1")
}

pub(crate) fn reconcile_orphan_scopes_with_runner_sync(
    db: &Db,
    runner: &dyn SystemctlRunner,
    daemon_marker: &str,
    dry_run: bool,
) -> Result<usize, CcbdError> {
    reconcile_orphan_scopes_with_marker_sync(
        db,
        runner,
        &DaemonMarker::explicit(daemon_marker),
        dry_run,
    )
}

fn reconcile_orphan_scopes_with_marker_sync(
    db: &Db,
    runner: &dyn SystemctlRunner,
    daemon_marker: &DaemonMarker,
    dry_run: bool,
) -> Result<usize, CcbdError> {
    if !daemon_marker.may_stop_scopes() {
        tracing::warn!(
            daemon_marker = %daemon_marker.value,
            provenance = ?daemon_marker.provenance,
            "startup reconcile skipped orphan scope stops because daemon marker is not explicit"
        );
        return Ok(0);
    }
    let live_refs = active_session_and_agent_refs_sync(db)?;
    let scopes = match runner.list_scope_units() {
        Ok(scopes) => scopes,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(0),
        Err(err) => {
            tracing::warn!(error = %err, "failed to list systemd scope units during startup reconcile");
            return Ok(0);
        }
    };

    let mut stopped = 0;
    for scope in scopes {
        if !is_own_ccbd_scope(&scope, &daemon_marker.value) || !is_orphan_scope(&scope, &live_refs)
        {
            continue;
        }
        if dry_run {
            tracing::info!(unit = %scope.unit, "dry-run: would stop orphan systemd scope");
            stopped += 1;
            continue;
        }
        match runner.stop_unit(&scope.unit) {
            Ok(()) => stopped += 1,
            Err(err) if err.kind() == io::ErrorKind::NotFound => {}
            Err(err) => {
                tracing::warn!(unit = %scope.unit, error = %err, "failed to stop orphan systemd scope")
            }
        }
    }
    Ok(stopped)
}

#[derive(Debug, Clone)]
struct StartupMasterRecoveryWindow {
    session_id: String,
    phase: String,
    defer_until: i64,
}

pub(crate) fn reconcile_master_recovery_windows_sync(
    db: &Db,
    now: i64,
) -> Result<usize, CcbdError> {
    reconcile_master_recovery_windows_with_marker_sync(
        db,
        now,
        &DaemonMarker::ambient("ccbd-unknown"),
        &RealSystemctlRunner,
    )
}

pub(crate) fn reconcile_master_recovery_windows_with_runner_sync(
    db: &Db,
    now: i64,
    daemon_marker: &str,
    runner: &dyn SystemctlRunner,
) -> Result<usize, CcbdError> {
    reconcile_master_recovery_windows_with_marker_sync(
        db,
        now,
        &DaemonMarker::explicit(daemon_marker),
        runner,
    )
}

fn reconcile_master_recovery_windows_with_marker_sync(
    db: &Db,
    now: i64,
    daemon_marker: &DaemonMarker,
    runner: &dyn SystemctlRunner,
) -> Result<usize, CcbdError> {
    let windows = {
        let conn = db.conn();
        let mut stmt = conn
            .prepare(
                "SELECT master_recovery_windows.session_id,
                        master_recovery_windows.phase,
                        master_recovery_windows.defer_until
                 FROM master_recovery_windows
                 ORDER BY master_recovery_windows.created_at ASC,
                          master_recovery_windows.session_id ASC",
            )
            .map_err(|err| map_db_error("prepare master recovery startup reconcile", err))?;
        let rows = stmt
            .query_map([], |row| {
                Ok(StartupMasterRecoveryWindow {
                    session_id: row.get(0)?,
                    phase: row.get(1)?,
                    defer_until: row.get(2)?,
                })
            })
            .map_err(|err| map_db_error("query master recovery startup reconcile", err))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|err| map_db_error("collect master recovery startup reconcile", err))?
    };

    let mut reconciled_count = 0;
    for window in windows {
        if matches!(window.phase.as_str(), "COMPLETED" | "FAILED" | "FUSED") {
            continue;
        }

        if now <= window.defer_until {
            tracing::info!(
                session_id = %window.session_id,
                phase = %window.phase,
                defer_until = window.defer_until,
                now,
                "master recovery window still active during startup reconcile"
            );
            continue;
        }

        let expired = {
            let conn = db.conn();
            expire_master_recovery_window_sync(&conn, &window.session_id, now)?
        };
        if expired {
            reconciled_count += 1;
            tracing::warn!(
                session_id = %window.session_id,
                phase = %window.phase,
                defer_until = window.defer_until,
                now,
                "master recovery window expired during startup reconcile; cascading session"
            );
            cascade_kill_session_agents_with_runner_sync(
                db,
                &window.session_id,
                "MASTER_REVIVE_WINDOW_EXPIRED",
                daemon_marker
                    .may_stop_scopes()
                    .then_some(daemon_marker.value.as_str()),
                runner,
            )?;
        }
    }
    Ok(reconciled_count)
}

fn unixepoch() -> i64 {
    match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(duration) => duration.as_secs() as i64,
        Err(_) => 0,
    }
}

fn active_session_and_agent_refs_sync(db: &Db) -> Result<HashSet<String>, CcbdError> {
    let conn = db.conn();
    let mut refs = HashSet::new();
    {
        let mut stmt = conn
            .prepare(
                // Recovery-eligible providers mirror provider::manifest::is_recovery_eligible_provider;
                // keep this SQL in sync if that single source of truth changes.
                "SELECT DISTINCT sessions.id \
                 FROM sessions \
                 JOIN agents ON agents.session_id = sessions.id \
                 WHERE sessions.status = 'ACTIVE' \
                   AND (agents.state NOT IN ('CRASHED', 'KILLED') \
                        OR (agents.state = 'CRASHED' AND agents.provider IN ('codex', 'claude', 'antigravity')))",
            )
            .map_err(|err| map_db_error("prepare active session refs", err))?;
        let rows = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|err| map_db_error("query active session refs", err))?;
        for session_id in rows {
            refs.insert(session_id.map_err(|err| map_db_error("collect active session ref", err))?);
        }
    }
    {
        let mut stmt = conn
            .prepare(
                // Recovery-eligible providers mirror provider::manifest::is_recovery_eligible_provider;
                // keep this SQL in sync if that single source of truth changes.
                "SELECT agents.id \
                 FROM agents \
                 JOIN sessions ON sessions.id = agents.session_id \
                 WHERE sessions.status = 'ACTIVE' \
                   AND (agents.state NOT IN ('CRASHED', 'KILLED') \
                        OR (agents.state = 'CRASHED' AND agents.provider IN ('codex', 'claude', 'antigravity'))) \
                 ORDER BY agents.id ASC",
            )
            .map_err(|err| map_db_error("prepare active agent refs", err))?;
        let rows = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|err| map_db_error("query active agent refs", err))?;
        for agent_id in rows {
            refs.insert(agent_id.map_err(|err| map_db_error("collect active agent ref", err))?);
        }
    }
    Ok(refs)
}

fn is_own_ccbd_scope(scope: &ScopeUnit, daemon_marker: &str) -> bool {
    crate::platform::sys::scope::is_own_ccbd_scope(scope, daemon_marker)
}

fn is_orphan_scope(scope: &ScopeUnit, live_refs: &HashSet<String>) -> bool {
    crate::platform::sys::scope::is_orphan_scope(scope, live_refs)
}

pub(crate) fn reconcile_active_agents_to_crashed_sync(
    db: &Db,
    state_dir: Option<&Path>,
    socket_name: Option<&str>,
) -> Result<usize, CcbdError> {
    startup_reconcile_phase_prompt_pending_preserve(db)?;
    let candidates = startup_reconcile_phase_a_select_candidates(db)?;
    let (dead, alive) = startup_reconcile_phase_b_probe_pids(candidates);
    let (reattach_dead, reattachable_alive) =
        startup_reconcile_phase_b2_prepare_alive_io(state_dir, socket_name, alive);
    let dead = dead.into_iter().chain(reattach_dead).collect::<Vec<_>>();
    let changed = startup_reconcile_phase_c_crash_dead(db, &dead)?;
    if let Some(state_dir) = state_dir {
        for candidate in &dead {
            if crate::provider::manifest::is_recovery_eligible_provider(&candidate.agent.provider) {
                remove_agent_sandbox_dir_preserving_home_sync(
                    state_dir,
                    &candidate.agent.session_id,
                    &candidate.agent.id,
                );
            } else {
                remove_agent_sandbox_dir_sync(
                    state_dir,
                    &candidate.agent.session_id,
                    &candidate.agent.id,
                );
            }
        }
    }
    startup_reconcile_phase_d_reregister_alive(db, reattachable_alive)?;
    Ok(changed)
}

fn startup_reconcile_phase_prompt_pending_preserve(db: &Db) -> Result<(), CcbdError> {
    let conn = db.conn();
    let mut stmt = conn
        .prepare("SELECT id FROM agents WHERE state = ? ORDER BY id ASC")
        .map_err(|err| map_db_error("prepare prompt pending reconcile query", err))?;
    let rows = stmt
        .query_map([STATE_PROMPT_PENDING], |row| row.get::<_, String>(0))
        .map_err(|err| map_db_error("query prompt pending reconcile agents", err))?;
    let agent_ids = rows
        .collect::<Result<Vec<_>, _>>()
        .map_err(|err| map_db_error("collect prompt pending reconcile agents", err))?;
    for agent_id in agent_ids {
        tracing::info!(
            agent_id = %agent_id,
            state = STATE_PROMPT_PENDING,
            reason = "startup reconcile preserves pending prompt",
            impact = "agent is not restarted, reset, dispatched, or crashed until resolve_prompt",
            "startup reconcile preserved prompt pending agent"
        );
    }
    Ok(())
}

fn startup_reconcile_phase_a_select_candidates(
    db: &Db,
) -> Result<Vec<StartupAgentCandidate>, CcbdError> {
    let mut conn = db.conn();
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|err| map_db_error("begin startup reconcile candidate scan", err))?;
    let candidates = {
        let mut stmt = tx
            .prepare(
                "SELECT id, session_id, pid, state, provider FROM agents WHERE state IN ('SPAWNING', 'WAITING_FOR_ACK', 'BUSY', 'IDLE')",
            )
            .map_err(|err| map_db_error("prepare active agents query", err))?;
        let rows = stmt
            .query_map([], |row| {
                Ok(StartupAgentCandidate {
                    id: row.get(0)?,
                    session_id: row.get(1)?,
                    pid: row.get(2)?,
                    state: row.get(3)?,
                    provider: row.get(4)?,
                })
            })
            .map_err(|err| map_db_error("query active agents", err))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|err| map_db_error("collect active agents", err))?
    };
    tx.commit()
        .map_err(|err| map_db_error("commit startup reconcile candidate scan", err))?;
    Ok(candidates)
}

fn startup_reconcile_phase_b_probe_pids(
    candidates: Vec<StartupAgentCandidate>,
) -> (Vec<StartupCrashCandidate>, Vec<StartupAgentCandidate>) {
    let mut dead = Vec::new();
    let mut alive = Vec::new();
    for candidate in candidates {
        let Some(pid) = candidate.pid else {
            dead.push(StartupCrashCandidate {
                agent: candidate,
                reason: "STARTUP_RECONCILE_DEAD_PID",
            });
            continue;
        };
        match probe_pid_alive(pid as i32) {
            Ok(true) => alive.push(candidate),
            Ok(false) => dead.push(StartupCrashCandidate {
                agent: candidate,
                reason: "STARTUP_RECONCILE_DEAD_PID",
            }),
            Err(err) => {
                tracing::warn!(agent_id = %candidate.id, pid, error = %err, "startup reconcile pid probe failed");
                dead.push(StartupCrashCandidate {
                    agent: candidate,
                    reason: "STARTUP_RECONCILE_DEAD_PID",
                });
            }
        }
    }
    (dead, alive)
}

fn startup_reconcile_phase_b2_prepare_alive_io(
    state_dir: Option<&Path>,
    socket_name: Option<&str>,
    alive: Vec<StartupAgentCandidate>,
) -> (Vec<StartupCrashCandidate>, Vec<StartupAliveIoCandidate>) {
    let Some(state_dir) = state_dir else {
        return (Vec::new(), Vec::new());
    };
    let socket_name = socket_name
        .map(ToString::to_string)
        .unwrap_or_else(|| crate::tmux::compute_socket_name(state_dir));

    let mut dead = Vec::new();
    let mut reattachable = Vec::new();
    for agent in alive {
        let fifo_path = state_dir.join("pipes").join(format!("{}.fifo", agent.id));
        if !fifo_path.exists() {
            tracing::warn!(agent_id = %agent.id, ?fifo_path, "startup reconcile cannot reattach missing fifo");
            dead.push(StartupCrashCandidate {
                agent,
                reason: "STARTUP_RECONCILE_FIFO_REATTACH_FAILED",
            });
            continue;
        }
        match open_fifo_for_reattach(&fifo_path) {
            Ok(fifo_file) => reattachable.push(StartupAliveIoCandidate {
                agent,
                fifo_path,
                fifo_file,
                socket_name: socket_name.clone(),
            }),
            Err(err) => {
                tracing::warn!(agent_id = %agent.id, ?fifo_path, error = %err, "startup reconcile cannot open fifo");
                dead.push(StartupCrashCandidate {
                    agent,
                    reason: "STARTUP_RECONCILE_FIFO_REATTACH_FAILED",
                });
            }
        }
    }
    (dead, reattachable)
}

#[cfg(unix)]
fn open_fifo_for_reattach(fifo_path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(fifo_path)
}

#[cfg(windows)]
fn open_fifo_for_reattach(_fifo_path: &Path) -> io::Result<File> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "Windows startup FIFO reattach is not implemented until M2 stream reattach",
    ))
}

fn probe_pid_alive(pid: i32) -> Result<bool, std::io::Error> {
    match crate::platform::sys::proc_info::kill_zero_check(pid) {
        crate::platform::sys::proc_info::ProcessLiveness::Alive => Ok(true),
        crate::platform::sys::proc_info::ProcessLiveness::Dead => Ok(false),
        crate::platform::sys::proc_info::ProcessLiveness::Unknown => Err(io::Error::other(
            format!("process liveness is unknown for pid {pid}"),
        )),
    }
}

fn startup_reconcile_phase_c_crash_dead(
    db: &Db,
    dead: &[StartupCrashCandidate],
) -> Result<usize, CcbdError> {
    if dead.is_empty() {
        return Ok(0);
    }
    let mut conn = db.conn();
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|err| map_db_error("begin startup reconcile crash dead pids", err))?;
    let mut changed = 0;
    for crash in dead {
        let candidate = &crash.agent;
        let changes = tx
            .execute(
                "UPDATE agents SET state = 'CRASHED', error_code = ?, state_version = state_version + 1, updated_at = unixepoch() WHERE id = ? AND state IN ('SPAWNING', 'WAITING_FOR_ACK', 'BUSY', 'IDLE')",
                params![crash.reason, candidate.id],
            )
            .map_err(|err| map_db_error("mark dead-pid agent crashed", err))?;
        if changes == 1 {
            changed += 1;
            let payload = serde_json::json!({
                "from": candidate.state,
                "to": "CRASHED",
                "reason": crash.reason,
            })
            .to_string();
            tx.execute(
                "INSERT INTO events (agent_id, request_id, event_type, payload) VALUES (?, NULL, 'state_change', ?)",
                params![candidate.id, payload],
            )
            .map_err(|err| map_db_error("insert startup reconcile dead-pid state_change", err))?;
            mark_dispatched_jobs_failed_for_agent_conn_sync(&tx, &candidate.id, crash.reason)?;
        }
    }
    tx.commit()
        .map_err(|err| map_db_error("commit startup reconcile crash dead pids", err))?;
    for crash in dead {
        let policy =
            if recovery_eligible_orphan_scope_should_be_preserved(&crash.agent.provider, "CRASHED")
            {
                crate::agent_io::registry::RuntimeCleanupPolicy::PreserveRecoverableCrashedHome
            } else {
                crate::agent_io::registry::RuntimeCleanupPolicy::Default
            };
        crate::agent_io::registry::cleanup_agent_runtime_resources_with_policy(
            &crash.agent.id,
            Some(&crash.agent.session_id),
            policy,
        );
    }
    Ok(changed)
}

fn startup_reconcile_phase_d_reregister_alive(
    db: &Db,
    alive: Vec<StartupAliveIoCandidate>,
) -> Result<(), CcbdError> {
    for alive_agent in alive {
        let candidate = alive_agent.agent;
        let Some(pid) = candidate.pid else {
            continue;
        };
        let pidfd = match crate::monitor::pidfd_open(pid as i32) {
            Ok(pidfd) => pidfd,
            Err(CcbdError::AgentUnexpectedExit { .. }) => continue,
            Err(err) => return Err(err),
        };
        let task_fd = pidfd
            .try_clone()
            .map_err(|err| CcbdError::EnvironmentNotSupported {
                details: format!("clone agent pidfd for {}: {err}", candidate.id),
            })?;
        crate::monitor::register(candidate.id.clone(), pidfd);
        let can_spawn_tasks = tokio::runtime::Handle::try_current().is_ok();
        if can_spawn_tasks {
            crate::monitor::agent_watch::spawn_agent_pidfd_watch_task(
                candidate.id.clone(),
                candidate.session_id.clone(),
                pid as i32,
                task_fd,
                Arc::new(db.clone()),
                None,
            );
        } else {
            drop(task_fd);
        }

        if can_spawn_tasks {
            let manifest = crate::provider::manifest::get_manifest(&candidate.provider);
            let parser_handle = Arc::new(std::sync::Mutex::new(vt100::Parser::new(200, 200, 0)));
            crate::marker::parser_registry::register(candidate.id.clone(), parser_handle.clone());
            let matcher = Arc::new(crate::marker::MarkerMatcher::from_manifest(&manifest));
            let idle_scan_enabled = Arc::new(std::sync::atomic::AtomicBool::new(true));
            let (output_tx, output_rx) = tokio::sync::mpsc::channel(128);
            let reader_handle = crate::agent_io::spawn_agent_io_reader_task(
                candidate.id.clone(),
                alive_agent.fifo_file,
                output_tx,
            );
            crate::marker::spawn_perception_stream_processor_task(
                candidate.id.clone(),
                db.clone(),
                parser_handle.clone(),
                output_rx,
                crate::marker::PerceptionStreamConfig {
                    matcher: matcher.clone(),
                    stability_ms: manifest.stability_ms,
                    idle_scan_enabled: idle_scan_enabled.clone(),
                },
            );
            crate::agent_io::registry::register(
                candidate.id.clone(),
                crate::agent_io::registry::AgentIoEntry {
                    session_id: candidate.session_id.clone(),
                    pane_id: TmuxPaneId(format!("reattached:{}", candidate.id)),
                    expected_pid: Some(pid),
                    reader_handle,
                    fifo_path: alive_agent.fifo_path,
                    socket_name: alive_agent.socket_name,
                    idle_scan_enabled,
                },
            );
            let timer_kind = if candidate.state == "SPAWNING" {
                crate::marker::TimerKind::Startup
            } else {
                crate::marker::TimerKind::Busy
            };
            let marker_handle = crate::marker::spawn_marker_timer_task(
                candidate.id.clone(),
                timer_kind,
                Arc::new(db.clone()),
                parser_handle,
            );
            crate::marker::registry::register(candidate.id.clone(), marker_handle);
        }
    }
    Ok(())
}

pub async fn reconcile_startup(db: Db) -> Result<usize, CcbdError> {
    let state_dir = crate::env::resolve_state_dir();
    reconcile_startup_with_tmux_socket_and_provenance(
        db,
        state_dir,
        None,
        DaemonMarkerProvenance::Ambient,
    )
    .await
}

pub async fn reconcile_startup_with_state_dir(
    db: Db,
    state_dir: PathBuf,
) -> Result<usize, CcbdError> {
    reconcile_startup_with_tmux_socket(db, state_dir, None).await
}

pub async fn reconcile_startup_with_tmux_socket(
    db: Db,
    state_dir: PathBuf,
    current_socket_name: Option<String>,
) -> Result<usize, CcbdError> {
    reconcile_startup_with_tmux_socket_and_gateway(db, state_dir, current_socket_name, None).await
}

pub async fn reconcile_startup_with_tmux_socket_and_gateway(
    db: Db,
    state_dir: PathBuf,
    current_socket_name: Option<String>,
    claude_gateway: Option<Arc<crate::claude_gateway::ClaudeGatewayService>>,
) -> Result<usize, CcbdError> {
    let reconciled = reconcile_startup_with_tmux_socket_and_provenance(
        db.clone(),
        state_dir.clone(),
        current_socket_name,
        DaemonMarkerProvenance::Explicit,
    )
    .await?;

    if claude_gateway.is_some() {
        tracing::debug!("startup reconcile complete; rebuilding claude gateway seats");
    }

    if let Some(claude_gateway) = claude_gateway {
        reconcile_claude_gateway_seats(db, state_dir, claude_gateway).await?;
    }
    Ok(reconciled)
}

async fn reconcile_startup_with_tmux_socket_and_provenance(
    db: Db,
    state_dir: PathBuf,
    current_socket_name: Option<String>,
    marker_provenance: DaemonMarkerProvenance,
) -> Result<usize, CcbdError> {
    spawn_db("system::reconcile_startup", move || {
        reconcile_startup_with_tmux_socket_sync_and_runner(
            &db,
            &state_dir,
            current_socket_name.as_deref(),
            marker_provenance,
            &RealSystemctlRunner,
            unixepoch(),
            reconcile_orphan_scopes_dry_run_enabled(),
        )
    })
    .await
}

fn reconcile_startup_with_tmux_socket_sync_and_runner(
    db: &Db,
    state_dir: &Path,
    current_socket_name: Option<&str>,
    marker_provenance: DaemonMarkerProvenance,
    runner: &dyn SystemctlRunner,
    now: i64,
    orphan_dry_run: bool,
) -> Result<usize, CcbdError> {
    let socket_name = current_socket_name
        .map(str::to_string)
        .unwrap_or_else(|| crate::tmux::compute_socket_name(state_dir));
    let daemon_marker = DaemonMarker {
        value: socket_name.clone(),
        provenance: marker_provenance,
    };
    let agents_count =
        reconcile_active_agents_to_crashed_sync(db, Some(state_dir), Some(&socket_name))?;
    let recovery_count =
        reconcile_master_recovery_windows_with_marker_sync(db, now, &daemon_marker, runner)?;
    let scopes_count =
        reconcile_orphan_scopes_with_marker_sync(db, runner, &daemon_marker, orphan_dry_run)?;
    sweep_stale_tmux_sockets_sync(current_socket_name)?;
    Ok(agents_count + recovery_count + scopes_count)
}

fn select_startup_claude_gateway_seats_sync(db: &Db) -> Result<StartupClaudeSeats, CcbdError> {
    let conn = db.conn();
    let workers = {
        let mut stmt = conn
            .prepare(
                "SELECT id, session_id
                 FROM agents
                 WHERE provider = 'claude'
                   AND state IN ('SPAWNING', 'WAITING_FOR_ACK', 'BUSY', 'IDLE', 'PROMPT_PENDING')
                 ORDER BY created_at ASC, id ASC",
            )
            .map_err(|err| map_db_error("prepare startup claude gateway workers", err))?;
        let rows = stmt
            .query_map([], |row| {
                Ok(StartupClaudeWorkerSeat {
                    id: row.get(0)?,
                    session_id: row.get(1)?,
                })
            })
            .map_err(|err| map_db_error("query startup claude gateway workers", err))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|err| map_db_error("collect startup claude gateway workers", err))?
    };
    let sessions = {
        let mut stmt = conn
            .prepare(
                "SELECT id FROM sessions WHERE status = 'ACTIVE' ORDER BY created_at ASC, id ASC",
            )
            .map_err(|err| map_db_error("prepare startup claude gateway sessions", err))?;
        let rows = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|err| map_db_error("query startup claude gateway sessions", err))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|err| map_db_error("collect startup claude gateway sessions", err))?
    };
    Ok(StartupClaudeSeats { workers, sessions })
}

async fn reconcile_claude_gateway_seats(
    db: Db,
    state_dir: PathBuf,
    claude_gateway: Arc<crate::claude_gateway::ClaudeGatewayService>,
) -> Result<(), CcbdError> {
    let seats = spawn_db("system::select_startup_claude_gateway_seats", move || {
        select_startup_claude_gateway_seats_sync(&db)
    })
    .await?;

    for session_id in seats.sessions {
        let sandbox_dir = state_dir.join("sandboxes").join(&session_id).join("master");
        match claude_gateway
            .register_master(&session_id, &sandbox_dir)
            .await
        {
            Ok(topology) => tracing::info!(
                session_id = %session_id,
                host_uds = %topology.host_uds_path.display(),
                "startup reconcile restored claude gateway master listener"
            ),
            Err(err) => tracing::warn!(
                session_id = %session_id,
                error = %err,
                "startup reconcile could not restore claude gateway master listener"
            ),
        }
    }

    for worker in seats.workers {
        let sandbox_dir = state_dir
            .join("sandboxes")
            .join(&worker.session_id)
            .join(&worker.id);
        match claude_gateway
            .register_worker(&worker.id, &sandbox_dir)
            .await
        {
            Ok(topology) => tracing::info!(
                agent_id = %worker.id,
                session_id = %worker.session_id,
                host_uds = %topology.host_uds_path.display(),
                "startup reconcile restored claude gateway worker listener"
            ),
            Err(err) => tracing::warn!(
                agent_id = %worker.id,
                session_id = %worker.session_id,
                error = %err,
                "startup reconcile could not restore claude gateway worker listener"
            ),
        }
    }

    Ok(())
}

#[cfg(test)]
pub(crate) async fn reconcile_startup_with_tmux_socket_and_runner_for_test(
    db: Db,
    state_dir: PathBuf,
    current_socket_name: Option<String>,
    runner: Arc<dyn SystemctlRunner + Send + Sync>,
    now: i64,
    orphan_dry_run: bool,
) -> Result<usize, CcbdError> {
    spawn_db("system::reconcile_startup_test", move || {
        reconcile_startup_with_tmux_socket_sync_and_runner(
            &db,
            &state_dir,
            current_socket_name.as_deref(),
            DaemonMarkerProvenance::Explicit,
            runner.as_ref(),
            now,
            orphan_dry_run,
        )
    })
    .await
}

#[cfg(test)]
pub(crate) async fn reconcile_startup_with_ambient_marker_and_runner_for_test(
    db: Db,
    state_dir: PathBuf,
    current_socket_name: Option<String>,
    runner: Arc<dyn SystemctlRunner + Send + Sync>,
    now: i64,
    orphan_dry_run: bool,
) -> Result<usize, CcbdError> {
    spawn_db("system::reconcile_startup_ambient_test", move || {
        reconcile_startup_with_tmux_socket_sync_and_runner(
            &db,
            &state_dir,
            current_socket_name.as_deref(),
            DaemonMarkerProvenance::Ambient,
            runner.as_ref(),
            now,
            orphan_dry_run,
        )
    })
    .await
}

#[cfg(test)]
pub(crate) async fn reconcile_startup_from_ambient_env_with_runner_for_test(
    db: Db,
    runner: Arc<dyn SystemctlRunner + Send + Sync>,
    now: i64,
    orphan_dry_run: bool,
) -> Result<usize, CcbdError> {
    let state_dir = crate::env::resolve_state_dir();
    spawn_db("system::reconcile_startup_ambient_env_test", move || {
        reconcile_startup_with_tmux_socket_sync_and_runner(
            &db,
            &state_dir,
            None,
            DaemonMarkerProvenance::Ambient,
            runner.as_ref(),
            now,
            orphan_dry_run,
        )
    })
    .await
}
