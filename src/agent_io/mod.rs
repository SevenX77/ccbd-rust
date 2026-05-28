pub mod reader;
pub mod registry;
pub mod writer;

use crate::error::CcbdError;

pub use reader::{
    ReaderMarkerConfig, spawn_agent_io_reader_task, spawn_agent_io_reader_task_with_config,
};
pub use registry::{
    AgentIoEntry, cleanup_agent_runtime_resources, contains, pane_id, register, remove,
    set_idle_scan_enabled,
};
pub use writer::send_text_to_pane;

pub async fn send_text_to_registered_pane(agent_id: &str, text: String) -> Result<(), CcbdError> {
    let Some((pane, socket_name)) = registry::pane_binding(agent_id) else {
        tracing::warn!(
            agent_id,
            "agent pane not registered; skipping text injection"
        );
        return Ok(());
    };
    let tmux = std::sync::Arc::new(crate::tmux::TmuxServer::from_socket_name(socket_name));
    send_text_to_pane(tmux, agent_id, pane, text).await
}

pub async fn shutdown_reader(agent_id: &str) -> Result<(), CcbdError> {
    registry::cleanup_agent_runtime_resources(agent_id);
    Ok(())
}
