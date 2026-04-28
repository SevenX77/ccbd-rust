pub mod db;
mod env;
pub mod error;
pub mod marker;
pub mod monitor;
pub mod pty;
pub mod rpc;
pub mod sandbox;

use std::process::ExitCode;
use tracing_subscriber::EnvFilter;

#[tokio::main(flavor = "multi_thread")]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let sandbox_env = match sandbox::check_environment() {
        Ok(env_state) => env_state,
        Err(err) => {
            eprintln!("{}", err.to_rpc_error());
            return ExitCode::FAILURE;
        }
    };
    if !sandbox_env.unsafe_no_sandbox && std::env::var_os("INVOCATION_ID").is_none() {
        tracing::warn!(
            "ccbd not running under systemd; cascade cleanup will rely on Startup Reconcile only"
        );
    }

    let dir = env::resolve_state_dir();
    tracing::info!(?dir, "ccbd starting");

    let db_path = dir.join("ccbd.sqlite");
    match db::init(&db_path) {
        Ok(db) => {
            tracing::info!(?db_path, "database initialized");
            let reconcile_result = db::queries::reconcile_startup(&db);

            match reconcile_result {
                Ok(count) => {
                    tracing::info!(reconciled = count, "startup reconcile complete");
                    let socket_path = dir.join("ccbd.sock");
                    let ctx = rpc::Ctx {
                        db,
                        state_dir: dir.clone(),
                        env_state: sandbox_env,
                    };
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
