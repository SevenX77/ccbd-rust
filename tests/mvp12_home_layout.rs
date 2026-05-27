use ccbd::provider::home_layout::prepare_home_layout;
use serde_json::json;
use std::path::Path;

#[test]
fn test_provider_home_layout_materialization() {
    let host_home = tempfile::tempdir().unwrap();
    let cache_home = tempfile::tempdir().unwrap();
    let sandbox_root = tempfile::tempdir().unwrap();
    let project_root = tempfile::tempdir().unwrap();
    let workspace_key = project_root
        .path()
        .canonicalize()
        .unwrap()
        .display()
        .to_string();

    std::fs::create_dir_all(host_home.path().join(".gemini")).unwrap();
    std::fs::write(
        host_home.path().join(".gemini/trustedFolders.json"),
        serde_json::to_string_pretty(&json!({
            "/host/project": true,
            "/home/agent": false
        }))
        .unwrap(),
    )
    .unwrap();
    std::fs::write(
        host_home.path().join(".gemini/settings.json"),
        serde_json::to_string_pretty(&json!({
            "security": {"auth": {"selectedType": "oauth-personal"}},
            "ui": {"showMemoryUsage": true, "errorVerbosity": "full"}
        }))
        .unwrap(),
    )
    .unwrap();

    std::fs::write(
        host_home.path().join(".claude.json"),
        serde_json::to_string_pretty(&json!({
            "trusted": true,
            "hasCompletedOnboarding": true,
            "lastOnboardingVersion": "2.1.116"
        }))
        .unwrap(),
    )
    .unwrap();
    std::fs::create_dir_all(host_home.path().join(".claude")).unwrap();
    std::fs::write(
        host_home.path().join(".claude/.credentials.json"),
        "{\"token\":\"host\"}\n",
    )
    .unwrap();
    std::fs::create_dir_all(host_home.path().join(".codex")).unwrap();
    std::fs::write(
        host_home.path().join(".codex/auth.json"),
        "{\"token\":\"codex-host\"}\n",
    )
    .unwrap();

    let old_home = std::env::var_os("HOME");
    let old_cache = std::env::var_os("XDG_CACHE_HOME");
    unsafe {
        std::env::set_var("HOME", host_home.path());
        std::env::set_var("XDG_CACHE_HOME", cache_home.path());
    }

    let gemini = prepare_home_layout("gemini", sandbox_root.path(), project_root.path()).unwrap();
    let trusted = read_json(
        gemini
            .home_root
            .join(".gemini/trustedFolders.json")
            .as_path(),
    );
    assert_eq!(trusted["/host/project"], true);
    assert_eq!(trusted[workspace_key.as_str()], "TRUST_FOLDER");
    assert!(trusted.get("/home/agent").is_none());
    let gemini_settings = read_json(gemini.home_root.join(".gemini/settings.json").as_path());
    assert_eq!(
        gemini_settings["security"]["auth"]["selectedType"],
        "oauth-personal"
    );
    assert_eq!(gemini_settings["ui"]["showMemoryUsage"], true);
    assert_eq!(
        gemini.extra_env.get("GEMINI_CLI_HOME").unwrap(),
        &gemini.home_root.join(".gemini").display().to_string()
    );
    assert_eq!(
        gemini.extra_env.get("HOME").unwrap(),
        &gemini.home_root.display().to_string()
    );

    let claude = prepare_home_layout("claude", sandbox_root.path(), project_root.path()).unwrap();
    let claude_trust_path = claude.home_root.join(".claude.json");
    assert!(
        std::fs::symlink_metadata(&claude_trust_path)
            .unwrap()
            .file_type()
            .is_file()
    );
    let claude_trust = read_json(claude_trust_path.as_path());
    assert_eq!(claude_trust["trusted"], true);
    assert_eq!(
        claude_trust["projects"][workspace_key.as_str()]["hasTrustDialogAccepted"],
        true
    );
    assert!(claude_trust["projects"].get("/home/agent").is_none());
    let claude_config_dir_state = read_json(
        claude
            .home_root
            .join(".claude/.claude.json")
            .as_path(),
    );
    assert_eq!(claude_config_dir_state["trusted"], true);
    assert_eq!(claude_config_dir_state["hasCompletedOnboarding"], true);
    assert_eq!(
        claude_config_dir_state["lastOnboardingVersion"],
        "2.1.116"
    );
    let claude_settings = read_json(claude.home_root.join(".claude/settings.json").as_path());
    assert_eq!(claude_settings["skipDangerousModePermissionPrompt"], true);
    let credentials = claude.home_root.join(".claude/.credentials.json");
    assert!(
        std::fs::symlink_metadata(&credentials)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(
        std::fs::read_link(&credentials).unwrap(),
        host_home.path().join(".claude/.credentials.json")
    );
    assert_eq!(
        std::fs::read_to_string(&credentials).unwrap(),
        "{\"token\":\"host\"}\n"
    );
    assert_eq!(
        claude.extra_env.get("CLAUDE_CONFIG_DIR").unwrap(),
        &claude.home_root.join(".claude").display().to_string()
    );
    assert_eq!(
        claude.extra_env.get("HOME").unwrap(),
        &claude.home_root.display().to_string()
    );

    let codex = prepare_home_layout("codex", sandbox_root.path(), project_root.path()).unwrap();
    let codex_config_path = codex.home_root.join(".codex/config.toml");
    assert!(codex_config_path.is_file());
    let codex_config: toml::Value = std::fs::read_to_string(&codex_config_path)
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(
        codex_config["projects"][workspace_key.as_str()]["trust_level"].as_str(),
        Some("trusted")
    );
    assert!(codex_config["projects"].get("/home/agent").is_none());
    assert_eq!(
        codex_config["tui"]["model_availability_nux"]["gpt-5.5"].as_integer(),
        Some(4)
    );
    let codex_auth = codex.home_root.join(".codex/auth.json");
    assert!(
        std::fs::symlink_metadata(&codex_auth)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(
        std::fs::read_link(&codex_auth).unwrap(),
        host_home.path().join(".codex/auth.json")
    );
    assert_eq!(
        std::fs::read_to_string(&codex_auth).unwrap(),
        "{\"token\":\"codex-host\"}\n"
    );
    assert!(codex.home_root.join(".codex/sessions").is_dir());
    assert_eq!(
        codex.extra_env.get("CODEX_HOME").unwrap(),
        &codex.home_root.join(".codex").display().to_string()
    );
    assert_eq!(
        codex.extra_env.get("HOME").unwrap(),
        &codex.home_root.display().to_string()
    );
    assert!(!codex.extra_env.contains_key("CODEX_SESSION_ROOT"));

    restore_env("HOME", old_home);
    restore_env("XDG_CACHE_HOME", old_cache);
}

#[test]
fn trust_key_uses_canonical_workspace_path() {
    let host_home = tempfile::tempdir().unwrap();
    let cache_home = tempfile::tempdir().unwrap();
    let sandbox_root = tempfile::tempdir().unwrap();
    let workspace_parent = tempfile::tempdir().unwrap();
    let real_workspace = workspace_parent.path().join("real-workspace");
    let linked_workspace = workspace_parent.path().join("linked-workspace");
    std::fs::create_dir_all(&real_workspace).unwrap();
    std::os::unix::fs::symlink(&real_workspace, &linked_workspace).unwrap();
    let workspace_key = real_workspace.canonicalize().unwrap().display().to_string();
    let symlink_key = linked_workspace.display().to_string();

    std::fs::create_dir_all(host_home.path().join(".gemini")).unwrap();
    std::fs::write(host_home.path().join(".gemini/trustedFolders.json"), "{}\n").unwrap();

    let old_home = std::env::var_os("HOME");
    let old_cache = std::env::var_os("XDG_CACHE_HOME");
    unsafe {
        std::env::set_var("HOME", host_home.path());
        std::env::set_var("XDG_CACHE_HOME", cache_home.path());
    }

    let gemini = prepare_home_layout("gemini", sandbox_root.path(), &linked_workspace).unwrap();
    let trusted = read_json(
        gemini
            .home_root
            .join(".gemini/trustedFolders.json")
            .as_path(),
    );
    assert_eq!(trusted[workspace_key.as_str()], "TRUST_FOLDER");
    assert!(trusted.get(symlink_key.as_str()).is_none());
    assert!(trusted.get("/home/agent").is_none());

    restore_env("HOME", old_home);
    restore_env("XDG_CACHE_HOME", old_cache);
}

fn read_json(path: &Path) -> serde_json::Value {
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn restore_env(key: &str, old_value: Option<std::ffi::OsString>) {
    unsafe {
        if let Some(old_value) = old_value {
            std::env::set_var(key, old_value);
        } else {
            std::env::remove_var(key);
        }
    }
}
