#![cfg(unix)]

use ah::claude_gateway::{
    CredentialEvent, GatewayCore, GatewayRequest, GatewayResponse, RecordedCredentialEvents,
    TokenSet, UpstreamError, UpstreamResult, fake_worker_jwt, gateway_worker_topology,
    validate_credential_path_not_wsl_windows_mount,
};
use ah::provider::home_layout::prepare_home_layout;
use serde_json::json;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Barrier, LazyLock, Mutex};
use std::thread;
use std::time::{Duration, SystemTime};

static ENV_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

#[test]
fn ac1_concurrent_expired_requests_single_flight_refresh_once() {
    let upstream = Arc::new(RecordingUpstream::new());
    let events = RecordedCredentialEvents::default();
    let gateway = Arc::new(GatewayCore::new(
        expired_token("real-access-initial", "real-refresh"),
        upstream.clone(),
        events,
    ));
    let barrier = Arc::new(Barrier::new(8));

    let handles = (0..8)
        .map(|idx| {
            let gateway = gateway.clone();
            let barrier = barrier.clone();
            thread::spawn(move || {
                barrier.wait();
                gateway.forward_messages(worker_request(idx)).unwrap()
            })
        })
        .collect::<Vec<_>>();

    for handle in handles {
        assert_eq!(handle.join().unwrap().status, 200);
    }
    assert_eq!(
        upstream.refresh_calls(),
        1,
        "expired concurrent workers must share one real upstream refresh"
    );
    assert_eq!(upstream.message_calls(), 8);
}

#[test]
fn ac2_refresh_by_worker_a_does_not_fail_worker_b() {
    let upstream = Arc::new(RecordingUpstream::new());
    let events = RecordedCredentialEvents::default();
    let gateway = Arc::new(GatewayCore::new(
        expired_token("real-access-initial", "real-refresh"),
        upstream.clone(),
        events,
    ));
    let barrier = Arc::new(Barrier::new(2));

    let worker_a = {
        let gateway = gateway.clone();
        let barrier = barrier.clone();
        thread::spawn(move || {
            barrier.wait();
            gateway.forward_messages(worker_request(1)).unwrap()
        })
    };
    let worker_b = {
        let gateway = gateway.clone();
        let barrier = barrier.clone();
        thread::spawn(move || {
            barrier.wait();
            gateway.forward_messages(worker_request(2)).unwrap()
        })
    };

    assert_eq!(worker_a.join().unwrap().status, 200);
    assert_eq!(worker_b.join().unwrap().status, 200);
    assert_eq!(upstream.refresh_calls(), 1);
    assert_eq!(upstream.message_calls(), 2);
}

#[test]
#[serial_test::serial(global_env)]
fn ac3_claude_worker_home_has_no_real_credentials_file_or_token() {
    let _env = TestHomeEnv::new();
    let host_cred = _env.host_home.join(".claude/.credentials.json");
    std::fs::create_dir_all(host_cred.parent().unwrap()).unwrap();
    std::fs::write(
        &host_cred,
        json!({
            "accessToken": "real-access-token",
            "refreshToken": "real-refresh-token"
        })
        .to_string(),
    )
    .unwrap();

    let sandbox_root = tempfile::tempdir().unwrap();
    let project_root = tempfile::tempdir().unwrap();
    let claude = prepare_home_layout("claude", sandbox_root.path(), project_root.path()).unwrap();

    assert!(
        !claude.home_root.join(".claude/.credentials.json").exists(),
        "worker home must not contain a copied or symlinked Claude credentials file"
    );
    let grep = std::process::Command::new("rg")
        .arg("--fixed-strings")
        .arg("real-refresh-token")
        .arg(&claude.home_root)
        .output()
        .unwrap();
    assert!(
        !grep.status.success(),
        "true refresh token leaked into sandbox home: {}",
        String::from_utf8_lossy(&grep.stdout)
    );
    assert_eq!(
        claude.extra_env.get("CLAUDE_CODE_USE_GATEWAY"),
        Some(&"1".to_string())
    );
    assert_eq!(
        claude.extra_env.get("ANTHROPIC_BASE_URL").unwrap(),
        "http://localhost:8206"
    );
    assert_fake_jwt_claims(
        claude.extra_env.get("ANTHROPIC_AUTH_TOKEN").unwrap(),
        "worker",
    );
}

