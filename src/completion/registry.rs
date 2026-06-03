use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{LazyLock, Mutex};

use tokio::sync::oneshot;

use crate::completion::reader::LogCursorMap;

pub struct LogMonitorEntry {
    pub provider: String,
    pub log_root: PathBuf,
    pub cursors: LogCursorMap,
    pub cancel_tx: oneshot::Sender<()>,
}

static LOG_MONITORS: LazyLock<Mutex<HashMap<String, LogMonitorEntry>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

pub fn register(agent_id: String, entry: LogMonitorEntry) {
    match LOG_MONITORS.lock() {
        Ok(mut map) => {
            if let Some(old) = map.insert(agent_id, entry) {
                let _ = old.cancel_tx.send(());
            }
        }
        Err(err) => tracing::warn!(error = %err, "LOG_MONITORS mutex poisoned during register"),
    }
}

pub fn contains(agent_id: &str) -> bool {
    LOG_MONITORS
        .lock()
        .is_ok_and(|map| map.contains_key(agent_id))
}

pub fn cursor_snapshot(agent_id: &str) -> Option<LogCursorMap> {
    LOG_MONITORS
        .lock()
        .ok()
        .and_then(|map| map.get(agent_id).map(|entry| entry.cursors.clone()))
}

pub fn entry_snapshot(agent_id: &str) -> Option<(String, PathBuf, LogCursorMap)> {
    LOG_MONITORS.lock().ok().and_then(|map| {
        map.get(agent_id).map(|entry| {
            (
                entry.provider.clone(),
                entry.log_root.clone(),
                entry.cursors.clone(),
            )
        })
    })
}

pub fn update_cursors(agent_id: &str, cursors: LogCursorMap) {
    match LOG_MONITORS.lock() {
        Ok(mut map) => {
            if let Some(entry) = map.get_mut(agent_id) {
                entry.cursors = cursors;
            }
        }
        Err(err) => {
            tracing::warn!(error = %err, "LOG_MONITORS mutex poisoned during cursor update")
        }
    }
}

pub fn take(agent_id: &str) -> Option<LogMonitorEntry> {
    LOG_MONITORS
        .lock()
        .ok()
        .and_then(|mut map| map.remove(agent_id))
}

pub fn cancel(agent_id: &str) {
    if let Some(entry) = take(agent_id) {
        let _ = entry.cancel_tx.send(());
    }
}

#[cfg(test)]
mod tests {}
