use crate::tmux::{TmuxPaneId, agent_session_name};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex};

pub struct AgentIoEntry {
    pub session_id: String,
    pub pane_id: TmuxPaneId,
    pub reader_handle: tokio::task::JoinHandle<()>,
    pub fifo_path: PathBuf,
    pub socket_name: String,
    pub idle_scan_enabled: Arc<AtomicBool>,
}

pub static TMUX_PANE_MAP: LazyLock<Arc<Mutex<HashMap<String, AgentIoEntry>>>> =
    LazyLock::new(|| Arc::new(Mutex::new(HashMap::new())));

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeCleanupPolicy {
    Default,
    PreserveRecoverableCrashedHome,
}

pub fn register(agent_id: String, entry: AgentIoEntry) {
    match TMUX_PANE_MAP.lock() {
        Ok(mut map) => {
            map.insert(agent_id, entry);
        }
        Err(err) => tracing::warn!(error = %err, "TMUX_PANE_MAP mutex poisoned during register"),
    }
}

pub fn pane_id(agent_id: &str) -> Option<TmuxPaneId> {
    TMUX_PANE_MAP
        .lock()
        .ok()
        .and_then(|map| map.get(agent_id).map(|entry| entry.pane_id.clone()))
}

pub fn update_pane_id(agent_id: &str, pane_id: TmuxPaneId) -> bool {
    match TMUX_PANE_MAP.lock() {
        Ok(mut map) => {
            let Some(entry) = map.get_mut(agent_id) else {
                return false;
            };
            entry.pane_id = pane_id;
            true
        }
        Err(err) => {
            tracing::warn!(error = %err, "TMUX_PANE_MAP mutex poisoned during pane refresh");
            false
        }
    }
}

pub fn pane_binding(agent_id: &str) -> Option<(TmuxPaneId, String)> {
    TMUX_PANE_MAP.lock().ok().and_then(|map| {
        map.get(agent_id)
            .map(|entry| (entry.pane_id.clone(), entry.socket_name.clone()))
    })
}

pub fn init_probe_binding(agent_id: &str) -> Option<(TmuxPaneId, Arc<AtomicBool>)> {
    TMUX_PANE_MAP.lock().ok().and_then(|map| {
        map.get(agent_id)
            .map(|entry| (entry.pane_id.clone(), entry.idle_scan_enabled.clone()))
    })
}

pub fn set_idle_scan_enabled(agent_id: &str, enabled: bool) {
    match TMUX_PANE_MAP.lock() {
        Ok(map) => {
            if let Some(entry) = map.get(agent_id) {
                entry.idle_scan_enabled.store(enabled, Ordering::SeqCst);
            }
        }
        Err(err) => {
            tracing::warn!(error = %err, "TMUX_PANE_MAP mutex poisoned during idle scan toggle");
        }
    }
}

pub fn remove(agent_id: &str) -> Option<AgentIoEntry> {
    TMUX_PANE_MAP
        .lock()
        .ok()
        .and_then(|mut map| map.remove(agent_id))
}

pub fn contains(agent_id: &str) -> bool {
    TMUX_PANE_MAP
        .lock()
        .is_ok_and(|map| map.contains_key(agent_id))
}

pub fn cleanup_agent_runtime_resources(agent_id: &str) {
    cleanup_agent_runtime_resources_with_policy(agent_id, RuntimeCleanupPolicy::Default);
}

pub fn cleanup_agent_runtime_resources_with_policy(agent_id: &str, policy: RuntimeCleanupPolicy) {
    if let Some(entry) = remove(agent_id) {
        entry.reader_handle.abort();
        if let Err(err) = std::fs::remove_file(&entry.fifo_path)
            && err.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!(agent_id, error = %err, "failed to remove agent fifo");
        }

        let server = crate::tmux::TmuxServer::from_socket_name(entry.socket_name.clone());
        let state_dir = state_dir_from_fifo_path(&entry.fifo_path);
        capture_pane_at_death(agent_id, &server, &entry.pane_id, state_dir.as_deref());
        let session_name = agent_session_name(agent_id);
        if let Err(err) = server.kill_session_sync(&session_name) {
            tracing::warn!(agent_id, session_name = %session_name, error = %err, "failed to kill agent session during cleanup");
        } else {
            tracing::info!(agent_id, session_name = %session_name, "killed agent session during cleanup");
        }

        if let Some(state_dir) = state_dir_from_fifo_path(&entry.fifo_path) {
            match policy {
                RuntimeCleanupPolicy::Default => {
                    crate::db::system::remove_agent_sandbox_dir_sync(
                        &state_dir,
                        &entry.session_id,
                        agent_id,
                    );
                }
                RuntimeCleanupPolicy::PreserveRecoverableCrashedHome => {
                    let sandbox_dir = state_dir
                        .join("sandboxes")
                        .join(&entry.session_id)
                        .join(agent_id);
                    if let Err(err) = std::fs::remove_dir_all(&sandbox_dir)
                        && err.kind() != std::io::ErrorKind::NotFound
                    {
                        tracing::warn!(
                            ?sandbox_dir,
                            error = %err,
                            "failed to remove preserved-home agent sandbox dir"
                        );
                    }
                }
            }
        }
    }

    if let Some(handle) = crate::marker::registry::take(agent_id) {
        let _ = handle.cancel_tx.send(());
    }
    crate::completion::registry::cancel(agent_id);
    let _ = crate::marker::parser_registry::remove(agent_id);
    let _ = crate::monitor::remove(agent_id);
}

