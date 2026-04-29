pub mod reader;
pub mod registry;
pub mod writer;

use crate::error::CcbdError;

pub use reader::spawn_agent_io_reader_task;
pub use registry::{AgentIoEntry, contains, pane_id, register, remove};
pub use writer::send_text_to_pane;

pub async fn shutdown_reader(agent_id: &str) -> Result<(), CcbdError> {
    if let Some(entry) = registry::remove(agent_id) {
        entry.reader_handle.abort();
        let _ = entry.reader_handle.await;
        if let Err(err) = std::fs::remove_file(&entry.fifo_path)
            && err.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!(agent_id, error = %err, "failed to remove agent fifo");
        }
    }
    Ok(())
}
