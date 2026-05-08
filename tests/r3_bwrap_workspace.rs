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

#[test]
fn bwrap_workspace_git_is_read_only() {
    if which::which("bwrap").is_err() {
        eprintln!("skipping: bwrap binary not found");
        return;
    }
    let sandbox = tempfile::TempDir::new().unwrap();
    let project = tempfile::TempDir::new().unwrap();
    std::fs::create_dir(project.path().join(".git")).unwrap();

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
            "touch /workspace/.git/test 2>/tmp/touch.err; code=$?; cat /tmp/touch.err; test $code -ne 0",
        ])
        .output()
        .expect("bwrap should run");

    assert!(
        output.status.success(),
        "touch unexpectedly succeeded: stdout={}, stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("Read-only file system"),
        "expected EROFS output, got stdout={}, stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn bwrap_extra_ro_bind_is_mounted() {
    if which::which("bwrap").is_err() {
        eprintln!("skipping: bwrap binary not found");
        return;
    }
    let sandbox = tempfile::TempDir::new().unwrap();
    let project = tempfile::TempDir::new().unwrap();
    let extra = tempfile::TempDir::new().unwrap();
    std::fs::write(extra.path().join("extra-marker.txt"), "extra").unwrap();

    let args = bwrap::build_args(
        sandbox.path(),
        project.path(),
        &bwrap::SandboxOverrides {
            network: None,
            extra_ro_binds: vec![bwrap::RoBind {
                host_path: extra.path().to_path_buf(),
                sandbox_path: "/extra".into(),
            }],
        },
        None,
    )
    .unwrap();
    let output = Command::new("bwrap")
        .args(args)
        .args(["/bin/sh", "-c", "cat /extra/extra-marker.txt"])
        .output()
        .expect("bwrap should run");

    assert!(
        output.status.success(),
        "bwrap failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "extra");
}
