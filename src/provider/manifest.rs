use std::collections::HashMap;
use std::sync::LazyLock;

#[derive(Debug, Clone)]
pub struct ProviderManifest {
    pub provider_name: &'static str,
    pub auth_mount_paths: Vec<&'static str>,
    /// Actual command and arguments used to spawn this provider.
    pub command: &'static [&'static str],
    /// Host environment variable names allowed into the sandbox.
    pub env_passthrough: &'static [&'static str],
    /// Environment variables injected by ccbd. These override passthrough.
    pub injected_env_vars: &'static [(&'static str, &'static str)],
    pub readiness_timeout_s: u32,
    pub startup_sequence: &'static [StartupStep],
    pub interactive_prompt_handlers: &'static [PromptHandler],
    pub idle_detection_mode: IdleDetectionMode,
    pub marker_pattern: &'static str,
    pub stability_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdleDetectionMode {
    LineEndRegex,
    ObservedStability,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StartupStep {
    WaitMs(u64),
    SendKeysVerified {
        keys: &'static str,
        verify_pattern: Option<&'static str>,
        verify_timeout_ms: u64,
        retry_fallback_keys: Option<&'static [&'static str]>,
    },
    ClearLine {
        expected_after: Option<&'static str>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptHandler {
    pub pattern: &'static str,
    pub response_keys: &'static str,
    pub max_triggers: u32,
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

// Reserved for future provider wiring; no opencode manifest is added in G11.1.
pub const OPENCODE_INJECTED_ENV: &[(&str, &str)] = &[("CCB_SESSION_ID", "<session_id>")];
pub const PANE_LOG_INJECTED_ENV: &[(&str, &str)] = &[
    ("CCB_PANE_LOG_POLL_INTERVAL", "2.0"),
    ("CCB_SYNC_TIMEOUT", "3600"),
];

const SEND_KEYS_FALLBACK: &[&str] = &["Return", "C-m"];

const BASH_INJECTED_ENV: &[(&str, &str)] = &[("PS1", "$ ")];
const BASH_STARTUP_SEQUENCE: &[StartupStep] = &[];
const CODEX_STARTUP_SEQUENCE: &[StartupStep] = &[StartupStep::SendKeysVerified {
    keys: "Enter",
    verify_pattern: None,
    verify_timeout_ms: 1500,
    retry_fallback_keys: Some(SEND_KEYS_FALLBACK),
}];
const CLAUDE_STARTUP_SEQUENCE: &[StartupStep] = &[StartupStep::SendKeysVerified {
    keys: "Enter",
    verify_pattern: None,
    verify_timeout_ms: 1500,
    retry_fallback_keys: Some(SEND_KEYS_FALLBACK),
}];
const GEMINI_STARTUP_SEQUENCE: &[StartupStep] = &[
    StartupStep::WaitMs(500),
    StartupStep::SendKeysVerified {
        keys: "Enter",
        verify_pattern: None,
        verify_timeout_ms: 1500,
        retry_fallback_keys: Some(SEND_KEYS_FALLBACK),
    },
    StartupStep::WaitMs(500),
    StartupStep::SendKeysVerified {
        keys: "Enter",
        verify_pattern: None,
        verify_timeout_ms: 1500,
        retry_fallback_keys: Some(SEND_KEYS_FALLBACK),
    },
];

const NO_PROMPT_HANDLERS: &[PromptHandler] = &[];
const CODEX_PROMPT_HANDLERS: &[PromptHandler] = &[PromptHandler {
    pattern: "Update now",
    response_keys: "Escape",
    max_triggers: 3,
}];
const CLAUDE_PROMPT_HANDLERS: &[PromptHandler] = &[PromptHandler {
    pattern: "Trust",
    response_keys: "1",
    max_triggers: 1,
}];

pub static MANIFESTS: LazyLock<HashMap<&'static str, ProviderManifest>> = LazyLock::new(|| {
    let mut manifests = HashMap::new();
    manifests.insert(
        "bash",
        ProviderManifest {
            provider_name: "bash",
            auth_mount_paths: vec![],
            command: &["bash", "--noprofile", "--norc", "-i"],
            env_passthrough: ENV_PASSTHROUGH,
            injected_env_vars: BASH_INJECTED_ENV,
            readiness_timeout_s: 10,
            startup_sequence: BASH_STARTUP_SEQUENCE,
            interactive_prompt_handlers: NO_PROMPT_HANDLERS,
            idle_detection_mode: IdleDetectionMode::LineEndRegex,
            marker_pattern: r"[\$#>✦]\s*$",
            stability_ms: 0,
        },
    );
    manifests.insert(
        "codex",
        ProviderManifest {
            provider_name: "codex",
            auth_mount_paths: vec![".codex", ".config/gcloud"],
            command: &["codex", "--dangerously-bypass-approvals-and-sandbox"],
            env_passthrough: ENV_PASSTHROUGH,
            injected_env_vars: CODEX_INJECTED_ENV,
            readiness_timeout_s: 60,
            startup_sequence: CODEX_STARTUP_SEQUENCE,
            interactive_prompt_handlers: CODEX_PROMPT_HANDLERS,
            idle_detection_mode: IdleDetectionMode::ObservedStability,
            marker_pattern: r"(?m)^›\s",
            stability_ms: 300,
        },
    );
    manifests.insert(
        "gemini",
        ProviderManifest {
            provider_name: "gemini",
            auth_mount_paths: vec![".config/gemini", ".config/gcloud"],
            command: &["gemini"],
            env_passthrough: ENV_PASSTHROUGH,
            injected_env_vars: GEMINI_INJECTED_ENV,
            readiness_timeout_s: 60,
            startup_sequence: GEMINI_STARTUP_SEQUENCE,
            interactive_prompt_handlers: NO_PROMPT_HANDLERS,
            idle_detection_mode: IdleDetectionMode::ObservedStability,
            marker_pattern: r"Type your message or @path/to/file",
            stability_ms: 300,
        },
    );
    manifests.insert(
        "claude",
        ProviderManifest {
            provider_name: "claude",
            auth_mount_paths: vec![".anthropic", ".claude"],
            command: &["claude"],
            env_passthrough: ENV_PASSTHROUGH,
            injected_env_vars: CLAUDE_INJECTED_ENV,
            readiness_timeout_s: 60,
            startup_sequence: CLAUDE_STARTUP_SEQUENCE,
            interactive_prompt_handlers: CLAUDE_PROMPT_HANDLERS,
            idle_detection_mode: IdleDetectionMode::ObservedStability,
            marker_pattern: r"(?m)^❯\s*$",
            stability_ms: 300,
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
            env_passthrough: ENV_PASSTHROUGH,
            injected_env_vars: BASH_INJECTED_ENV,
            readiness_timeout_s: 10,
            startup_sequence: BASH_STARTUP_SEQUENCE,
            interactive_prompt_handlers: NO_PROMPT_HANDLERS,
            idle_detection_mode: IdleDetectionMode::LineEndRegex,
            marker_pattern: r"[\$#>✦]\s*$",
            stability_ms: 0,
        })
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
    use super::{IdleDetectionMode, MANIFESTS, StartupStep, collect_spawn_env, get_manifest};
    use std::collections::HashMap;

    #[test]
    fn test_builtin_providers_registered() {
        for provider in ["bash", "codex", "gemini", "claude"] {
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
        assert_eq!(manifest.injected_env_vars, [("PS1", "$ ")]);
    }

    #[test]
    fn test_codex_and_gemini_auth_mounts_are_non_empty() {
        assert!(!get_manifest("codex").auth_mount_paths.is_empty());
        assert!(!get_manifest("gemini").auth_mount_paths.is_empty());
    }

    #[test]
    fn test_provider_idle_patterns_and_commands_match_probe_calibration() {
        let codex = get_manifest("codex");
        assert_eq!(
            codex.command,
            ["codex", "--dangerously-bypass-approvals-and-sandbox"]
        );
        assert_eq!(codex.marker_pattern, r"(?m)^›\s");
        assert_eq!(codex.stability_ms, 300);

        let gemini = get_manifest("gemini");
        assert_eq!(gemini.command, ["gemini"]);
        assert_eq!(gemini.marker_pattern, r"Type your message or @path/to/file");
        assert_eq!(gemini.stability_ms, 300);

        let claude = get_manifest("claude");
        assert_eq!(claude.command, ["claude"]);
        assert_eq!(claude.marker_pattern, r"(?m)^❯\s*$");
        assert_eq!(claude.stability_ms, 300);
    }

    #[test]
    fn test_bash_has_zero_stability() {
        let manifest = get_manifest("bash");

        assert_eq!(manifest.stability_ms, 0);
        assert_eq!(
            manifest.idle_detection_mode,
            IdleDetectionMode::LineEndRegex
        );
        assert_eq!(manifest.command, ["bash", "--noprofile", "--norc", "-i"]);
        assert_eq!(manifest.injected_env_vars, [("PS1", "$ ")]);
    }

    #[test]
    fn test_real_provider_manifest_parity_fields_are_populated() {
        for provider in ["codex", "gemini", "claude"] {
            let manifest = get_manifest(provider);
            assert!(!manifest.env_passthrough.is_empty(), "{provider}");
            assert!(!manifest.injected_env_vars.is_empty(), "{provider}");
            assert!(manifest.readiness_timeout_s > 0, "{provider}");
            assert!(!manifest.startup_sequence.is_empty(), "{provider}");
        }
    }

    #[test]
    fn test_send_keys_verified_has_retry_fallback_keys() {
        for provider in ["codex", "gemini", "claude"] {
            let manifest = get_manifest(provider);
            assert!(
                manifest.startup_sequence.iter().any(|step| matches!(
                    step,
                    StartupStep::SendKeysVerified {
                        retry_fallback_keys: Some(keys),
                        ..
                    } if *keys == ["Return", "C-m"]
                )),
                "{provider}"
            );
        }
    }

    #[test]
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
