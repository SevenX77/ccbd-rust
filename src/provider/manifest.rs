use std::collections::HashMap;
use std::sync::LazyLock;

#[derive(Debug, Clone)]
pub struct ProviderManifest {
    pub provider_name: &'static str,
    pub auth_mount_paths: Vec<&'static str>,
    /// Actual command and arguments used to spawn this provider.
    pub command: &'static [&'static str],
    /// Arguments appended only when recovering a crashed worker for this provider.
    pub resume_args: &'static [&'static str],
    /// Host environment variable names allowed into the sandbox.
    pub env_passthrough: &'static [&'static str],
    /// Environment variables injected by ccbd. These override passthrough.
    pub injected_env_vars: &'static [(&'static str, &'static str)],
    pub readiness_timeout_s: u32,
    pub requires_home_materialization: bool,
    pub init_probe: InitProbeKind,
    pub idle_detection_mode: IdleDetectionMode,
    pub stability_ms: u64,
    pub idle_anti_pattern: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdleDetectionMode {
    LineEndRegex,
    ObservedStability,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InitProbeKind {
    Bash,
    Codex,
    Claude,
    Gemini,
    Antigravity,
    OpenCode,
    Unknown,
}

impl InitProbeKind {
    pub fn build(self) -> Box<dyn crate::provider::init_probe::InitGateProbe> {
        match self {
            Self::Bash => Box::new(crate::provider::init_probe::BashInitProbe),
            Self::Codex => Box::new(crate::provider::init_probe::CodexInitProbe),
            Self::Claude => Box::new(crate::provider::init_probe::ClaudeInitProbe),
            Self::Gemini => Box::new(crate::provider::init_probe::GeminiInitProbe),
            Self::Antigravity => Box::new(crate::provider::init_probe::AntigravityInitProbe),
            Self::OpenCode | Self::Unknown => Box::new(crate::provider::init_probe::BashInitProbe),
        }
    }
}

pub const ENV_PASSTHROUGH: &[&str] = &[
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_AUTH_TOKEN",
    "ANTHROPIC_BASE_URL",
    "CCB_BACKEND_ENV",
    "CCB_CCBD_MIN_POLL_INTERVAL_S",
    "CCB_CLAUDE_READY_TIMEOUT_S",
    "CCB_DEBUG",
    "CCB_GEMINI_READY_TIMEOUT_S",
    "CCB_KEEPER_PID",
    "CCB_KEEPER_PING_TIMEOUT_S",
    "CCB_LANG",
    "CCB_MASTER_CLAUDE_PID",
    "CCB_PER_AGENT_SUBCGROUP",
    "CCB_NO_ATTACH",
    "CCB_REPLY_LANG",
    "CCB_JOB_ID",
    "CCB_SOCKET",
    "CCB_STDIN_ENCODING",
    "CCB_TMUX_ENTER_DELAY",
    "CCB_TMUX_SECOND_ENTER_DELAY",
    "CCB_TMUX_SOCKET",
    "CCB_TMUX_SOCKET_PATH",
    "CCB_VERIFY_DELIVERY",
    "CCB_VERIFY_POST_DELAY_MS",
    "CCB_VERIFY_RETRY_KEYCODES",
    "CCB_VERSION",
    "GEMINI_API_KEY",
    "GOOGLE_API_BASE",
    "GOOGLE_API_KEY",
    "GOOGLE_GENAI_USE_VERTEXAI",
    "HOME",
    "LANG",
    "LC_ALL",
    "LC_MESSAGES",
    "LOCALAPPDATA",
    "OPENAI_API_BASE",
    "OPENAI_API_KEY",
    "OPENAI_BASE_URL",
    "OPENAI_ORG_ID",
    "OPENAI_ORGANIZATION",
    "PATH",
    "PYTHONPATH",
    "PYTHONUNBUFFERED",
    "SHELL",
    "SYSTEMROOT",
    "TERM",
    "TMP",
    "TEMP",
    "TMPDIR",
    "USER",
    "USERPROFILE",
    "XDG_CACHE_HOME",
    "XDG_CONFIG_HOME",
    "XDG_DATA_HOME",
    "XDG_RUNTIME_DIR",
];

pub const CLAUDE_INJECTED_ENV: &[(&str, &str)] = &[
    ("CCB_CLAUDE_SKILLS", "true"),
    ("CCB_CLAUDE_READY_TIMEOUT_S", "60.0"),
    ("CCB_CLAUDE_MD_MODE", "route"),
    ("CCB_REPLY_LANG", "zh"),
    ("CCB_LANG", "zh"),
    ("CCB_CTX_TRANSFER_LAST_N", "20"),
    ("CCB_CTX_TRANSFER_ENABLED", "true"),
];

pub const CODEX_INJECTED_ENV: &[(&str, &str)] = &[
    ("CCB_TMUX_ENTER_DELAY", "0.5"),
    ("CCB_TMUX_SECOND_ENTER_DELAY", "0.0"),
];

pub const GEMINI_INJECTED_ENV: &[(&str, &str)] = &[("CCB_GEMINI_READY_TIMEOUT_S", "60.0")];
pub const ANTIGRAVITY_INJECTED_ENV: &[(&str, &str)] = &[("CCB_GEMINI_READY_TIMEOUT_S", "60.0")];

// Reserved for future provider wiring; no opencode manifest is added in G11.1.
pub const OPENCODE_INJECTED_ENV: &[(&str, &str)] = &[("CCB_SESSION_ID", "<session_id>")];
pub const PANE_LOG_INJECTED_ENV: &[(&str, &str)] = &[
    ("CCB_PANE_LOG_POLL_INTERVAL", "2.0"),
    ("CCB_SYNC_TIMEOUT", "3600"),
];

const BASH_INJECTED_ENV: &[(&str, &str)] = &[("PS1", "$ ")];

pub static MANIFESTS: LazyLock<HashMap<&'static str, ProviderManifest>> = LazyLock::new(|| {
    let mut manifests = HashMap::new();
    manifests.insert(
        "bash",
        ProviderManifest {
            provider_name: "bash",
            auth_mount_paths: vec![],
            command: &["bash", "--noprofile", "--norc", "-i"],
            resume_args: &[],
            env_passthrough: ENV_PASSTHROUGH,
            injected_env_vars: BASH_INJECTED_ENV,
            readiness_timeout_s: 10,
            requires_home_materialization: false,
            init_probe: InitProbeKind::Bash,
            idle_detection_mode: IdleDetectionMode::LineEndRegex,
            stability_ms: 0,
            idle_anti_pattern: "",
        },
    );
    manifests.insert(
        "codex",
        ProviderManifest {
            provider_name: "codex",
            auth_mount_paths: vec![".codex", ".config/gcloud"],
            command: &[
                "codex",
                "--dangerously-bypass-approvals-and-sandbox",
                "-c",
                "disable_paste_burst=true",
                "-c",
                "trust_level=\"trusted\"",
                "-c",
                "approval_policy=\"never\"",
                "-c",
                "sandbox_mode=\"danger-full-access\"",
            ],
            resume_args: &[],
            env_passthrough: ENV_PASSTHROUGH,
            injected_env_vars: CODEX_INJECTED_ENV,
            readiness_timeout_s: 60,
            requires_home_materialization: true,
            init_probe: InitProbeKind::Codex,
            idle_detection_mode: IdleDetectionMode::ObservedStability,
            stability_ms: 300,
            idle_anti_pattern: r"(?m)\besc to interrupt\b",
        },
    );
    manifests.insert(
        "gemini",
        ProviderManifest {
            provider_name: "gemini",
            auth_mount_paths: vec![".config/gemini", ".config/gcloud"],
            // mvp12 M12.6: --yolo bypasses trust prompt + auto-approves all tools (sandbox-equivalent)
            command: &["gemini", "--yolo"],
            resume_args: &[],
            env_passthrough: ENV_PASSTHROUGH,
            injected_env_vars: GEMINI_INJECTED_ENV,
            readiness_timeout_s: 60,
            requires_home_materialization: true,
            init_probe: InitProbeKind::Gemini,
            idle_detection_mode: IdleDetectionMode::ObservedStability,
            stability_ms: 300,
            idle_anti_pattern: r"[\u{2800}-\u{28FF}]",
        },
    );
    manifests.insert(
        "claude",
        ProviderManifest {
            provider_name: "claude",
            auth_mount_paths: vec![".anthropic", ".claude"],
            // mvp12 M12.6: --dangerously-skip-permissions bypasses trust dialog + permission prompts (sandbox)
            command: &["claude", "--dangerously-skip-permissions"],
            resume_args: &["--continue"],
            env_passthrough: ENV_PASSTHROUGH,
            injected_env_vars: CLAUDE_INJECTED_ENV,
            readiness_timeout_s: 60,
            requires_home_materialization: true,
            init_probe: InitProbeKind::Claude,
            idle_detection_mode: IdleDetectionMode::ObservedStability,
            stability_ms: 300,
            idle_anti_pattern: "",
        },
    );
    manifests.insert(
        "antigravity",
        ProviderManifest {
            provider_name: "antigravity",
            auth_mount_paths: vec![".gemini/antigravity-cli"],
            command: &["agy", "--dangerously-skip-permissions"],
            resume_args: &[],
            env_passthrough: ENV_PASSTHROUGH,
            injected_env_vars: ANTIGRAVITY_INJECTED_ENV,
            readiness_timeout_s: 60,
            requires_home_materialization: true,
            init_probe: InitProbeKind::Antigravity,
            idle_detection_mode: IdleDetectionMode::LineEndRegex,
            stability_ms: 300,
            idle_anti_pattern: r"(?m)^\s*esc to cancel\b",
        },
    );
    manifests
});

