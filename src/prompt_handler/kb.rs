//! File-backed prompt-handler knowledge base loading and saving.

use crate::prompt_handler::schema::{PromptHandlerError, PromptKb, PromptResult};
use crate::prompt_handler::seeds::default_cases;
#[cfg(unix)]
#[allow(deprecated)]
use nix::fcntl::{FlockArg, flock};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
#[cfg(unix)]
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};

#[cfg(windows)]
#[derive(Clone, Copy)]
enum FlockArg {
    LockExclusive,
    Unlock,
}

pub fn load_or_bootstrap_kb(path: &Path) -> PromptResult<PromptKb> {
    tracing::info!(path = %path.display(), "prompt KB load start");
    let lock = lock_for_path(path, FlockArg::LockExclusive)?;
    tracing::info!(path = %path.display(), "prompt KB lock acquired for load");

    let result = if path.exists() {
        load_existing_kb(path)
    } else {
        tracing::info!(
            path = %path.display(),
            "prompt KB missing; bootstrapping built-in cases"
        );
        let kb = PromptKb::new(default_cases());
        write_kb_unlocked(path, &kb).map(|()| kb)
    };

    unlock_for_path(path, &lock)?;
    tracing::info!(path = %path.display(), "prompt KB load complete");
    result
}

pub fn save_kb_atomic(path: &Path, kb: &PromptKb) -> PromptResult<()> {
    tracing::info!(path = %path.display(), "prompt KB save start");
    let lock = lock_for_path(path, FlockArg::LockExclusive)?;
    tracing::info!(path = %path.display(), "prompt KB lock acquired for save");
    let result = write_kb_unlocked(path, kb);
    unlock_for_path(path, &lock)?;
    tracing::info!(path = %path.display(), "prompt KB save complete");
    result
}

fn load_existing_kb(path: &Path) -> PromptResult<PromptKb> {
    tracing::info!(path = %path.display(), "prompt KB read start");
    let raw = fs::read_to_string(path).map_err(|err| {
        tracing::error!(
            path = %path.display(),
            error = %err,
            reason = "failed to read prompt KB",
            impact = "prompt-handler cannot use saved prompt cases",
            "prompt KB read failed"
        );
        PromptHandlerError::Io(err)
    })?;
    let kb = serde_json::from_str::<PromptKb>(&raw).map_err(|err| {
        tracing::warn!(
            path = %path.display(),
            error = %err,
            reason = "failed to parse prompt KB JSON",
            impact = "prompt-handler cannot use saved prompt cases",
            "prompt KB parse failed"
        );
        PromptHandlerError::Json(err)
    })?;
    kb.validate()?;
    tracing::info!(path = %path.display(), "prompt KB read complete");
    Ok(kb)
}

