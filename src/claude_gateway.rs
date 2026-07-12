use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
#[cfg(unix)]
use tokio::net::{TcpListener, TcpStream, UnixListener, UnixStream};

pub const INVALID_GRANT_ERROR_CODE: &str = "AH_CLAUDE_GATEWAY_REFRESH_INVALID_GRANT";
pub const REFRESH_FAILED_ERROR_CODE: &str = "AH_CLAUDE_GATEWAY_REFRESH_FAILED";
pub const WORKER_ID_MISMATCH_ERROR_CODE: &str = "AH_CLAUDE_GATEWAY_WORKER_ID_MISMATCH";
pub const AUTH_INVALID_ERROR_CODE: &str = "AH_CLAUDE_GATEWAY_AUTH_INVALID";
pub const SANDBOX_UDS_PATH: &str = "/var/run/ah-gateway.sock";
pub const GATEWAY_SANDBOX_ROOT_ENV: &str = "AH_CLAUDE_GATEWAY_SANDBOX_ROOT";
pub const FAILURE_CACHE_TTL: Duration = Duration::from_secs(30);
const TOKEN_EXPIRY_SAFETY_WINDOW: Duration = Duration::from_secs(5 * 60);
const HEADER_LIMIT: usize = 8 * 1024;
const BODY_LIMIT: usize = 10 * 1024 * 1024;
const IO_TIMEOUT: Duration = Duration::from_secs(15);
const CLAUDE_REFRESH_URL: &str = "https://platform.claude.com/v1/oauth/token";
const CLAUDE_MESSAGES_BASE_URL: &str = "https://api.anthropic.com";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenSet {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at: SystemTime,
}

