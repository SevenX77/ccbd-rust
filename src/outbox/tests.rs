//! Contract tests for the R1 outbox transport. Assertions anchor to observable
//! boundary artifacts — durable files on disk and committed DB rows — never to the
//! module's internal state.

use super::*;
use std::fs;

fn sample_hook_record() -> OutboxRecord {
    OutboxRecord {
        event_id: new_event_id(),
        kind: OutboxKind::HookEvent,
        agent_id: "a1".to_string(),
        provider: Some("claude".to_string()),
        event: Some("stop".to_string()),
        attempt_cookie: Some("job_abc:3".to_string()),
        job_id: None,
        reply_text: None,
        reason: None,
        hook_fired_at: Some(1_720_000_000),
        payload: Some(serde_json::json!({"k": "v"})),
    }
}

// ---------------------------------------------------------------------------
// R1-T1 — journal-first durable write (CP-R1.1)
// ---------------------------------------------------------------------------

#[test]
fn journal_writes_durable_json_that_round_trips() {
    let tmp = tempfile::tempdir().unwrap();
    let outbox = tmp.path().join("outbox").join("a1");
    let record = sample_hook_record();

    let path = journal_record(&outbox, &record).expect("journal must succeed on a writable dir");

    // Durable file exists under the pinned `.json` name.
    assert!(path.exists(), "durable .json must exist after journal");
    assert_eq!(path.extension().and_then(|e| e.to_str()), Some("json"));

    // It round-trips back to the exact record (union wire form preserved).
    let bytes = fs::read(&path).unwrap();
    let parsed: OutboxRecord = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(parsed, record, "durable record must round-trip byte-for-byte");
}

#[test]
fn journal_leaves_no_tmp_and_matches_scanner_glob() {
    let tmp = tempfile::tempdir().unwrap();
    let outbox = tmp.path().join("outbox").join("a1");
    let record = sample_hook_record();

    journal_record(&outbox, &record).unwrap();

    // The scanner globs `*.json`; a durable record is exactly one such file and there is
    // NO leftover `.tmp` for the scanner to trip over (F-7: writer glob == scanner glob).
    let entries: Vec<_> = fs::read_dir(&outbox)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    let jsons: Vec<_> = entries.iter().filter(|n| n.ends_with(".json")).collect();
    let tmps: Vec<_> = entries.iter().filter(|n| n.ends_with(".tmp")).collect();
    assert_eq!(jsons.len(), 1, "exactly one durable .json expected, got {entries:?}");
    assert!(tmps.is_empty(), "no .tmp may survive a committed journal, got {tmps:?}");
}

#[test]
fn journal_creates_missing_outbox_dir() {
    let tmp = tempfile::tempdir().unwrap();
    // Neither `outbox/` nor the per-agent subdir exists yet.
    let outbox = tmp.path().join("outbox").join("a1");
    assert!(!outbox.exists());

    journal_record(&outbox, &sample_hook_record()).expect("journal must create the outbox dir");
    assert!(outbox.is_dir());
}

#[test]
fn journal_failure_returns_error_never_silent_ok() {
    // Fault injection: the "outbox dir" path has a *file* where a parent directory is
    // required, so `create_dir_all`/write cannot succeed. The contract: journal returns
    // Err (the CLI turns this into a loud non-zero exit) — it must NEVER return Ok when
    // nothing durable landed. This is the mirror image of the exit-0-on-failed-journal bug.
    let tmp = tempfile::tempdir().unwrap();
    let blocker = tmp.path().join("blocker");
    fs::write(&blocker, b"i am a file, not a directory").unwrap();
    let outbox = blocker.join("outbox").join("a1"); // parent is a file → cannot mkdir

    let result = journal_record(&outbox, &sample_hook_record());
    assert!(
        result.is_err(),
        "a journal that cannot make its record durable MUST return Err, not silent Ok"
    );
}

#[test]
fn record_union_wire_form_round_trips_for_job_done() {
    let record = OutboxRecord {
        event_id: new_event_id(),
        kind: OutboxKind::JobDone,
        agent_id: "a2".to_string(),
        provider: Some("antigravity".to_string()),
        event: None,
        attempt_cookie: Some("job_xyz:1".to_string()),
        job_id: Some("job_xyz".to_string()),
        reply_text: Some("the declared result".to_string()),
        reason: None,
        hook_fired_at: Some(1_720_000_500),
        payload: None,
    };
    let json = serde_json::to_string(&record).unwrap();
    // Absent optional fields are omitted on the wire (union schema, mostly-null elided).
    assert!(!json.contains("\"event\""), "absent field must be omitted: {json}");
    assert!(json.contains("\"kind\":\"job_done\""), "kind wire string pinned: {json}");
    let parsed: OutboxRecord = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed, record);
}

#[test]
fn selfcheck_id_is_recognized_as_reserved() {
    assert!(is_reserved_selfcheck_id("selfcheck:a1:boot-7"));
    assert!(!is_reserved_selfcheck_id(&new_event_id()));
}

// ---------------------------------------------------------------------------
// JC-1 — transport dedup ledger + consume funnel (CP-R1.2)
// ---------------------------------------------------------------------------

use crate::db::Db;
use rusqlite::params;

/// A real, migrated ahd database on a temp file (WAL requires a file). The returned
/// `TempDir` must be kept alive for the DB path to stay valid.
fn test_db() -> (Db, tempfile::TempDir) {
    let tmp = tempfile::tempdir().unwrap();
    let db = crate::db::init(&tmp.path().join("ahd.sqlite")).unwrap();
    (db, tmp)
}

