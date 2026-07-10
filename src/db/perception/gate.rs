//! C1 — Perception write gate: the single entry point that may mutate `agents.state`.
//!
//! All state producers (health check, marker timer, hook/log paths) must route state
//! changes through this one function instead of issuing `UPDATE agents SET state` in their
//! own call frame. Phase 1 only *builds* the gate and proves it works against a synthetic
//! caller (the acceptance tests) — no producer is migrated yet (that is Phase 2).
//!
//! g1-m1: implement the body to satisfy `super::phase1_acceptance`. The observable contract:
//! CAS on `expected_version`; honor the `from_states` guard; on a matched write, transition
//! `agents.state` to `to_state`, bump `state_version` by exactly one, and record a
//! `state_change` audit event carrying the caller-supplied `reason` **verbatim** (see the
//! existing `state_machine::transit_agent_state_conn_inner` payload convention — do not
//! hardcode a reason). Return `Ok(true)` on a matched write, `Ok(false)` on a rejected
//! precondition (missing agent / from-state mismatch / stale version). This is the only
//! place a raw `agents.state` write is permitted; the CI grep rule enforces that.

use crate::error::CcbdError;
use rusqlite::Connection;

/// The single arbitration entry point for `agents.state`.
///
/// Returns `Ok(true)` iff the CAS write applied; `Ok(false)` for a rejected precondition.
pub(crate) fn transit_agent_perception_state_sync(
    conn: &mut Connection,
    agent_id: &str,
    from_states: &[&str],
    to_state: &str,
    reason: &str,
    expected_version: i64,
) -> Result<bool, CcbdError> {
    let _ = (
        conn,
        agent_id,
        from_states,
        to_state,
        reason,
        expected_version,
    );
    todo!(
        "g1-m1: C1 perception write-gate — see src/db/perception/phase1_acceptance.rs for the contract"
    )
}