impl TokenSet {
    fn is_valid_for_forwarding(&self) -> bool {
        self.expires_at
            .duration_since(SystemTime::now())
            .is_ok_and(|remaining| remaining > TOKEN_EXPIRY_SAFETY_WINDOW)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GatewayRequest {
    pub worker_id: String,
    pub method: String,
    pub path: String,
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

#[derive(Debug, Clone)]
struct CachedRefreshFailure {
    error: GatewayError,
    failed_at: Instant,
}

pub struct GatewayCore<U> {
    token: RwLock<TokenSet>,
    refresh_lock: Mutex<()>,
    refresh_failure: Mutex<Option<CachedRefreshFailure>>,
    upstream: Arc<U>,
    events: RecordedCredentialEvents,
}

impl<U: ClaudeUpstream> GatewayCore<U> {
    pub fn new(seed: TokenSet, upstream: Arc<U>, events: RecordedCredentialEvents) -> Self {
        Self {
            token: RwLock::new(seed),
            refresh_lock: Mutex::new(()),
            refresh_failure: Mutex::new(None),
            upstream,
            events,
        }
    }

    pub fn forward_messages(
        &self,
        request: GatewayRequest,
    ) -> Result<GatewayResponse, GatewayError> {
        let fake_token = validate_worker_identity(&request)?;
        let access_token = self.valid_access_token()?;
        let request = rewrite_authorization(request, &access_token, &fake_token);
        self.upstream.messages(request).map_err(map_upstream_error)
    }

    fn valid_access_token(&self) -> Result<String, GatewayError> {
        {
            let token = self.token.read().unwrap();
            if token.is_valid_for_forwarding() {
                return Ok(token.access_token.clone());
            }
        }

        let _guard = self.refresh_lock.lock().unwrap();
        {
            let token = self.token.read().unwrap();
            if token.is_valid_for_forwarding() {
                return Ok(token.access_token.clone());
            }
        }
        if let Some(cached) = self.cached_failure_inside_refresh_lock() {
            return Err(cached);
        }

        let refresh_token = self.token.read().unwrap().refresh_token.clone();
        match self.upstream.refresh(&refresh_token) {
            Ok(refreshed) => {
                let access_token = refreshed.access_token.clone();
                *self.token.write().unwrap() = refreshed;
                *self.refresh_failure.lock().unwrap() = None;
                Ok(access_token)
            }
            Err(err) => {
                let cache_failure = matches!(err, UpstreamError::InvalidGrant { .. });
                let gateway_error = map_refresh_error(err);
                if cache_failure {
                    *self.refresh_failure.lock().unwrap() = Some(CachedRefreshFailure {
                        error: gateway_error.clone(),
                        failed_at: Instant::now(),
                    });
                }
                self.events.record(CredentialEvent::RefreshFailed {
                    error_code: gateway_error.error_code.clone(),
                    upstream_error: refresh_event_name(&gateway_error),
                });
                Err(gateway_error)
            }
        }
    }

    fn cached_failure_inside_refresh_lock(&self) -> Option<GatewayError> {
        let cached = self.refresh_failure.lock().unwrap();
        cached.as_ref().and_then(|failure| {
            (failure.failed_at.elapsed() < FAILURE_CACHE_TTL).then(|| failure.error.clone())
        })
    }
}

pub fn validate_worker_identity(request: &GatewayRequest) -> Result<String, GatewayError> {
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
    format!(
        "{}.{}.",
        base64url_encode(header.as_bytes()),
        base64url_encode(payload.as_bytes())
    )
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
    let host_uds_path = short_host_uds_path(worker_sandbox_root, worker_id);
    validate_credential_path_not_wsl_windows_mount(&host_uds_path)?;
    Ok(GatewayWorkerTopology {
        worker_id: worker_id.to_string(),
        host_uds_path,
        sandbox_uds_path: PathBuf::from(SANDBOX_UDS_PATH),
    })
}

fn short_host_uds_path(worker_sandbox_root: &Path, worker_id: &str) -> PathBuf {
    let mut input = worker_sandbox_root.as_os_str().as_encoded_bytes().to_vec();
    input.push(0);
    input.extend_from_slice(worker_id.as_bytes());
    let digest = sha256_hex(&input);
    std::env::temp_dir().join(format!("ah-gw-{}.sock", &digest[..16]))
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub struct ClaudeGatewayService {
    core: Mutex<Option<Arc<GatewayCore<ProductionUpstream>>>>,
    listeners: Mutex<HashMap<String, GatewayListener>>,
    events: RecordedCredentialEvents,
    seed_path: PathBuf,
}

impl ClaudeGatewayService {
    pub fn new() -> Self {
        Self::new_with_seed_path(default_seed_path())
    }

    pub fn new_with_seed_path(seed_path: PathBuf) -> Self {
        Self {
            core: Mutex::new(None),
            listeners: Mutex::new(HashMap::new()),
            events: RecordedCredentialEvents::default(),
            seed_path,
        }
    }

    pub fn events(&self) -> RecordedCredentialEvents {
        self.events.clone()
    }

    pub async fn register_worker(
        &self,
        worker_id: &str,
        worker_sandbox_root: &Path,
    ) -> Result<GatewayWorkerTopology, String> {
        let topology = gateway_worker_topology(worker_sandbox_root, worker_id)?;
        self.register_listener(worker_id.to_string(), topology.clone())
            .await?;
        Ok(topology)
    }

    pub async fn register_master(
        &self,
        session_id: &str,
        master_sandbox_root: &Path,
    ) -> Result<GatewayWorkerTopology, String> {
        let topology = gateway_worker_topology(master_sandbox_root, session_id)?;
        self.register_listener(session_id.to_string(), topology.clone())
            .await?;
        Ok(topology)
    }

    pub async fn deregister(&self, key: &str) {
        let listener = self.listeners.lock().unwrap().remove(key);
        if let Some(listener) = listener {
            listener.shutdown().await;
        }
    }

    async fn register_listener(
        &self,
        key: String,
        topology: GatewayWorkerTopology,
    ) -> Result<(), String> {
        let core = self.core()?;
        let listener = register_worker(
            core,
            topology.worker_id.clone(),
            topology.host_uds_path.clone(),
        )
        .map_err(|err| format!("bind Claude gateway listener: {err}"))?;
        let old = self.listeners.lock().unwrap().insert(key, listener);
        if let Some(old) = old {
            old.shutdown().await;
        }
        Ok(())
    }

    fn core(&self) -> Result<Arc<GatewayCore<ProductionUpstream>>, String> {
        let mut core = self.core.lock().unwrap();
        if let Some(core) = core.as_ref() {
            return Ok(core.clone());
        }
        let seed = read_seed_credentials(&self.seed_path)?;
        let upstream = Arc::new(ProductionUpstream::new(self.seed_path.clone()));
        let initialized = Arc::new(GatewayCore::new(seed, upstream, self.events.clone()));
        *core = Some(initialized.clone());
        Ok(initialized)
    }
}

impl Default for ClaudeGatewayService {
    fn default() -> Self {
        Self::new()
    }
}

pub struct ProductionUpstream {
    credentials_path: PathBuf,
    refresh_url: String,
    messages_base_url: String,
}

impl ProductionUpstream {
    pub fn new(credentials_path: PathBuf) -> Self {
        Self {
            credentials_path,
            refresh_url: CLAUDE_REFRESH_URL.to_string(),
            messages_base_url: CLAUDE_MESSAGES_BASE_URL.to_string(),
        }
    }

    pub fn new_with_urls(
        credentials_path: PathBuf,
        refresh_url: String,
        messages_base_url: String,
    ) -> Self {
        Self {
            credentials_path,
            refresh_url,
            messages_base_url,
        }
    }
}

impl ClaudeUpstream for ProductionUpstream {
    fn refresh(&self, refresh_token: &str) -> UpstreamResult<TokenSet> {
        let response = ureq::post(&self.refresh_url)
            .timeout(Duration::from_secs(15))
            .set("content-type", "application/x-www-form-urlencoded")
            .send_form(&[
                ("grant_type", "refresh_token"),
                ("refresh_token", refresh_token),
            ]);
        let response = match response {
            Ok(response) => response,
            Err(ureq::Error::Status(status, response)) => {
                let body = response.into_string().unwrap_or_default();
                if status == 400 && response_body_is_invalid_grant(&body) {
                    return Err(UpstreamError::InvalidGrant { body });
                }
                return Err(UpstreamError::Http { status, body });
            }
            Err(err) => {
                return Err(UpstreamError::Http {
                    status: 502,
                    body: format!("refresh request failed: {err}"),
                });
            }
        };
        let body = response.into_string().map_err(|err| UpstreamError::Http {
            status: 502,
            body: format!("read refresh response failed: {err}"),
        })?;
        let json: Value = serde_json::from_str(&body).map_err(|err| UpstreamError::Http {
            status: 502,
            body: format!("parse refresh response failed: {err}"),
        })?;
        let access_token = json
            .get("access_token")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| UpstreamError::Http {
                status: 502,
                body: "refresh response missing access_token".to_string(),
            })?
            .to_string();
        let refresh_token = json
            .get("refresh_token")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| UpstreamError::Http {
                status: 502,
                body: "refresh response missing refresh_token".to_string(),
            })?
            .to_string();
        let expires_in = json
            .get("expires_in")
            .and_then(Value::as_u64)
            .ok_or_else(|| UpstreamError::Http {
                status: 502,
                body: "refresh response missing expires_in".to_string(),
            })?;
        let token = TokenSet {
            access_token,
            refresh_token,
            expires_at: SystemTime::now() + Duration::from_secs(expires_in),
        };
        write_seed_credentials_guarded(&self.credentials_path, &token).map_err(|err| {
            UpstreamError::Http {
                status: 502,
                body: format!("write refreshed credentials failed: {err}"),
            }
        })?;
        Ok(token)
    }

    fn messages(&self, request: GatewayRequest) -> UpstreamResult<GatewayResponse> {
        let url = format!("{}{}", self.messages_base_url, request.path);
        let method = request.method.to_ascii_uppercase();
        let mut upstream = match method.as_str() {
            "POST" => ureq::post(&url),
            "GET" => ureq::get(&url),
            _ => {
                return Err(UpstreamError::Http {
                    status: 400,
                    body: format!("unsupported gateway method: {}", request.method),
                });
            }
        }
        .timeout(Duration::from_secs(60));
        for (name, value) in request.headers {
            if name.eq_ignore_ascii_case("host") || name.eq_ignore_ascii_case("content-length") {
                continue;
            }
            upstream = upstream.set(&name, &value);
        }
        let result = if method == "GET" {
            upstream.call()
        } else {
            upstream.send_bytes(&request.body)
        };
        match result {
            Ok(response) => {
                let status = response.status();
                let headers = response
                    .headers_names()
                    .into_iter()
                    .filter_map(|name| {
                        response
                            .header(&name)
                            .map(|value| (name.to_ascii_lowercase(), value.to_string()))
                    })
                    .collect::<Vec<_>>();
                let mut reader = response.into_reader();
                let mut body = Vec::new();
                reader
                    .read_to_end(&mut body)
                    .map_err(|err| UpstreamError::Http {
                        status: 502,
                        body: format!("read messages response failed: {err}"),
                    })?;
                Ok(GatewayResponse {
                    status,
                    headers,
                    body,
                })
            }
            Err(ureq::Error::Status(status, response)) => {
                let body = response.into_string().unwrap_or_default();
                Err(UpstreamError::Http { status, body })
            }
            Err(err) => Err(UpstreamError::Http {
                status: 502,
                body: format!("messages request failed: {err}"),
            }),
        }
    }
}

pub fn read_seed_credentials(path: &Path) -> Result<TokenSet, String> {
    let content = std::fs::read_to_string(path).map_err(|err| {
        format!(
            "Claude seed credentials not found on host at {}: {err}; run /login",
            path.display()
        )
    })?;
    let value: Value = serde_json::from_str(&content).map_err(|err| {
        format!(
            "Claude seed credentials JSON is invalid at {}: {err}",
            path.display()
        )
    })?;
    let oauth = value
        .get("claudeAiOauth")
        .and_then(Value::as_object)
        .map(|_| &value["claudeAiOauth"])
        .unwrap_or(&value);
    let refresh_token = string_field(oauth, "refreshToken")
        .or_else(|| string_field(oauth, "refresh_token"))
        .ok_or_else(|| "Claude seed credentials missing refreshToken; run /login".to_string())?;
    let access_token = string_field(oauth, "accessToken")
        .or_else(|| string_field(oauth, "access_token"))
        .unwrap_or_default();
    let expires_at_ms = oauth
        .get("expiresAt")
        .or_else(|| oauth.get("expires_at"))
        .and_then(Value::as_i64)
        .unwrap_or(0);
    let expires_at = if expires_at_ms <= 0 {
        UNIX_EPOCH
    } else {
        UNIX_EPOCH + Duration::from_millis(expires_at_ms as u64)
    };
    Ok(TokenSet {
        access_token,
        refresh_token,
        expires_at,
    })
}

pub fn write_seed_credentials_guarded(path: &Path, token: &TokenSet) -> Result<(), String> {
    let canonical = match path.canonicalize() {
        Ok(path) => path,
        Err(_) if symlink_target_is_wsl_windows_mount(path) => {
            tracing::warn!(
                path = %path.display(),
                "skipping Claude seed credential writeback because symlink target resolves under /mnt/c"
            );
            return Ok(());
        }
        Err(_) => path.to_path_buf(),
    };
    if validate_credential_path_not_wsl_windows_mount(&canonical).is_err() {
        tracing::warn!(
            path = %canonical.display(),
            "skipping Claude seed credential writeback because target resolves under /mnt/c"
        );
        return Ok(());
    }
    let content = std::fs::read_to_string(path)
        .map_err(|err| format!("read existing Claude seed credentials before writeback: {err}"))?;
    let mut value: Value = serde_json::from_str(&content)
        .map_err(|err| format!("parse existing Claude seed credentials before writeback: {err}"))?;
    let target = if value
        .get("claudeAiOauth")
        .and_then(Value::as_object)
        .is_some()
    {
        value.get_mut("claudeAiOauth").unwrap()
    } else {
        &mut value
    };
    target["accessToken"] = Value::String(token.access_token.clone());
    target["refreshToken"] = Value::String(token.refresh_token.clone());
    target["expiresAt"] = Value::Number(serde_json::Number::from(
        token
            .expires_at
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64,
    ));
    let parent = path
        .parent()
        .ok_or_else(|| format!("credential path has no parent: {}", path.display()))?;
    let tmp = parent.join(format!(
        ".{}.tmp.{}",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("credentials.json"),
        std::process::id()
    ));
    let serialized = serde_json::to_vec_pretty(&value)
        .map_err(|err| format!("serialize Claude seed credentials: {err}"))?;
    std::fs::write(&tmp, serialized).map_err(|err| {
        format!(
            "write temporary Claude seed credentials {}: {err}",
            tmp.display()
        )
    })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))
            .map_err(|err| format!("chmod temporary Claude seed credentials: {err}"))?;
    }
    std::fs::rename(&tmp, path)
        .map_err(|err| format!("rename refreshed Claude seed credentials into place: {err}"))?;
    Ok(())
}

