//! R4.1+R4.3: master cmd template and empty-cmd compatibility.

use ah::cli::config::{ProjectConfig, load_project_config};

const DEFAULT_MASTER_CMD: &str = "claude";

#[test]
fn loads_default_master_cmd_long_form() {
    let config = toml::from_str::<ProjectConfig>(
        r#"
version = "1"

[agents.a1]
provider = "bash"
"#,
    )
    .unwrap();

    assert_eq!(config.master.cmd, DEFAULT_MASTER_CMD);
    assert!(config.master.enabled);
}

#[test]
fn loads_explicit_master_cmd_through_sh_lc() {
    let config = toml::from_str::<ProjectConfig>(
        r#"
version = "1"

[master]
cmd = "claude --extra 'quoted value'"

[agents.a1]
provider = "bash"
"#,
    )
    .unwrap();

    assert_eq!(config.master.cmd, "claude --extra 'quoted value'");
}

#[test]
fn empty_master_cmd_normalizes_to_claude() {
    let config = toml::from_str::<ProjectConfig>(
        r#"
version = "1"

[master]
cmd = ""

[agents.a1]
provider = "bash"
"#,
    )
    .unwrap();

    assert_eq!(config.master.cmd, "claude");
}

#[test]
fn load_project_config_default_master_cmd_long_form() {
    let dir = tempfile::TempDir::new().unwrap();
    let shared_credentials_dir = tempfile::TempDir::new().unwrap();
    let config_path = dir.path().join("ah.toml");
    std::fs::write(
        &config_path,
        format!(
            r#"
version = "1"

[providers.claude]
shared_credentials_dir = "{}"

[agents.a1]
provider = "bash"
"#,
            shared_credentials_dir.path().display()
        ),
    )
    .unwrap();

    let config = load_project_config(&config_path).unwrap();

    assert_eq!(config.master.cmd, DEFAULT_MASTER_CMD);
}