fn state_dir_from_fifo_path(fifo_path: &std::path::Path) -> Option<std::path::PathBuf> {
    fifo_path
        .parent()
        .filter(|pipes_dir| pipes_dir.file_name().and_then(|name| name.to_str()) == Some("pipes"))
        .and_then(|pipes_dir| pipes_dir.parent())
        .map(std::path::Path::to_path_buf)
}

fn capture_pane_at_death(
    agent_id: &str,
    server: &crate::tmux::TmuxServer,
    pane_id: &TmuxPaneId,
    state_dir: Option<&std::path::Path>,
) {
    let capture = match server.capture_pane_sync(pane_id) {
        Ok(capture) => capture,
        Err(err) => {
            tracing::warn!(agent_id, pane_id = %pane_id.0, error = %err, "failed to capture pane before cleanup");
            return;
        }
    };
    let Some(state_dir) = state_dir else {
        tracing::warn!(
            agent_id,
            "state_dir unavailable; skipping pane death evidence capture"
        );
        return;
    };
    let evidence_path = state_dir
        .join("evidence")
        .join(agent_id)
        .join("pane_at_death.txt");
    if let Some(parent) = evidence_path.parent()
        && let Err(err) = std::fs::create_dir_all(parent)
    {
        tracing::warn!(agent_id, path = %parent.display(), error = %err, "failed to create pane death evidence directory");
        return;
    }
    if let Err(err) = std::fs::write(&evidence_path, capture) {
        tracing::warn!(agent_id, path = %evidence_path.display(), error = %err, "failed to write pane death evidence");
    }
}

#[cfg(test)]
mod tests {
    use super::{AgentIoEntry, cleanup_agent_runtime_resources, contains, register};
    use crate::tmux::{TmuxPaneId, TmuxServer, agent_session_name};
    use std::process::Command;
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;

    #[tokio::test]
    async fn test_cleanup_agent_runtime_resources_removes_fifo_and_sandbox() {
        let tmp = tempfile::TempDir::new().unwrap();
        let pipes = tmp.path().join("pipes");
        let session_id = "s_cleanup";
        let sandbox = tmp
            .path()
            .join("sandboxes")
            .join(session_id)
            .join("ag_cleanup");
        std::fs::create_dir_all(&pipes).unwrap();
        std::fs::create_dir_all(&sandbox).unwrap();
        let home_root =
            crate::provider::home_layout::sandbox_home_for_sandbox_dir(&sandbox).unwrap();
        std::fs::create_dir_all(home_root.join(".codex")).unwrap();
        std::fs::write(home_root.join(".codex/auth.json"), b"token").unwrap();
        let fifo_path = pipes.join("ag_cleanup.fifo");
        std::fs::write(&fifo_path, b"").unwrap();
        let reader_handle = tokio::spawn(async {
            std::future::pending::<()>().await;
        });

        register(
            "ag_cleanup".to_string(),
            AgentIoEntry {
                session_id: session_id.to_string(),
                pane_id: TmuxPaneId("%1".to_string()),
                reader_handle,
                fifo_path: fifo_path.clone(),
                socket_name: "missing-test-socket".to_string(),
                idle_scan_enabled: Arc::new(AtomicBool::new(true)),
            },
        );

        cleanup_agent_runtime_resources("ag_cleanup");

        assert!(!contains("ag_cleanup"));
        assert!(!fifo_path.exists());
        assert!(!sandbox.exists());
        assert!(!home_root.exists());
    }

    #[tokio::test]
    async fn test_cleanup_agent_runtime_resources_kills_agent_session() {
        let tmp = tempfile::TempDir::new().unwrap();
        let server = TmuxServer::new(tmp.path());

        let agent_id = "ag_cleanup_session";
        let session_id = "s_cleanup_session";
        let session_name = agent_session_name(agent_id);
        let pipes = tmp.path().join("pipes");
        let sandbox = tmp.path().join("sandboxes").join(session_id).join(agent_id);
        std::fs::create_dir_all(&pipes).unwrap();
        std::fs::create_dir_all(&sandbox).unwrap();
        let fifo_path = pipes.join("ag_cleanup_session.fifo");
        std::fs::write(&fifo_path, b"").unwrap();
        let reader_handle = tokio::spawn(async {
            std::future::pending::<()>().await;
        });

        let result = (|| {
            server.ensure_session_sync(&session_name, tmp.path())?;
            register(
                agent_id.to_string(),
                AgentIoEntry {
                    session_id: session_id.to_string(),
                    pane_id: TmuxPaneId("%1".to_string()),
                    reader_handle,
                    fifo_path: fifo_path.clone(),
                    socket_name: server.socket_name().to_string(),
                    idle_scan_enabled: Arc::new(AtomicBool::new(true)),
                },
            );

            cleanup_agent_runtime_resources(agent_id);

            let has_session = Command::new("tmux")
                .args([
                    "-L",
                    server.socket_name(),
                    "has-session",
                    "-t",
                    &session_name,
                ])
                .output()
                .map_err(crate::tmux::error::TmuxError::from)?;
            assert!(!has_session.status.success());
            assert!(!contains(agent_id));
            assert!(!fifo_path.exists());
            assert!(!sandbox.exists());
            Ok::<(), crate::error::CcbdError>(())
        })();

        let _ = Command::new("tmux")
            .args(["-L", server.socket_name(), "kill-server"])
            .output();
        result.unwrap();
    }
}
