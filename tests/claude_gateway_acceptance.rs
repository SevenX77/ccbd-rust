#![cfg(unix)]
//! Plan B Fake Gateway acceptance tests for Claude per-worker credentials.
//!
//! These tests assert externally visible behavior only:
//! - workers send fake gateway JWTs to a local gateway;
//! - the mock upstream only ever sees the real seed access token;
//! - refresh is single-flight under concurrent worker traffic;
//! - worker sandboxes do not receive real Claude credential files or token bytes;
//! - refresh failures surface as a distinct credential failure event.

mod common;

use ah::provider::claude_gateway::{
    ClaudeGateway, ClaudeGatewayConfig, CredentialFailureCode, GatewayBind, SeedCredential,
    WorkerGatewayEnv, build_fake_worker_jwt_for_test, decode_fake_worker_jwt_claims,
};
use ah::provider::home_layout::{
    HomeLayoutRole, prepare_claude_home_layout_with_gateway,
    prepare_home_layout_with_extensions_for_slot,
};
use ah::provider::extensions::ExtensionConfig;
use serde_json::json;
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime};

const REAL_ACCESS_INITIAL: &str = "real-access-initial-secret";
const REAL_ACCESS_REFRESHED: &str = "real-access-refreshed-secret";
const REAL_REFRESH_TOKEN: &str = "real-refresh-token-secret";
const SANDBOX_GATEWAY_BASE_URL: &str = "http://localhost:8206";

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ac1_concurrent_expired_worker_requests_refresh_single_flight() {
    let upstream = MockAnthropicUpstream::start(MockMode::RefreshSucceeds {
        refresh_delay: Duration::from_millis(150),
    });
    let gateway_root = tempfile::tempdir().unwrap();
    let gateway = spawn_expired_gateway(&upstream, gateway_root.path(), None).await;
    let worker = gateway.worker_gateway_for_test("worker-a").await.unwrap();

    let mut joins = Vec::new();
    for i in 0..12 {
        let base_url = worker.test_bridge_base_url.clone();
        let fake_jwt = worker.env.auth_token.clone();
        joins.push(tokio::task::spawn_blocking(move || {
            post_message(&base_url, &fake_jwt, &format!("hello-{i}"))
        }));
    }

    for join in joins {
        let response = join.await.unwrap();
        assert_eq!(response.status, 200, "worker request failed: {response:?}");
        assert!(
            response.body.contains("ok"),
            "worker did not receive upstream success body: {response:?}"
        );
    }

    assert_eq!(
        upstream.refresh_count(),
        1,
        "expired-token burst must produce exactly one upstream refresh call"
    );
    assert_eq!(
        upstream.messages_count(),
        12,
        "all concurrent worker requests must be forwarded after refresh"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ac2_refresh_from_worker_a_does_not_disrupt_worker_b() {
    let upstream = MockAnthropicUpstream::start(MockMode::RefreshSucceeds {
        refresh_delay: Duration::from_millis(100),
    });
    let gateway_root = tempfile::tempdir().unwrap();
    let gateway = spawn_expired_gateway(&upstream, gateway_root.path(), None).await;
    let worker_a = gateway.worker_gateway_for_test("worker-a").await.unwrap();
    let worker_b = gateway.worker_gateway_for_test("worker-b").await.unwrap();
    assert_ne!(
        worker_a.host_uds_path, worker_b.host_uds_path,
        "each worker must receive a physically distinct host-side UDS"
    );
    assert_eq!(
        worker_a.env.base_url, SANDBOX_GATEWAY_BASE_URL,
        "worker-visible Claude gateway URL must be the sandbox TCP-to-UDS bridge"
    );
    assert_eq!(
        worker_b.env.base_url, SANDBOX_GATEWAY_BASE_URL,
        "each sandbox may use the same localhost port inside its own namespace"
    );

    let a = tokio::task::spawn_blocking({
        let base_url = worker_a.test_bridge_base_url.clone();
        let fake_jwt = worker_a.env.auth_token.clone();
        move || post_message(&base_url, &fake_jwt, "worker-a-refreshes")
    });
    let b = tokio::task::spawn_blocking({
        let base_url = worker_b.test_bridge_base_url.clone();
        let fake_jwt = worker_b.env.auth_token.clone();
        move || post_message(&base_url, &fake_jwt, "worker-b-concurrent")
    });

    let response_a = a.await.unwrap();
    let response_b = b.await.unwrap();

    assert_eq!(response_a.status, 200, "worker A failed: {response_a:?}");
    assert_eq!(response_b.status, 200, "worker B failed: {response_b:?}");
    assert_eq!(
        upstream.refresh_count(),
        1,
        "worker A/B concurrent refresh window must share one seed refresh"
    );
}

#[test]
fn ac3_worker_home_contains_no_credentials_file_or_real_token_bytes() {
    let fixture = HostFixture::new();
    let sandbox = tempfile::tempdir().unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let gateway_env = WorkerGatewayEnv {
        base_url: SANDBOX_GATEWAY_BASE_URL.to_string(),
        auth_token: fake_worker_jwt("worker-a"),
        sandbox_uds_path: PathBuf::from("/var/run/ah-gateway.sock"),
        bridge_port: 8206,
    };

    let layout = prepare_claude_home_layout_with_gateway(
        sandbox.path(),
        workspace.path(),
        HomeLayoutRole::Worker,
        &gateway_env,
    )
    .unwrap();

    let credentials_path = layout.home_root.join(".claude/.credentials.json");
    assert!(
        std::fs::symlink_metadata(&credentials_path).is_err(),
        "worker sandbox must not contain .credentials.json as either a symlink or a copy"
    );
    assert_token_absent(&layout.home_root, REAL_ACCESS_INITIAL);
    assert_token_absent(&layout.home_root, REAL_REFRESH_TOKEN);

    assert_eq!(
        layout
            .extra_env
            .get("CLAUDE_CODE_USE_GATEWAY")
            .map(String::as_str),
        Some("1")
    );
    assert_eq!(
        layout.extra_env.get("ANTHROPIC_BASE_URL"),
        Some(&gateway_env.base_url)
    );
    assert_eq!(
        layout.extra_env.get("ANTHROPIC_AUTH_TOKEN"),
        Some(&gateway_env.auth_token)
    );
    assert_fake_jwt_for_worker(&gateway_env.auth_token, "worker-a");

    drop(fixture);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ac4_gateway_rewrites_authorization_and_never_forwards_fake_jwt() {
    let upstream = MockAnthropicUpstream::start(MockMode::RefreshSucceeds {
        refresh_delay: Duration::ZERO,
    });
    let gateway_root = tempfile::tempdir().unwrap();
    let gateway = spawn_expired_gateway(&upstream, gateway_root.path(), None).await;
    let worker = gateway.worker_gateway_for_test("worker-a").await.unwrap();

    let response = tokio::task::spawn_blocking({
        let base_url = worker.test_bridge_base_url.clone();
        let fake_jwt = worker.env.auth_token.clone();
        move || post_message(&base_url, &fake_jwt, "rewrite-check")
    })
    .await
    .unwrap();

    assert_eq!(response.status, 200, "worker request failed: {response:?}");
    let message = upstream
        .recorded_requests()
        .into_iter()
        .find(|request| request.path == "/v1/messages")
        .expect("mock upstream did not receive /v1/messages");

    assert_eq!(
        header(&message.headers, "authorization"),
        Some(format!("Bearer {REAL_ACCESS_REFRESHED}")),
        "gateway must replace the worker fake JWT with the real access token"
    );
    assert!(
        !message.contains_header_value(&worker.env.auth_token),
        "fake worker JWT must not appear in any upstream header: {message:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn design_worker_jwt_must_match_physical_uds_identity() {
    let upstream = MockAnthropicUpstream::start(MockMode::RefreshSucceeds {
        refresh_delay: Duration::ZERO,
    });
    let gateway_root = tempfile::tempdir().unwrap();
    let gateway = spawn_expired_gateway(&upstream, gateway_root.path(), None).await;
    let worker_a = gateway.worker_gateway_for_test("worker-a").await.unwrap();
    let worker_b = gateway.worker_gateway_for_test("worker-b").await.unwrap();

    let response = tokio::task::spawn_blocking({
        let base_url = worker_b.test_bridge_base_url.clone();
        let worker_a_jwt = worker_a.env.auth_token.clone();
        move || post_message(&base_url, &worker_a_jwt, "wrong-uds")
    })
    .await
    .unwrap();

    assert_eq!(
        response.status, 403,
        "gateway must reject a token whose worker_id does not match the physical UDS"
    );
    assert_eq!(
        upstream.messages_count(),
        0,
        "identity-confused requests must not be forwarded upstream"
    );
}

#[test]
fn design_real_claude_worker_home_layout_uses_gateway_deterministically() {
    let fixture = HostFixture::new();
    let sandbox = tempfile::tempdir().unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let slot_id = "worker-a";
    let extensions = ExtensionConfig::default();

    let overrides = prepare_home_layout_with_extensions_for_slot(
        "claude",
        sandbox.path(),
        workspace.path(),
        HomeLayoutRole::Worker,
        slot_id,
        &extensions,
        None,
    )
    .unwrap();

    assert_eq!(
        overrides.extra_env.get("CLAUDE_CODE_USE_GATEWAY").map(|s| s.as_str()),
        Some("1"),
        "worker layout must enable gateway"
    );
    assert_eq!(
        overrides.extra_env.get("ANTHROPIC_BASE_URL").map(|s| s.as_str()),
        Some("http://localhost:8206"),
        "worker layout must target local gateway bridge address"
    );
    
    let token = overrides.extra_env.get("ANTHROPIC_AUTH_TOKEN").unwrap();
    let claims = decode_fake_worker_jwt_claims(token).unwrap();
    assert_eq!(
        claims.worker_id, slot_id,
        "fake token must bind the stable worker identity (slot_id)"
    );

    let credentials_path = overrides.home_root.join(".claude").join("credentials.json");
    assert!(
        !credentials_path.exists(),
        "credentials.json must not exist in worker home layout in gateway mode"
    );

    drop(fixture);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn design_worker_jwt_signature_must_be_valid() {
    let upstream = MockAnthropicUpstream::start(MockMode::RefreshSucceeds {
        refresh_delay: Duration::ZERO,
    });
    let gateway_root = tempfile::tempdir().unwrap();
    let gateway = spawn_expired_gateway(&upstream, gateway_root.path(), None).await;
    let worker_a = gateway.worker_gateway_for_test("worker-a").await.unwrap();

    let mut claims = decode_fake_worker_jwt_claims(&worker_a.env.auth_token).unwrap();
    claims.signature = Some("forged-signature-value".to_string());
    
    let header = "eyJhbGciOiJub25lIiwidHlwIjoiSldUIn0";
    let claims_json = serde_json::to_string(&claims).unwrap();
    let mut result = String::with_capacity((claims_json.len() + 2) / 3 * 4);
    let mut temp;
    let mut i = 0;
    let input = claims_json.as_bytes();
    const CHARSET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
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
    let forged_jwt = format!("{header}.{result}.");

    let response = tokio::task::spawn_blocking({
        let base_url = worker_a.test_bridge_base_url.clone();
        move || post_message(&base_url, &forged_jwt, "forged-sig")
    })
    .await
    .unwrap();

    assert_eq!(
        response.status, 403,
        "gateway must reject a token with an invalid/forged signature"
    );
    assert_eq!(
        upstream.messages_count(),
        0,
        "identity-confused/forged requests must not be forwarded upstream"
    );
}

#[test]
fn ac5_credential_like_paths_do_not_resolve_under_wsl_mnt_c() {
    let fixture = HostFixture::new();
    let sandbox = tempfile::tempdir().unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let gateway_env = WorkerGatewayEnv {
        base_url: SANDBOX_GATEWAY_BASE_URL.to_string(),
        auth_token: fake_worker_jwt("worker-a"),
        sandbox_uds_path: PathBuf::from("/var/run/ah-gateway.sock"),
        bridge_port: 8206,
    };

    let layout = prepare_claude_home_layout_with_gateway(
        sandbox.path(),
        workspace.path(),
        HomeLayoutRole::Worker,
        &gateway_env,
    )
    .unwrap();

    for path in credential_like_paths(&layout.home_root) {
        let resolved = std::fs::canonicalize(&path).unwrap_or(path.clone());
        assert!(
            !resolved.starts_with("/mnt/c"),
            "credential-like path resolved under /mnt/c: {} -> {}",
            path.display(),
            resolved.display()
        );
    }

    drop(fixture);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ac6_invalid_grant_is_distinct_and_records_credential_failure_event() {
    let upstream = MockAnthropicUpstream::start(MockMode::RefreshFailsInvalidGrant);
    let event_log = tempfile::NamedTempFile::new().unwrap();
    let gateway_root = tempfile::tempdir().unwrap();
    let gateway = spawn_expired_gateway(
        &upstream,
        gateway_root.path(),
        Some(event_log.path().to_path_buf()),
    )
    .await;
    let worker = gateway.worker_gateway_for_test("worker-a").await.unwrap();

    let response = tokio::task::spawn_blocking({
        let base_url = worker.test_bridge_base_url.clone();
        let fake_jwt = worker.env.auth_token.clone();
        move || post_message(&base_url, &fake_jwt, "invalid-grant")
    })
    .await
    .unwrap();

    assert_eq!(
        response.status, 401,
        "invalid_grant must be distinguishable from an ordinary upstream 5xx: {response:?}"
    );
    assert!(
        response
            .body
            .contains(CredentialFailureCode::SeedRefreshInvalidGrant.as_str()),
        "worker-visible body must contain the credential failure code: {response:?}"
    );

    let events = std::fs::read_to_string(event_log.path()).unwrap();
    assert!(
        events.contains(CredentialFailureCode::SeedRefreshInvalidGrant.as_str()),
        "daemon-side credential failure event missing from event log: {events}"
    );
    assert!(
        events.contains("manual_reauth_required"),
        "event log must make the operator action explicit: {events}"
    );
}

async fn spawn_expired_gateway(
    upstream: &MockAnthropicUpstream,
    socket_root: &Path,
    credential_event_log: Option<PathBuf>,
) -> ClaudeGateway {
    ClaudeGateway::spawn_for_test(ClaudeGatewayConfig {
        bind: GatewayBind::PerWorkerUds {
            socket_root: socket_root.to_path_buf(),
            sandbox_uds_path: PathBuf::from("/var/run/ah-gateway.sock"),
            bridge_port: 8206,
        },
        upstream_base_url: upstream.base_url(),
        token_endpoint_url: upstream.url("/oauth/token"),
        seed: SeedCredential {
            access_token: REAL_ACCESS_INITIAL.to_string(),
            refresh_token: REAL_REFRESH_TOKEN.to_string(),
            expires_at: SystemTime::now() - Duration::from_secs(60),
        },
        credential_event_log,
    })
    .await
    .unwrap()
}

fn fake_worker_jwt(worker_id: &str) -> String {
    build_fake_worker_jwt_for_test(worker_id).unwrap()
}

fn assert_fake_jwt_for_worker(jwt: &str, worker_id: &str) {
    assert!(
        jwt.ends_with('.'),
        "fake JWT must preserve the third segment delimiter"
    );
    assert!(
        jwt.split('.').count() >= 3,
        "fake JWT must be parseable as a three-segment JWT"
    );
    let claims = decode_fake_worker_jwt_claims(jwt).unwrap();
    assert_eq!(
        claims.worker_id, worker_id,
        "fake JWT must bind the worker_id for gateway-side identity checks"
    );
    assert_eq!(
        claims.exp, 32503680000,
        "fake JWT must use the frozen long-lived gateway exp"
    );
}

#[derive(Debug)]
struct TestHttpResponse {
    status: u16,
    body: String,
}

fn post_message(base_url: &str, fake_jwt: &str, text: &str) -> TestHttpResponse {
    let response = ureq::post(&format!("{base_url}/v1/messages"))
        .set("authorization", &format!("Bearer {fake_jwt}"))
        .set("x-fake-worker-token-copy", fake_jwt)
        .set("content-type", "application/json")
        .send_string(
            &json!({ "model": "claude-test", "messages": [{ "role": "user", "content": text }] })
                .to_string(),
        );

    match response {
        Ok(ok) => TestHttpResponse {
            status: ok.status(),
            body: ok.into_string().unwrap(),
        },
        Err(ureq::Error::Status(status, error)) => TestHttpResponse {
            status,
            body: error.into_string().unwrap_or_default(),
        },
        Err(err) => panic!("gateway request transport failure: {err}"),
    }
}

#[derive(Clone, Copy)]
enum MockMode {
    RefreshSucceeds { refresh_delay: Duration },
    RefreshFailsInvalidGrant,
}

#[derive(Clone)]
struct MockAnthropicUpstream {
    addr: SocketAddr,
    requests: Arc<Mutex<Vec<RecordedRequest>>>,
    refresh_count: Arc<AtomicUsize>,
}

impl MockAnthropicUpstream {
    fn start(mode: MockMode) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let refresh_count = Arc::new(AtomicUsize::new(0));
        let server = Self {
            addr,
            requests: Arc::clone(&requests),
            refresh_count: Arc::clone(&refresh_count),
        };

        thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let requests = Arc::clone(&requests);
                let refresh_count = Arc::clone(&refresh_count);
                thread::spawn(move || {
                    handle_upstream_connection(stream, mode, requests, refresh_count);
                });
            }
        });

        server
    }

    fn base_url(&self) -> String {
        format!("http://{}", self.addr)
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url(), path)
    }

    fn refresh_count(&self) -> usize {
        self.refresh_count.load(Ordering::SeqCst)
    }

    fn messages_count(&self) -> usize {
        self.recorded_requests()
            .into_iter()
            .filter(|request| request.path == "/v1/messages")
            .count()
    }

    fn recorded_requests(&self) -> Vec<RecordedRequest> {
        self.requests.lock().unwrap().clone()
    }
}

fn handle_upstream_connection(
    mut stream: TcpStream,
    mode: MockMode,
    requests: Arc<Mutex<Vec<RecordedRequest>>>,
    refresh_count: Arc<AtomicUsize>,
) {
    let request = read_http_request(&mut stream);
    if request.path == "/oauth/token" {
        refresh_count.fetch_add(1, Ordering::SeqCst);
    }
    requests.lock().unwrap().push(request.clone());

    match (request.path.as_str(), mode) {
        ("/oauth/token", MockMode::RefreshSucceeds { refresh_delay }) => {
            thread::sleep(refresh_delay);
            write_json(
                &mut stream,
                200,
                &json!({
                    "access_token": REAL_ACCESS_REFRESHED,
                    "refresh_token": "rotated-refresh-token",
                    "expires_in": 3600
                })
                .to_string(),
            );
        }
        ("/oauth/token", MockMode::RefreshFailsInvalidGrant) => {
            write_json(
                &mut stream,
                400,
                &json!({
                    "error": "invalid_grant",
                    "error_description": "seed refresh token revoked"
                })
                .to_string(),
            );
        }
        ("/v1/messages", _) => write_json(
            &mut stream,
            200,
            &json!({ "id": "msg_test", "content": [{ "type": "text", "text": "ok" }] }).to_string(),
        ),
        _ => write_json(
            &mut stream,
            404,
            &json!({ "error": format!("unexpected path {}", request.path) }).to_string(),
        ),
    }
}

#[derive(Clone, Debug)]
struct RecordedRequest {
    path: String,
    headers: BTreeMap<String, String>,
    body: String,
}

impl RecordedRequest {
    fn contains_header_value(&self, needle: &str) -> bool {
        self.headers.values().any(|value| value.contains(needle))
    }
}

fn read_http_request(stream: &mut TcpStream) -> RecordedRequest {
    let mut buffer = Vec::new();
    let mut chunk = [0; 1024];
    loop {
        let read = stream.read(&mut chunk).unwrap();
        assert_ne!(read, 0, "connection closed before HTTP headers completed");
        buffer.extend_from_slice(&chunk[..read]);
        if buffer.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
    }

    let header_end = buffer
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .unwrap()
        + 4;
    let header_text = String::from_utf8_lossy(&buffer[..header_end]);
    let mut lines = header_text.lines();
    let request_line = lines.next().unwrap();
    let path = request_line.split_whitespace().nth(1).unwrap().to_string();
    let mut headers = BTreeMap::new();
    for line in lines {
        if line.is_empty() {
            continue;
        }
        if let Some((name, value)) = line.split_once(':') {
            headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_string());
        }
    }

    let content_length = header(&headers, "content-length")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0);
    while buffer.len() - header_end < content_length {
        let read = stream.read(&mut chunk).unwrap();
        assert_ne!(read, 0, "connection closed before HTTP body completed");
        buffer.extend_from_slice(&chunk[..read]);
    }
    let body = String::from_utf8_lossy(&buffer[header_end..header_end + content_length]).into();

    RecordedRequest {
        path,
        headers,
        body,
    }
}

