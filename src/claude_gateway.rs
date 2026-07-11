use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
use std::time::SystemTime;

pub const INVALID_GRANT_ERROR_CODE: &str = "AH_CLAUDE_GATEWAY_REFRESH_INVALID_GRANT";
pub const REFRESH_FAILED_ERROR_CODE: &str = "AH_CLAUDE_GATEWAY_REFRESH_FAILED";
pub const WORKER_ID_MISMATCH_ERROR_CODE: &str = "AH_CLAUDE_GATEWAY_WORKER_ID_MISMATCH";
pub const AUTH_INVALID_ERROR_CODE: &str = "AH_CLAUDE_GATEWAY_AUTH_INVALID";
pub const SANDBOX_TCP_BASE_URL: &str = "http://localhost:8206";
pub const SANDBOX_UDS_PATH: &str = "/var/run/ah-gateway.sock";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenSet {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at: SystemTime,
}

impl TokenSet {
    fn is_expired(&self) -> bool {
        SystemTime::now() >= self.expires_at
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GatewayRequest {
    pub worker_id: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GatewayResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GatewayError {
    pub status: u16,
    pub error_code: String,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpstreamError {
    InvalidGrant { body: String },
    Http { status: u16, body: String },
}

pub type UpstreamResult<T> = Result<T, UpstreamError>;

pub trait ClaudeUpstream: Send + Sync + 'static {
    fn refresh(&self, refresh_token: &str) -> UpstreamResult<TokenSet>;
    fn messages(&self, request: GatewayRequest) -> UpstreamResult<GatewayResponse>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GatewayWorkerTopology {
    pub worker_id: String,
    pub host_uds_path: PathBuf,
    pub sandbox_uds_path: PathBuf,
    pub sandbox_tcp_base_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CredentialEvent {
    RefreshFailed {
        error_code: String,
        upstream_error: String,
    },
}

#[derive(Debug, Default, Clone)]
pub struct RecordedCredentialEvents {
    inner: Arc<Mutex<Vec<CredentialEvent>>>,
}

impl RecordedCredentialEvents {
    pub fn record(&self, event: CredentialEvent) {
        self.inner.lock().unwrap().push(event);
    }

    pub fn snapshot(&self) -> Vec<CredentialEvent> {
        self.inner.lock().unwrap().clone()
    }
}

pub struct GatewayCore<U> {
    token: RwLock<TokenSet>,
    refresh_lock: Mutex<()>,
    upstream: Arc<U>,
    events: RecordedCredentialEvents,
}

impl<U: ClaudeUpstream> GatewayCore<U> {
    pub fn new(seed: TokenSet, upstream: Arc<U>, events: RecordedCredentialEvents) -> Self {
        Self {
            token: RwLock::new(seed),
            refresh_lock: Mutex::new(()),
            upstream,
            events,
        }
    }

    pub fn forward_messages(&self, request: GatewayRequest) -> Result<GatewayResponse, GatewayError> {
        let fake_token = validate_worker_identity(&request)?;
        let access_token = self.valid_access_token()?;
        let request = rewrite_authorization(request, &access_token, &fake_token);
        self.upstream.messages(request).map_err(map_upstream_error)
    }

    fn valid_access_token(&self) -> Result<String, GatewayError> {
        {
            let token = self.token.read().unwrap();
            if !token.is_expired() {
                return Ok(token.access_token.clone());
            }
        }

        let _guard = self.refresh_lock.lock().unwrap();
        {
            let token = self.token.read().unwrap();
            if !token.is_expired() {
                return Ok(token.access_token.clone());
            }
        }

        let refresh_token = self.token.read().unwrap().refresh_token.clone();
        match self.upstream.refresh(&refresh_token) {
            Ok(refreshed) => {
                let access_token = refreshed.access_token.clone();
                *self.token.write().unwrap() = refreshed;
                Ok(access_token)
            }
            Err(err) => {
                let gateway_error = map_refresh_error(err);
                self.events.record(CredentialEvent::RefreshFailed {
                    error_code: gateway_error.error_code.clone(),
                    upstream_error: refresh_event_name(&gateway_error),
                });
                Err(gateway_error)
            }
        }
    }
}

fn validate_worker_identity(request: &GatewayRequest) -> Result<String, GatewayError> {
    let token = bearer_token(request).ok_or_else(|| GatewayError {
        status: 401,
        error_code: AUTH_INVALID_ERROR_CODE.to_string(),
        body: "missing bearer token".to_string(),
    })?;
    let jwt_worker_id = fake_jwt_worker_id(token).map_err(|err| GatewayError {
        status: 401,
        error_code: AUTH_INVALID_ERROR_CODE.to_string(),
        body: err,
    })?;
    if jwt_worker_id != request.worker_id {
        return Err(GatewayError {
            status: 403,
            error_code: WORKER_ID_MISMATCH_ERROR_CODE.to_string(),
            body: format!(
                "worker identity mismatch: channel={} token={jwt_worker_id}",
                request.worker_id
            ),
        });
    }
    Ok(token.to_string())
}

fn bearer_token(request: &GatewayRequest) -> Option<&str> {
    request
        .headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("authorization"))
        .and_then(|(_, value)| value.strip_prefix("Bearer "))
}

fn rewrite_authorization(
    mut request: GatewayRequest,
    access_token: &str,
    fake_token: &str,
) -> GatewayRequest {
    request.headers.retain(|(name, value)| {
        !name.eq_ignore_ascii_case("authorization") && !value.contains(fake_token)
    });
    request.headers.push((
        "authorization".to_string(),
        format!("Bearer {access_token}"),
    ));
    request
}

fn map_refresh_error(err: UpstreamError) -> GatewayError {
    match err {
        UpstreamError::InvalidGrant { body } => GatewayError {
            status: 401,
            error_code: INVALID_GRANT_ERROR_CODE.to_string(),
            body,
        },
        UpstreamError::Http { status, body } => GatewayError {
            status,
            error_code: REFRESH_FAILED_ERROR_CODE.to_string(),
            body,
        },
    }
}

fn map_upstream_error(err: UpstreamError) -> GatewayError {
    match err {
        UpstreamError::InvalidGrant { body } => GatewayError {
            status: 401,
            error_code: INVALID_GRANT_ERROR_CODE.to_string(),
            body,
        },
        UpstreamError::Http { status, body } => GatewayError {
            status,
            error_code: "AH_CLAUDE_GATEWAY_UPSTREAM_HTTP".to_string(),
            body,
        },
    }
}

fn refresh_event_name(error: &GatewayError) -> String {
    if error.error_code == INVALID_GRANT_ERROR_CODE {
        "invalid_grant".to_string()
    } else {
        error.error_code.clone()
    }
}

pub fn fake_worker_jwt(worker_id: &str) -> String {
    let header = r#"{"alg":"none","typ":"JWT"}"#;
    let payload = format!(
        r#"{{"exp":32503680000,"sub":"ah-worker-session","worker_id":"{}"}}"#,
        json_escape(worker_id)
    );
    format!("{}.{}.", base64url_encode(header.as_bytes()), base64url_encode(payload.as_bytes()))
}

pub fn fake_jwt_worker_id(token: &str) -> Result<String, String> {
    let parts = token.split('.').collect::<Vec<_>>();
    if parts.len() != 3 || !parts[2].is_empty() {
        return Err("fake gateway JWT must be alg:none three-segment token".to_string());
    }
    let header = decode_json_segment(parts[0])?;
    if header.get("alg").and_then(Value::as_str) != Some("none")
        || header.get("typ").and_then(Value::as_str) != Some("JWT")
    {
        return Err("fake gateway JWT header is not alg:none JWT".to_string());
    }
    let payload = decode_json_segment(parts[1])?;
    if payload.get("exp").and_then(Value::as_i64) != Some(32_503_680_000) {
        return Err("fake gateway JWT exp does not match design contract".to_string());
    }
    if payload.get("sub").and_then(Value::as_str) != Some("ah-worker-session") {
        return Err("fake gateway JWT subject does not match design contract".to_string());
    }
    payload
        .get("worker_id")
        .and_then(Value::as_str)
        .filter(|worker_id| !worker_id.is_empty())
        .map(str::to_string)
        .ok_or_else(|| "fake gateway JWT missing worker_id".to_string())
}

pub fn validate_credential_path_not_wsl_windows_mount(path: &Path) -> Result<(), String> {
    let path = path.to_string_lossy();
    if path == "/mnt/c" || path.starts_with("/mnt/c/") {
        return Err("credential path resolves under /mnt/c".to_string());
    }
    Ok(())
}

pub fn gateway_worker_topology(
    worker_sandbox_root: &Path,
    worker_id: &str,
) -> Result<GatewayWorkerTopology, String> {
    validate_credential_path_not_wsl_windows_mount(worker_sandbox_root)?;
    let host_uds_path = worker_sandbox_root.join("tmp/ah-gateway.sock");
    validate_credential_path_not_wsl_windows_mount(&host_uds_path)?;
    Ok(GatewayWorkerTopology {
        worker_id: worker_id.to_string(),
        host_uds_path,
        sandbox_uds_path: PathBuf::from(SANDBOX_UDS_PATH),
        sandbox_tcp_base_url: SANDBOX_TCP_BASE_URL.to_string(),
    })
}

fn decode_json_segment(segment: &str) -> Result<Value, String> {
    let bytes = base64url_decode(segment)?;
    serde_json::from_slice(&bytes).map_err(|err| format!("invalid fake gateway JWT JSON: {err}"))
}

fn json_escape(input: &str) -> String {
    input
        .chars()
        .flat_map(|ch| match ch {
            '"' => "\\\"".chars().collect::<Vec<_>>(),
            '\\' => "\\\\".chars().collect::<Vec<_>>(),
            '\n' => "\\n".chars().collect::<Vec<_>>(),
            '\r' => "\\r".chars().collect::<Vec<_>>(),
            '\t' => "\\t".chars().collect::<Vec<_>>(),
            ch => vec![ch],
        })
        .collect()
}

fn base64url_encode(input: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::new();
    for chunk in input.chunks(3) {
        let b0 = chunk[0];
        let b1 = *chunk.get(1).unwrap_or(&0);
        let b2 = *chunk.get(2).unwrap_or(&0);
        let n = ((b0 as u32) << 16) | ((b1 as u32) << 8) | b2 as u32;
        out.push(TABLE[((n >> 18) & 0x3f) as usize] as char);
        out.push(TABLE[((n >> 12) & 0x3f) as usize] as char);
        if chunk.len() > 1 {
            out.push(TABLE[((n >> 6) & 0x3f) as usize] as char);
        }
        if chunk.len() > 2 {
            out.push(TABLE[(n & 0x3f) as usize] as char);
        }
    }
    out
}

fn base64url_decode(input: &str) -> Result<Vec<u8>, String> {
    let mut bits = 0_u32;
    let mut bit_count = 0_u8;
    let mut out = Vec::new();
    for byte in input.bytes() {
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'-' => 62,
            b'_' => 63,
            _ => return Err(format!("invalid base64url byte {byte}")),
        } as u32;
        bits = (bits << 6) | value;
        bit_count += 6;
        if bit_count >= 8 {
            bit_count -= 8;
            out.push(((bits >> bit_count) & 0xff) as u8);
        }
    }
    Ok(out)
}
