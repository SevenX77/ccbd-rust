use crate::db::Db;
use crate::sandbox::EnvState;
use crate::tmux::TmuxServer;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;

pub mod handlers;
pub mod router;

#[derive(Clone)]
pub struct Ctx {
    pub db: Db,
    pub state_dir: PathBuf,
    pub env_state: EnvState,
    pub daemon_unit: Option<String>,
    pub tmux_server: Arc<TmuxServer>,
}

pub async fn run_server(socket_path: &Path, ctx: Ctx) -> io::Result<()> {
    if socket_path.exists() {
        let _ = std::fs::remove_file(socket_path);
    }

    let listener = UnixListener::bind(socket_path)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        std::fs::set_permissions(socket_path, std::fs::Permissions::from_mode(0o600))?;
    }

    tracing::info!(?socket_path, "UDS RPC server listening");

    loop {
        let (stream, _) = listener.accept().await?;
        let ctx = ctx.clone();

        tokio::spawn(async move {
            let (reader, mut writer) = stream.into_split();
            let mut reader = BufReader::new(reader);
            let mut line = String::new();

            loop {
                line.clear();
                match reader.read_line(&mut line).await {
                    Ok(0) => break,
                    Ok(_) => {
                        let response = router::dispatch(line.trim_end(), &ctx).await;
                        if writer.write_all(response.as_bytes()).await.is_err() {
                            break;
                        }
                        if writer.write_all(b"\n").await.is_err() {
                            break;
                        }
                    }
                    Err(err) => {
                        tracing::warn!(error = %err, "UDS read failed");
                        break;
                    }
                }
            }
        });
    }
}