/// Insert the minimal FK chain (projects → sessions → agents) so a `hook_event` record can
/// land its `events` row.
fn insert_agent(conn: &rusqlite::Connection, agent_id: &str) {
    conn.execute(
        "INSERT OR IGNORE INTO projects(id, absolute_path) VALUES ('p1', '/tmp/p1')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT OR IGNORE INTO sessions(id, project_id, master_pid) VALUES ('s1', 'p1', 1)",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO agents(id, session_id, provider, state) VALUES (?1, 's1', 'claude', 'BUSY')",
        params![agent_id],
    )
    .unwrap();
}

fn job_done_record(job_id: &str, agent_id: &str) -> OutboxRecord {
    OutboxRecord {
        event_id: new_event_id(),
        kind: OutboxKind::JobDone,
        agent_id: agent_id.to_string(),
        provider: Some("claude".to_string()),
        event: None,
        attempt_cookie: Some(format!("{job_id}:1")),
        job_id: Some(job_id.to_string()),
        reply_text: Some("declared result".to_string()),
        reason: None,
        hook_fired_at: Some(1_720_000_000),
        payload: None,
    }
}

fn count(conn: &rusqlite::Connection, sql: &str, id: &str) -> i64 {
    conn.query_row(sql, params![id], |r| r.get(0)).unwrap()
}

#[test]
fn jc1_dedup_f3_job_done_no_double_apply() {
    // RED contract: feed the SAME job_done event_id twice (at-least-once redelivery /
    // cold-scan replay). The second must dedup at the ledger — exactly one stub row and one
    // ledger row survive. This is the g1 (Track A) JC-1 acceptance: a replayed events-path
    // record dedups; here proven on the F3 stub path too.
    let (db, _tmp) = test_db();
    let mut guard = db.conn();
    let conn = &mut *guard;

    let rec = job_done_record("job1", "a1");
    assert_eq!(consume_record(conn, &rec).unwrap(), ConsumeOutcome::FirstSeen);
    assert_eq!(
        consume_record(conn, &rec).unwrap(),
        ConsumeOutcome::Duplicate,
        "a replayed job_done must dedup, not re-apply"
    );

    assert_eq!(
        count(conn, "SELECT COUNT(*) FROM outbox_job_declaration_stub WHERE event_id=?1", &rec.event_id),
        1,
        "replayed job_done must not double-apply to the F3 sink"
    );
    assert_eq!(
        count(conn, "SELECT COUNT(*) FROM outbox_consumed WHERE event_id=?1", &rec.event_id),
        1,
    );
}

#[test]
fn jc1_dedup_f2_hook_event_no_double_apply() {
    // The F2 events-path arm of the SAME single ledger: a replayed hook_event lands exactly
    // one `events` row.
    let (db, _tmp) = test_db();
    let mut guard = db.conn();
    let conn = &mut *guard;
    insert_agent(conn, "a1");

    let rec = sample_hook_record();
    assert_eq!(consume_record(conn, &rec).unwrap(), ConsumeOutcome::FirstSeen);
    assert_eq!(consume_record(conn, &rec).unwrap(), ConsumeOutcome::Duplicate);

    assert_eq!(
        count(
            conn,
            "SELECT COUNT(*) FROM events WHERE agent_id=?1 AND event_type='hook_event'",
            "a1"
        ),
        1,
        "replayed hook_event must not double-insert into events"
    );
}

#[test]
fn jc1_dedup_is_keyed_on_event_id_not_job() {
    // Two DIFFERENT event_ids for the SAME job both apply — the ledger keys on the outbox
    // event_id, so distinct deliveries are distinct effects (not collapsed by job_id).
    let (db, _tmp) = test_db();
    let mut guard = db.conn();
    let conn = &mut *guard;

    let a = job_done_record("job1", "a1");
    let b = job_done_record("job1", "a1"); // same job, fresh event_id
    assert_eq!(consume_record(conn, &a).unwrap(), ConsumeOutcome::FirstSeen);
    assert_eq!(consume_record(conn, &b).unwrap(), ConsumeOutcome::FirstSeen);
    assert_eq!(
        count(conn, "SELECT COUNT(*) FROM outbox_job_declaration_stub WHERE job_id=?1", "job1"),
        2,
    );
}

#[test]
fn jc1_dedup_and_effect_are_one_transaction() {
    // Atomicity (design R1-Q2: "dedup insert and the handler's effect commit in one
    // transaction"). A hook_event for a NON-existent agent fails the FK on the effect insert;
    // the whole tx must roll back so NO ledger row is left behind — otherwise the record
    // would be silently swallowed (a hole, not a replay). Prove the ledger is clean so a
    // later cold-scan can still retry/quarantine it.
    let (db, _tmp) = test_db();
    let mut guard = db.conn();
    let conn = &mut *guard;
    // note: no insert_agent → the events FK will reject the effect.

    let rec = sample_hook_record();
    let outcome = consume_record(conn, &rec);
    assert!(outcome.is_err(), "effect FK failure must surface as Err");
    assert_eq!(
        count(conn, "SELECT COUNT(*) FROM outbox_consumed WHERE event_id=?1", &rec.event_id),
        0,
        "a rolled-back effect must NOT leave a committed ledger row (dedup+effect atomic)"
    );
}
