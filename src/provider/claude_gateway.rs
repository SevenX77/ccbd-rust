use std::collections::HashMap;
use std::fs;
use std::io::Read;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
#[cfg(unix)]
use tokio::net::UnixListener;
use tokio::sync::RwLock;

const CHARSET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

fn base64url_encode(input: &[u8]) -> String {
    let mut result = String::with_capacity((input.len() + 2) / 3 * 4);
    let mut temp;
    let mut i = 0;
    while i + 3 <= input.len() {
        temp = ((input[i] as u32) << 16) | ((input[i + 1] as u32) << 8) | (input[i + 2] as u32);
        result.push(CHARSET[((temp >> 18) & 0x3F) as usize] as char);
        result.push(CHARSET[((temp >> 12) & 0x3F) as usize] as char);
        result.push(CHARSET[((temp >> 6) & 0x3F) as usize] as char);
        result.push(CHARSET[(temp & 0x3F) as usize] as char);
        i += 3;
    }
    let remaining = input.len() - i;
    if remaining == 1 {
        temp = (input[i] as u32) << 16;
        result.push(CHARSET[((temp >> 18) & 0x3F) as usize] as char);
        result.push(CHARSET[((temp >> 12) & 0x3F) as usize] as char);
    } else if remaining == 2 {
        temp = ((input[i] as u32) << 16) | ((input[i + 1] as u32) << 8);
        result.push(CHARSET[((temp >> 18) & 0x3F) as usize] as char);
        result.push(CHARSET[((temp >> 12) & 0x3F) as usize] as char);
        result.push(CHARSET[((temp >> 6) & 0x3F) as usize] as char);
    }
    result
}

fn match_val(c: char) -> Result<u32, &'static str> {
    match c {
        'A'..='Z' => Ok(c as u32 - 'A' as u32),
        'a'..='z' => Ok(c as u32 - 'a' as u32 + 26),
        '0'..='9' => Ok(c as u32 - '0' as u32 + 52),
        '-' => Ok(62),
        '_' => Ok(63),
        _ => Err("Invalid base64url character"),
    }
}

fn base64url_decode(input: &str) -> Result<Vec<u8>, &'static str> {
    let mut buffer = Vec::new();
    let mut accum = 0u32;
    let mut bits = 0;
    for c in input.chars() {
        if c == '=' {
            break;
        }
        let val = match_val(c)?;
        accum = (accum << 6) | val;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            buffer.push((accum >> bits) as u8);
        }
    }
    Ok(buffer)
}

use std::sync::OnceLock;

static GLOBAL_SECRET: OnceLock<String> = OnceLock::new();

fn get_global_secret() -> &'static str {
    GLOBAL_SECRET.get_or_init(|| {
        uuid::Uuid::new_v4().to_string()
    })
}

fn calculate_signature(worker_id: &str, exp: u64, sub: &str) -> String {
    use sha2::{Sha256, Digest};
    let secret = get_global_secret();
    let data = format!("{}:{}:{}:{}", secret, worker_id, exp, sub);
    let mut hasher = Sha256::new();
    hasher.update(data.as_bytes());
    base64url_encode(&hasher.finalize())
}


#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FakeClaims {
    pub exp: u64,
    pub sub: String,
    pub worker_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
}

pub fn build_fake_worker_jwt_for_test(worker_id: &str) -> Result<String, String> {
    let header = "eyJhbGciOiJub25lIiwidHlwIjoiSldUIn0"; // {"alg":"none","typ":"JWT"}
    let exp = 32503680000;
    let sub = "ah-worker-session";
    let sig = calculate_signature(worker_id, exp, sub);
    let claims = FakeClaims {
        exp,
        sub: sub.to_string(),
        worker_id: worker_id.to_string(),
        signature: Some(sig),
    };
    let claims_json = serde_json::to_string(&claims).map_err(|e| e.to_string())?;
    let payload = base64url_encode(claims_json.as_bytes());
    Ok(format!("{header}.{payload}."))
}