pub fn get_manifest(provider: &str) -> ProviderManifest {
    MANIFESTS
        .get(provider)
        .cloned()
        .unwrap_or_else(|| ProviderManifest {
            provider_name: "unknown",
            auth_mount_paths: vec![],
            command: &["bash", "--noprofile", "--norc", "-i"],
            resume_args: &[],
            env_passthrough: ENV_PASSTHROUGH,
            injected_env_vars: BASH_INJECTED_ENV,
            readiness_timeout_s: 10,
            requires_home_materialization: false,
            init_probe: InitProbeKind::Unknown,
            idle_detection_mode: IdleDetectionMode::LineEndRegex,
            stability_ms: 0,
            idle_anti_pattern: "",
        })
}

pub fn cancel_keysyms_for_provider(provider: &str) -> &'static [&'static str] {
    match provider {
        "antigravity" => &["Escape"],
        _ => &["C-c"],
    }
}

pub fn collect_spawn_env(
    manifest: &ProviderManifest,
    extra_env_vars: &HashMap<String, String>,
) -> Vec<(String, String)> {
    let mut env = HashMap::new();
    for key in manifest.env_passthrough {
        if let Ok(value) = std::env::var(key) {
            env.insert((*key).to_string(), value);
        }
    }
    for (key, value) in manifest.injected_env_vars {
        env.insert((*key).to_string(), (*value).to_string());
    }
    for (key, value) in extra_env_vars {
        env.insert(key.clone(), value.clone());
    }
    let mut env = env.into_iter().collect::<Vec<_>>();
    env.sort_by(|(left, _), (right, _)| left.cmp(right));
    env
}