fn write_json(stream: &mut TcpStream, status: u16, body: &str) {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        _ => "Internal Server Error",
    };
    write!(
        stream,
        "HTTP/1.1 {status} {reason}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
        body.len(),
        body
    )
    .unwrap();
}

fn header(headers: &BTreeMap<String, String>, name: &str) -> Option<String> {
    headers.get(&name.to_ascii_lowercase()).cloned()
}

struct HostFixture {
    _host_home: tempfile::TempDir,
    _cache_home: tempfile::TempDir,
    old_home: Option<std::ffi::OsString>,
    old_cache: Option<std::ffi::OsString>,
}

impl HostFixture {
    fn new() -> Self {
        let host_home = tempfile::tempdir().unwrap();
        let cache_home = tempfile::tempdir().unwrap();
        let host = host_home.path();
        std::fs::write(host.join(".claude.json"), "{\"trusted\":true}\n").unwrap();
        std::fs::create_dir_all(host.join(".claude")).unwrap();
        std::fs::write(
            host.join(".claude/.credentials.json"),
            format!(
                "{{\"access_token\":\"{REAL_ACCESS_INITIAL}\",\"refresh_token\":\"{REAL_REFRESH_TOKEN}\"}}\n"
            ),
        )
        .unwrap();

        let old_home = std::env::var_os("HOME");
        let old_cache = std::env::var_os("XDG_CACHE_HOME");
        unsafe {
            std::env::set_var("HOME", host);
            std::env::set_var("XDG_CACHE_HOME", cache_home.path());
        }

        Self {
            _host_home: host_home,
            _cache_home: cache_home,
            old_home,
            old_cache,
        }
    }
}