pub fn decode_fake_worker_jwt_claims(jwt: &str) -> Result<FakeClaims, String> {
    let parts: Vec<&str> = jwt.split('.').collect();
    if parts.len() < 3 {
        return Err("Invalid JWT format".to_string());
    }
    let payload_b64 = parts[1];
    let payload_bytes = base64url_decode(payload_b64).map_err(|e| e.to_string())?;
    let claims: FakeClaims = serde_json::from_slice(&payload_bytes).map_err(|e| e.to_string())?;
    
    if let Some(ref sig) = claims.signature {
        let expected = calculate_signature(&claims.worker_id, claims.exp, &claims.sub);
        if sig != &expected {
            return Err("Invalid signature".to_string());
        }
    } else {
        return Err("Missing signature".to_string());
    }
    
    Ok(claims)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum CredentialFailureCode {
    SeedRefreshInvalidGrant,
}

impl CredentialFailureCode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::SeedRefreshInvalidGrant => "SeedRefreshInvalidGrant",
        }
    }
}

#[derive(Debug, Clone)]
pub enum GatewayBind {
    PerWorkerUds {
        socket_root: PathBuf,
        sandbox_uds_path: PathBuf,
        bridge_port: u16,
    },
}

#[derive(Debug, Clone)]
pub struct SeedCredential {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at: SystemTime,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct WorkerGatewayEnv {
    pub base_url: String,
    pub auth_token: String,
    pub sandbox_uds_path: PathBuf,
    pub bridge_port: u16,
}

pub fn port_from_slot_id(slot_id: &str) -> u16 {
    let mut hash = 0u32;
    for b in slot_id.bytes() {
        hash = hash.wrapping_mul(31).wrapping_add(b as u32);
    }
    8200 + (hash % 800) as u16
}

#[derive(Debug, Clone)]
pub struct ClaudeGatewayConfig {
    pub bind: GatewayBind,
    pub upstream_base_url: String,
    pub token_endpoint_url: String,
    pub seed: SeedCredential,
    pub credential_event_log: Option<PathBuf>,
}

struct TokenCache {
    access_token: String,
    refresh_token: String,
    expires_at: SystemTime,
    last_failure: Option<CredentialFailureCode>,
}

struct CredentialsState {
    cache: RwLock<TokenCache>,
    refresh_mutex: tokio::sync::Mutex<()>,
    token_endpoint_url: String,
    credential_event_log: Option<PathBuf>,
}

struct RefreshResponse {
    access_token: String,
    refresh_token: String,
    expires_in: u64,
}

async fn perform_real_refresh(
    token_endpoint_url: &str,
    refresh_token: &str,
) -> Result<RefreshResponse, CredentialFailureCode> {
    let url = token_endpoint_url.to_string();
    let token = refresh_token.to_string();
    
    tokio::task::spawn_blocking(move || {
        let resp = ureq::post(&url)
            .send_form(&[
                ("grant_type", "refresh_token"),
                ("refresh_token", &token),
            ]);
            
        match resp {
            Ok(response) => {
                if let Ok(text) = response.into_string() {
                    if let Ok(data) = serde_json::from_str::<serde_json::Value>(&text) {
                        let access_token = data["access_token"].as_str().map(String::from);
                        let refresh_token = data["refresh_token"].as_str().map(String::from).unwrap_or(token);
                        let expires_in = data["expires_in"].as_u64().unwrap_or(3600);
                        if let Some(access_token) = access_token {
                            return Ok(RefreshResponse {
                                access_token,
                                refresh_token,
                                expires_in,
                            });
                        }
                    }
                }
                Err(CredentialFailureCode::SeedRefreshInvalidGrant)
            }
            Err(ureq::Error::Status(code, response)) => {
                if code == 400 {
                    if let Ok(text) = response.into_string() {
                        if let Ok(data) = serde_json::from_str::<serde_json::Value>(&text) {
                            if data["error"].as_str() == Some("invalid_grant") {
                                return Err(CredentialFailureCode::SeedRefreshInvalidGrant);
                            }
                        }
                    }
                }
                Err(CredentialFailureCode::SeedRefreshInvalidGrant)
            }
            Err(_) => Err(CredentialFailureCode::SeedRefreshInvalidGrant),
        }
    })
    .await
    .unwrap_or(Err(CredentialFailureCode::SeedRefreshInvalidGrant))
}

impl CredentialsState {
    pub async fn get_valid_token(&self) -> Result<String, CredentialFailureCode> {
        // 1. Fast path: check if cached token is still valid or if there's a cached failure
        {
            let cache = self.cache.read().await;
            if let Some(err) = cache.last_failure {
                return Err(err);
            }
            let buffer = Duration::from_secs(300);
            if cache.expires_at > SystemTime::now() + buffer {
                return Ok(cache.access_token.clone());
            }
        }

        // 2. Slow path: serialize refresh operations
        let _guard = self.refresh_mutex.lock().await;

        // Double check after acquiring the lock
        {
            let cache = self.cache.read().await;
            if let Some(err) = cache.last_failure {
                return Err(err);
            }
            let buffer = Duration::from_secs(300);
            if cache.expires_at > SystemTime::now() + buffer {
                return Ok(cache.access_token.clone());
            }
        }

        let (refresh_token, token_endpoint_url) = {
            let cache = self.cache.read().await;
            (cache.refresh_token.clone(), self.token_endpoint_url.clone())
        };

        match perform_real_refresh(&token_endpoint_url, &refresh_token).await {
            Ok(new_token) => {
                let mut cache = self.cache.write().await;
                cache.access_token = new_token.access_token.clone();
                cache.refresh_token = new_token.refresh_token.clone();
                cache.expires_at = SystemTime::now() + Duration::from_secs(new_token.expires_in);
                cache.last_failure = None;
                Ok(new_token.access_token)
            }
            Err(failure_code) => {
                {
                    let mut cache = self.cache.write().await;
                    cache.last_failure = Some(failure_code);
                }
                if failure_code == CredentialFailureCode::SeedRefreshInvalidGrant {
                    if let Some(ref path) = self.credential_event_log {
                        let event = serde_json::json!({
                            "event": "credential_failure",
                            "code": failure_code.as_str(),
                            "message": "manual_reauth_required"
                        });
                        if let Ok(mut file) = fs::OpenOptions::new()
                            .create(true)
                            .append(true)
                            .open(path)
                        {
                            use std::io::Write as _;
                            let _ = writeln!(file, "{}", event.to_string());
                        }
                    }
                }
                Err(failure_code)
            }
        }
    }
}

#[derive(Clone)]
pub struct TestWorkerGateway {
    pub host_uds_path: PathBuf,
    pub env: WorkerGatewayEnv,
    pub test_bridge_base_url: String,
}

pub struct ClaudeGateway {
    config: Arc<ClaudeGatewayConfig>,
    state: Arc<CredentialsState>,
    workers: Arc<Mutex<HashMap<String, TestWorkerGateway>>>,
    abort_handles: Arc<Mutex<Vec<tokio::task::JoinHandle<()>>>>,
}

impl Drop for ClaudeGateway {
    fn drop(&mut self) {
        let mut handles = self.abort_handles.lock().unwrap();
        for handle in handles.drain(..) {
            handle.abort();
        }
        let workers = self.workers.lock().unwrap();
        for worker in workers.values() {
            let _ = fs::remove_file(&worker.host_uds_path);
        }
    }
}

impl ClaudeGateway {
    pub async fn spawn_for_test(config: ClaudeGatewayConfig) -> Result<Self, String> {
        let state = Arc::new(CredentialsState {
            cache: RwLock::new(TokenCache {
                access_token: config.seed.access_token.clone(),
                refresh_token: config.seed.refresh_token.clone(),
                expires_at: config.seed.expires_at,
                last_failure: None,
            }),
            refresh_mutex: tokio::sync::Mutex::new(()),
            token_endpoint_url: config.token_endpoint_url.clone(),
            credential_event_log: config.credential_event_log.clone(),
        });
        
        Ok(Self {
            config: Arc::new(config),
            state,
            workers: Arc::new(Mutex::new(HashMap::new())),
            abort_handles: Arc::new(Mutex::new(Vec::new())),
        })
    }
    
