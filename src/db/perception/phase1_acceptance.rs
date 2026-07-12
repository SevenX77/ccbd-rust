//! Phase 1 RED acceptance tests — authored by g1 (gatekeeper) from the spec contract,
//! before any implementation existed. g1-m1 makes these GREEN by implementing the
//! `todo!()` bodies in `gate.rs` / `events.rs` and the CI grep script; **g1-m1 must not
//! edit this file**. See `super` module doc for the pinned contract.
//!
//! Anchoring discipline: every assertion here is on a **contract-boundary observable** —
//! the durable `agents` row after a gate write, the durable `events` row after an emit, or
//! the exit status of the CI checker over a fixture tree — never on an implementation
//! internal. Rolling back the core write/emit logic (restoring `todo!()`) turns every test
//! RED, and a stub that returns a fixed value cannot pass the distinctness assertions.

use crate::db::agents::insert_agent_sync;
use crate::db::init;
use crate::db::perception::events::{emit_perception_event_sync, query_perception_events_sync};
use crate::db::perception::gate::transit_agent_perception_state_sync;
use crate::db::perception::types::{PerceptionLayer, Verdict};
use crate::db::sessions::insert_session_sync;
use rusqlite::Connection;
use serde_json::json;

fn with_test_db<T>(test: impl FnOnce(&mut Connection) -> T) -> T {
    let file = tempfile::NamedTempFile::new().unwrap();
    let db = init(file.path()).unwrap();
    let mut conn = db.conn();
    test(&mut conn)
}

fn seed_session(conn: &Connection) {
    insert_session_sync(conn, "s1", "p1", "/tmp/ah-perception").unwrap();
}

