pub use crate::platform::sys::service::ServiceUnitError;
use std::io;
use std::path::{Path, PathBuf};

pub fn derive_unit_name(state_dir: &Path) -> String {
    crate::platform::sys::service::derive_unit_name(state_dir)
}

pub fn escape_systemd_env_value(value: &str) -> Result<String, ServiceUnitError> {
    crate::platform::sys::service::escape_systemd_env_value(value)
}

pub fn escape_systemd_exec_token(value: &str) -> Result<String, ServiceUnitError> {
    crate::platform::sys::service::escape_systemd_exec_token(value)
}

pub fn render_unit_file(
    unit_name: &str,
    ahd_bin: &Path,
    state_dir: &Path,
    env: &[(String, String)],
) -> Result<String, ServiceUnitError> {
    crate::platform::sys::service::render_unit_file(unit_name, ahd_bin, state_dir, env)
}

pub fn resolve_user_systemd_dir(
    xdg_config_home: Option<&str>,
    home: Option<&str>,
) -> Result<PathBuf, ServiceUnitError> {
    crate::platform::sys::service::resolve_user_systemd_dir(xdg_config_home, home)
}

pub fn resolve_user_systemd_dir_from_env() -> Result<PathBuf, ServiceUnitError> {
    crate::platform::sys::service::resolve_user_systemd_dir_from_env()
}

