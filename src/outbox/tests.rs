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
