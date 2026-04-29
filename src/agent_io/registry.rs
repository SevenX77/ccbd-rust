use crate::tmux::TmuxPaneId;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, LazyLock, Mutex};

pub struct AgentIoEntry {
    pub pane_id: TmuxPaneId,
    pub reader_handle: tokio::task::JoinHandle<()>,
    pub fifo_path: PathBuf,
}

pub static TMUX_PANE_MAP: LazyLock<Arc<Mutex<HashMap<String, AgentIoEntry>>>> =
    LazyLock::new(|| Arc::new(Mutex::new(HashMap::new())));

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
