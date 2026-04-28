//! Global marker timer handle registry keyed by agent id.

use crate::marker::timer::MarkerTimerHandle;
use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Instant;

pub static MARKER_TIMER_REGISTRY: LazyLock<Arc<Mutex<HashMap<String, MarkerTimerHandle>>>> =
    LazyLock::new(|| Arc::new(Mutex::new(HashMap::new())));

/// Register a marker timer handle, canceling any previous timer for the key.
pub fn register(key: String, handle: MarkerTimerHandle) {
    match MARKER_TIMER_REGISTRY.lock() {
        Ok(mut registry) => {
            if let Some(old) = registry.insert(key, handle) {
                let _ = old.cancel_tx.send(());
            }
        }
        Err(err) => {
            tracing::warn!(error = %err, "marker timer registry mutex poisoned during register")
        }
    }
}

/// Remove and return a marker timer handle.
pub fn take(key: &str) -> Option<MarkerTimerHandle> {
    match MARKER_TIMER_REGISTRY.lock() {
        Ok(mut registry) => registry.remove(key),
        Err(err) => {
            tracing::warn!(error = %err, "marker timer registry mutex poisoned during take");
            None
        }
    }
}

/// Reset the marker timer for an agent, if one is registered.
pub fn reset(key: &str) {
    match MARKER_TIMER_REGISTRY.lock() {
        Ok(registry) => {
            if let Some(handle) = registry.get(key) {
                let _ = handle.reset_tx.send(Instant::now());
            }
        }
        Err(err) => {
            tracing::warn!(error = %err, "marker timer registry mutex poisoned during reset")
        }
    }
}

/// Return true when an agent has a registered marker timer.
pub fn contains(key: &str) -> bool {
    match MARKER_TIMER_REGISTRY.lock() {
        Ok(registry) => registry.contains_key(key),
        Err(err) => {
            tracing::warn!(error = %err, "marker timer registry mutex poisoned during contains");
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{contains, register, reset, take};
    use crate::db;
    use crate::marker::timer::{TimerKind, spawn_marker_timer_task};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    #[tokio::test(flavor = "multi_thread")]
    async fn test_register_take_contains() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let db = db::init(file.path()).unwrap();
        let key = format!("ag_{}", uuid::Uuid::new_v4());
        let parser = Arc::new(Mutex::new(vt100::Parser::new(200, 200, 0)));
        let handle = spawn_marker_timer_task(key.clone(), TimerKind::Busy, Arc::new(db), parser);

        register(key.clone(), handle);
        assert!(contains(&key));
        let handle = take(&key).unwrap();
        let _ = handle.cancel_tx.send(());
        assert!(!contains(&key));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_replace_cancels_old_handle_and_reset_is_safe() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let db = db::init(file.path()).unwrap();
        let key = format!("ag_{}", uuid::Uuid::new_v4());
        let old = spawn_marker_timer_task(
            key.clone(),
            TimerKind::Busy,
            Arc::new(db.clone()),
            Arc::new(Mutex::new(vt100::Parser::new(200, 200, 0))),
        );
        register(key.clone(), old);

        let new = spawn_marker_timer_task(
            key.clone(),
            TimerKind::Busy,
            Arc::new(db),
            Arc::new(Mutex::new(vt100::Parser::new(200, 200, 0))),
        );
        register(key.clone(), new);
        reset(&key);

        let handle = take(&key).unwrap();
        let _ = handle.cancel_tx.send(());
        tokio::time::sleep(Duration::from_millis(10)).await;
        assert!(!contains(&key));
    }
}
