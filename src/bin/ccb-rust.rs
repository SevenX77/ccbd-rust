use ccbd::cli::config_cmd::{migrate_stub, run_config_validate};
use ccbd::cli::doctor::{has_failures, print_doctor, run_doctor};
use ccbd::cli::logs::run_logs;
use ccbd::cli::output::{
    agent_row, array_len, parse_event_payload, print_terminal_job, print_tmux_hint, session_row,
    string_field,
};
use ccbd::cli::prompt::{PromptResolveOptions, run_prompt_resolve};
use ccbd::cli::rpc_client::{
    CliError, RpcClient, UnixRpcClient, exit_code, resolve_socket_path_for_config,
};
use ccbd::cli::start::{StartOptions, print_start_summary, start_from_options};
use ccbd::tmux::{agent_session_name, compute_socket_name};
use clap::{Parser, Subcommand};
use serde_json::{Value, json};
use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};
use tabled::Table;

#[derive(Parser)]
#[command(name = "ccb-rust", version, about = "Claude Code Bridge CLI")]
struct Cli {
    /// Path to ccb.toml.
    #[arg(long, global = true)]
    config: Option<PathBuf>,
    #[command(subcommand)]
    cmd: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Probe the daemon liveness.
    Ping,
    /// Print the CLI version.
    Version,
    /// List sessions, agents, and pending evidence.
    Ps,
    /// Start a project from ccb.toml.
    Start {
        #[arg(long)]
        wait: bool,
    },
    /// Submit an ask job to an agent.
    Ask {
        agent_id: String,
        text: String,
        #[arg(long)]
        wait: bool,
        #[arg(long)]
        request_id: Option<String>,
    },
    /// Wait for a submitted job to finish.
    Pend { job_id: String },
    /// Cancel a queued or running job.
    Cancel { job_id: String },
    /// Kill an agent, or a whole session with --session.
    Kill {
        target_id: String,
        #[arg(long)]
        session: bool,
        #[arg(long)]
        force: bool,
    },
    /// Stream agent output events.
    Watch {
        agent_id: String,
        #[arg(long, default_value_t = 0)]
        since_event_id: i64,
    },
    /// Print stored output for an agent.
    Logs {
        agent_id: String,
        #[arg(long, default_value_t = 0)]
        since: i64,
    },
    /// Attach to an agent's tmux session.
    Attach {
        /// Agent id (e.g. "a1"). Maps to tmux session "agent_<agent_id>".
        agent_id: String,
    },
    /// Shut down the daemon gracefully.
    Stop,
    /// Run local environment diagnostics.
    Doctor,
    /// Validate or migrate project configuration.
    Config {
        #[command(subcommand)]
        cmd: ConfigCmd,
    },
    /// Resolve an interactive prompt.
    Prompt {
        #[command(subcommand)]
        cmd: PromptCmd,
    },
    /// Generate per-project PATH shadow opt-in files.
    ProjectInit {
        /// Project root to opt in.
        project: PathBuf,
        /// Path to the ccb-rust binary. Defaults to this executable.
        #[arg(long)]
        ccb_rust: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
enum ConfigCmd {
    /// Validate a ccb.toml file.
    Validate {
        #[arg(long)]
        config: PathBuf,
    },
    /// Print migration guidance for legacy .ccb/ccb.config.
    Migrate,
}

#[derive(Subcommand)]
enum PromptCmd {
    /// Send an action to a PROMPT_PENDING agent.
    Resolve {
        agent_id: String,
        #[arg(long, conflicts_with = "keys")]
        action: Option<String>,
        #[arg(long, conflicts_with = "action")]
        keys: Option<String>,
        #[arg(long)]
        save_to_kb: bool,
    },
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let cli = Cli::parse();
    let socket = resolve_socket_path_for_config(cli.config.as_deref());
    let client = UnixRpcClient::new(socket);
    let result = match cli.cmd {
        None => default_action(&client, cli.config).await,
        Some(Cmd::Ping) => cmd_ping(&client).await,
        Some(Cmd::Version) => {
            println!("{}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Some(Cmd::Ps) => cmd_ps(&client).await,
        Some(Cmd::Start { wait }) => cmd_start(&client, cli.config, wait).await,
        Some(Cmd::Ask {
            agent_id,
            text,
            wait,
            request_id,
        }) => cmd_ask(&client, agent_id, text, wait, request_id).await,
        Some(Cmd::Pend { job_id }) => cmd_pend(&client, job_id).await,
        Some(Cmd::Cancel { job_id }) => cmd_cancel(&client, job_id).await,
        Some(Cmd::Kill {
            target_id,
            session,
            force,
        }) => cmd_kill(&client, target_id, session, force).await,
        Some(Cmd::Watch {
            agent_id,
            since_event_id,
        }) => cmd_watch(&client, agent_id, since_event_id).await,
        Some(Cmd::Logs { agent_id, since }) => run_logs(&client, &agent_id, since).await,
        Some(Cmd::Attach { agent_id }) => cmd_attach(&client, &agent_id),
        Some(Cmd::Stop) => cmd_stop(&client).await,
        Some(Cmd::Doctor) => cmd_doctor(&client).await,
        Some(Cmd::Config { cmd }) => match cmd {
            ConfigCmd::Validate { config } => run_config_validate(&config),
            ConfigCmd::Migrate => cmd_config_migrate(),
        },
        Some(Cmd::Prompt { cmd }) => match cmd {
            PromptCmd::Resolve {
                agent_id,
                action,
                keys,
                save_to_kb,
            } => {
                run_prompt_resolve(
                    &client,
                    PromptResolveOptions {
                        agent_id,
                        action_json: action,
                        keys,
                        save_to_kb,
                    },
                )
                .await
            }
        },
        Some(Cmd::ProjectInit { project, ccb_rust }) => cmd_project_init(project, ccb_rust),
    };

    if let Err(err) = result {
        let code = exit_code(&err);
        eprintln!("\x1b[31m{err}\x1b[0m");
        if matches!(
            err,
            CliError::DaemonNotRunning(_) | CliError::DaemonNotAccepting(_, _)
        ) {
            eprintln!("Start it with: ccb-rust start");
        }
        std::process::exit(code);
    }
}

async fn default_action(client: &UnixRpcClient, config: Option<PathBuf>) -> Result<(), CliError> {
    check_nested_environment()?;
    ensure_daemon_running(client.socket())?;
    let cwd = std::env::current_dir()?;
    let summary = start_from_options(
        client,
        StartOptions {
            config_path: config,
            cwd,
            wait: true,
        },
    )
    .await?;
    print_start_summary(&summary);
    println!("Session ready. Attach via: ccb-rust attach <agent_id>");
    Ok(())
}

fn ensure_daemon_running(socket: &Path) -> Result<(), CliError> {
    if socket.exists() {
        if std::os::unix::net::UnixStream::connect(socket).is_ok() {
            return Ok(());
        }
        eprintln!("Removing stale socket {}", socket.display());
        let _ = std::fs::remove_file(socket);
    }

    let ccbd_bin = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|dir| dir.join("ccbd")))
        .filter(|p| p.is_file())
        .ok_or_else(|| {
            CliError::Config("cannot locate ccbd binary next to ccb-rust".to_string())
        })?;

    let state_dir = socket.parent().unwrap();
    std::fs::create_dir_all(state_dir).map_err(|e| {
        CliError::Config(format!(
            "failed to create state dir {}: {e}",
            state_dir.display()
        ))
    })?;

    if std::env::var("CCB_ENV").as_deref() == Ok("dev") {
        for ext in ["", "-wal", "-shm"] {
            let p = state_dir.join(format!("ccbd.sqlite{ext}"));
            let _ = std::fs::remove_file(&p);
        }
    }

    let log_path = state_dir.join("ccbd.log");
    let log_file = std::fs::File::create(&log_path).map_err(|e| {
        CliError::Config(format!("failed to create log {}: {e}", log_path.display()))
    })?;

    eprintln!("Starting ccbd daemon (log: {})...", log_path.display());
    Command::new(&ccbd_bin)
        .stdin(std::process::Stdio::null())
        .stdout(log_file.try_clone().unwrap())
        .stderr(log_file)
        .env_remove("INVOCATION_ID")
        .env("CCBD_STATE_DIR", state_dir)
        .spawn()
        .map_err(|e| CliError::Config(format!("failed to spawn ccbd: {e}")))?;

    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if socket.exists() && std::os::unix::net::UnixStream::connect(socket).is_ok() {
            eprintln!("ccbd daemon ready.");
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Err(CliError::Config(format!(
        "ccbd failed to start within 10s (socket {} not accepting connections)",
        socket.display()
    )))
}

fn check_nested_environment() -> Result<(), CliError> {
    let tmux_env = std::env::var("TMUX").ok();
    let cgroup_data = std::fs::read_to_string("/proc/self/cgroup").unwrap_or_default();
    if let Some(reason) = detect_nesting(tmux_env.as_deref(), &cgroup_data) {
        return Err(CliError::Config(format!(
            "Agent Nesting Forbidden: 当前已在 ccbd-rust 环境内, 不能再启动 ccb-rust ({reason})"
        )));
    }
    Ok(())
}

fn detect_nesting(tmux_env: Option<&str>, cgroup_data: &str) -> Option<String> {
    if let Some(tmux_env) = tmux_env
        && (tmux_env.contains("/ccbd-") || tmux_env.starts_with("ccbd-"))
    {
        return Some("via TMUX env".to_string());
    }
    if cgroup_data.contains("/ccb-") || cgroup_data.contains("ccbd-agent-") {
        return Some("via cgroup".to_string());
    }
    None
}

fn attach_session_name(agent_id: &str) -> String {
    agent_session_name(agent_id)
}

fn cmd_attach(client: &UnixRpcClient, agent_id: &str) -> Result<(), CliError> {
    let socket = tmux_socket_path_from_daemon_socket(client.socket())?;
    if !socket.exists() {
        return Err(CliError::Config(format!(
            "tmux socket not found: {}. Is a session running?",
            socket.display()
        )));
    }
    exec_tmux_attach(socket, attach_session_name(agent_id))
}

async fn cmd_stop(client: &UnixRpcClient) -> Result<(), CliError> {
    client.call("system.shutdown", json!({})).await?;
    eprintln!("ccbd shutting down.");
    Ok(())
}

async fn cmd_ping(client: &UnixRpcClient) -> Result<(), CliError> {
    let result = client.call("system.dump", json!({})).await?;
    let sessions = array_len(&result, "sessions");
    let agents = array_len(&result, "agents");

    println!("ok=true socket={}", client.socket().display());
    println!("sessions={sessions} agents={agents}");
    Ok(())
}

async fn cmd_ps(client: &UnixRpcClient) -> Result<(), CliError> {
    let sessions = client.call("session.list", json!({})).await?;
    let session_rows = sessions
        .get("sessions")
        .and_then(Value::as_array)
        .map(|sessions| sessions.iter().map(session_row).collect::<Vec<_>>())
        .unwrap_or_default();
    println!("sessions");
    println!("{}", Table::new(session_rows));

    let dump = client.call("system.dump", json!({})).await?;
    let rows = dump
        .get("agents")
        .and_then(Value::as_array)
        .map(|agents| agents.iter().map(agent_row).collect::<Vec<_>>())
        .unwrap_or_default();
    println!();
    println!("agents");
    println!("{}", Table::new(rows));
    print_tmux_hint(client.socket())
}

async fn cmd_doctor(client: &UnixRpcClient) -> Result<(), CliError> {
    let cwd = std::env::current_dir()?;
    let checks = run_doctor(client, &cwd).await?;
    print_doctor(&checks);
    if has_failures(&checks) {
        Err(CliError::Config("doctor found failed checks".into()))
    } else {
        Ok(())
    }
}

fn cmd_config_migrate() -> Result<(), CliError> {
    let cwd = std::env::current_dir()?;
    migrate_stub(&cwd)
}

fn cmd_project_init(project: PathBuf, ccb_rust: Option<PathBuf>) -> Result<(), CliError> {
    let project_root = canonical_existing_dir(&project, "project")?;
    let ccb_rust_bin = match ccb_rust {
        Some(path) => canonical_existing_file(&path, "ccb-rust")?,
        None => std::env::current_exe().map_err(CliError::Io)?,
    };
    let bin_dir = ccb_rust_bin
        .parent()
        .ok_or_else(|| {
            CliError::Config(format!(
                "ccb-rust path has no parent directory: {}",
                ccb_rust_bin.display()
            ))
        })?
        .to_path_buf();
    let ccbd_bin = bin_dir.join("ccbd");
    if !ccbd_bin.is_file() {
        return Err(CliError::Config(format!(
            "ccbd binary must live next to ccb-rust: {}",
            ccbd_bin.display()
        )));
    }

    let opt_in_dir = project_root.join(".ccb-rust");
    let shadow_bin_dir = opt_in_dir.join("bin");
    fs::create_dir_all(&shadow_bin_dir).map_err(|err| {
        CliError::Config(format!(
            "failed to create opt-in bin dir {}: {err}",
            shadow_bin_dir.display()
        ))
    })?;

    let wrapper_path = shadow_bin_dir.join("ccb");
    fs::write(
        &wrapper_path,
        render_project_ccb_wrapper(&project_root, &ccb_rust_bin),
    )
    .map_err(|err| {
        CliError::Config(format!(
            "failed to write wrapper {}: {err}",
            wrapper_path.display()
        ))
    })?;
    fs::set_permissions(&wrapper_path, fs::Permissions::from_mode(0o755))?;

    let activate_path = opt_in_dir.join("activate.sh");
    fs::write(&activate_path, render_project_activate(&project_root)).map_err(|err| {
        CliError::Config(format!(
            "failed to write activate script {}: {err}",
            activate_path.display()
        ))
    })?;
    fs::set_permissions(&activate_path, fs::Permissions::from_mode(0o644))?;

    println!("project={}", project_root.display());
    println!("wrapper={}", wrapper_path.display());
    println!("activate={}", activate_path.display());
    println!("ccb_rust={}", ccb_rust_bin.display());
    println!("ccbd={}", ccbd_bin.display());
    Ok(())
}

fn canonical_existing_dir(path: &Path, label: &str) -> Result<PathBuf, CliError> {
    let canonical = path.canonicalize().map_err(|err| {
        CliError::Config(format!(
            "{label} path does not exist or cannot be resolved: {}: {err}",
            path.display()
        ))
    })?;
    if !canonical.is_dir() {
        return Err(CliError::Config(format!(
            "{label} path is not a directory: {}",
            canonical.display()
        )));
    }
    Ok(canonical)
}

fn canonical_existing_file(path: &Path, label: &str) -> Result<PathBuf, CliError> {
    let canonical = path.canonicalize().map_err(|err| {
        CliError::Config(format!(
            "{label} path does not exist or cannot be resolved: {}: {err}",
            path.display()
        ))
    })?;
    if !canonical.is_file() {
        return Err(CliError::Config(format!(
            "{label} path is not a file: {}",
            canonical.display()
        )));
    }
    Ok(canonical)
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn render_project_ccb_wrapper(project_root: &Path, ccb_rust_bin: &Path) -> String {
    let project_root = shell_quote(&project_root.display().to_string());
    let ccb_rust_bin = shell_quote(&ccb_rust_bin.display().to_string());
    format!(
        r#"#!/usr/bin/env bash
set -euo pipefail

project_root={project_root}
ccb_rust_bin={ccb_rust_bin}
ccbd_bin="$(dirname "$ccb_rust_bin")/ccbd"

if [ ! -x "$ccb_rust_bin" ]; then
  echo "ccb-rust opt-in: missing executable $ccb_rust_bin" >&2
  exit 127
fi
if [ ! -x "$ccbd_bin" ]; then
  echo "ccb-rust opt-in: ccbd must live next to ccb-rust: $ccbd_bin" >&2
  exit 127
fi

unset CCB_SOCKET
unset CCB_ENV
export CCB_CONFIG_PATH="${{project_root}}/ccb.toml"

exec "$ccb_rust_bin" "$@"
"#
    )
}

fn render_project_activate(project_root: &Path) -> String {
    let project_root = shell_quote(&project_root.display().to_string());
    format!(
        r#"#!/usr/bin/env bash
project_root={project_root}
shadow_bin="${{project_root}}/.ccb-rust/bin"

case ":${{PATH}}:" in
  *":${{shadow_bin}}:"*) ;;
  *) export PATH="${{shadow_bin}}:${{PATH}}" ;;
esac
"#
    )
}

async fn cmd_start(
    client: &UnixRpcClient,
    config: Option<PathBuf>,
    wait: bool,
) -> Result<(), CliError> {
    ensure_daemon_running(client.socket())?;
    let cwd = std::env::current_dir()?;
    let summary = start_from_options(
        client,
        StartOptions {
            config_path: config,
            cwd,
            wait,
        },
    )
    .await?;
    print_start_summary(&summary);
    Ok(())
}

async fn cmd_ask(
    client: &UnixRpcClient,
    agent_id: String,
    text: String,
    wait: bool,
    request_id: Option<String>,
) -> Result<(), CliError> {
    let mut params = json!({
        "agent_id": agent_id,
        "text": text,
    });
    if let Some(request_id) = request_id {
        params["request_id"] = Value::String(request_id);
    }
    let result = client.call("job.submit", params).await?;
    let job_id = string_field(&result, "job_id");
    let status = string_field(&result, "status");
    println!("job_id={job_id} status={status}");
    if wait {
        print_terminal_job(wait_for_job(client, &job_id).await?)?;
    }
    Ok(())
}

async fn cmd_pend(client: &UnixRpcClient, job_id: String) -> Result<(), CliError> {
    print_terminal_job(wait_for_job(client, &job_id).await?)?;
    Ok(())
}

async fn cmd_cancel(client: &UnixRpcClient, job_id: String) -> Result<(), CliError> {
    let result = client
        .call(
            "job.cancel",
            json!({
                "job_id": job_id,
            }),
        )
        .await?;
    let job_id = string_field(&result, "job_id");
    let status = string_field(&result, "status");
    println!("job_id={job_id} status={status}");
    Ok(())
}

async fn cmd_kill(
    client: &UnixRpcClient,
    target_id: String,
    session: bool,
    force: bool,
) -> Result<(), CliError> {
    let (method, params) = if session {
        (
            "session.kill",
            json!({
                "session_id": target_id,
                "force": force,
            }),
        )
    } else {
        (
            "agent.kill",
            json!({
                "agent_id": target_id,
            }),
        )
    };
    let result = client.call(method, params).await?;
    println!("state={}", string_field(&result, "state"));
    Ok(())
}

async fn cmd_watch(
    client: &UnixRpcClient,
    agent_id: String,
    mut since_event_id: i64,
) -> Result<(), CliError> {
    loop {
        let result = client
            .call(
                "agent.watch",
                json!({
                    "agent_id": agent_id,
                    "since_event_id": since_event_id,
                    "timeout": 30,
                }),
            )
            .await?;
        let Some(events) = result.get("events").and_then(Value::as_array) else {
            return Err(CliError::InvalidResponse(
                "agent.watch missing events array".into(),
            ));
        };
        for event in events {
            if let Some(seq_id) = event.get("seq_id").and_then(Value::as_i64) {
                since_event_id = since_event_id.max(seq_id);
            }
            match event.get("event_type").and_then(Value::as_str) {
                Some("output_chunk") => {
                    let payload = parse_event_payload(event)?;
                    if let Some(text) = payload.get("text").and_then(Value::as_str) {
                        print!("{text}");
                        std::io::stdout().flush()?;
                    }
                }
                Some("state_change") => {
                    let payload = parse_event_payload(event)?;
                    println!("\n--- state_change {} ---", payload);
                }
                _ => {}
            }
        }
    }
}

async fn wait_for_job(client: &UnixRpcClient, job_id: &str) -> Result<Value, CliError> {
    loop {
        match client
            .call(
                "job.wait",
                json!({
                    "job_id": job_id,
                    "timeout": 30,
                }),
            )
            .await
        {
            Ok(result) => {
                let status = string_field(&result, "status");
                if status == "COMPLETED" || status == "FAILED" || status == "CANCELLED" {
                    return Ok(result);
                }
            }
            Err(CliError::Rpc { code, message }) if message == "PTY_IO_ERROR" => {
                let _ = code;
                continue;
            }
            Err(err) => return Err(err),
        }
    }
}

fn tmux_socket_path_from_daemon_socket(socket: &Path) -> Result<PathBuf, CliError> {
    let state_dir = socket.parent().ok_or_else(|| {
        CliError::InvalidResponse(format!(
            "socket path has no parent directory: {}",
            socket.display()
        ))
    })?;
    let socket_name = compute_socket_name(state_dir);
    Ok(PathBuf::from(format!(
        "/tmp/tmux-{}/{}",
        unsafe { libc::geteuid() },
        socket_name
    )))
}

fn prepare_attach_command(socket: &Path, session_name: &str) -> Vec<String> {
    vec![
        "tmux".to_string(),
        "-S".to_string(),
        socket.display().to_string(),
        "attach".to_string(),
        "-t".to_string(),
        session_name.to_string(),
    ]
}

fn exec_tmux_attach(socket: PathBuf, session_name: String) -> ! {
    use std::os::unix::process::CommandExt;
    let cmd = prepare_attach_command(&socket, &session_name);
    let err = std::process::Command::new(&cmd[0]).args(&cmd[1..]).exec();
    eprintln!("exec tmux attach failed: {err}");
    std::process::exit(1);
}

#[cfg(test)]
mod tests {
    use super::{
        attach_session_name, cmd_project_init, detect_nesting, prepare_attach_command,
        render_project_activate, render_project_ccb_wrapper, shell_quote,
    };
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;

    #[test]
    fn test_prepare_attach_command_returns_tmux_attach() {
        let session_name = attach_session_name("a1");
        let cmd = prepare_attach_command(Path::new("/tmp/tmux-1001/ccbd-test"), &session_name);

        assert_eq!(
            cmd,
            vec![
                "tmux".to_string(),
                "-S".to_string(),
                "/tmp/tmux-1001/ccbd-test".to_string(),
                "attach".to_string(),
                "-t".to_string(),
                "agent_a1".to_string(),
            ]
        );
    }

    #[test]
    fn test_attach_session_name_maps_agent_id() {
        assert_eq!(attach_session_name("a1"), "agent_a1");
        assert_eq!(attach_session_name("agent-42"), "agent_agent-42");
    }

    #[test]
    fn test_check_nested_environment_detects_tmux_var() {
        let reason = detect_nesting(Some("/tmp/tmux-1001/ccbd-1234567890abcdef,1,0"), "");

        assert!(reason.unwrap().contains("TMUX"));
    }

    #[test]
    fn test_check_nested_environment_detects_cgroup() {
        let reason = detect_nesting(None, "0::/user.slice/ccb-project-ccbd-agents.slice\n");

        assert!(reason.unwrap().contains("cgroup"));
    }

    #[test]
    fn test_check_nested_environment_passes_normal() {
        assert_eq!(detect_nesting(None, "0::/user.slice/session.scope\n"), None);
    }

    #[test]
    fn test_project_wrapper_forwards_all_args_and_sets_config() {
        let wrapper = render_project_ccb_wrapper(
            Path::new("/tmp/project"),
            Path::new("/opt/ccbd-rust/bin/ccb-rust"),
        );

        assert!(wrapper.contains("unset CCB_SOCKET"));
        assert!(wrapper.contains("unset CCB_ENV"));
        assert!(wrapper.contains("export CCB_CONFIG_PATH=\"${project_root}/ccb.toml\""));
        assert!(wrapper.contains("exec \"$ccb_rust_bin\" \"$@\""));
        assert!(wrapper.contains("ccbd_bin=\"$(dirname \"$ccb_rust_bin\")/ccbd\""));
    }

    #[test]
    fn test_project_activate_prepends_shadow_path_once() {
        let activate = render_project_activate(Path::new("/tmp/project"));

        assert!(activate.contains("shadow_bin=\"${project_root}/.ccb-rust/bin\""));
        assert!(activate.contains("export PATH=\"${shadow_bin}:${PATH}\""));
    }

    #[test]
    fn test_shell_quote_handles_single_quotes() {
        assert_eq!(shell_quote("/tmp/has'quote"), "'/tmp/has'\\''quote'");
    }

    #[test]
    fn test_project_init_writes_shadow_files() {
        let project = tempfile::tempdir().unwrap();
        let bin = tempfile::tempdir().unwrap();
        let ccb_rust = bin.path().join("ccb-rust");
        let ccbd = bin.path().join("ccbd");
        fs::write(&ccb_rust, "#!/bin/sh\n").unwrap();
        fs::write(&ccbd, "#!/bin/sh\n").unwrap();
        fs::set_permissions(&ccb_rust, fs::Permissions::from_mode(0o755)).unwrap();
        fs::set_permissions(&ccbd, fs::Permissions::from_mode(0o755)).unwrap();

        cmd_project_init(project.path().to_path_buf(), Some(ccb_rust.clone())).unwrap();

        let wrapper = project.path().join(".ccb-rust/bin/ccb");
        let activate = project.path().join(".ccb-rust/activate.sh");
        let wrapper_text = fs::read_to_string(&wrapper).unwrap();
        assert!(wrapper_text.contains(&format!(
            "ccb_rust_bin={}",
            shell_quote(&ccb_rust.canonicalize().unwrap().display().to_string())
        )));
        assert!(
            fs::read_to_string(&activate)
                .unwrap()
                .contains(".ccb-rust/bin")
        );
        assert_ne!(
            fs::metadata(wrapper).unwrap().permissions().mode() & 0o111,
            0
        );
    }
}
