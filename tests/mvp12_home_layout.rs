use ccbd::provider::home_layout::prepare_home_layout;
use serde_json::json;
use std::path::Path;

#[test]
fn test_provider_home_layout_materialization() {
    let host_home = tempfile::tempdir().unwrap();
    let cache_home = tempfile::tempdir().unwrap();
    let project_root = tempfile::tempdir().unwrap();

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
        host_home.path().join(".claude.json"),
        "{\"trusted\":true}\n",
    )
    .unwrap();
    std::fs::create_dir_all(host_home.path().join(".claude")).unwrap();
    std::fs::write(
        host_home.path().join(".claude/.credentials.json"),
        "{\"token\":\"host\"}\n",
    )
    .unwrap();

    let old_home = std::env::var_os("HOME");
    let old_cache = std::env::var_os("XDG_CACHE_HOME");
    unsafe {
        std::env::set_var("HOME", host_home.path());
        std::env::set_var("XDG_CACHE_HOME", cache_home.path());
    }

    let gemini = prepare_home_layout("gemini", project_root.path()).unwrap();
    let trusted = read_json(
        gemini
            .home_root
            .join(".gemini/trustedFolders.json")
            .as_path(),
    );
    assert_eq!(trusted["/host/project"], true);
    assert_eq!(trusted["/home/agent"], "TRUST_FOLDER");
    assert_eq!(
        gemini.extra_env.get("GEMINI_ROOT").unwrap(),
        "/home/agent/.gemini/tmp"
    );

    let claude = prepare_home_layout("claude", project_root.path()).unwrap();
    let claude_trust = read_json(claude.home_root.join(".claude.json").as_path());
    assert_eq!(claude_trust["trusted"], true);
    assert_eq!(
        claude_trust["projects"]["/home/agent"]["hasTrustDialogAccepted"],
        true
    );
    assert_eq!(
        claude_trust["projects"]["/home/agent"]["projectOnboardingSeenCount"],
        1
    );
    assert_eq!(
        claude_trust["projects"]["/home/agent"]["allowedTools"],
        json!([])
    );
    assert_eq!(
        claude_trust["projects"]["/home/agent"]["mcpServers"],
        json!({})
    );
    let credentials = claude.home_root.join(".claude/.credentials.json");
    assert!(credentials.is_symlink());
    assert_eq!(
        credentials.canonicalize().unwrap(),
        host_home
            .path()
            .join(".claude/.credentials.json")
            .canonicalize()
            .unwrap()
    );
    assert_eq!(
        claude.extra_env.get("CLAUDE_PROJECTS_ROOT").unwrap(),
        "/home/agent/.claude/projects"
    );

    let codex = prepare_home_layout("codex", project_root.path()).unwrap();
    let codex_config_path = codex.home_root.join(".codex/config.toml");
    assert!(codex_config_path.is_file());
    let codex_config: toml::Value = std::fs::read_to_string(&codex_config_path)
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(
        codex_config["projects"]["/home/agent"]["trust_level"].as_str(),
        Some("trusted")
    );
    assert!(codex.home_root.join(".codex/sessions").is_dir());
    assert_eq!(
        codex.extra_env.get("CODEX_HOME").unwrap(),
        "/home/agent/.codex"
    );
    assert_eq!(
        codex.extra_env.get("CODEX_SESSION_ROOT").unwrap(),
        "/home/agent/.codex/sessions"
    );

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
