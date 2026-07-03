//! Windows service stubs for the M0 compile gate.

use crate::provider::manifest::ENV_PASSTHROUGH;
use sha2::{Digest, Sha256};
use std::env;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ServiceUnitError {
    #[error("Windows service value contains unsupported control character")]
    InvalidValue,

    #[error("ahd binary path must be absolute: {0}")]
    RelativeAhdPath(PathBuf),

    #[error("neither LOCALAPPDATA nor USERPROFILE is set for Windows ah config dir")]
    MissingUserConfigHome,
}

pub fn derive_unit_name(state_dir: &Path) -> String {
    let normalized = normalized_absolute_path(state_dir);
    let mut hasher = Sha256::new();
    hasher.update(normalized.to_string_lossy().as_bytes());
    let digest = hasher.finalize();
    let hex = format!("{digest:x}");
    format!("ah-{}", &hex[..16])
}

pub fn build_ahd_systemd_run_command(ahd_bin: &Path, state_dir: &Path) -> Vec<String> {
    let env = ENV_PASSTHROUGH
        .iter()
        .filter_map(|key| {
            std::env::var(key)
                .ok()
                .map(|value| ((*key).to_string(), value))
        })
        .collect::<Vec<_>>();
    build_ahd_systemd_run_command_with_env(ahd_bin, state_dir, &env)
}

pub fn build_ahd_systemd_run_command_with_env(
    ahd_bin: &Path,
    state_dir: &Path,
    env: &[(String, String)],
) -> Vec<String> {
    let mut cmd = Vec::new();
    for key in ENV_PASSTHROUGH {
        if let Some((_, value)) = env.iter().find(|(candidate, _)| candidate == key) {
            cmd.push(format!("{key}={value}"));
        }
    }
    cmd.push(format!("AH_STATE_DIR={}", state_dir.display()));
    cmd.push(ahd_bin.display().to_string());
    cmd
}

pub fn ahd_reset_failed_is_best_effort(_unit: &str) -> bool {
    true
}

pub fn escape_systemd_env_value(value: &str) -> Result<String, ServiceUnitError> {
    escape_common(value)
}

pub fn escape_systemd_exec_token(value: &str) -> Result<String, ServiceUnitError> {
    escape_common(value)
}

pub fn render_unit_file(
    unit_name: &str,
    ahd_bin: &Path,
    state_dir: &Path,
    env: &[(String, String)],
) -> Result<String, ServiceUnitError> {
    if !ahd_bin.is_absolute() {
        return Err(ServiceUnitError::RelativeAhdPath(ahd_bin.to_path_buf()));
    }

    let mut content = String::new();
    content.push_str("# ah-generated Windows task placeholder\n");
    content.push_str(&format!("Name={unit_name}\n"));
    content.push_str(&format!("Exec={}\n", ahd_bin.display()));
    content.push_str(&format!("AH_STATE_DIR={}\n", state_dir.display()));
    for (key, value) in env {
        content.push_str(&format!("{key}={}\n", escape_systemd_env_value(value)?));
    }
    Ok(content)
}

pub fn resolve_user_systemd_dir(
    local_app_data: Option<&str>,
    user_profile: Option<&str>,
) -> Result<PathBuf, ServiceUnitError> {
    if let Some(local_app_data) = local_app_data.filter(|value| !value.is_empty()) {
        return Ok(PathBuf::from(local_app_data).join("ah"));
    }
    if let Some(user_profile) = user_profile.filter(|value| !value.is_empty()) {
        return Ok(PathBuf::from(user_profile).join("AppData/Local/ah"));
    }
    Err(ServiceUnitError::MissingUserConfigHome)
}

pub fn resolve_user_systemd_dir_from_env() -> Result<PathBuf, ServiceUnitError> {
    let local_app_data = env::var("LOCALAPPDATA").ok();
    let user_profile = env::var("USERPROFILE").ok();
    resolve_user_systemd_dir(local_app_data.as_deref(), user_profile.as_deref())
}

pub fn atomic_write_unit(path: &Path, content: &str) -> io::Result<()> {
    let parent = path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("unit path has no parent: {}", path.display()),
        )
    })?;
    fs::create_dir_all(parent)?;

    let temp_path = unique_temp_path(parent, path);
    let mut temp = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp_path)?;
    temp.write_all(content.as_bytes())?;
    temp.sync_all()?;
    drop(temp);
    match fs::rename(&temp_path, path) {
        Ok(()) => Ok(()),
        Err(err) => {
            let _ = fs::remove_file(&temp_path);
            Err(err)
        }
    }
}

fn normalized_absolute_path(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| {
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            env::current_dir()
                .map(|cwd| cwd.join(path))
                .unwrap_or_else(|_| path.to_path_buf())
        }
    })
}

fn unique_temp_path(parent: &Path, target: &Path) -> PathBuf {
    let file_name = target
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("unit");
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    parent.join(format!(".{file_name}.{}.{}.tmp", std::process::id(), nanos))
}

fn escape_common(value: &str) -> Result<String, ServiceUnitError> {
    if value.chars().any(char::is_control) {
        return Err(ServiceUnitError::InvalidValue);
    }
    Ok(value.to_string())
}
