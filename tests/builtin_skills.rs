#![cfg(unix)]

use ah::provider::builtin;
use ah::provider::extensions::ExtensionConfig;
use ah::provider::home_layout::{HomeLayoutRole, prepare_home_layout_with_extensions_for_slot};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex, MutexGuard};

static ENV_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

struct HostFixture {
    _lock: MutexGuard<'static, ()>,
    _host_home: tempfile::TempDir,
    _cache_home: tempfile::TempDir,
    old_home: Option<OsString>,
    old_cache: Option<OsString>,
    old_user: Option<OsString>,
}

impl HostFixture {
    fn new() -> Self {
        let lock = ENV_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let host_home = tempfile::tempdir().unwrap();
        let cache_home = tempfile::tempdir().unwrap();
        let old_home = std::env::var_os("HOME");
        let old_cache = std::env::var_os("XDG_CACHE_HOME");
        let old_user = std::env::var_os("USER");
        unsafe {
            std::env::set_var("HOME", host_home.path());
            std::env::set_var("XDG_CACHE_HOME", cache_home.path());
            std::env::set_var("USER", "builtin-skills-test");
        }
        Self {
            _lock: lock,
            _host_home: host_home,
            _cache_home: cache_home,
            old_home,
            old_cache,
            old_user,
        }
    }
}

impl Drop for HostFixture {
    fn drop(&mut self) {
        unsafe {
            restore_env("HOME", &self.old_home);
            restore_env("XDG_CACHE_HOME", &self.old_cache);
            restore_env("USER", &self.old_user);
        }
    }
}

unsafe fn restore_env(key: &str, old: &Option<OsString>) {
    match old {
        Some(value) => unsafe { std::env::set_var(key, value) },
        None => unsafe { std::env::remove_var(key) },
    }
}

fn prepare(
    provider: &str,
    role: HomeLayoutRole,
    slot: &str,
) -> (tempfile::TempDir, tempfile::TempDir, PathBuf) {
    let sandbox = tempfile::tempdir().unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let layout = prepare_home_layout_with_extensions_for_slot(
        provider,
        sandbox.path(),
        workspace.path(),
        role,
        slot,
        &ExtensionConfig::default(),
        None,
    )
    .unwrap();
    (sandbox, workspace, layout.home_root)
}

fn skill_md_path(home_root: &Path, provider: &str) -> PathBuf {
    match provider {
        "claude" => home_root.join(".claude/skills/ah-commands/SKILL.md"),
        "codex" => home_root.join(".codex/skills/ah-commands/SKILL.md"),
        "antigravity" => home_root.join(".gemini/config/skills/ah-commands/SKILL.md"),
        other => panic!("unexpected provider {other}"),
    }
}

fn skill_dir_path(home_root: &Path, provider: &str) -> PathBuf {
    skill_md_path(home_root, provider)
        .parent()
        .unwrap()
        .to_path_buf()
}

#[test]
fn master_role_gets_builtin_ah_commands_for_all_wired_providers_without_extension_declaration() {
    let _fixture = HostFixture::new();

    for (provider, slot) in [
        ("claude", "master"),
        ("codex", "master"),
        ("antigravity", "master"),
    ] {
        let (_sandbox, _workspace, home_root) = prepare(provider, HomeLayoutRole::Master, slot);
        let skill_dir = skill_dir_path(&home_root, provider);
        let dir_metadata = std::fs::symlink_metadata(&skill_dir).unwrap();
        assert!(
            !dir_metadata.file_type().is_symlink(),
            "builtin skill directory must be written into the sandbox, not symlinked"
        );

        let skill_path = skill_md_path(&home_root, provider);
        let metadata = std::fs::symlink_metadata(&skill_path).unwrap();
        assert!(
            !metadata.file_type().is_symlink(),
            "builtin skill must be written into the sandbox, not symlinked"
        );
        let skill = std::fs::read_to_string(skill_path).unwrap();
        assert!(skill.contains("agent-facing"));
        assert!(skill.contains("ah ps"));
    }
}

