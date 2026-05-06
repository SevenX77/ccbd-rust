//! Sandbox filesystem path resolution.

use crate::error::CcbdError;
use std::path::{Path, PathBuf};

/// Resolve and create the per-agent sandbox directory below the ccbd state dir.
pub fn resolve_sandbox_dir(
    state_dir: &Path,
    session_id: &str,
    agent_id: &str,
) -> Result<PathBuf, CcbdError> {
    validate_id_charset("session_id", session_id)?;
    validate_id_charset("agent_id", agent_id)?;

    let sandbox_dir = state_dir.join("sandboxes").join(session_id).join(agent_id);
    std::fs::create_dir_all(&sandbox_dir).map_err(|err| CcbdError::SandboxMountFailed {
        details: format!("create sandbox dir {}: {err}", sandbox_dir.display()),
    })?;

    Ok(sandbox_dir)
}

fn validate_id_charset(field: &str, value: &str) -> Result<(), CcbdError> {
    if value.is_empty()
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        return Err(CcbdError::IpcInvalidRequest(format!(
            "invalid {field} for sandbox path: {value}"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::resolve_sandbox_dir;
    use crate::error::CcbdError;

    #[test]
    fn test_resolve_sandbox_dir_creates_directory() {
        let tmp = tempfile::TempDir::new().unwrap();
        let dir = resolve_sandbox_dir(tmp.path(), "sess_abc", "ag_1").unwrap();

        assert_eq!(
            dir,
            tmp.path().join("sandboxes").join("sess_abc").join("ag_1")
        );
        assert!(dir.is_dir());
    }

    #[test]
    fn test_resolve_sandbox_dir_includes_session_id() {
        let tmp = tempfile::TempDir::new().unwrap();
        let dir = resolve_sandbox_dir(tmp.path(), "sess_abc", "ag_1").unwrap();

        assert_eq!(
            dir,
            tmp.path().join("sandboxes").join("sess_abc").join("ag_1")
        );
        assert!(dir.is_dir());
    }

    #[test]
    fn test_resolve_sandbox_dir_isolates_agents_by_session() {
        let tmp = tempfile::TempDir::new().unwrap();
        let left = resolve_sandbox_dir(tmp.path(), "sess_abc", "ag_1").unwrap();
        let right = resolve_sandbox_dir(tmp.path(), "sess_def", "ag_1").unwrap();

        assert_ne!(left, right);
        assert_eq!(
            left,
            tmp.path().join("sandboxes").join("sess_abc").join("ag_1")
        );
        assert_eq!(
            right,
            tmp.path().join("sandboxes").join("sess_def").join("ag_1")
        );
    }

    #[test]
    fn test_resolve_sandbox_dir_rejects_invalid_session_id() {
        let tmp = tempfile::TempDir::new().unwrap();
        let empty = resolve_sandbox_dir(tmp.path(), "", "ag_1").unwrap_err();
        let traversal = resolve_sandbox_dir(tmp.path(), "../escape", "ag_1").unwrap_err();

        assert!(matches!(empty, CcbdError::IpcInvalidRequest(_)));
        assert!(matches!(traversal, CcbdError::IpcInvalidRequest(_)));
    }

    #[test]
    fn test_resolve_sandbox_dir_rejects_path_traversal() {
        let tmp = tempfile::TempDir::new().unwrap();
        let err = resolve_sandbox_dir(tmp.path(), "sess_abc", "../escape").unwrap_err();

        assert!(matches!(err, CcbdError::IpcInvalidRequest(_)));
    }
}
