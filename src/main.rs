pub mod db;
mod env;
pub mod error;
pub mod pty;
pub mod rpc;

use std::process::ExitCode;
use tracing_subscriber::EnvFilter;

#[tokio::main(flavor = "multi_thread")]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();
    let dir = env::resolve_state_dir();
    tracing::info!(?dir, "ccbd starting");

    let db_path = dir.join("ccbd.sqlite");
    match db::init(&db_path) {
        Ok(db) => {
            tracing::info!(?db_path, "database initialized");
            let reconcile_result = {
                let mut conn = db.conn();
                db::queries::reconcile_active_agents_to_crashed(&mut conn)
            };

            match reconcile_result {
                Ok(count) => {
                    tracing::info!(reconciled = count, "startup reconcile complete");
                    let socket_path = dir.join("ccbd.sock");
                    let ctx = rpc::Ctx { db };
                    match rpc::run_server(&socket_path, ctx).await {
                        Ok(()) => ExitCode::SUCCESS,
                        Err(err) => {
                            tracing::error!(?socket_path, error = %err, "UDS server failed");
                            ExitCode::FAILURE
                        }
                    }
                }
                Err(err) => {
                    tracing::error!(error = %err, "startup reconcile failed");
                    ExitCode::FAILURE
                }
            }
        }
        Err(err) => {
            tracing::error!(?db_path, error = %err, "database initialization failed");
            ExitCode::FAILURE
        }
    }
}
