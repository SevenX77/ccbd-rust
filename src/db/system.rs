use crate::db::Db;
use crate::db::agents_lifecycle::mark_agent_killed_sync;
use crate::db::common::{map_db_error, spawn_db};
use crate::db::jobs::mark_dispatched_jobs_failed_for_agent_conn_sync;
use crate::error::CcbdError;
use crate::tmux::TmuxPaneId;
use rusqlite::{TransactionBehavior, params};
use serde_json::{Value, json};
use std::fs::{File, OpenOptions};
use std::io;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

#[derive(Debug, Clone)]
struct StartupAgentCandidate {
    id: String,
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
}

pub(crate) fn system_dump_sync(db: &Db) -> Result<Value, CcbdError> {
    let conn = db.conn();
    let projects = {
        let mut stmt = conn
            .prepare("SELECT id, absolute_path, created_at FROM projects ORDER BY created_at ASC")
            .map_err(|err| map_db_error("prepare dump projects", err))?;
        stmt.query_map([], |row| {
            Ok(json!({
                "id": row.get::<_, String>(0)?,
                "absolute_path": row.get::<_, String>(1)?,
                "created_at": row.get::<_, i64>(2)?,
            }))
        })
        .map_err(|err| map_db_error("query dump projects", err))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|err| map_db_error("collect dump projects", err))?
    };
    let sessions = {
        let mut stmt = conn
            .prepare("SELECT id, project_id, created_at FROM sessions ORDER BY created_at ASC")
            .map_err(|err| map_db_error("prepare dump sessions", err))?;
        stmt.query_map([], |row| {
            Ok(json!({
                "id": row.get::<_, String>(0)?,
                "project_id": row.get::<_, String>(1)?,
                "created_at": row.get::<_, i64>(2)?,
            }))
        })
        .map_err(|err| map_db_error("query dump sessions", err))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|err| map_db_error("collect dump sessions", err))?
    };
    let agents = {
        let mut stmt = conn
            .prepare(
                "SELECT id, session_id, provider, state, sub_state, state_version, pid, exit_code, error_code, created_at, updated_at FROM agents ORDER BY created_at ASC",
            )
            .map_err(|err| map_db_error("prepare dump agents", err))?;
        stmt.query_map([], |row| {
            Ok(json!({
                "id": row.get::<_, String>(0)?,
                "session_id": row.get::<_, String>(1)?,
                "provider": row.get::<_, String>(2)?,
                "state": row.get::<_, String>(3)?,
                "sub_state": row.get::<_, Option<String>>(4)?,
                "state_version": row.get::<_, i64>(5)?,
                "pid": row.get::<_, Option<i64>>(6)?,
                "exit_code": row.get::<_, Option<i64>>(7)?,
                "error_code": row.get::<_, Option<String>>(8)?,
                "created_at": row.get::<_, i64>(9)?,
                "updated_at": row.get::<_, i64>(10)?,
            }))
        })
        .map_err(|err| map_db_error("query dump agents", err))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|err| map_db_error("collect dump agents", err))?
    };
    let evidence_pending = {
        let mut stmt = conn
            .prepare(
                "SELECT id, agent_id, status, created_at FROM evidence WHERE status = 'PENDING' ORDER BY created_at DESC LIMIT 100",
            )
            .map_err(|err| map_db_error("prepare dump evidence", err))?;
        stmt.query_map([], |row| {
            Ok(json!({
                "id": row.get::<_, String>(0)?,
                "agent_id": row.get::<_, String>(1)?,
                "status": row.get::<_, String>(2)?,
                "created_at": row.get::<_, i64>(3)?,
            }))
        })
        .map_err(|err| map_db_error("query dump evidence", err))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|err| map_db_error("collect dump evidence", err))?
    };

    Ok(json!({
        "projects": projects,
        "sessions": sessions,
        "agents": agents,
        "evidence_pending": evidence_pending,
    }))
}