fn symlink_target_is_wsl_windows_mount(path: &Path) -> bool {
    let Ok(target) = std::fs::read_link(path) else {
        return false;
    };
    let resolved = if target.is_absolute() {
        target
    } else {
        path.parent().unwrap_or_else(|| Path::new(".")).join(target)
    };
    validate_credential_path_not_wsl_windows_mount(&resolved).is_err()
}

fn default_seed_path() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".claude/.credentials.json")
}

fn string_field(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn response_body_is_invalid_grant(body: &str) -> bool {
    serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|value| {
            value
                .get("error")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .as_deref()
        == Some("invalid_grant")
}

#[cfg(unix)]
pub struct GatewayListener {
    shutdown: tokio::sync::oneshot::Sender<()>,
    join: tokio::task::JoinHandle<()>,
}

#[cfg(not(unix))]
pub struct GatewayListener;

#[cfg(unix)]
impl GatewayListener {
    pub async fn shutdown(self) {
        let _ = self.shutdown.send(());
        let _ = self.join.await;
    }
}

#[cfg(not(unix))]
impl GatewayListener {
    pub async fn shutdown(self) {}
}

#[cfg(unix)]
pub fn register_worker<U: ClaudeUpstream>(
    core: Arc<GatewayCore<U>>,
    worker_id: String,
    host_uds_path: PathBuf,
) -> io::Result<GatewayListener> {
    if let Some(parent) = host_uds_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if host_uds_path.exists() {
        std::fs::remove_file(&host_uds_path)?;
    }
    let listener = UnixListener::bind(host_uds_path)?;
    let (shutdown, mut shutdown_rx) = tokio::sync::oneshot::channel();
    let join = tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = &mut shutdown_rx => break,
                accepted = listener.accept() => {
                    let Ok((stream, _addr)) = accepted else {
                        break;
                    };
                    let core = core.clone();
                    let worker_id = worker_id.clone();
                    tokio::spawn(async move {
                        let _ = handle_gateway_connection(core, worker_id, stream).await;
                    });
                }
            }
        }
    });
    Ok(GatewayListener { shutdown, join })
}