pub fn atomic_write_unit(path: &Path, content: &str) -> io::Result<()> {
    crate::platform::sys::service::atomic_write_unit(path, content)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::systemd_unit::detect_current_service_unit_from_cgroup;
    use std::fs;
    use std::path::Path;

    #[test]
    fn service_unit_derive_unit_name_is_deterministic_for_normalized_state_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let state_dir = tmp.path().join("state");
        fs::create_dir(&state_dir).unwrap();
        let alias = tmp.path().join("alias");
        std::os::unix::fs::symlink(&state_dir, &alias).unwrap();

        assert_eq!(derive_unit_name(&state_dir), derive_unit_name(&alias));
    }

    #[test]
    fn service_unit_derive_unit_name_differs_by_state_dir_and_matches_daemon_detection() {
        let tmp = tempfile::tempdir().unwrap();
        let left = tmp.path().join("left");
        let right = tmp.path().join("right");
        fs::create_dir(&left).unwrap();
        fs::create_dir(&right).unwrap();

        let left_unit = derive_unit_name(&left);
        let right_unit = derive_unit_name(&right);

        assert_ne!(left_unit, right_unit);
        assert!(left_unit.starts_with("ah-"));
        assert!(left_unit.ends_with(".service"));
        let cgroup =
            format!("0::/user.slice/user-1001.slice/user@1001.service/app.slice/{left_unit}");
        assert_eq!(
            detect_current_service_unit_from_cgroup(&cgroup).as_deref(),
            Some(left_unit.as_str())
        );
    }

    #[test]
    fn service_unit_escape_systemd_env_value_escapes_systemd_specials_and_quotes() {
        assert_eq!(escape_systemd_env_value("plain").unwrap(), "plain");
        assert_eq!(escape_systemd_env_value("a b").unwrap(), "\"a b\"");
        assert_eq!(escape_systemd_env_value("a\"b").unwrap(), "\"a\\\"b\"");
        assert_eq!(escape_systemd_env_value("a\\b").unwrap(), "\"a\\\\b\"");
        assert_eq!(escape_systemd_env_value("$TOKEN").unwrap(), "$TOKEN");
        assert_eq!(escape_systemd_env_value("${TOKEN}").unwrap(), "${TOKEN}");
        assert_eq!(escape_systemd_env_value("$$").unwrap(), "$$");
        assert_eq!(escape_systemd_env_value("%h/%n").unwrap(), "%%h/%%n");
        assert_eq!(escape_systemd_env_value("%%").unwrap(), "%%%%");
    }

    #[test]
    fn service_unit_escape_systemd_exec_token_doubles_dollars() {
        assert_eq!(escape_systemd_exec_token("$TOKEN").unwrap(), "$$TOKEN");
        assert_eq!(escape_systemd_exec_token("${TOKEN}").unwrap(), "$${TOKEN}");
        assert_eq!(escape_systemd_exec_token("$$").unwrap(), "$$$$");
    }

    #[test]
    fn service_unit_escape_systemd_env_and_exec_share_non_dollar_escaping() {
        for escaped in [
            escape_systemd_env_value("%h a\"b\\c").unwrap(),
            escape_systemd_exec_token("%h a\"b\\c").unwrap(),
        ] {
            assert_eq!(escaped, "\"%%h a\\\"b\\\\c\"");
        }
    }

    #[test]
    fn service_unit_escape_systemd_values_reject_control_characters() {
        assert!(escape_systemd_env_value("a\nb").is_err());
        assert!(escape_systemd_env_value("a\rb").is_err());
        assert!(escape_systemd_env_value("a\0b").is_err());
        assert!(escape_systemd_env_value("a\tb").is_err());
        assert!(escape_systemd_env_value("a\x01b").is_err());

        assert!(escape_systemd_exec_token("a\nb").is_err());
        assert!(escape_systemd_exec_token("a\rb").is_err());
        assert!(escape_systemd_exec_token("a\0b").is_err());
        assert!(escape_systemd_exec_token("a\tb").is_err());
        assert!(escape_systemd_exec_token("a\x01b").is_err());
    }

    #[test]
    fn service_unit_render_unit_file_emits_expected_fields_and_passthrough_subset() {
        let tmp = tempfile::tempdir().unwrap();
        let bin_dir = tmp.path().join("bin dir");
        let state_dir = tmp.path().join("state dir");
        fs::create_dir(&bin_dir).unwrap();
        fs::create_dir(&state_dir).unwrap();
        let ahd_bin = bin_dir.join("ahd");
        fs::write(&ahd_bin, "").unwrap();
        let env = vec![
            (
                "ANTHROPIC_API_KEY".to_string(),
                "tok$en % value".to_string(),
            ),
            ("NOT_PASSED".to_string(), "ignored".to_string()),
        ];

        let unit = render_unit_file("ah-test.service", &ahd_bin, &state_dir, &env).unwrap();

        assert!(unit.starts_with("# ah-generated unit; AH_STATE_DIR="));
        assert!(unit.contains("[Unit]\n"));
        assert!(unit.contains("Description=ah daemon\n"));
        assert!(unit.contains("StartLimitIntervalSec=60\n"));
        assert!(unit.contains("StartLimitBurst=5\n"));
        assert!(unit.contains("[Service]\n"));
        assert!(unit.contains("Type=simple\n"));
        assert!(unit.contains(&format!("ExecStart=\"{}\"\n", ahd_bin.display())));
        assert!(unit.contains("Restart=on-failure\n"));
        assert!(unit.contains("RestartSec=1s\n"));
        assert!(unit.contains("OOMScoreAdjust=-900\n"));
        assert!(unit.contains(&format!(
            "Environment=AH_STATE_DIR=\"{}\"\n",
            state_dir.display()
        )));
        // systemd.exec(5) Environment= does not perform variable expansion:
        // '$' is a literal env value byte, while '%' specifiers still expand.
        assert!(unit.contains("Environment=ANTHROPIC_API_KEY=\"tok$en %% value\"\n"));
        assert!(!unit.contains("NOT_PASSED"));
        assert!(unit.contains("[Install]\n"));
        assert!(unit.contains("WantedBy=default.target\n"));
        assert!(!unit.contains("StandardOutput"));
        assert!(!unit.contains("StandardError"));
    }

    #[test]
    fn service_unit_render_unit_file_rejects_relative_exec_path() {
        let err = render_unit_file(
            "ah-test.service",
            Path::new("relative-ahd"),
            Path::new("/tmp/state"),
            &[],
        )
        .unwrap_err();

        assert!(err.to_string().contains("absolute"));
    }

    #[test]
    fn service_unit_resolve_user_systemd_dir_uses_xdg_then_home_then_error() {
        assert_eq!(
            resolve_user_systemd_dir(Some("/xdg"), Some("/home/me")).unwrap(),
            Path::new("/xdg/systemd/user")
        );
        assert_eq!(
            resolve_user_systemd_dir(None, Some("/home/me")).unwrap(),
            Path::new("/home/me/.config/systemd/user")
        );
        assert!(resolve_user_systemd_dir(None, None).is_err());
    }

    #[test]
    fn service_unit_atomic_write_unit_writes_and_overwrites() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("ah-test.service");

        atomic_write_unit(&path, "first").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "first");

        atomic_write_unit(&path, "second").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "second");
    }
}
