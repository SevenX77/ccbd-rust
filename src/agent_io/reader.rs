use std::fs::File;
#[cfg(unix)]
use std::io::Read;
#[cfg(unix)]
use tokio::io::unix::AsyncFd;
use tokio::sync::mpsc;

pub fn spawn_agent_io_reader_task(
    agent_id: String,
    fifo: File,
    output_tx: mpsc::Sender<Vec<u8>>,
) -> tokio::task::JoinHandle<()> {
    #[cfg(windows)]
    {
        let _ = (fifo, output_tx);
        return tokio::spawn(async move {
            tracing::warn!(
                agent_id = %agent_id,
                "Windows agent IO stream reader is not implemented until M2"
            );
        });
    }

    #[cfg(unix)]
    tokio::spawn(async move {
        let async_fifo = match AsyncFd::new(fifo) {
            Ok(fd) => fd,
            Err(err) => {
                tracing::warn!(agent_id = %agent_id, error = %err, "failed to create AsyncFd for fifo");
                return;
            }
        };
        let mut buf = vec![0_u8; 8192];

        loop {
            let mut guard = match async_fifo.readable().await {
                Ok(guard) => guard,
                Err(err) => {
                    tracing::warn!(agent_id = %agent_id, error = %err, "fifo readiness wait failed");
                    break;
                }
            };

            let read_result = guard.try_io(|inner| {
                let mut file = inner.get_ref();
                file.read(&mut buf)
            });
            let n = match read_result {
                Ok(Ok(0)) => break,
                Ok(Ok(n)) => n,
                Ok(Err(err)) => {
                    tracing::warn!(agent_id = %agent_id, error = %err, "fifo read failed");
                    break;
                }
                Err(_would_block) => continue,
            };

            if output_tx.send(buf[..n].to_vec()).await.is_err() {
                tracing::warn!(agent_id = %agent_id, "agent IO output receiver dropped");
                break;
            }
        }
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn reader_source_stays_below_persistence_and_marker_layers() {
        let source = include_str!("reader.rs");

        for forbidden in [
            concat!("crate::", "db"),
            concat!("crate::", "marker"),
            concat!("use crate::", "db"),
            concat!("use crate::", "marker"),
        ] {
            assert!(
                !source.contains(forbidden),
                "agent_io::reader must not depend on {forbidden}"
            );
        }
    }
}