#[cfg(not(unix))]
pub fn register_worker<U: ClaudeUpstream>(
    _core: Arc<GatewayCore<U>>,
    _worker_id: String,
    _host_uds_path: PathBuf,
) -> io::Result<GatewayListener> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "Claude gateway UDS listeners are only supported on Unix",
    ))
}

#[cfg(unix)]
async fn handle_gateway_connection<U: ClaudeUpstream>(
    core: Arc<GatewayCore<U>>,
    worker_id: String,
    mut stream: UnixStream,
) -> io::Result<()> {
    let request = match tokio::time::timeout(IO_TIMEOUT, read_http_request(&mut stream, worker_id))
        .await
    {
        Ok(Ok(request)) => request,
        Ok(Err(err)) => {
            let _ = write_http_response(&mut stream, simple_response(err.status, &err.body)).await;
            return Ok(());
        }
        Err(_) => return Ok(()),
    };
    let response = match tokio::task::spawn_blocking(move || core.forward_messages(request)).await {
        Ok(Ok(response)) => response,
        Ok(Err(err)) => simple_response(err.status, &err.body),
        Err(err) => simple_response(500, &format!("gateway task failed: {err}")),
    };
    let _ = tokio::time::timeout(IO_TIMEOUT, write_http_response(&mut stream, response)).await;
    Ok(())
}

