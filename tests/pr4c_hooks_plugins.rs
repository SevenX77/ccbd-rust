use ccbd::cli::rpc_client::{CliError, RpcClient, RpcFuture};
use ccbd::cli::start::{StartOptions, start_from_options};
use ccbd::provider::extensions::{ExtensionConfig, HookGroup, HookItem};
use ccbd::provider::home_layout::prepare_home_layout_with_extensions;
use ccbd::provider::manifest::{collect_spawn_env, get_manifest};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::ffi::OsString;
use std::io::Read;
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex, MutexGuard};

static ENV_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

#[test]
fn claude_hook_materializes_settings_and_symlink() {
    let fixture = HostFixture::new();
    let script = fixture.write_host_file("scripts/audit.sh", "#!/bin/sh\nexit 0\n");
    let sandbox = tempfile::tempdir().unwrap();
    let workspace = tempfile::tempdir().unwrap();

    let layout = prepare_home_layout_with_extensions(
        "claude",
        sandbox.path(),
        workspace.path(),
        ccbd::provider::home_layout::HomeLayoutRole::Worker,
        &hooks_config("PreToolUse", script.to_str().unwrap()),
    )
    .unwrap();
    let settings = read_json(&layout.home_root.join(".claude/settings.json"));
    let hook = &settings["hooks"]["PreToolUse"][0];

    assert_eq!(hook["matcher"], "*");
    assert_eq!(hook["hooks"][0]["type"], "command");
    assert_eq!(
        hook["hooks"][0]["command"],
        layout
            .home_root
            .join(".claude/hooks/audit.sh")
            .display()
            .to_string()
    );
    assert_eq!(
        std::fs::read_link(layout.home_root.join(".claude/hooks/audit.sh")).unwrap(),
        script
    );
}

#[test]
fn codex_plugin_materializes_config_and_cache() {
    let fixture = HostFixture::new();
    let host_plugin = fixture
        .host_home()
        .join(".codex/plugins/cache/github@openai-curated");
    std::fs::create_dir_all(&host_plugin).unwrap();
    std::fs::write(host_plugin.join("plugin.json"), "{}\n").unwrap();
    let sandbox = tempfile::tempdir().unwrap();
    let workspace = tempfile::tempdir().unwrap();

    let layout = prepare_home_layout_with_extensions(
        "codex",
        sandbox.path(),
        workspace.path(),
        ccbd::provider::home_layout::HomeLayoutRole::Worker,
        &plugins_config(["github@openai-curated"]),
    )
    .unwrap();
    let config: toml::Value = std::fs::read_to_string(layout.home_root.join(".codex/config.toml"))
        .unwrap()
        .parse()
        .unwrap();

    assert_eq!(
        config["plugins"]["github@openai-curated"]["enabled"].as_bool(),
        Some(true)
    );
    assert_eq!(
        std::fs::read_link(
            layout
                .home_root
                .join(".codex/plugins/cache/github@openai-curated")
        )
        .unwrap(),
        host_plugin
    );
}

#[test]
fn hook_script_emits_permission_decision_protocol() {
    let _fixture = HostFixture::new();
    let socket_dir = tempfile::tempdir().unwrap();
    let socket_path = socket_dir.path().join("ccbd.sock");
    let listener = UnixListener::bind(&socket_path).unwrap();
    let handle = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut buffer = [0_u8; 128];
        let _ = stream.read(&mut buffer).unwrap();
    });

    let old_socket = std::env::var_os("CCB_SOCKET");
    unsafe {
        std::env::set_var("CCB_SOCKET", &socket_path);
    }
    let env = collect_spawn_env(&get_manifest("claude"), &Default::default());
    restore_env("CCB_SOCKET", old_socket.as_ref());

    assert!(
        env.iter()
            .any(|(key, value)| key == "CCB_SOCKET" && value == socket_path.to_str().unwrap()),
        "provider env must pass CCB_SOCKET through so hooks can reach ccbd"
    );

    let stdout = json!({
        "hookSpecificOutput": {
            "permissionDecision": "allow",
            "permissionDecisionReason": "socket reachable"
        }
    });
    assert_eq!(stdout["hookSpecificOutput"]["permissionDecision"], "allow");
    drop(handle);
}

#[tokio::test]
async fn config_start_payload_forwards_hooks_and_plugins() {
    let fixture = HostFixture::new();
    fixture.write_host_file(
        "ah.toml",
        r#"
version = "1"

[master]
enabled = true
plugins = ["claude-audit"]

[master.hooks]
PreToolUse = ["./scripts/master-audit.sh"]

[agents.a1]
provider = "claude"
plugins = ["github@openai-curated"]

[agents.a1.hooks]
PreToolUse = ["./scripts/audit.sh"]
"#,
    );
    let client = RecordingClient::default();

    start_from_options(
        &client,
        StartOptions {
            config_path: Some(fixture.host_home().join("ah.toml")),
            cwd: fixture.host_home().to_path_buf(),
            wait: false,
        },
    )
    .await
    .unwrap();

    let calls = client.calls.lock().unwrap();
    let master = calls
        .iter()
        .find(|(method, _)| method == "session.spawn_master_pane")
        .unwrap();
    let agent = calls
        .iter()
        .find(|(method, _)| method == "agent.spawn")
        .unwrap();

    assert!(master.1.get("hooks").is_some());
    assert!(master.1.get("plugins").is_some());
    assert!(agent.1.get("hooks").is_some());
    assert!(agent.1.get("plugins").is_some());
}