fn write_kb_unlocked(path: &Path, kb: &PromptKb) -> PromptResult<()> {
    kb.validate()?;
    let parent = path.parent().ok_or_else(|| {
        let message = format!("path has no parent: {}", path.display());
        tracing::error!(
            path = %path.display(),
            reason = %message,
            impact = "prompt KB cannot be written",
            "prompt KB path invalid"
        );
        PromptHandlerError::InvalidKb(message)
    })?;
    tracing::info!(path = %path.display(), "prompt KB write start");
    fs::create_dir_all(parent).map_err(|err| {
        tracing::error!(
            path = %parent.display(),
            error = %err,
            reason = "failed to create prompt KB parent directory",
            impact = "prompt KB cannot be written",
            "prompt KB mkdir failed"
        );
        PromptHandlerError::Io(err)
    })?;

    let tmp_path = tmp_path_for(path);
    let serialized = serde_json::to_vec_pretty(kb).map_err(|err| {
        tracing::error!(
            path = %path.display(),
            error = %err,
            reason = "failed to serialize prompt KB",
            impact = "prompt KB cannot be written",
            "prompt KB serialization failed"
        );
        PromptHandlerError::Json(err)
    })?;
    {
        let mut tmp = File::create(&tmp_path).map_err(|err| {
            tracing::error!(
                path = %tmp_path.display(),
                error = %err,
                reason = "failed to create prompt KB temp file",
                impact = "prompt KB cannot be written atomically",
                "prompt KB temp create failed"
            );
            PromptHandlerError::Io(err)
        })?;
        tmp.write_all(&serialized).map_err(|err| {
            tracing::error!(
                path = %tmp_path.display(),
                error = %err,
                reason = "failed to write prompt KB temp file",
                impact = "prompt KB cannot be written atomically",
                "prompt KB temp write failed"
            );
            PromptHandlerError::Io(err)
        })?;
        tmp.write_all(b"\n").map_err(|err| {
            tracing::error!(
                path = %tmp_path.display(),
                error = %err,
                reason = "failed to finish prompt KB temp file",
                impact = "prompt KB cannot be written atomically",
                "prompt KB temp newline write failed"
            );
            PromptHandlerError::Io(err)
        })?;
        tmp.sync_all().map_err(|err| {
            tracing::error!(
                path = %tmp_path.display(),
                error = %err,
                reason = "failed to fsync prompt KB temp file",
                impact = "prompt KB durability is not guaranteed",
                "prompt KB temp fsync failed"
            );
            PromptHandlerError::Io(err)
        })?;
    }

    fs::rename(&tmp_path, path).map_err(|err| {
        tracing::error!(
            src = %tmp_path.display(),
            dst = %path.display(),
            error = %err,
            reason = "failed to atomically rename prompt KB temp file",
            impact = "prompt KB update did not take effect",
            "prompt KB rename failed"
        );
        PromptHandlerError::Io(err)
    })?;
    fsync_dir(parent)?;
    tracing::info!(path = %path.display(), "prompt KB write complete");
    Ok(())
}

fn tmp_path_for(path: &Path) -> PathBuf {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("prompt-cases.json");
    path.with_file_name(format!(".{file_name}.{}.tmp", std::process::id()))
}

fn lock_path_for(path: &Path) -> PathBuf {
    path.with_extension("lock")
}

fn lock_for_path(path: &Path, arg: FlockArg) -> PromptResult<File> {
    let lock_path = lock_path_for(path);
    if let Some(parent) = lock_path.parent() {
        fs::create_dir_all(parent).map_err(|err| {
            tracing::error!(
                path = %path.display(),
                lock_path = %lock_path.display(),
                error = %err,
                reason = "failed to create prompt KB lock parent directory",
                impact = "prompt KB cannot be safely read or written",
                "prompt KB lock mkdir failed"
            );
            PromptHandlerError::Io(err)
        })?;
    }
    tracing::info!(
        path = %path.display(),
        lock_path = %lock_path.display(),
        "prompt KB lock start"
    );
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)
        .map_err(|err| {
            tracing::error!(
                path = %path.display(),
                lock_path = %lock_path.display(),
                error = %err,
                reason = "failed to open prompt KB lock file",
                impact = "prompt KB cannot be safely read or written",
                "prompt KB lock open failed"
            );
            PromptHandlerError::Io(err)
        })?;
    flock_file(&file, arg).map_err(|err| {
        tracing::error!(
            path = %path.display(),
            lock_path = %lock_path.display(),
            error = %err,
            reason = "failed to acquire prompt KB advisory lock",
            impact = "prompt KB cannot be safely read or written",
            "prompt KB lock failed"
        );
        PromptHandlerError::Lock(err.to_string())
    })?;
    Ok(file)
}

fn unlock_for_path(path: &Path, file: &File) -> PromptResult<()> {
    tracing::info!(path = %path.display(), "prompt KB unlock start");
    flock_file(file, FlockArg::Unlock).map_err(|err| {
        tracing::error!(
            path = %path.display(),
            error = %err,
            reason = "failed to release prompt KB advisory lock",
            impact = "lock may remain held until process exits",
            "prompt KB unlock failed"
        );
        PromptHandlerError::Lock(err.to_string())
    })?;
    tracing::info!(path = %path.display(), "prompt KB unlock complete");
    Ok(())
}

