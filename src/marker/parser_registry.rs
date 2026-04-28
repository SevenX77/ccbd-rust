//! Global vt100 parser registry keyed by agent id.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};

pub type ParserHandle = Arc<Mutex<vt100::Parser>>;

pub static PARSER_REGISTRY: LazyLock<Arc<Mutex<HashMap<String, ParserHandle>>>> =
    LazyLock::new(|| Arc::new(Mutex::new(HashMap::new())));

/// Register a parser handle for one agent.
pub fn register(agent_id: String, handle: ParserHandle) {
    match PARSER_REGISTRY.lock() {
        Ok(mut registry) => {
            registry.insert(agent_id, handle);
        }
        Err(err) => {
            tracing::warn!(error = %err, "parser registry mutex poisoned during register")
        }
    }
}

/// Get a cloned parser handle for one agent.
pub fn get(agent_id: &str) -> Option<ParserHandle> {
    match PARSER_REGISTRY.lock() {
        Ok(registry) => registry.get(agent_id).cloned(),
        Err(err) => {
            tracing::warn!(error = %err, "parser registry mutex poisoned during get");
            None
        }
    }
}

/// Remove a parser handle for one agent.
pub fn remove(agent_id: &str) -> Option<ParserHandle> {
    match PARSER_REGISTRY.lock() {
        Ok(mut registry) => registry.remove(agent_id),
        Err(err) => {
            tracing::warn!(error = %err, "parser registry mutex poisoned during remove");
            None
        }
    }
}

/// Return true when a parser is registered for one agent.
pub fn contains(agent_id: &str) -> bool {
    match PARSER_REGISTRY.lock() {
        Ok(registry) => registry.contains_key(agent_id),
        Err(err) => {
            tracing::warn!(error = %err, "parser registry mutex poisoned during contains");
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{contains, get, register, remove};
    use std::sync::{Arc, Mutex};

    #[test]
    fn test_register_get_remove_parser_handle() {
        let key = format!("ag_{}", uuid::Uuid::new_v4());
        let handle = Arc::new(Mutex::new(vt100::Parser::new(200, 200, 0)));

        register(key.clone(), handle.clone());
        assert!(contains(&key));
        assert!(Arc::ptr_eq(&get(&key).unwrap(), &handle));
        assert!(remove(&key).is_some());
        assert!(!contains(&key));
    }
}
