use serde_json::{Value, json};
use std::error::Error;
use std::fmt;
use std::future::Future;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::pin::Pin;

#[derive(Debug)]
pub enum CliError {
    Config(String),
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
            Self::Config(message) => write!(f, "{message}"),
            Self::DaemonNotRunning(path) => {
                write!(f, "ahd daemon is not running at {}", path.display())
            }
            Self::DaemonNotAccepting(path, err) => write!(
                f,
                "ahd daemon socket exists but is not accepting connections at {}: {}",
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

impl From<toml::de::Error> for CliError {
    fn from(err: toml::de::Error) -> Self {
        Self::Config(format!("invalid ah.toml: {err}"))
    }
}

pub type RpcFuture<'a> = Pin<Box<dyn Future<Output = Result<Value, CliError>> + Send + 'a>>;

pub trait RpcClient {
    fn call<'a>(&'a self, method: &'a str, params: Value) -> RpcFuture<'a>;
}

#[derive(Clone)]
pub struct UnixRpcClient {
    socket: PathBuf,
}

impl UnixRpcClient {
    pub fn new(socket: PathBuf) -> Self {
        Self { socket }
    }

    pub fn socket(&self) -> &Path {
        &self.socket
    }
}

impl RpcClient for UnixRpcClient {
    fn call<'a>(&'a self, method: &'a str, params: Value) -> RpcFuture<'a> {
        Box::pin(async move { rpc_call(&self.socket, method, params) })
    }
}

pub fn exit_code(err: &CliError) -> i32 {
    match err {
        CliError::DaemonNotRunning(_) | CliError::DaemonNotAccepting(_, _) => 1,
        CliError::Rpc { .. } => 2,
        CliError::InvalidJson(_) | CliError::InvalidResponse(_) | CliError::Config(_) => 3,
        CliError::Io(_) => 1,
    }
}

pub fn resolve_socket_path() -> PathBuf {
    resolve_socket_path_for_config(None)
}

pub fn resolve_socket_path_for_config(config_path: Option<&Path>) -> PathBuf {
    if let Ok(path) = std::env::var("CCB_SOCKET") {
        return PathBuf::from(path);
    }
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    crate::state_layout::resolve_state_layout(&crate::state_layout::StateLayoutRequest {
        cwd,
        config_path: config_path.map(Path::to_path_buf),
    })
    .state_dir
    .join("ahd.sock")
}

pub fn rpc_call(socket: &Path, method: &str, params: Value) -> Result<Value, CliError> {
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

pub fn rpc_stream_first(socket: &Path, method: &str, params: Value) -> Result<Value, CliError> {
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

    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line)?;
    if line.trim().is_empty() {
        return Err(CliError::InvalidResponse("empty stream response".into()));
    }
    serde_json::from_str(line.trim()).map_err(CliError::InvalidJson)
}
