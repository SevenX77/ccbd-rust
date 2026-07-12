use crate::error::CcbdError;
use serde_json::{Map, Value};
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

pub const CLAUDE_CREDENTIALS_RELATIVE: &str = ".claude/.credentials.json";
const DEFAULT_TOKEN_TTL_MS: i64 = 60 * 60 * 1000;
const REFRESH_AT_REMAINING_TTL_RATIO: i64 = 20;

#[derive(Clone, Debug)]
pub struct Layer1CredentialRefresher {
    private_path: PathBuf,
    gate: Arc<Mutex<()>>,
}

impl Layer1CredentialRefresher {
    pub fn new(private_path: PathBuf) -> Self {
        Self {
            private_path,
            gate: Arc::new(Mutex::new(())),
        }
    }

    pub fn refresh_if_needed<F>(
        &self,
        worker_homes: &[PathBuf],
        refresh: F,
    ) -> Result<bool, CcbdError>
    where
        F: FnOnce(Value) -> Result<Value, CcbdError>,
    {
        let _guard = self
            .gate
            .lock()
            .map_err(|_| CcbdError::EnvironmentNotSupported {
                details: "layer1 credential refresh lock poisoned".to_string(),
            })?;
        let current = read_json_file(&self.private_path)?;
        if !needs_refresh(&current, now_ms()) {
            return Ok(false);
        }

        let refreshed = refresh(current)?;
        write_private_credentials(&self.private_path, &refreshed)?;
        for worker_home in worker_homes {
            materialize_neutered_worker_credentials_from_private(&self.private_path, worker_home)?;
        }
        Ok(true)
    }
}

pub fn materialize_layer1_worker_credentials(
    source_home: &Path,
    worker_home: &Path,
) -> Result<Option<PathBuf>, CcbdError> {
    let source = source_home.join(CLAUDE_CREDENTIALS_RELATIVE);
    if !source.is_file() {
        return Ok(None);
    }

    let private_path = private_credentials_path_for_worker_home(worker_home);
    seed_private_credentials_once(&source, &private_path)?;
    materialize_neutered_worker_credentials_from_private(&private_path, worker_home)?;
    Ok(Some(private_path))
}

pub fn private_credentials_path_for_worker_home(worker_home: &Path) -> PathBuf {
    if let Some(ah_root) = ah_root_before_sandboxes(worker_home) {
        return ah_root.join("credentials/claude/.credentials.json");
    }
    let outside_worker = worker_home
        .parent()
        .and_then(Path::parent)
        .unwrap_or_else(|| Path::new("/tmp"));
    outside_worker.join(".ahd/credentials/claude/.credentials.json")
}

pub fn seed_private_credentials_once(source: &Path, private_path: &Path) -> Result<(), CcbdError> {
    if private_path.exists() {
        ensure_private_permissions(private_path)?;
        return Ok(());
    }
    let value = read_json_file(source)?;
    write_private_credentials(private_path, &value)
}

pub fn materialize_neutered_worker_credentials_from_private(
    private_path: &Path,
    worker_home: &Path,
) -> Result<(), CcbdError> {
    let private = read_json_file(private_path)?;
    let target = worker_home.join(CLAUDE_CREDENTIALS_RELATIVE);
    write_worker_credentials(&target, &neutered_credentials(&private)?)
}

pub fn true_refresh_token(credentials: &Value) -> Option<&str> {
    credentials
        .get("claudeAiOauth")
        .and_then(Value::as_object)
        .and_then(|oauth| oauth.get("refreshToken"))
        .and_then(Value::as_str)
}

pub fn worker_credentials_path(worker_home: &Path) -> PathBuf {
    worker_home.join(CLAUDE_CREDENTIALS_RELATIVE)
}

fn neutered_credentials(private: &Value) -> Result<Value, CcbdError> {
    let oauth = private
        .get("claudeAiOauth")
        .and_then(Value::as_object)
        .ok_or_else(|| CcbdError::EnvironmentNotSupported {
            details: "claude credentials missing claudeAiOauth object".to_string(),
        })?;

    let mut neutered_oauth = Map::new();
    if let Some(access_token) = oauth.get("accessToken") {
        neutered_oauth.insert("accessToken".to_string(), access_token.clone());
    }
    if let Some(expires_at) = oauth.get("expiresAt") {
        neutered_oauth.insert("expiresAt".to_string(), expires_at.clone());
    }
    neutered_oauth.insert(
        "refreshToken".to_string(),
        Value::String(format!("ahd-neutered-{}", Uuid::new_v4())),
    );

    let mut root = Map::new();
    root.insert("claudeAiOauth".to_string(), Value::Object(neutered_oauth));
    Ok(Value::Object(root))
}

fn needs_refresh(credentials: &Value, now_ms: i64) -> bool {
    let Some(expires_at) = credentials
        .get("claudeAiOauth")
        .and_then(Value::as_object)
        .and_then(|oauth| oauth.get("expiresAt"))
        .and_then(Value::as_i64)
    else {
        return true;
    };
    let refresh_lead_ms = DEFAULT_TOKEN_TTL_MS * REFRESH_AT_REMAINING_TTL_RATIO / 100;
    expires_at <= now_ms + refresh_lead_ms
}