pub(crate) fn cascade_kill_session_agents_sync(
    db: &Db,
    session_id: &str,
    reason: &str,
) -> Result<usize, CcbdError> {
    let agent_ids = {
        let conn = db.conn();
        let mut stmt = conn
            .prepare(
                "SELECT id FROM agents WHERE session_id = ? AND state NOT IN ('CRASHED', 'KILLED')",
            )
            .map_err(|err| map_db_error("prepare cascade agent query", err))?;
        let rows = stmt
            .query_map(params![session_id], |row| row.get::<_, String>(0))
            .map_err(|err| map_db_error("query cascade agents", err))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|err| map_db_error("collect cascade agents", err))?
    };

    let mut changed = 0;
    for agent_id in agent_ids {
        match crate::monitor::with_borrowed(&agent_id, crate::monitor::pidfd_send_sigkill) {
            Some(Ok(())) => {}
            Some(Err(err)) => {
                tracing::warn!(agent_id = %agent_id, error = %err, "failed to SIGKILL agent during cascade")
            }
            None => tracing::debug!(agent_id = %agent_id, "no pidfd registered during cascade"),
        }
        changed += mark_agent_killed_sync(db, &agent_id, reason)?;
        if let Some(handle) = crate::marker::registry::take(&agent_id) {
            let _ = handle.cancel_tx.send(());
        }
        let _ = crate::marker::parser_registry::remove(&agent_id);
    }
    Ok(changed)
}

pub(crate) fn session_agent_ids_sync(db: &Db, session_id: &str) -> Result<Vec<String>, CcbdError> {
    let conn = db.conn();
    let mut stmt = conn
        .prepare("SELECT id FROM agents WHERE session_id = ? ORDER BY created_at ASC, id ASC")
        .map_err(|err| map_db_error("prepare session agent ids", err))?;
    let rows = stmt
        .query_map(params![session_id], |row| row.get::<_, String>(0))
        .map_err(|err| map_db_error("query session agent ids", err))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|err| map_db_error("collect session agent ids", err))
}

/// Reconcile active agents during daemon startup.
#[cfg(test)]
pub(crate) fn reconcile_startup_sync(db: &Db) -> Result<usize, CcbdError> {
    reconcile_startup_sync_with_state_dir(db, None)
}

pub(crate) fn reconcile_startup_sync_with_state_dir(
    db: &Db,
    state_dir: Option<&Path>,
) -> Result<usize, CcbdError> {
    reconcile_active_agents_to_crashed_sync(db, state_dir)
}

