use std::collections::HashMap;

pub(crate) const AH_AGENT_ID: &str = "AH_AGENT_ID";
pub(crate) const AH_ROLE: &str = "AH_ROLE";
pub(crate) const AH_SESSION_ID: &str = "AH_SESSION_ID";
pub(crate) const AH_ROLE_MASTER: &str = "master";
pub(crate) const AH_ROLE_WORKER: &str = "worker";

pub(crate) fn inject_worker_identity(
    env: &mut HashMap<String, String>,
    session_id: &str,
    agent_id: &str,
) {
    env.insert(AH_ROLE.to_string(), AH_ROLE_WORKER.to_string());
    env.insert(AH_SESSION_ID.to_string(), session_id.to_string());
    env.insert(AH_AGENT_ID.to_string(), agent_id.to_string());
}

pub(crate) fn inject_master_identity(env: &mut HashMap<String, String>, session_id: &str) {
    env.insert(AH_ROLE.to_string(), AH_ROLE_MASTER.to_string());
    env.insert(AH_SESSION_ID.to_string(), session_id.to_string());
    env.remove(AH_AGENT_ID);
}

#[cfg(test)]
mod tests {
    use super::{AH_AGENT_ID, AH_ROLE, AH_SESSION_ID};

    #[test]
    fn process_identity_vars_are_not_daemon_identity_vars() {
        let daemon_identity_vars = [
            "CCB_SOCKET",
            "AH_STATE_DIR",
            "CCBD_STATE_DIR",
            "XDG_STATE_HOME",
        ];

        for key in [AH_AGENT_ID, AH_SESSION_ID, AH_ROLE] {
            assert!(
                !daemon_identity_vars.contains(&key),
                "{key} is per-process identity, not daemon socket/state identity"
            );
        }
    }
}
