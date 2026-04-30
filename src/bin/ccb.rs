use ccbd::tmux::{SESSION_NAME, compute_socket_name};
use clap::{Parser, Subcommand};
use serde_json::{Value, json};
use std::error::Error;
use std::fmt;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use tabled::{Table, Tabled};

#[derive(Parser)]
#[command(name = "ccb", version, about = "Claude Code Bridge CLI")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Probe the daemon liveness.
    Ping,
    /// List sessions, agents, and pending evidence.
    Ps,
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
    /// Stream agent output events.
    Watch {
        agent_id: String,
        #[arg(long, default_value_t = 0)]
        since_event_id: i64,
    },
}

#[derive(Debug)]
enum CliError {
    DaemonNotRunning(PathBuf),
    DaemonNotAccepting(PathBuf, std::io::Error),
    Io(std::io::Error),
    Rpc { code: i64, message: String },
    InvalidJson(serde_json::Error),
    InvalidResponse(String),
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DaemonNotRunning(path) => {
                write!(f, "ccbd daemon is not running at {}", path.display())
            }
            Self::DaemonNotAccepting(path, err) => write!(
                f,
                "ccbd daemon socket exists but is not accepting connections at {}: {}",
                path.display(),
                err
            ),
            Self::Io(err) => write!(f, "I/O error: {err}"),
            Self::Rpc { code, message } => write!(f, "RPC error {code}: {message}"),
            Self::InvalidJson(err) => write!(f, "invalid JSON response from daemon: {err}"),
            Self::InvalidResponse(message) => write!(f, "invalid response from daemon: {message}"),
        }
    }
}

impl Error for CliError {}

impl From<std::io::Error> for CliError {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err)
    }
}

impl From<serde_json::Error> for CliError {
    fn from(err: serde_json::Error) -> Self {
        Self::InvalidJson(err)
    }
}

#[derive(Tabled)]
struct AgentRow {
    agent_id: String,
    provider: String,
    state: String,
    sub_state: String,
    pid: String,
}

fn main() {
    let cli = Cli::parse();
    let result = match cli.cmd {
        Cmd::Ping => cmd_ping(),
        Cmd::Ps => cmd_ps(),
        Cmd::Ask {
            agent_id,
            text,
            wait,
            request_id,
        } => cmd_ask(agent_id, text, wait, request_id),
        Cmd::Pend { job_id } => cmd_pend(job_id),
        Cmd::Watch {
            agent_id,
            since_event_id,
        } => cmd_watch(agent_id, since_event_id),
    };

    if let Err(err) = result {
        let code = exit_code(&err);
        eprintln!("\x1b[31m{err}\x1b[0m");
        if matches!(
            err,
            CliError::DaemonNotRunning(_) | CliError::DaemonNotAccepting(_, _)
        ) {
            eprintln!("Start it with: scripts/start-daemon.sh");
        }
        std::process::exit(code);
    }
}

fn exit_code(err: &CliError) -> i32 {
    match err {
        CliError::DaemonNotRunning(_) | CliError::DaemonNotAccepting(_, _) => 1,
        CliError::Rpc { .. } => 2,
        CliError::InvalidJson(_) | CliError::InvalidResponse(_) => 3,
        CliError::Io(_) => 1,
    }
}

fn cmd_ping() -> Result<(), CliError> {
    let socket = resolve_socket_path();
    let result = rpc_call(&socket, "system.dump", json!({}))?;
    let sessions = array_len(&result, "sessions");
    let agents = array_len(&result, "agents");

    println!("ok=true socket={}", socket.display());
    println!("sessions={sessions} agents={agents}");
    Ok(())
}

fn cmd_ps() -> Result<(), CliError> {
    let socket = resolve_socket_path();
    let result = rpc_call(&socket, "system.dump", json!({}))?;
    let rows = result
        .get("agents")
        .and_then(Value::as_array)
        .map(|agents| agents.iter().map(agent_row).collect::<Vec<_>>())
        .unwrap_or_default();

    println!("{}", Table::new(rows));

    let state_dir = socket.parent().ok_or_else(|| {
        CliError::InvalidResponse(format!(
            "socket path has no parent directory: {}",
            socket.display()
        ))
    })?;
    let tmux_socket = compute_socket_name(state_dir);
    println!();
    println!(
        "\x1b[2m💡 To inspect agents live: tmux -L {tmux_socket} attach -t {SESSION_NAME}\x1b[0m"
    );
    Ok(())
}

fn cmd_ask(
    agent_id: String,
    text: String,
    wait: bool,
    request_id: Option<String>,
) -> Result<(), CliError> {
    let socket = resolve_socket_path();
    let mut params = json!({
        "agent_id": agent_id,
        "text": text,
    });
    if let Some(request_id) = request_id {
        params["request_id"] = Value::String(request_id);
    }
    let result = rpc_call(&socket, "job.submit", params)?;
    let job_id = string_field(&result, "job_id");
    let status = string_field(&result, "status");
    println!("job_id={job_id} status={status}");
    if wait {
        print_terminal_job(wait_for_job(&socket, &job_id)?)?;
    }
    Ok(())
}

