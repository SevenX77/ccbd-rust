#![cfg(unix)]

use ah::credentials::{
    Layer1CredentialRefresher, materialize_layer1_worker_credentials,
    private_credentials_path_for_worker_home, true_refresh_token, worker_credentials_path,
};
use ah::error::CcbdError;
use serde_json::{Value, json};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn layer1_worker_claude_credentials_are_regular_neutered_file() {
    let source_home = tempfile::tempdir().unwrap();
    let cache_home = tempfile::tempdir().unwrap();
    let worker_home = cache_home.path().join("ah/sandboxes/worker-a");
    write_source_credentials(source_home.path(), "true-refresh", now_ms() + 3_600_000);

    let private_path = materialize_layer1_worker_credentials(source_home.path(), &worker_home)
        .unwrap()
        .unwrap();
    let worker_path = worker_credentials_path(&worker_home);
    let metadata = std::fs::symlink_metadata(&worker_path).unwrap();

    assert!(metadata.file_type().is_file());
    assert!(!metadata.file_type().is_symlink());

    let private_credentials = read_json(&private_path);
    let worker_credentials = read_json(&worker_path);
    assert_eq!(
        true_refresh_token(&private_credentials),
        Some("true-refresh")
    );
    assert_neutered_preserves_non_refresh_fields(&private_credentials, &worker_credentials);
}

#[test]
fn layer1_private_credentials_are_0600_and_outside_worker_sandbox() {
    let source_home = tempfile::tempdir().unwrap();
    let cache_home = tempfile::tempdir().unwrap();
    let worker_home = cache_home.path().join("ah/sandboxes/worker-b");
    write_source_credentials(source_home.path(), "true-refresh", now_ms() + 3_600_000);

    let private_path = materialize_layer1_worker_credentials(source_home.path(), &worker_home)
        .unwrap()
        .unwrap();
    let mode = std::fs::metadata(&private_path)
        .unwrap()
        .permissions()
        .mode()
        & 0o777;

    assert_eq!(mode, 0o600);
    assert!(!private_path.starts_with(&worker_home));
    assert!(!private_path.starts_with(cache_home.path().join("ah/sandboxes/worker-b")));
    assert_eq!(
        private_path,
        private_credentials_path_for_worker_home(&worker_home)
    );
}

#[test]
fn layer1_concurrent_refresh_is_singleflight() {
    let source_home = tempfile::tempdir().unwrap();
    let cache_home = tempfile::tempdir().unwrap();
    let worker_homes = (0..8)
        .map(|idx| cache_home.path().join(format!("ah/sandboxes/worker-{idx}")))
        .collect::<Vec<_>>();
    let initial_expires_at = now_ms() - 1_000;
    write_source_credentials(source_home.path(), "true-refresh", initial_expires_at);
    let private_path = materialize_layer1_worker_credentials(source_home.path(), &worker_homes[0])
        .unwrap()
        .unwrap();

    let refresher = Layer1CredentialRefresher::new(private_path.clone());
    let upstream_calls = Arc::new(AtomicUsize::new(0));
    let mut handles = Vec::new();
    for _ in 0..8 {
        let refresher = refresher.clone();
        let worker_homes = worker_homes.clone();
        let upstream_calls = Arc::clone(&upstream_calls);
        handles.push(thread::spawn(move || {
            refresher
                .refresh_if_needed(&worker_homes, move |mut current| {
                    upstream_calls.fetch_add(1, Ordering::SeqCst);
                    let oauth = current
                        .get_mut("claudeAiOauth")
                        .and_then(Value::as_object_mut)
                        .ok_or_else(|| CcbdError::EnvironmentNotSupported {
                            details: "missing oauth object".to_string(),
                        })?;
                    oauth.insert(
                        "accessToken".to_string(),
                        Value::String("refreshed-access".to_string()),
                    );
                    oauth.insert("expiresAt".to_string(), Value::from(now_ms() + 3_600_000));
                    Ok(current)
                })
                .unwrap();
        }));
    }
    for handle in handles {
        handle.join().unwrap();
    }

    assert_eq!(upstream_calls.load(Ordering::SeqCst), 1);
    for worker_home in worker_homes {
        let worker_credentials = read_json(&worker_credentials_path(&worker_home));
        assert_eq!(
            worker_credentials["claudeAiOauth"]["accessToken"],
            "refreshed-access"
        );
        assert_ne!(
            true_refresh_token(&worker_credentials),
            Some("true-refresh")
        );
        assert_eq!(
            worker_credentials["claudeAiOauth"]["refreshTokenExpiresAt"],
            initial_expires_at + 86_400_000
        );
        assert_eq!(
            worker_credentials["claudeAiOauth"]["scopes"],
            json!(["user:inference"])
        );
        assert_eq!(
            worker_credentials["claudeAiOauth"]["subscriptionType"],
            "pro"
        );
    }
}

fn write_source_credentials(home: &Path, refresh_token: &str, expires_at: i64) {
    let claude_dir = home.join(".claude");
    std::fs::create_dir_all(&claude_dir).unwrap();
    std::fs::write(
        claude_dir.join(".credentials.json"),
        serde_json::to_string_pretty(&json!({
            "claudeAiOauth": {
                "accessToken": "source-access",
                "refreshToken": refresh_token,
                "expiresAt": expires_at,
                "refreshTokenExpiresAt": expires_at + 86_400_000,
                "scopes": ["user:inference"],
                "subscriptionType": "pro"
            }
        }))
        .unwrap(),
    )
    .unwrap();
}

fn read_json(path: &Path) -> Value {
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn assert_neutered_preserves_non_refresh_fields(private: &Value, worker: &Value) {
    assert_ne!(true_refresh_token(worker), true_refresh_token(private));
    for key in [
        "accessToken",
        "expiresAt",
        "refreshTokenExpiresAt",
        "scopes",
        "subscriptionType",
    ] {
        assert_eq!(
            worker["claudeAiOauth"][key], private["claudeAiOauth"][key],
            "field {key} must be preserved in the neutered worker copy"
        );
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
}