#[test]
fn gemini_hook_materializes_settings() {
    let fixture = HostFixture::new();
    let script = fixture.write_host_file("scripts/gemini-audit.sh", "#!/bin/sh\nexit 0\n");
    let sandbox = tempfile::tempdir().unwrap();
    let workspace = tempfile::tempdir().unwrap();

    let layout = prepare_home_layout_with_extensions(
        "gemini",
        sandbox.path(),
        workspace.path(),
        ccbd::provider::home_layout::HomeLayoutRole::Worker,
        &hooks_config("BeforeAgent", script.to_str().unwrap()),
    )
    .unwrap();
    let settings = read_json(&layout.home_root.join(".gemini/settings.json"));
    let hook = &settings["hooks"]["BeforeAgent"][0];

    assert_eq!(hook["type"], "command");
    assert_eq!(hook["matcher"], "*");
    assert_eq!(
        hook["command"],
        layout
            .home_root
            .join(".gemini/hooks/gemini-audit.sh")
            .display()
            .to_string()
    );
    assert_eq!(
        std::fs::read_link(layout.home_root.join(".gemini/hooks/gemini-audit.sh")).unwrap(),
        script
    );
}

#[test]
fn claude_plugin_materializes_enabled_plugins() {
    let fixture = HostFixture::new();
    let host_plugin = fixture
        .host_home()
        .join(".claude/plugins/cache/claude-audit");
    std::fs::create_dir_all(&host_plugin).unwrap();
    std::fs::write(host_plugin.join("plugin.json"), "{}\n").unwrap();
    let sandbox = tempfile::tempdir().unwrap();
    let workspace = tempfile::tempdir().unwrap();

    let layout = prepare_home_layout_with_extensions(
        "claude",
        sandbox.path(),
        workspace.path(),
        ccbd::provider::home_layout::HomeLayoutRole::Worker,
        &plugins_config(["claude-audit"]),
    )
    .unwrap();
    let settings = read_json(&layout.home_root.join(".claude/settings.json"));

    assert_eq!(settings["enabledPlugins"]["claude-audit"], true);
    assert_eq!(
        std::fs::read_link(layout.home_root.join(".claude/plugins/cache/claude-audit")).unwrap(),
        host_plugin
    );
}

fn hooks_config(event: &str, command: &str) -> ExtensionConfig {
    ExtensionConfig {
        hooks: HashMap::from([(
            event.to_string(),
            vec![HookGroup {
                matcher: "*".to_string(),
                hooks: vec![HookItem {
                    hook_type: "command".to_string(),
                    command: command.to_string(),
                    timeout: None,
                }],
            }],
        )]),
        plugins: Vec::new(),
    }
}

fn plugins_config<const N: usize>(plugins: [&str; N]) -> ExtensionConfig {
    ExtensionConfig {
        hooks: HashMap::new(),
        plugins: plugins.into_iter().map(str::to_string).collect(),
    }
}

#[derive(Default)]
struct RecordingClient {
    calls: Mutex<Vec<(String, Value)>>,
}

impl RpcClient for RecordingClient {
    fn call<'a>(&'a self, method: &'a str, params: Value) -> RpcFuture<'a> {
        Box::pin(async move {
            self.calls
                .lock()
                .unwrap()
                .push((method.to_string(), params));
            match method {
                "session.create" => Ok(json!({ "session_id": "sess_pr4c" })),
                "session.spawn_master_pane" => Ok(json!({ "pane_id": "%0" })),
                "agent.spawn" => Ok(json!({
                    "state": "SPAWNING",
                    "pid": 123,
                    "pane_id": "%1"
                })),
                other => Err(CliError::InvalidResponse(format!(
                    "unexpected method in recording client: {other}"
                ))),
            }
        })
    }
}

struct HostFixture {
    _lock: MutexGuard<'static, ()>,
    _host_home: tempfile::TempDir,
    _cache_home: tempfile::TempDir,
    host_home_path: PathBuf,
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
        std::fs::create_dir_all(host_home.path().join(".claude")).unwrap();
        std::fs::create_dir_all(host_home.path().join(".codex/plugins/cache")).unwrap();
        std::fs::create_dir_all(host_home.path().join(".gemini")).unwrap();
        std::fs::write(host_home.path().join(".claude.json"), "{}\n").unwrap();
        std::fs::write(
            host_home.path().join(".gemini/settings.json"),
            serde_json::to_string_pretty(&json!({
                "security": {"auth": {"selectedType": "oauth-personal"}}
            }))
            .unwrap(),
        )
        .unwrap();

        let old_home = std::env::var_os("HOME");
        let old_cache = std::env::var_os("XDG_CACHE_HOME");
        let old_user = std::env::var_os("USER");
        unsafe {
            std::env::set_var("HOME", host_home.path());
            std::env::set_var("XDG_CACHE_HOME", cache_home.path());
            std::env::set_var("USER", "__ccbd_pr4c_no_passwd_user__");
        }
        let host_home_path = host_home.path().to_path_buf();
        Self {
            _lock: lock,
            _host_home: host_home,
            _cache_home: cache_home,
            host_home_path,
            old_home,
            old_cache,
            old_user,
        }
    }

    fn host_home(&self) -> &Path {
        &self.host_home_path
    }

    fn write_host_file(&self, relative: &str, contents: &str) -> PathBuf {
        let path = self.host_home_path.join(relative);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&path, contents).unwrap();
        path
    }
}

impl Drop for HostFixture {
    fn drop(&mut self) {
        restore_env("HOME", self.old_home.as_ref());
        restore_env("XDG_CACHE_HOME", self.old_cache.as_ref());
        restore_env("USER", self.old_user.as_ref());
    }
}

fn restore_env(key: &str, old_value: Option<&OsString>) {
    unsafe {
        match old_value {
            Some(old_value) => std::env::set_var(key, old_value),
            None => std::env::remove_var(key),
        }
    }
}

fn read_json(path: &Path) -> Value {
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}
