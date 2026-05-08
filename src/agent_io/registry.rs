use crate::tmux::{TmuxPaneId, agent_session_name};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, LazyLock, Mutex, OnceLock};

pub struct AgentIoEntry {
    pub pane_id: TmuxPaneId,
    pub reader_handle: tokio::task::JoinHandle<()>,
    pub fifo_path: PathBuf,
}

pub static TMUX_PANE_MAP: LazyLock<Arc<Mutex<HashMap<String, AgentIoEntry>>>> =
    LazyLock::new(|| Arc::new(Mutex::new(HashMap::new())));

static TMUX_SOCKET_NAME: OnceLock<String> = OnceLock::new();

pub fn set_tmux_socket_name(name: String) {
    let _ = TMUX_SOCKET_NAME.set(name);
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
    if let Some(entry) = remove(agent_id) {
        entry.reader_handle.abort();
        if let Err(err) = std::fs::remove_file(&entry.fifo_path)
            && err.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!(agent_id, error = %err, "failed to remove agent fifo");
        }

        if let Some(socket_name) = TMUX_SOCKET_NAME.get() {
            let server = crate::tmux::TmuxServer::from_socket_name(socket_name.clone());
            let session_name = agent_session_name(agent_id);
            if let Err(err) = server.kill_session_sync(&session_name) {
                tracing::warn!(agent_id, session_name = %session_name, error = %err, "failed to kill agent session during cleanup");
            } else {
                tracing::info!(agent_id, session_name = %session_name, "killed agent session during cleanup");
            }
        }

        if let Some(pipes_dir) = entry.fifo_path.parent()
            && pipes_dir.file_name().and_then(|name| name.to_str()) == Some("pipes")
            && let Some(state_dir) = pipes_dir.parent()
        {
            let sandbox_dir = state_dir.join("sandboxes").join(agent_id);
            if let Err(err) = std::fs::remove_dir_all(&sandbox_dir)
                && err.kind() != std::io::ErrorKind::NotFound
            {
                tracing::warn!(agent_id, ?sandbox_dir, error = %err, "failed to remove agent sandbox dir");
            }
        }
    }

    if let Some(handle) = crate::marker::registry::take(agent_id) {
        let _ = handle.cancel_tx.send(());
    }
    let _ = crate::marker::parser_registry::remove(agent_id);
    let _ = crate::monitor::remove(agent_id);
}

#[cfg(test)]
mod tests {
    use super::{
        AgentIoEntry, cleanup_agent_runtime_resources, contains, register, set_tmux_socket_name,
    };
    use crate::tmux::{TmuxPaneId, TmuxServer, agent_session_name};
    use std::process::Command;

    #[tokio::test]
    async fn test_cleanup_agent_runtime_resources_removes_fifo_and_sandbox() {
        let tmp = tempfile::TempDir::new().unwrap();
        let pipes = tmp.path().join("pipes");
        let sandbox = tmp.path().join("sandboxes").join("ag_cleanup");
        std::fs::create_dir_all(&pipes).unwrap();
        std::fs::create_dir_all(&sandbox).unwrap();
        let fifo_path = pipes.join("ag_cleanup.fifo");
        std::fs::write(&fifo_path, b"").unwrap();
        let reader_handle = tokio::spawn(async {
            std::future::pending::<()>().await;
        });

        register(
            "ag_cleanup".to_string(),
            AgentIoEntry {
                pane_id: TmuxPaneId("%1".to_string()),
                reader_handle,
                fifo_path: fifo_path.clone(),
            },
        );

        cleanup_agent_runtime_resources("ag_cleanup");

        assert!(!contains("ag_cleanup"));
        assert!(!fifo_path.exists());
        assert!(!sandbox.exists());
    }

    #[tokio::test]
    async fn test_cleanup_agent_runtime_resources_kills_agent_session() {
        let tmp = tempfile::TempDir::new().unwrap();
        let server = TmuxServer::new(tmp.path());
        set_tmux_socket_name(server.socket_name().to_string());

        let agent_id = "ag_cleanup_session";
        let session_name = agent_session_name(agent_id);
        let pipes = tmp.path().join("pipes");
        let sandbox = tmp.path().join("sandboxes").join(agent_id);
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
                    pane_id: TmuxPaneId("%1".to_string()),
                    reader_handle,
                    fifo_path: fifo_path.clone(),
                },
            );

            cleanup_agent_runtime_resources(agent_id);

            let has_session = Command::new("tmux")
                .args(["-L", server.socket_name(), "has-session", "-t", &session_name])
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