    #[cfg(unix)]
    pub async fn worker_gateway_for_test(&self, worker_id: &str) -> Result<TestWorkerGateway, String> {
        let socket_root = match &self.config.bind {
            GatewayBind::PerWorkerUds { socket_root, .. } => socket_root,
        };
        
        let _ = fs::create_dir_all(socket_root);
        let host_uds_path = socket_root.join(format!("ah-worker-{worker_id}.sock"));
        
        if host_uds_path.exists() {
            let _ = fs::remove_file(&host_uds_path);
        }
        
        let listener = UnixListener::bind(&host_uds_path).map_err(|e| e.to_string())?;
        
        let tcp_listener = TcpListener::bind("127.0.0.1:0").await.map_err(|e| e.to_string())?;
        let local_addr = tcp_listener.local_addr().map_err(|e| e.to_string())?;
        let port = local_addr.port();
        let test_bridge_base_url = format!("http://127.0.0.1:{port}");
        
        let dynamic_port = port_from_slot_id(worker_id);
        let fake_jwt = build_fake_worker_jwt_for_test(worker_id)?;
        let env = WorkerGatewayEnv {
            base_url: format!("http://localhost:{}", dynamic_port),
            auth_token: fake_jwt.clone(),
            sandbox_uds_path: PathBuf::from("/var/run/ah-gateway.sock"),
            bridge_port: dynamic_port,
        };
        
        let worker_gateway = TestWorkerGateway {
            host_uds_path: host_uds_path.clone(),
            env: env.clone(),
            test_bridge_base_url: test_bridge_base_url.clone(),
        };
        
        self.workers.lock().unwrap().insert(worker_id.to_string(), worker_gateway.clone());
        
        let state = Arc::clone(&self.state);
        let config = Arc::clone(&self.config);
        let worker_id_str = worker_id.to_string();
        let fake_jwt_str = fake_jwt.clone();
        
        let uds_handle = tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                let state_inner = Arc::clone(&state);
                let config_inner = Arc::clone(&config);
                let worker_id_inner = worker_id_str.clone();
                let fake_jwt_inner = fake_jwt_str.clone();
                
                tokio::spawn(async move {
                    match read_http_request(&mut stream).await {
                        Ok((request_line, headers, body)) => {
                            let auth_ok = if let Some(auth_val) = headers.get("authorization") {
                                if let Some(jwt) = auth_val.strip_prefix("Bearer ") {
                                    if let Ok(claims) = decode_fake_worker_jwt_claims(jwt) {
                                        claims.worker_id == worker_id_inner
                                    } else {
                                        false
                                    }
                                } else {
                                    false
                                }
                            } else {
                                false
                            };
                            
                            if !auth_ok {
                                let _ = write_response(
                                    &mut stream,
                                    403,
                                    "Forbidden",
                                    &[],
                                    b"Forbidden",
                                ).await;
                                return;
                            }
                            
                            let parts: Vec<&str> = request_line.split_whitespace().collect();
                            let method = parts.first().copied().unwrap_or("POST").to_string();
                            let path = parts.get(1).copied().unwrap_or("/v1/messages").to_string();
                            
                            let real_token = match state_inner.get_valid_token().await {
                                Ok(token) => token,
                                Err(failure_code) => {
                                    let err_body = serde_json::json!({
                                        "error": {
                                            "type": "authentication_error",
                                            "message": failure_code.as_str()
                                        }
                                    });
                                    let body_bytes = err_body.to_string().into_bytes();
                                    let _ = write_response(
                                        &mut stream,
                                        401,
                                        "Unauthorized",
                                        &[("content-type", "application/json")],
                                        &body_bytes,
                                    ).await;
                                    return;
                                }
                            };
                            
                            let target_url = format!("{}{}", config_inner.upstream_base_url, path);
                            let response_res = tokio::task::spawn_blocking(move || {
                                let mut req = ureq::request(&method, &target_url);
                                for (name, value) in &headers {
                                    if name == "authorization" {
                                        req = req.set(name, &format!("Bearer {real_token}"));
                                    } else if value.contains(&fake_jwt_inner) {
                                        // Skip header
                                    } else {
                                        req = req.set(name, value);
                                    }
                                }
                                req.send_bytes(&body)
                            }).await;
                            
                            match response_res {
                                Ok(Ok(response)) => {
                                    let status = response.status();
                                    let reason = response.status_text().to_string();
                                    let mut resp_headers = Vec::new();
                                    for name in response.headers_names() {
                                        if let Some(value) = response.header(&name) {
                                            resp_headers.push((name.clone(), value.to_string()));
                                        }
                                    }
                                    let mut body_bytes = Vec::new();
                                    let _ = response.into_reader().read_to_end(&mut body_bytes);
                                    
                                    let header_refs: Vec<(&str, &str)> = resp_headers
                                        .iter()
                                        .map(|(n, v)| (n.as_str(), v.as_str()))
                                        .collect();
                                        
                                    let _ = write_response(
                                        &mut stream,
                                        status,
                                        &reason,
                                        &header_refs,
                                        &body_bytes,
                                    ).await;
                                }
                                Ok(Err(ureq::Error::Status(status, response))) => {
                                    let reason = response.status_text().to_string();
                                    let mut resp_headers = Vec::new();
                                    for name in response.headers_names() {
                                        if let Some(value) = response.header(&name) {
                                            resp_headers.push((name.clone(), value.to_string()));
                                        }
                                    }
                                    let mut body_bytes = Vec::new();
                                    let _ = response.into_reader().read_to_end(&mut body_bytes);
                                    
                                    let header_refs: Vec<(&str, &str)> = resp_headers
                                        .iter()
                                        .map(|(n, v)| (n.as_str(), v.as_str()))
                                        .collect();
                                        
                                    let _ = write_response(
                                        &mut stream,
                                        status,
                                        &reason,
                                        &header_refs,
                                        &body_bytes,
                                    ).await;
                                }
                                _ => {
                                    let _ = write_response(
                                        &mut stream,
                                        502,
                                        "Bad Gateway",
                                        &[],
                                        b"Bad Gateway",
                                    ).await;
                                }
                            }
                        }
                        Err(_) => {}
                    }
                });
            }
        });
        
        self.abort_handles.lock().unwrap().push(uds_handle);
        
        let host_uds_path_clone = host_uds_path.clone();
        let bridge_handle = tokio::spawn(async move {
            while let Ok((mut tcp_stream, _)) = tcp_listener.accept().await {
                let host_uds_path_inner = host_uds_path_clone.clone();
                tokio::spawn(async move {
                    if let Ok(mut uds_stream) = tokio::net::UnixStream::connect(&host_uds_path_inner).await {
                        let _ = tokio::io::copy_bidirectional(&mut tcp_stream, &mut uds_stream).await;
                    }
                });
            }
        });
        
        self.abort_handles.lock().unwrap().push(bridge_handle);
        
        Ok(worker_gateway)
    }

    #[cfg(not(unix))]
    pub async fn worker_gateway_for_test(&self, _worker_id: &str) -> Result<TestWorkerGateway, String> {
        Err("Claude gateway is not supported on this platform".to_string())
    }
}