#[cfg(test)]
mod tests {
    use super::{
        IdleDetectionMode, InitProbeKind, MANIFESTS, cancel_keysyms_for_provider,
        collect_spawn_env, get_manifest,
    };
    use std::collections::HashMap;

    #[test]
    fn codex_idle_anti_pattern_matches_real_working_line_not_idle_composer() {
        let manifest = get_manifest("codex");
        let anti_pattern = regex::Regex::new(manifest.idle_anti_pattern).unwrap();

        assert!(anti_pattern.is_match("• Working (4s • esc to interrupt)"));
        assert!(!anti_pattern.is_match("› Run /review on my current changes"));
        assert!(!anti_pattern.is_match("  gpt-5.5 default · /tmp/x"));
    }

    #[test]
    fn test_builtin_providers_registered() {
        for provider in ["bash", "codex", "gemini", "claude", "antigravity"] {
            assert!(
                MANIFESTS.contains_key(provider),
                "missing provider {provider}"
            );
            assert_eq!(get_manifest(provider).provider_name, provider);
        }
    }

    #[test]
    fn test_unknown_provider_returns_bash_style_default() {
        let manifest = get_manifest("custom-provider");

        assert_eq!(manifest.provider_name, "unknown");
        assert!(manifest.auth_mount_paths.is_empty());
        assert_eq!(
            manifest.idle_detection_mode,
            IdleDetectionMode::LineEndRegex
        );
        assert_eq!(manifest.stability_ms, 0);
        assert_eq!(manifest.command, ["bash", "--noprofile", "--norc", "-i"]);
        assert!(manifest.resume_args.is_empty());
        assert_eq!(manifest.injected_env_vars, [("PS1", "$ ")]);
        assert_eq!(manifest.init_probe, InitProbeKind::Unknown);
    }

    #[test]
    fn test_codex_and_gemini_auth_mounts_are_non_empty() {
        assert!(!get_manifest("codex").auth_mount_paths.is_empty());
        assert!(!get_manifest("gemini").auth_mount_paths.is_empty());
    }

