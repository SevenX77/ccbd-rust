use ah::db;
use rusqlite::{Connection, params};

fn setup_db() -> (tempfile::NamedTempFile, db::Db) {
    let file = tempfile::NamedTempFile::new().unwrap();
    let db = db::init(file.path()).unwrap();
    {
        let conn = db.conn();
        conn.execute(
            "INSERT INTO projects (id, absolute_path) VALUES ('p1', '/tmp/pr1a')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO sessions (id, project_id, master_pid) VALUES ('s1', 'p1', 1)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO agents (id, session_id, provider, state, pid) VALUES ('a1', 's1', 'bash', 'BUSY', 123)",
            [],
        )
        .unwrap();
    }
    (file, db)
}

fn insert_dispatched_job(
    conn: &Connection,
    job_id: &str,
    requires_physical_evidence: bool,
    requires_test_evidence: bool,
) {
    conn.execute(
        "INSERT INTO jobs (
            id, agent_id, request_id, prompt_text, status, dispatched_at, dispatched_at_seq_id,
            requires_physical_evidence, requires_test_evidence
        ) VALUES (?, 'a1', NULL, 'change code', 'DISPATCHED', unixepoch(), 0, ?, ?)",
        params![
            job_id,
            i64::from(requires_physical_evidence),
            i64::from(requires_test_evidence)
        ],
    )
    .unwrap();
}

fn insert_job_evidence(conn: &Connection, job_id: &str, evidence_type: &str) {
    conn.execute(
        "INSERT INTO events (agent_id, request_id, event_type, payload) VALUES ('a1', NULL, 'test_seed', '{}')",
        [],
    )
    .unwrap();
    let event_seq_id = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO evidence (
            id, agent_id, event_seq_id, pane_bytes, failed_rules, status,
            job_id, evidence_type, subject_path, payload
        ) VALUES (?, 'a1', ?, X'01', '[]', 'PENDING', ?, ?, NULL, '{}')",
        params![
            format!("evi_{job_id}_{evidence_type}"),
            event_seq_id,
            job_id,
            evidence_type
        ],
    )
    .unwrap();
}

fn insert_output(conn: &Connection, text: &str) {
    conn.execute(
        "INSERT INTO events (agent_id, request_id, event_type, payload) VALUES ('a1', NULL, 'output_chunk', ?)",
        params![serde_json::json!({ "text": text }).to_string()],
    )
    .unwrap();
}

fn state_and_job_status(db: &db::Db, job_id: &str) -> (String, String) {
    db.conn()
        .query_row(
            "SELECT agents.state, jobs.status FROM agents JOIN jobs ON jobs.agent_id = agents.id WHERE agents.id = 'a1' AND jobs.id = ?",
            params![job_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap()
}

fn evidence_denied_payload(db: &db::Db) -> Option<String> {
    db.conn()
        .query_row(
            "SELECT payload FROM events WHERE agent_id = 'a1' AND event_type = 'evidence_denied' ORDER BY seq_id DESC LIMIT 1",
            [],
            |row| row.get(0),
        )
        .ok()
}

#[tokio::test]
async fn missing_physical_evidence_blocks_idle_completion_and_records_deny() {
    let (_file, db) = setup_db();
    {
        let conn = db.conn();
        insert_dispatched_job(&conn, "job_physical", true, false);
    }

    let (changes, affected_job) =
        db::state_machine::mark_agent_idle_matched(db.clone(), "a1".to_string())
            .await
            .unwrap();

    assert_eq!(changes, 0);
    assert_eq!(affected_job, None);
    assert_eq!(
        state_and_job_status(&db, "job_physical"),
        ("BUSY".into(), "DISPATCHED".into())
    );
    let payload = evidence_denied_payload(&db).expect("missing evidence_denied event");
    assert!(payload.contains("SYSTEM DENY: Missing physical evidence"));
}

#[tokio::test]
async fn missing_test_passed_blocks_tdd_job_completion() {
    let (_file, db) = setup_db();
    {
        let conn = db.conn();
        insert_dispatched_job(&conn, "job_tdd", true, true);
        insert_job_evidence(&conn, "job_tdd", "diff_generated");
    }

    let (changes, affected_job) =
        db::state_machine::mark_agent_idle_matched(db.clone(), "a1".to_string())
            .await
            .unwrap();

    assert_eq!(changes, 0);
    assert_eq!(affected_job, None);
    assert_eq!(
        state_and_job_status(&db, "job_tdd"),
        ("BUSY".into(), "DISPATCHED".into())
    );
    let payload = evidence_denied_payload(&db).expect("missing evidence_denied event");
    assert!(payload.contains("test_passed"));
}

#[tokio::test]
async fn physical_and_test_evidence_allows_completion() {
    let (_file, db) = setup_db();
    {
        let conn = db.conn();
        insert_dispatched_job(&conn, "job_green", true, true);
        insert_job_evidence(&conn, "job_green", "diff_generated");
        insert_job_evidence(&conn, "job_green", "test_passed");
        insert_output(&conn, "done\n");
    }

    let (changes, affected_job) =
        db::state_machine::mark_agent_idle_matched(db.clone(), "a1".to_string())
            .await
            .unwrap();

    assert_eq!(changes, 1);
    assert_eq!(affected_job.as_deref(), Some("job_green"));
    assert_eq!(
        state_and_job_status(&db, "job_green"),
        ("IDLE".into(), "COMPLETED".into())
    );
}