#[allow(deprecated)]
fn flock_file(file: &File, arg: FlockArg) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        let _ = (file, arg);
        Ok(())
    }

    #[cfg(unix)]
    {
        flock(file.as_raw_fd(), arg).map_err(std::io::Error::from)
    }
}

fn fsync_dir(path: &Path) -> PromptResult<()> {
    let dir = File::open(path).map_err(|err| {
        tracing::error!(
            path = %path.display(),
            error = %err,
            reason = "failed to open prompt KB parent directory for fsync",
            impact = "prompt KB rename durability is not guaranteed",
            "prompt KB dir open failed"
        );
        PromptHandlerError::Io(err)
    })?;
    dir.sync_all().map_err(|err| {
        tracing::error!(
            path = %path.display(),
            error = %err,
            reason = "failed to fsync prompt KB parent directory",
            impact = "prompt KB rename durability is not guaranteed",
            "prompt KB dir fsync failed"
        );
        PromptHandlerError::Io(err)
    })
}

#[cfg(test)]
mod tests {
    use super::{load_or_bootstrap_kb, save_kb_atomic};
    use crate::prompt_handler::schema::{
        PromptAction, PromptCase, PromptFingerprint, PromptHandlerError, PromptKb,
    };

    fn user_case(id: &str) -> PromptCase {
        PromptCase {
            id: id.to_string(),
            provider: Some("test".to_string()),
            fingerprint: PromptFingerprint::Regex {
                pattern: "custom prompt".to_string(),
            },
            action: vec![PromptAction::Literal {
                value: "yes".to_string(),
            }],
            category: "auto-accept".to_string(),
            description: Some("user case".to_string()),
            confidence_threshold: Some(0.9),
            used_count: 7,
            created_at: None,
            last_used_at: None,
            created_by: Some("master-manual".to_string()),
            regex_flags: Vec::new(),
            trigger_state: None,
        }
    }

    #[test]
    fn bootstrap_writes_default_cases_when_file_is_missing() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("prompt-cases.json");

        let kb = load_or_bootstrap_kb(&path).unwrap();

        assert!(path.exists());
        assert_eq!(kb.version, "1");
        assert!(kb.cases.iter().any(|case| case.id == "codex_update_01"));
        assert!(kb.cases.iter().any(|case| case.id == "trust_path_01"));
    }

    #[test]
    fn existing_user_file_takes_priority_over_default_cases() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("prompt-cases.json");
        let kb = PromptKb::new(vec![user_case("user_custom_01")]);
        save_kb_atomic(&path, &kb).unwrap();

        let loaded = load_or_bootstrap_kb(&path).unwrap();

        assert_eq!(loaded.cases.len(), 1);
        assert_eq!(loaded.cases[0].id, "user_custom_01");
        assert_eq!(loaded.cases[0].used_count, 7);
    }

    #[test]
    fn bad_json_returns_typed_error() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("prompt-cases.json");
        std::fs::write(&path, "{not json").unwrap();

        let err = load_or_bootstrap_kb(&path).unwrap_err();

        assert!(matches!(err, PromptHandlerError::Json(_)));
    }

    #[test]
    fn save_is_atomic_and_loadable() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("nested").join("prompt-cases.json");
        let kb = PromptKb::new(vec![user_case("atomic_01")]);

        save_kb_atomic(&path, &kb).unwrap();
        let loaded = load_or_bootstrap_kb(&path).unwrap();

        assert_eq!(loaded.cases[0].id, "atomic_01");
        let leftovers = std::fs::read_dir(path.parent().unwrap())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().ends_with(".tmp"))
            .count();
        assert_eq!(leftovers, 0);
    }
}
