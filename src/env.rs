use std::path::PathBuf;

pub fn resolve_state_dir() -> PathBuf {
    let dir = if std::env::var("CCB_ENV").as_deref() == Ok("dev") {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join("dev_state")
    } else {
        directories::ProjectDirs::from("", "", "ccbd")
            .expect("failed to resolve XDG project directories")
            .state_dir()
            .expect("failed to resolve XDG state directory")
            .to_path_buf()
    };

    std::fs::create_dir_all(&dir).expect("failed to create ccbd state directory");
    dir
}
