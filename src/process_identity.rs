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
    unsafe {
        std::env::set_var(AH_ROLE, AH_ROLE_WORKER);
        std::env::set_var(AH_SESSION_ID, session_id);
        std::env::set_var(AH_AGENT_ID, agent_id);
    }
}

pub(crate) fn inject_master_identity(env: &mut HashMap<String, String>, session_id: &str) {
    env.insert(AH_ROLE.to_string(), AH_ROLE_MASTER.to_string());
    env.insert(AH_SESSION_ID.to_string(), session_id.to_string());
    env.remove(AH_AGENT_ID);
    unsafe {
        std::env::set_var(AH_ROLE, AH_ROLE_MASTER);
        std::env::set_var(AH_SESSION_ID, session_id);
        std::env::remove_var(AH_AGENT_ID);
    }
}

#[cfg(test)]
mod tests {
    use super::{AH_AGENT_ID, AH_ROLE, AH_SESSION_ID};
    use std::collections::HashMap;

    struct EnvGuard {
        old_role: Option<String>,
        old_session_id: Option<String>,
        old_agent_id: Option<String>,
    }

    impl EnvGuard {
        fn new() -> Self {
            Self {
                old_role: std::env::var(AH_ROLE).ok(),
                old_session_id: std::env::var(AH_SESSION_ID).ok(),
                old_agent_id: std::env::var(AH_AGENT_ID).ok(),
            }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            unsafe {
                if let Some(val) = &self.old_role {
                    std::env::set_var(AH_ROLE, val);
                } else {
                    std::env::remove_var(AH_ROLE);
                }
                if let Some(val) = &self.old_session_id {
                    std::env::set_var(AH_SESSION_ID, val);
                } else {
                    std::env::remove_var(AH_SESSION_ID);
                }
                if let Some(val) = &self.old_agent_id {
                    std::env::set_var(AH_AGENT_ID, val);
                } else {
                    std::env::remove_var(AH_AGENT_ID);
                }
            }
        }
    }

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

    #[test]
    fn test_inherited_env_leak_reproduction() {
        let _guard = EnvGuard::new();

        // 1. Simulate the daemon-inherited-env case for MASTER child
        unsafe {
            std::env::set_var(AH_AGENT_ID, "bogus");
            std::env::set_var(AH_ROLE, "worker");
            std::env::set_var(AH_SESSION_ID, "old_session");
        }

        let mut master_env = HashMap::new();
        master_env.insert(AH_AGENT_ID.to_string(), "bogus".to_string());
        master_env.insert(AH_ROLE.to_string(), "worker".to_string());
        master_env.insert(AH_SESSION_ID.to_string(), "old_session".to_string());

        super::inject_master_identity(&mut master_env, "correct_session");

        // Assert map is correct
        assert_eq!(master_env.get(AH_ROLE).map(|s| s.as_str()), Some("master"));
        assert_eq!(master_env.get(AH_SESSION_ID).map(|s| s.as_str()), Some("correct_session"));
        assert_eq!(master_env.get(AH_AGENT_ID), None);

        // Assert process env (representing the child inheriting) is correct
        assert_eq!(std::env::var(AH_ROLE).ok().as_deref(), Some("master"));
        assert_eq!(std::env::var(AH_SESSION_ID).ok().as_deref(), Some("correct_session"));
        assert_eq!(std::env::var(AH_AGENT_ID).ok(), None);

        // 2. Simulate the daemon-inherited-env case for WORKER child
        unsafe {
            std::env::set_var(AH_AGENT_ID, "bogus");
            std::env::set_var(AH_ROLE, "master");
            std::env::set_var(AH_SESSION_ID, "old_session");
        }

        let mut worker_env = HashMap::new();
        worker_env.insert(AH_AGENT_ID.to_string(), "bogus".to_string());
        worker_env.insert(AH_ROLE.to_string(), "master".to_string());
        worker_env.insert(AH_SESSION_ID.to_string(), "old_session".to_string());

        super::inject_worker_identity(&mut worker_env, "correct_session", "correct_agent");

        // Assert map is correct
        assert_eq!(worker_env.get(AH_ROLE).map(|s| s.as_str()), Some("worker"));
        assert_eq!(worker_env.get(AH_SESSION_ID).map(|s| s.as_str()), Some("correct_session"));
        assert_eq!(worker_env.get(AH_AGENT_ID).map(|s| s.as_str()), Some("correct_agent"));

        // Assert process env is correct
        assert_eq!(std::env::var(AH_ROLE).ok().as_deref(), Some("worker"));
        assert_eq!(std::env::var(AH_SESSION_ID).ok().as_deref(), Some("correct_session"));
        assert_eq!(std::env::var(AH_AGENT_ID).ok().as_deref(), Some("correct_agent"));
    }
}