impl Drop for HostFixture {
    fn drop(&mut self) {
        unsafe {
            match &self.old_home {
                Some(value) => std::env::set_var("HOME", value),
                None => std::env::remove_var("HOME"),
            }
            match &self.old_cache {
                Some(value) => std::env::set_var("XDG_CACHE_HOME", value),
                None => std::env::remove_var("XDG_CACHE_HOME"),
            }
        }
    }
}

fn assert_token_absent(root: &Path, token: &str) {
    for path in files_under(root) {
        let bytes = std::fs::read(&path).unwrap();
        assert!(
            !String::from_utf8_lossy(&bytes).contains(token),
            "real token leaked to worker sandbox file {}",
            path.display()
        );
    }
}

fn credential_like_paths(root: &Path) -> Vec<PathBuf> {
    files_under(root)
        .into_iter()
        .filter(|path| {
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default();
            name.contains("credential") || name.contains("auth") || name.contains("token")
        })
        .collect()
}

fn files_under(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    collect_files(root, &mut files);
    files
}

fn collect_files(dir: &Path, files: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(metadata) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        if metadata.is_dir() {
            collect_files(&path, files);
        } else if metadata.is_file() || metadata.file_type().is_symlink() {
            files.push(path);
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn design_production_agent_spawn_lifecycle_wires_claude_gateway_correctly() {
    use ah::rpc::handlers::handle_agent_spawn;
    use ah::provider::home_layout::{prepare_home_layout_with_extensions_for_slot, HomeLayoutRole};
    use ah::provider::extensions::ExtensionConfig;
    use ah::platform::sys::scope::wrap_command_with_recovery_and_sandbox_overrides;
    
    // 1. Setup host credentials and cache envs using HostFixture
    let fixture = HostFixture::new();
    
    // 2. Initialize a test Ctx with tmux and db setup
    let db_file = tempfile::NamedTempFile::new().unwrap();
    let state_dir = tempfile::TempDir::new().unwrap();
    let state_dir_path = state_dir.path().to_path_buf();
    
    // Create the UDS root cache dir on host
    let socket_root = fixture._host_home.path().join(".cache/ah/gateway");
    let _ = std::fs::create_dir_all(&socket_root);
    
    let tmux_guard = common::TmuxServerGuard::new(&state_dir_path);
    let ctx = ah::rpc::Ctx {
        db: ah::db::init(db_file.path()).unwrap(),
        state_dir: state_dir_path.clone(),
        env_state: ah::sandbox::EnvState {
            systemd_run_available: true,
            unsafe_no_sandbox: false,
            under_systemd: false,
        },
        daemon_unit: None,
        tmux_server: tmux_guard.server(),
    };
    
    // 3. Seed session in db using public insert_session
    ah::db::sessions::insert_session(
        ctx.db.clone(),
        "s1".to_string(),
        "p1".to_string(),
        "/tmp/s1".to_string(),
    )
    .await
    .unwrap();
    
    let conn = ctx.db.conn();
    
    // 4. Call handle_agent_spawn for a Claude worker
    let params = json!({
        "session_id": "s1",
        "agent_id": "production-claude-agent",
        "provider": "claude",
    });
    
    let result = handle_agent_spawn(params, &ctx).await.unwrap();
    assert_eq!(result["state"], "SPAWNING");
    
    // 5. Query spec from db to assert sandbox overrides has the read-write UDS socket bind mount
    let stored = ah::db::recovery::query_agent_spawn_spec_sync(&conn, "production-claude-agent")
        .unwrap()
        .unwrap();
        
    assert!(!stored.spec.sandbox_overrides.extra_rw_binds.is_empty(), "extra_rw_binds must not be empty");
    let uds_bind = &stored.spec.sandbox_overrides.extra_rw_binds[0];
    assert!(
        uds_bind.host_path.contains("ah-worker-production-claude-agent.sock"),
        "host path must point to per-worker UDS socket: {}",
        uds_bind.host_path
    );
    assert_eq!(
        uds_bind.sandbox_path,
        "/var/run/ah-gateway.sock",
        "sandbox path must be the standard gateway path"
    );

    // 6. Test the command wrapping contract directly
    let workspace_path = state_dir_path.join("workspace");
    let overrides = prepare_home_layout_with_extensions_for_slot(
        "claude",
        &state_dir_path.join("sandbox"),
        &workspace_path,
        HomeLayoutRole::Worker,
        "production-claude-agent",
        &ExtensionConfig::default(),
        None,
    ).unwrap();

    let manifest = ah::provider::manifest::try_get_manifest("claude").unwrap();
    let cmd = wrap_command_with_recovery_and_sandbox_overrides(
        "production-claude-agent",
        "p1",
        "ccbd-test",
        &ah::sandbox::EnvState {
            systemd_run_available: true,
            unsafe_no_sandbox: false,
            under_systemd: true,
        },
        ah::sandbox::systemd::RecoverySpawn {
            is_recovery: false,
            args: vec![],
        },
        None,
        &manifest,
        &overrides.extra_env,
        &stored.spec.sandbox_overrides,
    );

    let cmd_str = cmd.join(" ");
    assert!(cmd_str.contains("python3 -c"), "command must wrap payload with python3 bridge");
    let dynamic_port = ah::provider::claude_gateway::port_from_slot_id("production-claude-agent");
    assert!(
        cmd_str.contains(&format!("bind(('127.0.0.1', {}))", dynamic_port)),
        "python3 bridge must bind to the correct dynamic port"
    );
    assert!(
        cmd_str.contains("connect('/var/run/ah-gateway.sock')"),
        "python3 bridge must connect to the standard sandbox UDS path"
    );

    let base_url = overrides.extra_env.get("ANTHROPIC_BASE_URL").unwrap();
    assert_eq!(
        base_url,
        &format!("http://localhost:{}", dynamic_port),
        "injected ANTHROPIC_BASE_URL must match the dynamic port"
    );
    
    drop(tmux_guard);
}

#[tokio::test]
async fn design_seed_credentials_missing_fails_closed() {
    // Ensure ALLOW_DUMMY_CLAUDE_CREDENTIALS is not set so we test the fail-closed path
    unsafe {
        std::env::remove_var("ALLOW_DUMMY_CLAUDE_CREDENTIALS");
    }

    // 1. Check with a completely clean directory (file missing)
    let temp_dir = tempfile::tempdir().unwrap();
    let res = ah::provider::claude_gateway::load_seed_credential(temp_dir.path());
    assert!(res.is_err());
    let err_msg = res.err().unwrap();
    assert!(
        err_msg.contains("Claude seed credentials file (.claude/.credentials.json) not found on host"),
        "error message should be clear and identifiable: {}",
        err_msg
    );

    // 2. Check with invalid JSON contents
    let temp_dir = tempfile::tempdir().unwrap();
    let cred_dir = temp_dir.path().join(".claude");
    std::fs::create_dir_all(&cred_dir).unwrap();
    std::fs::write(cred_dir.join(".credentials.json"), "not json data").unwrap();
    let res = ah::provider::claude_gateway::load_seed_credential(temp_dir.path());
    assert!(res.is_err(), "must fail closed on invalid json");
}