async fn read_http_request<R: tokio::io::AsyncRead + Unpin>(
    reader: &mut R,
) -> Result<(String, std::collections::BTreeMap<String, String>, Vec<u8>), String> {
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 1024];
    loop {
        let read = reader.read(&mut chunk).await.map_err(|e| e.to_string())?;
        if read == 0 {
            return Err("Connection closed before headers completed".to_string());
        }
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(pos) = find_subsequence(&buffer, b"\r\n\r\n") {
            let header_end = pos + 4;
            let header_text = String::from_utf8_lossy(&buffer[..header_end]);
            let mut lines = header_text.lines();
            let request_line = lines.next().ok_or_else(|| "Empty request line".to_string())?;
            
            let mut headers = std::collections::BTreeMap::new();
            for line in lines {
                if line.is_empty() {
                    continue;
                }
                if let Some((name, value)) = line.split_once(':') {
                    headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_string());
                }
            }
            
            let content_length = headers.get("content-length")
                .and_then(|val| val.parse::<usize>().ok())
                .unwrap_or(0);
            
            let mut body = buffer[header_end..].to_vec();
            while body.len() < content_length {
                let read = reader.read(&mut chunk).await.map_err(|e| e.to_string())?;
                if read == 0 {
                    return Err("Connection closed before body completed".to_string());
                }
                body.extend_from_slice(&chunk[..read]);
            }
            body.truncate(content_length);
            
            return Ok((request_line.to_string(), headers, body));
        }
    }
}