#[test]
fn ac4_worker_fake_jwt_is_rewritten_to_real_access_token() {
    let upstream = Arc::new(RecordingUpstream::new());
    let gateway = GatewayCore::new(
        valid_token("real-access-token", "real-refresh"),
        upstream.clone(),
        RecordedCredentialEvents::default(),
    );

    let response = gateway.forward_messages(GatewayRequest {
        worker_id: "worker-a".to_string(),
        headers: vec![
            (
                "authorization".to_string(),
                format!("Bearer {}", fake_worker_jwt("worker-a")),
            ),
            ("x-api-key".to_string(), "fake-side-token".to_string()),
        ],
        body: b"{}".to_vec(),
    });

    assert_eq!(response.unwrap().status, 200);
    let headers = upstream.last_message_headers();
    let fake_token = fake_worker_jwt("worker-a");
    assert_eq!(
        header_value(&headers, "authorization"),
        Some("Bearer real-access-token")
    );
    assert!(
        headers
            .iter()
            .all(|(_, value)| !value.contains(&fake_token)),
        "fake worker JWT must never be forwarded upstream: {headers:?}"
    );
}

#[test]
fn ac4_gateway_rejects_fake_jwt_from_wrong_worker_channel() {
    let upstream = Arc::new(RecordingUpstream::new());
    let gateway = GatewayCore::new(
        valid_token("real-access-token", "real-refresh"),
        upstream.clone(),
        RecordedCredentialEvents::default(),
    );

    let err = gateway
        .forward_messages(GatewayRequest {
            worker_id: "worker-b-uds".to_string(),
            headers: vec![(
                "authorization".to_string(),
                format!("Bearer {}", fake_worker_jwt("worker-a")),
            )],
            body: b"{}".to_vec(),
        })
        .unwrap_err();

    assert_eq!(err.status, 403);
    assert_eq!(err.error_code, "AH_CLAUDE_GATEWAY_WORKER_ID_MISMATCH");
    assert_eq!(upstream.message_calls(), 0);
}

#[test]
fn ac5_credentials_paths_reject_wsl_windows_mounts() {
    assert!(
        validate_credential_path_not_wsl_windows_mount(Path::new(
            "/mnt/c/Users/alice/.claude/.credentials.json"
        ))
        .is_err()
    );
    assert!(
        validate_credential_path_not_wsl_windows_mount(Path::new(
            "/home/alice/.local/state/ah/claude-gateway/seed.json"
        ))
        .is_ok()
    );
    let topology = gateway_worker_topology(
        Path::new("/home/alice/.cache/ah/sandboxes/worker-a"),
        "worker-a",
    )
    .unwrap();
    assert_eq!(topology.sandbox_tcp_base_url, "http://localhost:8206");
    assert_eq!(
        topology.sandbox_uds_path,
        PathBuf::from("/var/run/ah-gateway.sock")
    );
    assert_eq!(
        topology.host_uds_path,
        PathBuf::from("/home/alice/.cache/ah/sandboxes/worker-a/tmp/ah-gateway.sock")
    );
}

#[test]
fn ac6_invalid_grant_returns_distinct_error_and_records_event() {
    let upstream = Arc::new(RecordingUpstream::new_invalid_grant());
    let events = RecordedCredentialEvents::default();
    let gateway = GatewayCore::new(expired_token("old", "revoked-refresh"), upstream, events.clone());

    let err = gateway.forward_messages(worker_request(1)).unwrap_err();

    assert_eq!(err.status, 401);
    assert_eq!(err.error_code, "AH_CLAUDE_GATEWAY_REFRESH_INVALID_GRANT");
    assert!(
        err.body.contains("invalid_grant"),
        "worker-visible response should distinguish seed credential revocation"
    );
    assert!(
        events.snapshot().iter().any(|event| matches!(
            event,
            CredentialEvent::RefreshFailed {
                error_code,
                upstream_error
            } if error_code == "AH_CLAUDE_GATEWAY_REFRESH_INVALID_GRANT"
                && upstream_error == "invalid_grant"
        )),
        "daemon-side credential event must be queryable"
    );
}

fn worker_request(idx: usize) -> GatewayRequest {
    let worker_id = format!("worker-{idx}");
    GatewayRequest {
        worker_id: worker_id.clone(),
        headers: vec![(
            "authorization".to_string(),
            format!("Bearer {}", fake_worker_jwt(&worker_id)),
        )],
        body: b"{}".to_vec(),
    }
}

fn valid_token(access_token: &str, refresh_token: &str) -> TokenSet {
    TokenSet {
        access_token: access_token.to_string(),
        refresh_token: refresh_token.to_string(),
        expires_at: SystemTime::now() + Duration::from_secs(3600),
    }
}

fn expired_token(access_token: &str, refresh_token: &str) -> TokenSet {
    TokenSet {
        access_token: access_token.to_string(),
        refresh_token: refresh_token.to_string(),
        expires_at: SystemTime::now() - Duration::from_secs(1),
    }
}