#[cfg(unix)]
async fn read_http_request(
    stream: &mut UnixStream,
    worker_id: String,
) -> Result<GatewayRequest, GatewayError> {
    let mut buf = Vec::new();
    let header_end = loop {
        let mut byte = [0_u8; 1];
        let n = stream
            .read(&mut byte)
            .await
            .map_err(|err| bad_request(&err.to_string()))?;
        if n == 0 {
            return Err(bad_request("connection closed before headers"));
        }
        buf.push(byte[0]);
        if buf.len() > HEADER_LIMIT {
            drain_oversized_headers(stream, &mut buf).await;
            return Err(bad_request("gateway request headers exceed 8KB"));
        }
        if buf.ends_with(b"\r\n\r\n") {
            break buf.len();
        }
    };
    let headers_raw = std::str::from_utf8(&buf[..header_end])
        .map_err(|_| bad_request("headers are not valid UTF-8"))?;
    let mut lines = headers_raw.split("\r\n");
    let request_line = lines
        .next()
        .ok_or_else(|| bad_request("missing request line"))?;
    let mut request_parts = request_line.split_whitespace();
    let method = request_parts
        .next()
        .ok_or_else(|| bad_request("missing method"))?
        .to_string();
    let path = request_parts
        .next()
        .ok_or_else(|| bad_request("missing path"))?
        .to_string();
    let mut headers = Vec::new();
    let mut content_length = 0_usize;
    for line in lines.filter(|line| !line.is_empty()) {
        let Some((name, value)) = line.split_once(':') else {
            return Err(bad_request("malformed header"));
        };
        let value = value.trim().to_string();
        if name.eq_ignore_ascii_case("content-length") {
            content_length = value
                .parse::<usize>()
                .map_err(|_| bad_request("invalid content-length"))?;
            if content_length > BODY_LIMIT {
                return Err(bad_request("gateway request body exceeds 10MB"));
            }
        }
        headers.push((name.trim().to_string(), value));
    }
    let mut body = vec![0_u8; content_length];
    if content_length > 0 {
        stream
            .read_exact(&mut body)
            .await
            .map_err(|err| bad_request(&err.to_string()))?;
    }
    Ok(GatewayRequest {
        worker_id,
        method,
        path,
        headers,
        body,
    })
}