fn read_state(conn: &Connection, agent_id: &str) -> (String, i64) {
    conn.query_row(
        "SELECT state, state_version FROM agents WHERE id = ?",
        [agent_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .unwrap()
}

/// Latest `state_change` event's `reason` for an agent (the audit trail the gate writes).
fn last_state_change_reason(conn: &Connection, agent_id: &str) -> String {
    let payload: String = conn
        .query_row(
            "SELECT payload FROM events WHERE agent_id = ? AND event_type = 'state_change' ORDER BY seq_id DESC LIMIT 1",
            [agent_id],
            |row| row.get(0),
        )
        .unwrap();
    let value: serde_json::Value = serde_json::from_str(&payload).unwrap();
    value["reason"].as_str().unwrap().to_string()
}

// ---------------------------------------------------------------------------
// C1 — single write gate
// ---------------------------------------------------------------------------

/// A matched CAS write transitions state and bumps `state_version` by exactly one.
#[test]
fn gate_applies_transition_and_bumps_version_on_matching_cas() {
    with_test_db(|conn| {
        seed_session(conn);
        insert_agent_sync(conn, "a1", "s1", "bash", "BUSY", Some(1)).unwrap();

        let wrote = transit_agent_perception_state_sync(
            conn,
            "a1",
            &["BUSY", "WAITING_FOR_ACK"],
            "STUCK",
            "HEALTH_CHECK_STUCK",
            1,
        )
        .unwrap();
        assert!(wrote, "matching-CAS transition must apply");

        let (state, version) = read_state(conn, "a1");
        assert_eq!(state, "STUCK");
        assert_eq!(version, 2, "state_version must bump exactly once");
    });
}

/// The gate persists the caller-supplied `reason` verbatim — it does not default/hardcode
/// one. This is the structural kill of the dual-`PANE_DIFF_STUCK` bug at the write layer:
/// two agents stalled by two different layers must carry two different reasons.
#[test]
fn gate_persists_caller_supplied_reason_verbatim_not_hardcoded() {
    with_test_db(|conn| {
        seed_session(conn);
        insert_agent_sync(conn, "a1", "s1", "bash", "BUSY", Some(1)).unwrap();
        insert_agent_sync(conn, "a2", "s1", "bash", "BUSY", Some(2)).unwrap();

        assert!(
            transit_agent_perception_state_sync(
                conn,
                "a1",
                &["BUSY"],
                "STUCK",
                "HEALTH_CHECK_STUCK",
                1,
            )
            .unwrap()
        );
        assert!(
            transit_agent_perception_state_sync(
                conn,
                "a2",
                &["BUSY"],
                "STUCK",
                "BUSY_MARKER_STALE_3H",
                1,
            )
            .unwrap()
        );

        let reason_a1 = last_state_change_reason(conn, "a1");
        let reason_a2 = last_state_change_reason(conn, "a2");
        assert_eq!(reason_a1, "HEALTH_CHECK_STUCK");
        assert_eq!(reason_a2, "BUSY_MARKER_STALE_3H");
        assert_ne!(
            reason_a1, reason_a2,
            "gate must not collapse distinct layer reasons into one hardcoded string"
        );
    });
}

/// A stale `expected_version` is rejected: no state change, no version bump.
#[test]
fn gate_rejects_write_on_stale_version_cas() {
    with_test_db(|conn| {
        seed_session(conn);
        insert_agent_sync(conn, "a1", "s1", "bash", "BUSY", Some(1)).unwrap();

        let wrote = transit_agent_perception_state_sync(
            conn,
            "a1",
            &["BUSY"],
            "STUCK",
            "HEALTH_CHECK_STUCK",
            99, // stale — real version is 1
        )
        .unwrap();
        assert!(!wrote, "stale-version write must be a rejected no-op");

        let (state, version) = read_state(conn, "a1");
        assert_eq!(state, "BUSY", "state unchanged after rejected CAS");
        assert_eq!(version, 1, "version unchanged after rejected CAS");
    });
}

/// A from-state the caller did not permit is rejected: the gate does not transition an
/// agent out of a disallowed state even when the version matches.
#[test]
fn gate_rejects_transition_when_from_state_not_allowed() {
    with_test_db(|conn| {
        seed_session(conn);
        insert_agent_sync(conn, "a1", "s1", "bash", "CRASHED", Some(1)).unwrap();

        let wrote = transit_agent_perception_state_sync(
            conn,
            "a1",
            &["BUSY", "WAITING_FOR_ACK"],
            "STUCK",
            "HEALTH_CHECK_STUCK",
            1,
        )
        .unwrap();
        assert!(!wrote, "transition from a disallowed state must be a no-op");

        let (state, _version) = read_state(conn, "a1");
        assert_eq!(
            state, "CRASHED",
            "state unchanged when from-state disallowed"
        );
    });
}

// ---------------------------------------------------------------------------
// C2 — perception_events channel (typed convention over the `events` table)
// ---------------------------------------------------------------------------

/// An emit appends exactly one row to the existing `events` table, and no dedicated
/// `perception_events` table is created (the master's new-table-vs-convention decision).
#[test]
fn perception_event_lands_in_events_table_and_no_dedicated_table_created() {
    with_test_db(|conn| {
        seed_session(conn);
        insert_agent_sync(conn, "a1", "s1", "bash", "IDLE", Some(1)).unwrap();

        let before: i64 = conn
            .query_row("SELECT COUNT(*) FROM events", [], |row| row.get(0))
            .unwrap();
        let seq = emit_perception_event_sync(
            conn,
            "a1",
            PerceptionLayer::Os,
            Verdict::True,
            7,
            1_700_000_000,
            &json!({ "pid_alive": true }),
        )
        .unwrap();
        assert!(seq > 0, "emit must return the inserted events.seq_id");

        let after: i64 = conn
            .query_row("SELECT COUNT(*) FROM events", [], |row| row.get(0))
            .unwrap();
        assert_eq!(after - before, 1, "emit must append exactly one events row");

        let dedicated_table: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'perception_events'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            dedicated_table, 0,
            "C2 decision: reuse the events table; do not create a perception_events table"
        );
    });
}

/// Round-trip with two events that differ in BOTH layer and verdict — a colluding stub
/// (always returning one fixed value) cannot satisfy the distinctness assertions. Also
/// proves append-only: two emits yield two rows.
#[test]
fn perception_events_roundtrip_distinct_layers_and_verdicts_append_only() {
    with_test_db(|conn| {
        seed_session(conn);
        insert_agent_sync(conn, "a1", "s1", "bash", "IDLE", Some(1)).unwrap();

        emit_perception_event_sync(
            conn,
            "a1",
            PerceptionLayer::Os,
            Verdict::True,
            7,
            1_700_000_000,
            &json!({ "k": "os" }),
        )
        .unwrap();
        emit_perception_event_sync(
            conn,
            "a1",
            PerceptionLayer::Hook,
            Verdict::Unknown,
            8,
            1_700_000_050,
            &json!({ "k": "hook" }),
        )
        .unwrap();

        let events = query_perception_events_sync(conn, "a1").unwrap();
        assert_eq!(events.len(), 2, "append-only: two emits -> two rows");

        let os = events
            .iter()
            .find(|e| e.epoch == 7)
            .expect("epoch-7 event present");
        let hook = events
            .iter()
            .find(|e| e.epoch == 8)
            .expect("epoch-8 event present");

        assert_eq!(os.agent_id, "a1");
        assert_eq!(os.layer, PerceptionLayer::Os);
        assert_eq!(os.verdict, Verdict::True);
        assert_eq!(os.observed_at, 1_700_000_000);
        assert_eq!(os.detail["k"], "os");

        assert_eq!(hook.layer, PerceptionLayer::Hook);
        assert_eq!(hook.verdict, Verdict::Unknown);
        assert_eq!(hook.observed_at, 1_700_000_050);
        assert_eq!(hook.detail["k"], "hook");

        assert_ne!(
            os.layer, hook.layer,
            "reader must not collapse distinct layers"
        );
        assert_ne!(
            os.verdict, hook.verdict,
            "reader must not collapse distinct verdicts"
        );
    });
}

/// The durable `events.payload` (read raw, independent of the typed reader) carries the
/// requirements-C2 fields. Anchors the storage boundary so a matching-writer/reader pair
/// cannot pass without actually persisting the values.
#[test]
fn perception_event_raw_payload_carries_spec_fields() {
    with_test_db(|conn| {
        seed_session(conn);
        insert_agent_sync(conn, "a1", "s1", "bash", "IDLE", Some(1)).unwrap();

        let seq = emit_perception_event_sync(
            conn,
            "a1",
            PerceptionLayer::Log,
            Verdict::False,
            42,
            1_700_000_123,
            &json!({ "quiet_secs": 900 }),
        )
        .unwrap();

        let payload: String = conn
            .query_row(
                "SELECT payload FROM events WHERE seq_id = ?",
                [seq],
                |row| row.get(0),
            )
            .unwrap();
        let value: serde_json::Value = serde_json::from_str(&payload).unwrap();

        assert_eq!(value["epoch"], 42, "payload must carry numeric epoch");
        assert_eq!(
            value["observed_at"], 1_700_000_123,
            "payload must carry observed_at"
        );
        assert_eq!(
            value["detail"]["quiet_secs"], 900,
            "payload must carry detail verbatim"
        );
        assert!(
            value.get("layer").is_some(),
            "payload must carry a layer field"
        );
        assert!(
            value.get("verdict").is_some(),
            "payload must carry a verdict field"
        );
    });
}

// ---------------------------------------------------------------------------
// CI grep rule — bans direct `UPDATE agents SET state` outside the gate
// ---------------------------------------------------------------------------

/// The checker discriminates a direct state write by location: it flags one in a non-gate
/// file and exempts the same write in the gate file. Hermetic (runs over fixture trees) so
/// it is robust to the live tree's evolving baseline.
///
/// The offending SQL is assembled at runtime from parts so the literal
/// `UPDATE agents SET state` never appears contiguously in THIS source file — otherwise the
/// checker, scanning the real tree, would count this test as a violation.
#[test]
fn ci_grep_rule_flags_direct_state_write_outside_gate_and_exempts_gate() {
    use std::process::Command;

    let script = format!(
        "{}/scripts/ci/check_state_write_gate.sh",
        env!("CARGO_MANIFEST_DIR")
    );
    assert!(
        std::path::Path::new(&script).exists(),
        "CI grep-rule script missing at {script} — g1-m1 must add it"
    );

    let offending = format!(
        "conn.execute(\"{} WHERE id = ?\", []);",
        ["UPDATE", "agents", "SET", "state", "=", "'STUCK'"].join(" ")
    );

    // (a) direct write in a NON-gate file -> checker must FAIL.
    let bad = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(bad.path().join("db")).unwrap();
    std::fs::write(bad.path().join("db/state_machine.rs"), &offending).unwrap();
    let bad_run = Command::new("bash")
        .arg(&script)
        .arg(bad.path())
        .output()
        .unwrap();
    assert!(
        !bad_run.status.success(),
        "checker must flag a direct state write in a non-gate file"
    );

    // (b) the same write inside the gate file -> checker must PASS (gate is exempt).
    let good = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(good.path().join("db/perception")).unwrap();
    std::fs::write(good.path().join("db/perception/gate.rs"), &offending).unwrap();
    let good_run = Command::new("bash")
        .arg(&script)
        .arg(good.path())
        .output()
        .unwrap();
    assert!(
        good_run.status.success(),
        "checker must exempt the gate file db/perception/gate.rs; stderr={}",
        String::from_utf8_lossy(&good_run.stderr)
    );
}

/// The checker passes on the Phase-1 repo tree: the gate exists and every pre-migration
/// direct-write site is baselined (Phase 2 shrinks the baseline). With no ROOT arg the
/// checker scans the repo `src/` resolved relative to its own location.
#[test]
fn ci_grep_rule_passes_on_current_repo_tree() {
    use std::process::Command;

    let script = format!(
        "{}/scripts/ci/check_state_write_gate.sh",
        env!("CARGO_MANIFEST_DIR")
    );
    assert!(
        std::path::Path::new(&script).exists(),
        "CI grep-rule script missing at {script} — g1-m1 must add it"
    );

    let run = Command::new("bash").arg(&script).output().unwrap();
    assert!(
        run.status.success(),
        "checker must pass on the Phase-1 tree (baseline the known pre-migration sites); stdout={} stderr={}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );
}
