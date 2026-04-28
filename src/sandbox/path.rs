//! Sandbox filesystem path resolution.

use crate::error::CcbdError;
use std::path::{Path, PathBuf};

/// Resolve and create the per-agent sandbox directory below the ccbd state dir.
pub fn resolve_sandbox_dir(state_dir: &Path, agent_id: &str) -> Result<PathBuf, CcbdError> {
    if agent_id.is_empty()
        || !agent_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        return Err(CcbdError::IpcInvalidRequest(format!(
            "invalid agent_id for sandbox path: {agent_id}"
        )));
    }

    let sandbox_dir = state_dir.join("sandboxes").join(agent_id);
    std::fs::create_dir_all(&sandbox_dir).map_err(|err| CcbdError::SandboxMountFailed {
        details: format!("create sandbox dir {}: {err}", sandbox_dir.display()),
    })?;

    Ok(sandbox_dir)
}

#[cfg(test)]
mod tests {
    use super::resolve_sandbox_dir;
    use crate::error::CcbdError;

    #[test]
    fn test_resolve_sandbox_dir_creates_directory() {
        let tmp = tempfile::TempDir::new().unwrap();
        let dir = resolve_sandbox_dir(tmp.path(), "ag_1").unwrap();

        assert_eq!(dir, tmp.path().join("sandboxes").join("ag_1"));
        assert!(dir.is_dir());
    }

    #[test]
    fn test_resolve_sandbox_dir_rejects_path_traversal() {
        let tmp = tempfile::TempDir::new().unwrap();
        let err = resolve_sandbox_dir(tmp.path(), "../escape").unwrap_err();

        assert!(matches!(err, CcbdError::IpcInvalidRequest(_)));
    }
}