fn cmd_pend(job_id: String) -> Result<(), CliError> {
    let socket = resolve_socket_path();
    print_terminal_job(wait_for_job(&socket, &job_id)?)?;
    Ok(())
}

fn cmd_watch(agent_id: String, mut since_event_id: i64) -> Result<(), CliError> {
    let socket = resolve_socket_path();
    loop {
        let result = rpc_call(
            &socket,
            "agent.watch",
            json!({
                "agent_id": agent_id,
                "since_event_id": since_event_id,
                "timeout": 30,
            }),
        )?;
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

fn wait_for_job(socket: &Path, job_id: &str) -> Result<Value, CliError> {
    loop {
        match rpc_call(
            socket,
            "job.wait",
            json!({
                "job_id": job_id,
                "timeout": 30,
            }),
        ) {
            Ok(result) => {
                let status = string_field(&result, "status");
                if status == "COMPLETED" || status == "FAILED" {
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

fn print_terminal_job(result: Value) -> Result<(), CliError> {
    match result.get("status").and_then(Value::as_str) {
        Some("COMPLETED") => {
            if let Some(reply_text) = result.get("reply_text").and_then(Value::as_str) {
                print!("{reply_text}");
                std::io::stdout().flush()?;
            }
            Ok(())
        }
        Some("FAILED") => {
            let reason = result
                .get("error_reason")
                .and_then(Value::as_str)
                .unwrap_or("job failed");
            eprintln!("{reason}");
            std::process::exit(2);
        }
        Some(other) => Err(CliError::InvalidResponse(format!(
            "job.wait returned non-terminal status {other}"
        ))),
        None => Err(CliError::InvalidResponse(
            "job.wait missing status field".into(),
        )),
    }
}

fn parse_event_payload(event: &Value) -> Result<Value, CliError> {
    let payload = event
        .get("payload")
        .and_then(Value::as_str)
        .ok_or_else(|| CliError::InvalidResponse("event missing payload string".into()))?;
    serde_json::from_str(payload).map_err(CliError::InvalidJson)
}

fn resolve_socket_path() -> PathBuf {
    if let Ok(path) = std::env::var("CCB_SOCKET") {
        return PathBuf::from(path);
    }
    let dev_socket = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("dev_state")
        .join("ccbd.sock");
    if std::env::var("CCB_ENV").as_deref() == Ok("dev") {
        return dev_socket;
    }
    let xdg_socket = directories::ProjectDirs::from("", "", "ccbd")
        .expect("failed to resolve XDG project directories")
        .state_dir()
        .expect("failed to resolve XDG state directory")
        .join("ccbd.sock");
    xdg_socket
}

fn rpc_call(socket: &Path, method: &str, params: Value) -> Result<Value, CliError> {
    if !socket.exists() {
        return Err(CliError::DaemonNotRunning(socket.to_path_buf()));
    }

    let mut stream = UnixStream::connect(socket).map_err(|err| {
        if err.kind() == std::io::ErrorKind::ConnectionRefused {
            CliError::DaemonNotAccepting(socket.to_path_buf(), err)
        } else {
            CliError::Io(err)
        }
    })?;
    let request = json!({
        "jsonrpc": "2.0",
        "method": method,
        "params": params,
        "id": 1,
    });
    stream.write_all(request.to_string().as_bytes())?;
    stream.write_all(b"\n")?;
    stream.shutdown(std::net::Shutdown::Write)?;

    let mut raw = String::new();
    stream.read_to_string(&mut raw)?;
    let response: Value = serde_json::from_str(raw.trim())?;

    if let Some(error) = response.get("error") {
        let code = error.get("code").and_then(Value::as_i64).unwrap_or(-32000);
        let message = error
            .get("data")
            .and_then(|data| data.get("error_code"))
            .and_then(Value::as_str)
            .or_else(|| error.get("message").and_then(Value::as_str))
            .unwrap_or("unknown RPC error")
            .to_string();
        return Err(CliError::Rpc { code, message });
    }

    response
        .get("result")
        .cloned()
        .ok_or_else(|| CliError::InvalidResponse("missing result field".into()))
}

fn array_len(value: &Value, key: &str) -> usize {
    value.get(key).and_then(Value::as_array).map_or(0, Vec::len)
}

fn agent_row(agent: &Value) -> AgentRow {
    AgentRow {
        agent_id: string_field(agent, "id"),
        provider: string_field(agent, "provider"),
        state: string_field(agent, "state"),
        sub_state: option_string_field(agent, "sub_state"),
        pid: option_i64_field(agent, "pid"),
    }
}

fn string_field(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or("-")
        .to_string()
}

fn option_string_field(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or("-")
        .to_string()
}

fn option_i64_field(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_i64)
        .map(|n| n.to_string())
        .unwrap_or_else(|| "-".into())
}