fn ah_root_before_sandboxes(path: &Path) -> Option<PathBuf> {
    let mut out = PathBuf::new();
    for component in path.components() {
        if component == Component::Normal("sandboxes".as_ref()) {
            return Some(out);
        }
        out.push(component.as_os_str());
    }
    None
}

fn read_json_file(path: &Path) -> Result<Value, CcbdError> {
    let data = fs::read_to_string(path).map_err(|err| io_err("read credentials", path, err))?;
    serde_json::from_str(&data).map_err(|err| CcbdError::EnvironmentNotSupported {
        details: format!("parse credentials {}: {err}", path.display()),
    })
}

fn write_private_credentials(path: &Path, value: &Value) -> Result<(), CcbdError> {
    atomic_write_json_0600(path, value)
}

fn write_worker_credentials(path: &Path, value: &Value) -> Result<(), CcbdError> {
    atomic_write_json_0600(path, value)
}

fn atomic_write_json_0600(path: &Path, value: &Value) -> Result<(), CcbdError> {
    let parent = path
        .parent()
        .ok_or_else(|| CcbdError::EnvironmentNotSupported {
            details: format!("credentials path has no parent: {}", path.display()),
        })?;
    fs::create_dir_all(parent).map_err(|err| io_err("create credentials parent", parent, err))?;

    if path.exists() {
        let metadata = fs::symlink_metadata(path)
            .map_err(|err| io_err("stat existing credentials", path, err))?;
        if metadata.file_type().is_symlink() || metadata.file_type().is_file() {
            fs::remove_file(path)
                .map_err(|err| io_err("remove existing credentials", path, err))?;
        } else {
            return Err(CcbdError::EnvironmentNotSupported {
                details: format!("credentials target is not a file: {}", path.display()),
            });
        }
    }

    let tmp = parent.join(format!(".credentials.json.tmp-{}", Uuid::new_v4()));
    let data =
        serde_json::to_vec_pretty(value).map_err(|err| CcbdError::EnvironmentNotSupported {
            details: format!("serialize credentials {}: {err}", path.display()),
        })?;
    fs::write(&tmp, data).map_err(|err| io_err("write credentials temp", &tmp, err))?;
    #[cfg(unix)]
    fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600))
        .map_err(|err| io_err("chmod credentials temp", &tmp, err))?;
    fs::rename(&tmp, path).map_err(|err| io_err("rename credentials", path, err))?;
    ensure_private_permissions(path)
}

fn ensure_private_permissions(path: &Path) -> Result<(), CcbdError> {
    #[cfg(unix)]
    {
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))
            .map_err(|err| io_err("chmod credentials", path, err))?;
    }
    Ok(())
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}

fn io_err(action: &str, path: &Path, err: std::io::Error) -> CcbdError {
    CcbdError::EnvironmentNotSupported {
        details: format!("{action} {}: {err}", path.display()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::thread;

    #[test]
    fn layer1_worker_credentials_are_regular_neutered_file() {
        let source_home = tempfile::tempdir().unwrap();
        let cache_home = tempfile::tempdir().unwrap();
        let worker_home = cache_home.path().join("ah/sandboxes/worker-a");
        write_source_credentials(source_home.path(), "true-refresh", now_ms() + 3_600_000);

        let private_path = materialize_layer1_worker_credentials(source_home.path(), &worker_home)
            .unwrap()
            .unwrap();
        let worker_path = worker_credentials_path(&worker_home);
        let metadata = fs::symlink_metadata(&worker_path).unwrap();

        assert!(metadata.file_type().is_file());
        assert!(!metadata.file_type().is_symlink());

        let private_credentials = read_json_file(&private_path).unwrap();
        let worker_credentials = read_json_file(&worker_path).unwrap();
        assert_eq!(
            true_refresh_token(&private_credentials),
            Some("true-refresh")
        );
        assert_ne!(
            true_refresh_token(&worker_credentials),
            true_refresh_token(&private_credentials)
        );
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
        #[cfg(unix)]
        let mode = fs::metadata(&private_path).unwrap().permissions().mode() & 0o777;

        #[cfg(unix)]
        assert_eq!(mode, 0o600);
        assert!(!private_path.starts_with(&worker_home));
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
        write_source_credentials(source_home.path(), "true-refresh", now_ms() - 1_000);
        let private_path =
            materialize_layer1_worker_credentials(source_home.path(), &worker_homes[0])
                .unwrap()
                .unwrap();

        let refresher = Layer1CredentialRefresher::new(private_path);
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
            let worker_credentials =
                read_json_file(&worker_credentials_path(&worker_home)).unwrap();
            assert_eq!(
                worker_credentials["claudeAiOauth"]["accessToken"],
                "refreshed-access"
            );
            assert_ne!(
                true_refresh_token(&worker_credentials),
                Some("true-refresh")
            );
        }
    }

    fn write_source_credentials(home: &Path, refresh_token: &str, expires_at: i64) {
        let claude_dir = home.join(".claude");
        fs::create_dir_all(&claude_dir).unwrap();
        fs::write(
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
}