fn find_subsequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|window| window == needle)
}

async fn write_response<W: tokio::io::AsyncWrite + Unpin>(
    writer: &mut W,
    status: u16,
    reason: &str,
    headers: &[(&str, &str)],
    body: &[u8],
) -> Result<(), String> {
    let mut header_str = format!("HTTP/1.1 {status} {reason}\r\n");
    for (name, val) in headers {
        header_str.push_str(&format!("{name}: {val}\r\n"));
    }
    header_str.push_str(&format!("content-length: {}\r\n\r\n", body.len()));
    writer.write_all(header_str.as_bytes()).await.map_err(|e| e.to_string())?;
    writer.write_all(body).await.map_err(|e| e.to_string())?;
    writer.flush().await.map_err(|e| e.to_string())?;
    Ok(())
}

#[derive(Debug, Clone)]
pub struct WorkerGateway {
    pub host_uds_path: PathBuf,
    pub env: WorkerGatewayEnv,
}

static PROD_GATEWAY: OnceLock<Arc<ClaudeGateway>> = OnceLock::new();

pub async fn get_or_init_production_gateway(source_home: &std::path::Path) -> Result<Arc<ClaudeGateway>, String> {
    if let Some(gw) = PROD_GATEWAY.get() {
        return Ok(Arc::clone(gw));
    }
    
    let seed = load_seed_credential(source_home)?;
    let socket_root = source_home.join(".cache/ah/gateway");
    let config = ClaudeGatewayConfig {
        bind: GatewayBind::PerWorkerUds {
            socket_root: socket_root.clone(),
            sandbox_uds_path: PathBuf::from("/var/run/ah-gateway.sock"),
            bridge_port: 8206,
        },
        upstream_base_url: "https://api.anthropic.com".to_string(),
        token_endpoint_url: "https://platform.claude.com/v1/oauth/token".to_string(),
        seed,
        credential_event_log: Some(source_home.join(".cache/ah/claude_credential_events.jsonl")),
    };
    
    let state = Arc::new(CredentialsState {
        cache: RwLock::new(TokenCache {
            access_token: config.seed.access_token.clone(),
            refresh_token: config.seed.refresh_token.clone(),
            expires_at: config.seed.expires_at,
            last_failure: None,
        }),
        refresh_mutex: tokio::sync::Mutex::new(()),
        token_endpoint_url: config.token_endpoint_url.clone(),
        credential_event_log: config.credential_event_log.clone(),
    });
    
    let gw = Arc::new(ClaudeGateway {
        config: Arc::new(config),
        state,
        workers: Arc::new(Mutex::new(HashMap::new())),
        abort_handles: Arc::new(Mutex::new(Vec::new())),
    });
    
    let _ = PROD_GATEWAY.set(Arc::clone(&gw));
    Ok(gw)
}