    #[test]
    fn test_provider_commands_and_probe_kinds_match_calibration() {
        let codex = get_manifest("codex");
        assert_eq!(
            codex.command,
            [
                "codex",
                "--dangerously-bypass-approvals-and-sandbox",
                "-c",
                "disable_paste_burst=true",
                "-c",
                "trust_level=\"trusted\"",
                "-c",
                "approval_policy=\"never\"",
                "-c",
                "sandbox_mode=\"danger-full-access\"",
            ]
        );
        assert_eq!(codex.init_probe, InitProbeKind::Codex);
        assert_eq!(codex.stability_ms, 300);
        assert!(codex.resume_args.is_empty());

        let gemini = get_manifest("gemini");
        assert_eq!(gemini.command, ["gemini", "--yolo"]);
        assert_eq!(gemini.init_probe, InitProbeKind::Gemini);
        assert_eq!(gemini.stability_ms, 300);
        assert!(gemini.resume_args.is_empty());

        let claude = get_manifest("claude");
        assert_eq!(claude.command, ["claude", "--dangerously-skip-permissions"]);
        assert_eq!(claude.init_probe, InitProbeKind::Claude);
        assert_eq!(claude.stability_ms, 300);
        assert_eq!(claude.resume_args, ["--continue"]);

        let antigravity = get_manifest("antigravity");
        assert_eq!(antigravity.provider_name, "antigravity");
        assert_eq!(
            antigravity.command,
            ["agy", "--dangerously-skip-permissions"]
        );
        assert!(
            antigravity
                .auth_mount_paths
                .contains(&".gemini/antigravity-cli"),
            "antigravity OAuth state should be mounted"
        );
        assert_eq!(
            antigravity.idle_detection_mode,
            IdleDetectionMode::LineEndRegex
        );
        assert_eq!(antigravity.init_probe, InitProbeKind::Antigravity);
        assert!(
            antigravity.idle_anti_pattern.contains("esc to cancel"),
            "antigravity busy status line must suppress idle"
        );
    }

    #[test]
    fn test_antigravity_cancel_uses_single_escape() {
        assert_eq!(cancel_keysyms_for_provider("antigravity"), ["Escape"]);
        assert_eq!(cancel_keysyms_for_provider("codex"), ["C-c"]);
    }

    #[test]
    fn test_bash_has_zero_stability() {
        let manifest = get_manifest("bash");

        assert_eq!(manifest.stability_ms, 0);
        assert_eq!(
            manifest.idle_detection_mode,
            IdleDetectionMode::LineEndRegex
        );
        assert_eq!(manifest.init_probe, InitProbeKind::Bash);
        assert_eq!(manifest.command, ["bash", "--noprofile", "--norc", "-i"]);
        assert!(manifest.resume_args.is_empty());
        assert_eq!(manifest.injected_env_vars, [("PS1", "$ ")]);
    }

    #[test]
    fn test_real_provider_manifest_parity_fields_are_populated() {
        for provider in ["codex", "gemini", "claude", "antigravity"] {
            let manifest = get_manifest(provider);
            assert!(!manifest.env_passthrough.is_empty(), "{provider}");
            assert!(!manifest.injected_env_vars.is_empty(), "{provider}");
            assert!(manifest.readiness_timeout_s > 0, "{provider}");
            assert!(
                matches!(
                    manifest.init_probe,
                    InitProbeKind::Codex
                        | InitProbeKind::Gemini
                        | InitProbeKind::Claude
                        | InitProbeKind::Antigravity
                ),
                "{provider}"
            );
        }
    }

    #[test]
    #[serial_test::serial(global_env)]
    fn test_collect_spawn_env_precedence() {
        unsafe {
            std::env::set_var("ANTHROPIC_API_KEY", "host-key");
            std::env::set_var("CCB_CLAUDE_MD_MODE", "host-mode");
        }
        let mut extra = HashMap::new();
        extra.insert("CCB_CLAUDE_MD_MODE".to_string(), "extra-mode".to_string());
        let env = collect_spawn_env(&get_manifest("claude"), &extra);

        assert!(env.contains(&("ANTHROPIC_API_KEY".to_string(), "host-key".to_string())));
        assert!(env.contains(&("CCB_CLAUDE_MD_MODE".to_string(), "extra-mode".to_string())));

        unsafe {
            std::env::remove_var("ANTHROPIC_API_KEY");
            std::env::remove_var("CCB_CLAUDE_MD_MODE");
        }
    }
}
