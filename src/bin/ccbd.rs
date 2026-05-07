use ccbd::{
    db, env, orchestrator, rpc, sandbox,
    tmux::{SESSION_NAME, TmuxServer},
};
use std::io;
use std::process::Command;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;
use tokio::signal::unix::{SignalKind, signal};
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
            let tmux_server = Arc::new(TmuxServer::new(&dir));
            ccbd::agent_io::registry::set_tmux_socket_name(tmux_server.socket_name().to_string());
            let reconcile_result = db::system::reconcile_startup_with_tmux_socket(
                db.clone(),
                dir.clone(),
                Some(tmux_server.socket_name().to_string()),
            )
            .await;

            match reconcile_result {
                Ok(count) => {
                    tracing::info!(reconciled = count, "startup reconcile complete");
                    let socket_path = dir.join("ccbd.sock");
                    let ctx = rpc::Ctx {
                        db,
                        state_dir: dir.clone(),
                        env_state: sandbox_env,
                        tmux_server,
                    };
                    orchestrator::spawn_orchestrator_task(ctx.clone());
                    run_until_shutdown(socket_path, ctx).await
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

async fn run_until_shutdown(socket_path: std::path::PathBuf, ctx: rpc::Ctx) -> ExitCode {
    let mut sigterm = match signal(SignalKind::terminate()) {
        Ok(signal) => signal,
        Err(err) => {
            tracing::error!(error = %err, "failed to register SIGTERM handler");
            return ExitCode::FAILURE;
        }
    };
    let mut sigint = match signal(SignalKind::interrupt()) {
        Ok(signal) => signal,
        Err(err) => {
            tracing::error!(error = %err, "failed to register SIGINT handler");
            return ExitCode::FAILURE;
        }
    };

    let shutdown_signal = async move {
        tokio::select! {
            _ = sigterm.recv() => tracing::info!("received SIGTERM"),
            _ = sigint.recv() => tracing::info!("received SIGINT"),
        }
    };

    tokio::select! {
        result = rpc::run_server(&socket_path, ctx.clone()) => {
            match result {
                Ok(()) => ExitCode::SUCCESS,
                Err(err) => {
                    tracing::error!(?socket_path, error = %err, "UDS server failed");
                    ExitCode::FAILURE
                }
            }
        }
        _ = shutdown_signal => {
            tracing::info!("shutdown initiated, cleaning tmux resources");
            cleanup_tmux_resources(&ctx).await;
            let _ = std::fs::remove_file(&socket_path);
            ExitCode::SUCCESS
        }
    }
}

async fn cleanup_tmux_resources(ctx: &rpc::Ctx) {
    let socket_name = ctx.tmux_server.socket_name().to_string();

    run_tmux_cleanup_command(
        Command::new("tmux")
            .args(["-L", &socket_name, "kill-session", "-t", SESSION_NAME])
            .output(),
        "tmux kill-session",
    );
    run_tmux_cleanup_command(
        Command::new("tmux")
            .args(["-L", &socket_name, "kill-server"])
            .output(),
        "tmux kill-server",
    );

    tokio::time::sleep(Duration::from_millis(50)).await;

    let socket_path = format!("/tmp/tmux-{}/{}", unsafe { libc::geteuid() }, socket_name);
    match std::fs::remove_file(&socket_path) {
        Ok(()) => {}
        Err(err) if err.kind() == io::ErrorKind::NotFound => {}
        Err(err) => tracing::warn!(error = %err, path = %socket_path, "socket file remove failed"),
    }
}

fn run_tmux_cleanup_command(result: io::Result<std::process::Output>, label: &str) {
    match result {
        Ok(output) if output.status.success() => {}
        Ok(output) => tracing::warn!(
            status = ?output.status,
            stderr = %String::from_utf8_lossy(&output.stderr).trim(),
            "{label} failed during shutdown"
        ),
        Err(err) => tracing::warn!(error = %err, "{label} failed during shutdown"),
    }
}