pub fn load_seed_credential(source_home: &std::path::Path) -> Result<SeedCredential, String> {
    let cred_path = source_home.join(".claude/.credentials.json");
    if !cred_path.exists() {
        #[cfg(test)]
        {
            if std::env::var("ALLOW_DUMMY_CLAUDE_CREDENTIALS").is_ok() {
                return Ok(SeedCredential {
                    access_token: "dummy_access_token".to_string(),
                    refresh_token: "dummy_refresh_token".to_string(),
                    expires_at: SystemTime::now() + Duration::from_secs(3600),
                });
            }
        }
        return Err("Claude seed credentials file (.claude/.credentials.json) not found on host. Please authenticate first.".to_string());
    }
    let data = fs::read_to_string(&cred_path).map_err(|e| e.to_string())?;
    let val: serde_json::Value = serde_json::from_str(&data).map_err(|e| e.to_string())?;
    let access_token = val["access_token"].as_str().ok_or("Missing access_token")?.to_string();
    let refresh_token = val["refresh_token"].as_str().ok_or("Missing refresh_token")?.to_string();
    let expires_at = SystemTime::now() + Duration::from_secs(3600);
    Ok(SeedCredential {
        access_token,
        refresh_token,
        expires_at,
    })
}

impl ClaudeGateway {
    #[cfg(unix)]
    pub async fn register_worker(&self, worker_id: &str) -> Result<WorkerGateway, String> {
        let socket_root = match &self.config.bind {
            GatewayBind::PerWorkerUds { socket_root, .. } => socket_root,
        };
        
        let _ = fs::create_dir_all(socket_root);
        let host_uds_path = socket_root.join(format!("ah-worker-{worker_id}.sock"));
        
        if host_uds_path.exists() {
            let _ = fs::remove_file(&host_uds_path);
        }
        
        let listener = UnixListener::bind(&host_uds_path).map_err(|e| e.to_string())?;
        
        let dynamic_port = port_from_slot_id(worker_id);
        let fake_jwt = build_fake_worker_jwt_for_test(worker_id)?;
        let env = WorkerGatewayEnv {
            base_url: format!("http://localhost:{}", dynamic_port),
            auth_token: fake_jwt.clone(),
            sandbox_uds_path: PathBuf::from("/var/run/ah-gateway.sock"),
            bridge_port: dynamic_port,
        };
        
        let state = Arc::clone(&self.state);
        let config = Arc::clone(&self.config);
        let worker_id_str = worker_id.to_string();
        let fake_jwt_str = fake_jwt.clone();
        
        let uds_handle = tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                let state_inner = Arc::clone(&state);
                let config_inner = Arc::clone(&config);
                let worker_id_inner = worker_id_str.clone();
                let fake_jwt_inner = fake_jwt_str.clone();
                
                tokio::spawn(async move {
                    match read_http_request(&mut stream).await {
                        Ok((request_line, headers, body)) => {
                            let auth_ok = if let Some(auth_val) = headers.get("authorization") {
                                if let Some(jwt) = auth_val.strip_prefix("Bearer ") {
                                    if let Ok(claims) = decode_fake_worker_jwt_claims(jwt) {
                                        claims.worker_id == worker_id_inner
                                    } else {
                                        false
                                    }
                                } else {
                                    false
                                }
                            } else {
                                false
                            };
                            
                            if !auth_ok {
                                let _ = write_response(
                                    &mut stream,
                                    403,
                                    "Forbidden",
                                    &[],
                                    b"Forbidden",
                                ).await;
                                return;
                            }
                            
                            let parts: Vec<&str> = request_line.split_whitespace().collect();
                            let method = parts.first().copied().unwrap_or("POST").to_string();
                            let path = parts.get(1).copied().unwrap_or("/v1/messages").to_string();
                            
                            let real_token = match state_inner.get_valid_token().await {
                                Ok(token) => token,
                                Err(failure_code) => {
                                    let err_body = serde_json::json!({
                                        "error": {
                                            "type": "authentication_error",
                                            "message": failure_code.as_str()
                                        }
                                    });
                                    let body_bytes = err_body.to_string().into_bytes();
                                    let _ = write_response(
                                        &mut stream,
                                        401,
                                        "Unauthorized",
                                        &[("content-type", "application/json")],
                                        &body_bytes,
                                    ).await;
                                    return;
                                }
                            };
                            
                            let target_url = format!("{}{}", config_inner.upstream_base_url, path);
                            let response_res = tokio::task::spawn_blocking(move || {
                                let mut req = ureq::request(&method, &target_url);
                                for (name, value) in &headers {
                                    if name == "authorization" {
                                        req = req.set(name, &format!("Bearer {real_token}"));
                                    } else if value.contains(&fake_jwt_inner) {
                                        // Skip header
                                    } else {
                                        req = req.set(name, value);
                                    }
                                }
                                req.send_bytes(&body)
                            }).await;
                            
                            match response_res {
                                Ok(Ok(response)) => {
                                    let status = response.status();
                                    let reason = response.status_text().to_string();
                                    let mut resp_headers = Vec::new();
                                    for name in response.headers_names() {
                                        if let Some(value) = response.header(&name) {
                                            resp_headers.push((name.clone(), value.to_string()));
                                        }
                                    }
                                    let mut body_bytes = Vec::new();
                                    let _ = response.into_reader().read_to_end(&mut body_bytes);
                                    
                                    let header_refs: Vec<(&str, &str)> = resp_headers
                                        .iter()
                                        .map(|(n, v)| (n.as_str(), v.as_str()))
                                        .collect();
                                        
                                    let _ = write_response(
                                        &mut stream,
                                        status,
                                        &reason,
                                        &header_refs,
                                        &body_bytes,
                                    ).await;
                                }
                                Ok(Err(ureq::Error::Status(status, response))) => {
                                    let reason = response.status_text().to_string();
                                    let mut resp_headers = Vec::new();
                                    for name in response.headers_names() {
                                        if let Some(value) = response.header(&name) {
                                            resp_headers.push((name.clone(), value.to_string()));
                                        }
                                    }
                                    let mut body_bytes = Vec::new();
                                    let _ = response.into_reader().read_to_end(&mut body_bytes);
                                    
                                    let header_refs: Vec<(&str, &str)> = resp_headers
                                        .iter()
                                        .map(|(n, v)| (n.as_str(), v.as_str()))
                                        .collect();
                                        
                                    let _ = write_response(
                                        &mut stream,
                                        status,
                                        &reason,
                                        &header_refs,
                                        &body_bytes,
                                    ).await;
                                }
                                _ => {
                                    let _ = write_response(
                                        &mut stream,
                                        502,
                                        "Bad Gateway",
                                        &[],
                                        b"Bad Gateway",
                                    ).await;
                                }
                            }
                        }
                        _ => {}
                    }
                });
            }
        });
        
        self.abort_handles.lock().unwrap().push(uds_handle);
        
        Ok(WorkerGateway {
            host_uds_path,
            env,
        })
    }

    #[cfg(not(unix))]
    pub async fn register_worker(&self, _worker_id: &str) -> Result<WorkerGateway, String> {
        Err("Claude gateway is not supported on this platform".to_string())
    }
}