pub(crate) fn reconcile_active_agents_to_crashed_sync(
    db: &Db,
    state_dir: Option<&Path>,
) -> Result<usize, CcbdError> {
    let candidates = startup_reconcile_phase_a_select_candidates(db)?;
    let (dead, alive) = startup_reconcile_phase_b_probe_pids(candidates);
    let (reattach_dead, reattachable_alive) =
        startup_reconcile_phase_b2_prepare_alive_io(state_dir, alive);
    let dead = dead.into_iter().chain(reattach_dead).collect::<Vec<_>>();
    let changed = startup_reconcile_phase_c_crash_dead(db, &dead)?;
    startup_reconcile_phase_d_reregister_alive(db, reattachable_alive)?;
    Ok(changed)
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
                "SELECT id, pid, state, provider FROM agents WHERE state IN ('SPAWNING', 'BUSY', 'IDLE')",
            )
            .map_err(|err| map_db_error("prepare active agents query", err))?;
        let rows = stmt
            .query_map([], |row| {
                Ok(StartupAgentCandidate {
                    id: row.get(0)?,
                    pid: row.get(1)?,
                    state: row.get(2)?,
                    provider: row.get(3)?,
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
    alive: Vec<StartupAgentCandidate>,
) -> (Vec<StartupCrashCandidate>, Vec<StartupAliveIoCandidate>) {
    let Some(state_dir) = state_dir else {
        return (Vec::new(), Vec::new());
    };

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
        match OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(&fifo_path)
        {
            Ok(fifo_file) => reattachable.push(StartupAliveIoCandidate {
                agent,
                fifo_path,
                fifo_file,
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

fn probe_pid_alive(pid: i32) -> Result<bool, std::io::Error> {
    // SAFETY: kill(pid, 0) does not deliver a signal; it only asks the kernel
    // to validate process existence and permissions for this pid.
    let rc = unsafe { libc::kill(pid, 0) };
    if rc == 0 {
        return Ok(true);
    }
    let err = std::io::Error::last_os_error();
    match err.raw_os_error() {
        Some(libc::EPERM) => Ok(true),
        Some(libc::ESRCH) => Ok(false),
        _ => Err(err),
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
                "UPDATE agents SET state = 'CRASHED', error_code = ?, state_version = state_version + 1, updated_at = unixepoch() WHERE id = ? AND state IN ('SPAWNING', 'BUSY', 'IDLE')",
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
        crate::agent_io::registry::cleanup_agent_runtime_resources(&crash.agent.id);
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
                task_fd,
                Arc::new(db.clone()),
            );
        } else {
            drop(task_fd);
        }

        if can_spawn_tasks {
            let manifest = crate::provider::manifest::get_manifest(&candidate.provider);
            let parser_handle = Arc::new(std::sync::Mutex::new(vt100::Parser::new(200, 200, 0)));
            crate::marker::parser_registry::register(candidate.id.clone(), parser_handle.clone());
            let matcher = Arc::new(crate::marker::MarkerMatcher::from_manifest(&manifest));
            let reader_handle = crate::agent_io::spawn_agent_io_reader_task_with_config(
                candidate.id.clone(),
                alive_agent.fifo_file,
                db.clone(),
                parser_handle.clone(),
                crate::agent_io::ReaderMarkerConfig {
                    matcher: matcher.clone(),
                    stability_ms: manifest.stability_ms,
                },
            );
            crate::agent_io::registry::register(
                candidate.id.clone(),
                crate::agent_io::registry::AgentIoEntry {
                    pane_id: TmuxPaneId(format!("reattached:{}", candidate.id)),
                    reader_handle,
                    fifo_path: alive_agent.fifo_path,
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

pub async fn system_dump(db: Db) -> Result<Value, CcbdError> {
    spawn_db("system::system_dump", move || system_dump_sync(&db)).await
}

pub async fn cascade_kill_session_agents(
    db: Db,
    session_id: String,
    reason: String,
) -> Result<usize, CcbdError> {
    spawn_db("system::cascade_kill_session_agents", move || {
        cascade_kill_session_agents_sync(&db, &session_id, &reason)
    })
    .await
}

pub async fn session_agent_ids(db: Db, session_id: String) -> Result<Vec<String>, CcbdError> {
    spawn_db("system::session_agent_ids", move || {
        session_agent_ids_sync(&db, &session_id)
    })
    .await
}

pub async fn reconcile_startup(db: Db) -> Result<usize, CcbdError> {
    let state_dir = crate::env::resolve_state_dir();
    reconcile_startup_with_tmux_socket(db, state_dir, None).await
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
    spawn_db("system::reconcile_startup", move || {
        let changed = reconcile_startup_sync_with_state_dir(&db, Some(&state_dir))?;
        sweep_stale_tmux_sockets_sync(current_socket_name.as_deref())?;
        Ok(changed)
    })
    .await
}

pub fn sweep_stale_tmux_sockets_sync(
    current_socket_name: Option<&str>,
) -> Result<usize, CcbdError> {
    let socket_dir = format!("/tmp/tmux-{}", unsafe { libc::geteuid() });
    let entries = match std::fs::read_dir(&socket_dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(0),
        Err(err) => {
            return Err(CcbdError::EnvironmentNotSupported {
                details: format!("read tmux socket dir {socket_dir}: {err}"),
            });
        }
    };

    let mut removed = 0;
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(err) => {
                tracing::warn!(error = %err, "failed to read tmux socket entry");
                continue;
            }
        };
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.starts_with("ccbd-") || current_socket_name == Some(name.as_str()) {
            continue;
        }
        let alive = tmux_sessions_alive(&name);
        if alive {
            continue;
        }
        match std::fs::remove_file(entry.path()) {
            Ok(()) => removed += 1,
            Err(err) if err.kind() == io::ErrorKind::NotFound => {}
            Err(err) => {
                tracing::warn!(socket = %name, error = %err, "failed to remove stale tmux socket")
            }
        }
    }
    Ok(removed)
}

fn tmux_sessions_alive(socket_name: &str) -> bool {
    match Command::new("tmux")
        .args(["-L", socket_name, "ls-sessions"])
        .output()
    {
        Ok(output) if output.status.success() => true,
        Ok(output) if String::from_utf8_lossy(&output.stderr).contains("unknown command") => {
            Command::new("tmux")
                .args(["-L", socket_name, "list-sessions"])
                .output()
                .map(|output| output.status.success())
                .unwrap_or(false)
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::{cascade_kill_session_agents_sync, reconcile_startup_sync};
    use crate::db::agents::insert_agent_sync;
    use crate::db::sessions::insert_session_sync;
    use crate::db::{Db, init};

    fn with_test_db_handle<T>(test: impl FnOnce(&Db) -> T) -> T {
        let file = tempfile::NamedTempFile::new().unwrap();
        let db = init(file.path()).unwrap();
        test(&db)
    }

    #[test]
    fn test_cascade_kill_session_agents_counts_active_only() {
        with_test_db_handle(|db| {
            {
                let conn = db.conn();
                insert_session_sync(&conn, "s1", "p1", "/tmp/foo").unwrap();
                insert_agent_sync(&conn, "a1", "s1", "bash", "IDLE", Some(1)).unwrap();
                insert_agent_sync(&conn, "a2", "s1", "bash", "BUSY", Some(2)).unwrap();
                insert_agent_sync(&conn, "a3", "s1", "bash", "CRASHED", Some(3)).unwrap();
            }
            let count = cascade_kill_session_agents_sync(db, "s1", "MASTER_DEATH").unwrap();
            let killed_count: i64 = db
                .conn()
                .query_row(
                    "SELECT COUNT(*) FROM agents WHERE state = 'KILLED'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(count, 2);
            assert_eq!(killed_count, 2);
        });
    }

    #[test]
    fn test_reconcile_startup_crashes_dead_pid_and_fails_dispatched_job() {
        with_test_db_handle(|db| {
            {
                let conn = db.conn();
                insert_session_sync(&conn, "s1", "p1", "/tmp/foo").unwrap();
                insert_agent_sync(&conn, "a1", "s1", "bash", "BUSY", Some(999_999_999)).unwrap();
                crate::db::jobs::insert_job_sync(&conn, "job_1", "a1", None, "echo hi").unwrap();
                conn.execute(
                    "UPDATE jobs SET status = 'DISPATCHED', dispatched_at = unixepoch() WHERE id = 'job_1'",
                    [],
                )
                .unwrap();
            }
            let count = reconcile_startup_sync(db).unwrap();
            let (state, error_reason): (String, Option<String>) = db
                .conn()
                .query_row(
                    "SELECT agents.state, jobs.error_reason FROM agents JOIN jobs ON jobs.agent_id = agents.id WHERE agents.id = 'a1'",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .unwrap();
            assert_eq!(count, 1);
            assert_eq!(state, "CRASHED");
            assert_eq!(error_reason.as_deref(), Some("STARTUP_RECONCILE_DEAD_PID"));
        });
    }

    #[test]
    fn test_reconcile_startup_keeps_alive_pid_active() {
        with_test_db_handle(|db| {
            {
                let conn = db.conn();
                insert_session_sync(&conn, "s1", "p1", "/tmp/foo").unwrap();
                insert_agent_sync(
                    &conn,
                    "a_alive",
                    "s1",
                    "bash",
                    "IDLE",
                    Some(std::process::id() as i64),
                )
                .unwrap();
            }
            let count = reconcile_startup_sync(db).unwrap();
            let state: String = db
                .conn()
                .query_row("SELECT state FROM agents WHERE id = 'a_alive'", [], |row| {
                    row.get(0)
                })
                .unwrap();
            assert_eq!(count, 0);
            assert_eq!(state, "IDLE");
            let _ = crate::monitor::remove("a_alive");
        });
    }
}