#[test]
fn workers_do_not_get_master_only_ah_commands_builtin_skill() {
    let _fixture = HostFixture::new();

    for (provider, slot) in [("claude", "a4"), ("codex", "a1"), ("antigravity", "a3")] {
        let (_sandbox, _workspace, home_root) = prepare(provider, HomeLayoutRole::Worker, slot);
        assert!(
            !skill_md_path(&home_root, provider).exists(),
            "{provider} worker {slot} must not receive master-only ah-commands"
        );
    }
}

#[test]
fn master_kernel_points_to_ah_commands_skill_instead_of_command_enumeration() {
    assert!(!builtin::MASTER_KERNEL.contains("`ah watch <agent_id>`"));
    assert!(!builtin::MASTER_KERNEL.contains("`ah logs <agent_id>`"));
    assert!(!builtin::MASTER_KERNEL.contains("`ah ps`"));
    assert!(!builtin::MASTER_KERNEL.contains("`ah attach`"));
    assert!(builtin::MASTER_KERNEL.contains("ah-commands"));
    assert!(builtin::MASTER_KERNEL.contains("ah <command> --help"));
    assert!(builtin::MASTER_KERNEL.contains("ah master ack-ready"));
    assert!(builtin::MASTER_KERNEL.contains("`ah ask"));
}

#[test]
fn ah_commands_skill_frontmatter_and_scope_are_valid() {
    let skill_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("assets/builtin/skills/ah-commands/SKILL.md");
    let skill = std::fs::read_to_string(skill_path).unwrap();
    assert!(skill.contains("name: ah-commands"));
    let description = skill
        .lines()
        .find(|line| line.starts_with("description: "))
        .unwrap();
    assert!(description.starts_with("description: Use when"));
    assert!(description.contains("wait for a dispatched job to finish"));
    assert!(description.contains("follow or retrieve a running agent's output"));
    assert!(skill.contains("agent"));
    assert!(skill.contains("dispatch"));
    for excluded in ["ah start", "ah setup", "ah bundle"] {
        assert!(
            !skill.contains(excluded),
            "ah-commands skill must not document operational command item {excluded:?}"
        );
    }
}

#[test]
fn project_skill_named_ah_commands_is_rejected_before_materialization() {
    let _fixture = HostFixture::new();
    let sandbox = tempfile::tempdir().unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let skill_dir = workspace.path().join(".ah/skills/ah-commands");
    std::fs::create_dir_all(&skill_dir).unwrap();
    std::fs::write(skill_dir.join("SKILL.md"), "---\nname: ah-commands\n---\n").unwrap();

    let extensions = ExtensionConfig {
        skills: vec!["ah-commands".to_string()],
        ..ExtensionConfig::default()
    };
    let err = prepare_home_layout_with_extensions_for_slot(
        "claude",
        sandbox.path(),
        workspace.path(),
        HomeLayoutRole::Master,
        "master",
        &extensions,
        None,
    )
    .unwrap_err();
    let message = err.to_string();
    assert!(message.contains("skill name \"ah-commands\" is reserved by an ah builtin skill"));
    assert!(message.contains("rename the project skill"));
}

#[test]
fn builtin_ah_commands_materialization_is_idempotent_for_prior_builtin_write() {
    let _fixture = HostFixture::new();
    let sandbox = tempfile::tempdir().unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let layout = prepare_home_layout_with_extensions_for_slot(
        "claude",
        sandbox.path(),
        workspace.path(),
        HomeLayoutRole::Master,
        "master",
        &ExtensionConfig::default(),
        None,
    )
    .unwrap();
    let skill_path = skill_md_path(&layout.home_root, "claude");
    std::fs::write(&skill_path, "stale builtin skill").unwrap();

    prepare_home_layout_with_extensions_for_slot(
        "claude",
        sandbox.path(),
        workspace.path(),
        HomeLayoutRole::Master,
        "master",
        &ExtensionConfig::default(),
        None,
    )
    .unwrap();

    let skill = std::fs::read_to_string(skill_path).unwrap();
    assert!(skill.contains("name: ah-commands"));
    assert!(skill.contains("ah ps"));
}