fn header_value<'a>(headers: &'a [(String, String)], key: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(key))
        .map(|(_, value)| value.as_str())
}

fn assert_fake_jwt_claims(token: &str, worker_id: &str) {
    let parts = token.split('.').collect::<Vec<_>>();
    assert_eq!(parts.len(), 3, "JWT must retain three segments");
    assert_eq!(parts[2], "", "alg:none JWT keeps an empty signature segment");
    let header: serde_json::Value =
        serde_json::from_slice(&base64url_decode(parts[0])).unwrap();
    let payload: serde_json::Value =
        serde_json::from_slice(&base64url_decode(parts[1])).unwrap();
    assert_eq!(header["alg"], "none");
    assert_eq!(header["typ"], "JWT");
    assert_eq!(payload["exp"], 32503680000_i64);
    assert_eq!(payload["sub"], "ah-worker-session");
    assert_eq!(payload["worker_id"], worker_id);
}

fn base64url_decode(input: &str) -> Vec<u8> {
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
            _ => panic!("invalid base64url byte {byte}"),
        } as u32;
        bits = (bits << 6) | value;
        bit_count += 6;
        if bit_count >= 8 {
            bit_count -= 8;
            out.push(((bits >> bit_count) & 0xff) as u8);
        }
    }
    out
}

struct RecordingUpstream {
    refresh_calls: AtomicUsize,
    message_calls: AtomicUsize,
    last_message_headers: Mutex<Vec<(String, String)>>,
    invalid_grant: bool,
}

impl RecordingUpstream {
    fn new() -> Self {
        Self {
            refresh_calls: AtomicUsize::new(0),
            message_calls: AtomicUsize::new(0),
            last_message_headers: Mutex::new(Vec::new()),
            invalid_grant: false,
        }
    }

    fn new_invalid_grant() -> Self {
        Self {
            invalid_grant: true,
            ..Self::new()
        }
    }

    fn refresh_calls(&self) -> usize {
        self.refresh_calls.load(Ordering::SeqCst)
    }

    fn message_calls(&self) -> usize {
        self.message_calls.load(Ordering::SeqCst)
    }

    fn last_message_headers(&self) -> Vec<(String, String)> {
        self.last_message_headers.lock().unwrap().clone()
    }
}

impl ah::claude_gateway::ClaudeUpstream for RecordingUpstream {
    fn refresh(&self, refresh_token: &str) -> UpstreamResult<TokenSet> {
        self.refresh_calls.fetch_add(1, Ordering::SeqCst);
        thread::sleep(Duration::from_millis(20));
        if self.invalid_grant {
            return Err(UpstreamError::InvalidGrant {
                body: "invalid_grant: seed credential revoked".to_string(),
            });
        }
        assert_eq!(refresh_token, "real-refresh");
        Ok(valid_token("real-access-refreshed", "real-refresh-rotated"))
    }

    fn messages(&self, request: GatewayRequest) -> UpstreamResult<GatewayResponse> {
        self.message_calls.fetch_add(1, Ordering::SeqCst);
        *self.last_message_headers.lock().unwrap() = request.headers;
        Ok(GatewayResponse {
            status: 200,
            headers: vec![],
            body: b"{\"ok\":true}".to_vec(),
        })
    }
}

struct TestHomeEnv {
    host_home: PathBuf,
    _cache_home: tempfile::TempDir,
    old_home: Option<OsString>,
    old_cache_home: Option<OsString>,
    _lock: std::sync::MutexGuard<'static, ()>,
}

impl TestHomeEnv {
    fn new() -> Self {
        let lock = ENV_LOCK.lock().unwrap();
        let host_home = tempfile::tempdir().unwrap();
        let cache_home = tempfile::tempdir().unwrap();
        let old_home = std::env::var_os("HOME");
        let old_cache_home = std::env::var_os("XDG_CACHE_HOME");
        unsafe {
            std::env::set_var("HOME", host_home.path());
            std::env::set_var("XDG_CACHE_HOME", cache_home.path());
        }
        Self {
            host_home: host_home.keep(),
            _cache_home: cache_home,
            old_home,
            old_cache_home,
            _lock: lock,
        }
    }
}

impl Drop for TestHomeEnv {
    fn drop(&mut self) {
        restore_env("HOME", self.old_home.take());
        restore_env("XDG_CACHE_HOME", self.old_cache_home.take());
    }
}

fn restore_env(key: &str, value: Option<OsString>) {
    unsafe {
        match value {
            Some(value) => std::env::set_var(key, value),
            None => std::env::remove_var(key),
        }
    }
}
