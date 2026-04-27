use crate::error::CcbdError;
use portable_pty::{Child, CommandBuilder, PtySize, native_pty_system};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::{LazyLock, Mutex};

pub mod tasks;

pub static PTY_MAP: LazyLock<Mutex<HashMap<String, Box<dyn Write + Send>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

pub struct SpawnResult {
    pub pid: u32,
    pub master_reader: Box<dyn Read + Send>,
    pub child: Box<dyn Child + Send + Sync>,
}

pub fn spawn_agent(agent_id: &str, provider: &str) -> Result<SpawnResult, CcbdError> {
    {
        let pty_map = PTY_MAP
            .lock()
            .map_err(|_| CcbdError::PtyOpenFailed("PTY_MAP mutex poisoned".into()))?;
        if pty_map.contains_key(agent_id) {
            return Err(CcbdError::PtyOpenFailed(format!(
                "agent_id collision: {agent_id}"
            )));
        }
    }

    let pty_system = native_pty_system();
    let pty_pair = pty_system
        .openpty(PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|err| CcbdError::PtyOpenFailed(format!("openpty failed: {err}")))?;

    let cmd = CommandBuilder::new(provider);
    let mut child = pty_pair
        .slave
        .spawn_command(cmd)
        .map_err(|err| CcbdError::PtyOpenFailed(format!("spawn {provider}: {err}")))?;
    let pid = child
        .process_id()
        .ok_or_else(|| CcbdError::PtyOpenFailed("pid not available".into()))?;

    let master_reader = pty_pair
        .master
        .try_clone_reader()
        .map_err(|err| CcbdError::PtyOpenFailed(format!("clone pty reader: {err}")))?;
    let master_writer = pty_pair
        .master
        .take_writer()
        .map_err(|err| CcbdError::PtyOpenFailed(format!("take pty writer: {err}")))?;

    let mut pty_map = PTY_MAP
        .lock()
        .map_err(|_| CcbdError::PtyOpenFailed("PTY_MAP mutex poisoned".into()))?;
    if pty_map.contains_key(agent_id) {
        let _ = child.kill();
        return Err(CcbdError::PtyOpenFailed(format!(
            "agent_id collision: {agent_id}"
        )));
    }
    pty_map.insert(agent_id.to_string(), master_writer);

    Ok(SpawnResult {
        pid,
        master_reader,
        child,
    })
}

#[cfg(test)]
mod tests {
    use super::{PTY_MAP, spawn_agent};
    use crate::error::CcbdError;
    use std::io::Write;

    fn remove_writer(agent_id: &str) {
        PTY_MAP.lock().unwrap().remove(agent_id);
    }

    fn write_to_agent(agent_id: &str, bytes: &[u8]) {
        let mut pty_map = PTY_MAP.lock().unwrap();
        match pty_map.get_mut(agent_id) {
            Some(writer) => {
                writer.write_all(bytes).unwrap();
                writer.flush().unwrap();
            }
            None => panic!("missing PTY writer for {agent_id}"),
        }
    }

    #[test]
    fn test_spawn_bash_then_exit() {
        let agent_id = "ag_test";
        remove_writer(agent_id);

        let mut result = spawn_agent(agent_id, "bash").unwrap();
        assert!(result.pid > 0);
        write_to_agent(agent_id, b"exit\n");

        let status = result.child.wait().unwrap();
        remove_writer(agent_id);

        assert!(status.success(), "exit_code={}", status.exit_code());
    }

    #[test]
    fn test_spawn_collision() {
        let agent_id = "ag_collide";
        remove_writer(agent_id);

        let mut first = spawn_agent(agent_id, "bash").unwrap();
        let err = match spawn_agent(agent_id, "bash") {
            Ok(_) => panic!("expected agent_id collision"),
            Err(err) => err,
        };
        write_to_agent(agent_id, b"exit\n");
        let _ = first.child.wait();
        remove_writer(agent_id);

        assert!(
            matches!(err, CcbdError::PtyOpenFailed(message) if message.contains("agent_id collision"))
        );
    }
}
