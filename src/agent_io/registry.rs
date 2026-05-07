use crate::tmux::TmuxPaneId;
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
            if let Err(err) = server.kill_pane_sync(&entry.pane_id) {
                tracing::warn!(agent_id, pane_id = %entry.pane_id.0, error = %err, "failed to kill agent pane during cleanup");
            } else {
                tracing::info!(agent_id, pane_id = %entry.pane_id.0, "killed agent pane during cleanup");
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
    use super::{AgentIoEntry, cleanup_agent_runtime_resources, contains, register};
    use crate::tmux::TmuxPaneId;

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
}
