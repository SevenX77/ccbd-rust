use ccbd::sandbox::bwrap;
use std::process::Command;

#[test]
fn bwrap_workspace_binds_project_root_and_chdirs() {
    if which::which("bwrap").is_err() {
        eprintln!("skipping: bwrap binary not found");
        return;
    }
    let sandbox = tempfile::TempDir::new().unwrap();
    let project = tempfile::TempDir::new().unwrap();
    std::fs::write(project.path().join("r3-marker.txt"), "project-root").unwrap();

    let args = bwrap::build_args(
        sandbox.path(),
        project.path(),
        &bwrap::SandboxOverrides::default(),
        None,
    )
    .unwrap();
    let output = Command::new("bwrap")
        .args(args)
        .args([
            "/bin/sh",
            "-c",
            "test -f /workspace/r3-marker.txt && pwd",
        ])
        .output()
        .expect("bwrap should run");

    assert!(
        output.status.success(),
        "bwrap failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "/workspace");
}
