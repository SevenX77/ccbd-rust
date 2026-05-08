use ccbd::db;

#[tokio::test(flavor = "multi_thread")]
async fn session_query_returns_project_absolute_path() {
    let file = tempfile::NamedTempFile::new().unwrap();
    let db = db::init(file.path()).unwrap();
    let absolute_path = "/tmp/r3-absolute-path-project";

    db::sessions::create_session(
        db.clone(),
        "s_r3_absolute".to_string(),
        "p_r3_absolute".to_string(),
        absolute_path.to_string(),
    )
    .await
    .unwrap();

    let session = db::sessions::query_session_by_id(db, "s_r3_absolute".to_string())
        .await
        .unwrap()
        .unwrap();

    assert_eq!(session.id, "s_r3_absolute");
    assert_eq!(session.project_id, "p_r3_absolute");
    assert_eq!(session.absolute_path, absolute_path);
}
