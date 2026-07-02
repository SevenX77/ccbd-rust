//! Linux systemd user service helpers for ahd.

use crate::provider::manifest::ENV_PASSTHROUGH;
use sha2::{Digest, Sha256};
use std::env;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ServiceUnitError {
    #[error("systemd unit value contains unsupported control character")]
    InvalidValue,

    #[error("ahd binary path must be absolute: {0}")]
    RelativeAhdPath(PathBuf),

    #[error("neither XDG_CONFIG_HOME nor HOME is set for user systemd dir")]
    MissingUserConfigHome,
}

pub fn derive_unit_name(state_dir: &Path) -> String {
    let normalized = normalized_absolute_path(state_dir);
    let mut hasher = Sha256::new();
    hasher.update(normalized.to_string_lossy().as_bytes());
    let digest = hasher.finalize();
    let hex = format!("{digest:x}");
    format!("ah-{}.service", &hex[..16])
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
    let mut cmd = vec![
        "systemd-run".to_string(),
        "--user".to_string(),
        "--unit=ahd.service".to_string(),
        "--property=Restart=on-failure".to_string(),
        "--property=RestartSec=1s".to_string(),
        "--property=StartLimitIntervalSec=60".to_string(),
        "--property=StartLimitBurst=5".to_string(),
        "--property=OOMScoreAdjust=-900".to_string(),
        "--setenv".to_string(),
        format!("AH_STATE_DIR={}", state_dir.display()),
    ];
    for key in ENV_PASSTHROUGH {
        if let Some((_, value)) = env.iter().find(|(candidate, _)| candidate == key) {
            cmd.push("--setenv".to_string());
            cmd.push(format!("{key}={value}"));
        }
    }
    cmd.push(ahd_bin.display().to_string());
    cmd
}

pub fn ahd_reset_failed_is_best_effort(unit: &str) -> bool {
    if let Err(err) = Command::new("systemctl")
        .args(["--user", "reset-failed", unit])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
    {
        tracing::warn!(unit, error = %err, "systemctl reset-failed failed before ahd bootstrap");
    }
    true
}

pub fn escape_systemd_env_value(value: &str) -> Result<String, ServiceUnitError> {
    escape_common(value, false)
}

pub fn escape_systemd_exec_token(value: &str) -> Result<String, ServiceUnitError> {
    escape_common(value, true)
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

    let ahd_bin = normalized_absolute_path(ahd_bin);
    let state_dir = normalized_absolute_path(state_dir);
    let ahd_bin = escape_systemd_exec_token(&ahd_bin.to_string_lossy())?;
    let state_dir_value = escape_systemd_env_value(&state_dir.to_string_lossy())?;

    let mut unit = String::new();
    unit.push_str(&format!(
        "# ah-generated unit; AH_STATE_DIR={}\n",
        state_dir.display()
    ));
    unit.push_str("[Unit]\n");
    unit.push_str("Description=ah daemon\n");
    unit.push_str("StartLimitIntervalSec=60\n");
    unit.push_str("StartLimitBurst=5\n\n");
    unit.push_str("[Service]\n");
    unit.push_str("Type=simple\n");
    unit.push_str(&format!("ExecStart={ahd_bin}\n"));
    unit.push_str("Restart=on-failure\n");
    unit.push_str("RestartSec=1s\n");
    unit.push_str("OOMScoreAdjust=-900\n");
    unit.push_str(&format!("Environment=AH_STATE_DIR={state_dir_value}\n"));
    for key in ENV_PASSTHROUGH {
        if let Some((_, value)) = env.iter().find(|(candidate, _)| candidate == key) {
            let escaped = escape_systemd_env_value(value)?;
            unit.push_str(&format!("Environment={key}={escaped}\n"));
        }
    }
    unit.push_str("\n[Install]\n");
    unit.push_str("WantedBy=default.target\n");

    let _ = unit_name;
    Ok(unit)
}

pub fn resolve_user_systemd_dir(
    xdg_config_home: Option<&str>,
    home: Option<&str>,
) -> Result<PathBuf, ServiceUnitError> {
    if let Some(xdg_config_home) = xdg_config_home.filter(|value| !value.is_empty()) {
        return Ok(PathBuf::from(xdg_config_home).join("systemd/user"));
    }
    if let Some(home) = home.filter(|value| !value.is_empty()) {
        return Ok(PathBuf::from(home).join(".config/systemd/user"));
    }
    Err(ServiceUnitError::MissingUserConfigHome)
}

pub fn resolve_user_systemd_dir_from_env() -> Result<PathBuf, ServiceUnitError> {
    let xdg_config_home = env::var("XDG_CONFIG_HOME").ok();
    let home = env::var("HOME").ok();
    resolve_user_systemd_dir(xdg_config_home.as_deref(), home.as_deref())
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

fn escape_common(value: &str, double_dollar: bool) -> Result<String, ServiceUnitError> {
    if value.chars().any(char::is_control) {
        return Err(ServiceUnitError::InvalidValue);
    }

    let mut out = String::with_capacity(value.len());
    let mut needs_quotes = value.is_empty();
    for ch in value.chars() {
        match ch {
            '$' if double_dollar => out.push_str("$$"),
            '%' => out.push_str("%%"),
            '"' => {
                needs_quotes = true;
                out.push_str("\\\"");
            }
            '\\' => {
                needs_quotes = true;
                out.push_str("\\\\");
            }
            ch if ch.is_whitespace() => {
                needs_quotes = true;
                out.push(ch);
            }
            ch => out.push(ch),
        }
    }

    if needs_quotes {
        Ok(format!("\"{out}\""))
    } else {
        Ok(out)
    }
}