#[cfg(unix)]
async fn drain_oversized_headers(stream: &mut UnixStream, buf: &mut Vec<u8>) {
    while buf.len() < HEADER_LIMIT * 2 && !buf.ends_with(b"\r\n\r\n") {
        let mut byte = [0_u8; 1];
        match tokio::time::timeout(Duration::from_millis(100), stream.read(&mut byte)).await {
            Ok(Ok(0)) | Ok(Err(_)) | Err(_) => break,
            Ok(Ok(_)) => buf.push(byte[0]),
        }
    }
}

#[cfg(unix)]
async fn write_http_response(stream: &mut UnixStream, response: GatewayResponse) -> io::Result<()> {
    let reason = reason_phrase(response.status);
    let mut out = format!("HTTP/1.1 {} {}\r\n", response.status, reason).into_bytes();
    let mut has_content_length = false;
    for (name, value) in response.headers {
        if name.eq_ignore_ascii_case("content-length") {
            has_content_length = true;
        }
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(b": ");
        out.extend_from_slice(value.as_bytes());
        out.extend_from_slice(b"\r\n");
    }
    if !has_content_length {
        out.extend_from_slice(format!("content-length: {}\r\n", response.body.len()).as_bytes());
    }
    out.extend_from_slice(b"\r\n");
    out.extend_from_slice(&response.body);
    stream.write_all(&out).await
}

