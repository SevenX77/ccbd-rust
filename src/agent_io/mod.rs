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

pub async fn shutdown_reader(agent_id: &str) -> Result<(), CcbdError> {
    registry::cleanup_agent_runtime_resources(agent_id);
    Ok(())
}
