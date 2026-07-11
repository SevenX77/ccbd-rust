use std::path::Path;
use std::sync::{Arc, Mutex, RwLock};
use std::time::SystemTime;

pub const INVALID_GRANT_ERROR_CODE: &str = "AH_CLAUDE_GATEWAY_REFRESH_INVALID_GRANT";
pub const REFRESH_FAILED_ERROR_CODE: &str = "AH_CLAUDE_GATEWAY_REFRESH_FAILED";

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
        let access_token = self.valid_access_token()?;
        let request = rewrite_authorization(request, &access_token);
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

fn rewrite_authorization(mut request: GatewayRequest, access_token: &str) -> GatewayRequest {
    request.headers.retain(|(name, value)| {
        !name.eq_ignore_ascii_case("authorization") && !value.contains("ah-fake-jwt")
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
    let worker = worker_id
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    format!("ah-fake-jwt.{worker}.long-lived")
}

pub fn validate_credential_path_not_wsl_windows_mount(path: &Path) -> Result<(), String> {
    let path = path.to_string_lossy();
    if path == "/mnt/c" || path.starts_with("/mnt/c/") {
        return Err("credential path resolves under /mnt/c".to_string());
    }
    Ok(())
}