#[cfg(unix)]
pub async fn run_internal_bridge(uds_path: &Path, port_file: Option<&Path>) -> io::Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();
    wait_for_uds_ready(uds_path).await?;
    if let Some(port_file) = port_file {
        std::fs::write(port_file, port.to_string())?;
    } else {
        println!("{port}");
    }
    loop {
        let (tcp, _addr) = listener.accept().await?;
        let uds_path = uds_path.to_path_buf();
        tokio::spawn(async move {
            let _ = bridge_one(tcp, uds_path).await;
        });
    }
}

#[cfg(not(unix))]
pub async fn run_internal_bridge(_uds_path: &Path, _port_file: Option<&Path>) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "Claude gateway internal bridge is only supported on Unix",
    ))
}

#[cfg(unix)]
async fn wait_for_uds_ready(uds_path: &Path) -> io::Result<()> {
    let mut last_err = None;
    for _ in 0..10 {
        match UnixStream::connect(uds_path).await {
            Ok(_stream) => return Ok(()),
            Err(err) => {
                last_err = Some(err);
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
    }
    Err(last_err.unwrap_or_else(|| io::Error::other("gateway UDS readiness probe failed")))
}

#[cfg(unix)]
async fn bridge_one(mut tcp: TcpStream, uds_path: PathBuf) -> io::Result<()> {
    let mut uds = UnixStream::connect(uds_path).await?;
    let _ = tokio::io::copy_bidirectional(&mut tcp, &mut uds).await?;
    Ok(())
}

pub fn bridge_wrapper_shell(
    inner_command: &str,
    ah_bin: &Path,
    uds_path: &Path,
    sandbox_root: &Path,
) -> String {
    let bridge_err = sandbox_root.join("bridge.err");
    let port_file = sandbox_root.join("bridge.port");
    format!(
        r#"rm -f {port_file}; {ah_bin} internal-bridge --uds {uds_path} --port-file {port_file} 2>{bridge_err} & bridge_pid=$!; i=0; while [ "$i" -lt 50 ]; do test -s {port_file} && break; i=$((i + 1)); sleep 0.1; done; if ! test -s {port_file}; then echo "bridge process did not write port file within 5s" >>{bridge_err}; kill "$bridge_pid" 2>/dev/null; exit 126; fi; port=$(cat {port_file}); ANTHROPIC_BASE_URL="http://localhost:$port" exec sh -lc {inner}"#,
        port_file = shell_quote_path(&port_file),
        ah_bin = shell_quote_path(ah_bin),
        uds_path = shell_quote_path(uds_path),
        bridge_err = shell_quote_path(&bridge_err),
        inner = shell_quote(inner_command),
    )
}

fn simple_response(status: u16, body: &str) -> GatewayResponse {
    GatewayResponse {
        status,
        headers: vec![("content-type".to_string(), "text/plain".to_string())],
        body: body.as_bytes().to_vec(),
    }
}

fn bad_request(body: &str) -> GatewayError {
    GatewayError {
        status: 400,
        error_code: "AH_CLAUDE_GATEWAY_BAD_REQUEST".to_string(),
        body: body.to_string(),
    }
}

fn reason_phrase(status: u16) -> &'static str {
    match status {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        500 => "Internal Server Error",
        _ => "Gateway Response",
    }
}

fn shell_quote_path(path: &Path) -> String {
    shell_quote(&path.display().to_string())
}

fn shell_quote(input: &str) -> String {
    format!("'{}'", input.replace('\'', "'\\''"))
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
            bits &= low_bits_mask(bit_count);
        }
    }
    Ok(out)
}

fn low_bits_mask(bit_count: u8) -> u32 {
    if bit_count == 0 {
        0
    } else {
        (1_u32 << bit_count) - 1
    }
}
