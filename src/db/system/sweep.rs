use crate::error::CcbdError;
use std::io;
use std::path::Path;
use std::process::Command;

pub fn remove_agent_sandbox_dir_sync(state_dir: &Path, session_id: &str, agent_id: &str) {
    let sandbox_dir = state_dir.join("sandboxes").join(session_id).join(agent_id);
    match crate::provider::home_layout::sandbox_home_for_sandbox_dir(&sandbox_dir) {
        Ok(home_root) => {
            if let Err(err) = std::fs::remove_dir_all(&home_root)
                && err.kind() != io::ErrorKind::NotFound
            {
                tracing::warn!(?home_root, error = %err, "failed to remove agent sandbox home");
            }
        }
        Err(err) => {
            tracing::warn!(?sandbox_dir, error = %err, "failed to resolve agent sandbox home");
        }
    }
    if let Err(err) = std::fs::remove_dir_all(&sandbox_dir)
        && err.kind() != io::ErrorKind::NotFound
    {
        tracing::warn!(?sandbox_dir, error = %err, "failed to remove agent sandbox dir");
    }
}

pub fn remove_agent_sandbox_dir_preserving_home_sync(
    state_dir: &Path,
    session_id: &str,
    agent_id: &str,
) {
    let sandbox_dir = state_dir.join("sandboxes").join(session_id).join(agent_id);
    if let Err(err) = std::fs::remove_dir_all(&sandbox_dir)
        && err.kind() != io::ErrorKind::NotFound
    {
        tracing::warn!(
            ?sandbox_dir,
            error = %err,
            "failed to remove preserved-home agent sandbox dir"
        );
    }
}

pub fn sweep_stale_tmux_sockets_sync(
    current_socket_name: Option<&str>,
) -> Result<usize, CcbdError> {
    #[cfg(windows)]
    {
        let _ = current_socket_name;
        return Ok(0);
    }

    #[cfg(unix)]
    {
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
            if !(name.starts_with("ahd-") || name.starts_with("ccbd-"))
                || current_socket_name == Some(name.as_str())
            {
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
