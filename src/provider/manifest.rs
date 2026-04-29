use std::collections::HashMap;
use std::sync::LazyLock;

#[derive(Debug, Clone)]
pub struct ProviderManifest {
    pub provider_name: &'static str,
    pub auth_mount_paths: Vec<&'static str>,
    pub idle_detection_mode: IdleDetectionMode,
    pub marker_pattern: &'static str,
    pub stability_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdleDetectionMode {
    LineEndRegex,
    ObservedStability,
}

pub static MANIFESTS: LazyLock<HashMap<&'static str, ProviderManifest>> = LazyLock::new(|| {
    let mut manifests = HashMap::new();
    manifests.insert(
        "bash",
        ProviderManifest {
            provider_name: "bash",
            auth_mount_paths: vec![],
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
            idle_detection_mode: IdleDetectionMode::ObservedStability,
            marker_pattern: r">_\s*Codex",
            stability_ms: 300,
        },
    );
    manifests.insert(
        "gemini",
        ProviderManifest {
            provider_name: "gemini",
            auth_mount_paths: vec![".config/gemini", ".config/gcloud"],
            idle_detection_mode: IdleDetectionMode::ObservedStability,
            marker_pattern: r"✦",
            stability_ms: 300,
        },
    );
    manifests.insert(
        "claude",
        ProviderManifest {
            provider_name: "claude",
            auth_mount_paths: vec![".anthropic", ".claude"],
            idle_detection_mode: IdleDetectionMode::ObservedStability,
            marker_pattern: r"▶",
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
            idle_detection_mode: IdleDetectionMode::LineEndRegex,
            marker_pattern: r"[\$#>✦]\s*$",
            stability_ms: 0,
        })
}

#[cfg(test)]
mod tests {
    use super::{IdleDetectionMode, MANIFESTS, get_manifest};

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
    }

    #[test]
    fn test_codex_and_gemini_auth_mounts_are_non_empty() {
        assert!(!get_manifest("codex").auth_mount_paths.is_empty());
        assert!(!get_manifest("gemini").auth_mount_paths.is_empty());
    }

    #[test]
    fn test_bash_has_zero_stability() {
        let manifest = get_manifest("bash");

        assert_eq!(manifest.stability_ms, 0);
        assert_eq!(
            manifest.idle_detection_mode,
            IdleDetectionMode::LineEndRegex
        );
    }
}
