use ccbd::tmux::scope::{ScopePolicy, UnitConfig, wrap_in_scope};

fn args(command: &std::process::Command) -> Vec<String> {
    command
        .get_args()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect()
}

#[test]
fn tmux_scope_bindsto_uses_ccbd_service() {
    let policy = ScopePolicy::Systemd(UnitConfig {
        unit_name: "ccbd-tmux-test1234".to_string(),
        slice: "ccbd-agents.slice".to_string(),
        binds_to: Some("ccbd.service".to_string()),
    });
    let command = wrap_in_scope("tmux", &["-L", "sock", "new-session"], &policy);
    let args = args(&command);

    assert!(args.contains(&"--property=BindsTo=ccbd.service".to_string()));
    assert!(!args.contains(&"--property=BindsTo=ccbd-rust.service".to_string()));
}
