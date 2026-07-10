//! C2 — `perception_events` channel.
//!
//! Decision (master, citing design.md 118-119 / requirements line 35): perception events
//! are a **typed convention over the existing `events` table**, not a new dedicated table.
//! A producer emits exactly one append-only `events` row; the `events.agent_id` column
//! carries the agent and the `events.payload` JSON carries the requirements-C2 fields
//! `layer` / `verdict` / `epoch` / `observed_at` / `detail`.
//!
//! g1-m1: implement both functions to satisfy `super::phase1_acceptance`. Reuse the existing
//! `events::insert_event_sync` INSERT path (or an equivalent single append into `events`) —
//! do **not** create a `perception_events` table, and do **not** expose any UPDATE/DELETE of
//! emitted rows to producer code (append-only).

use super::types::{PerceptionEvent, PerceptionLayer, Verdict};
use crate::error::CcbdError;
use rusqlite::Connection;
use serde_json::Value;

/// Emit one append-only perception event into the `events`-table channel.
///
/// Returns the `events.seq_id` of the inserted row.
pub(crate) fn emit_perception_event_sync(
    conn: &Connection,
    agent_id: &str,
    layer: PerceptionLayer,
    verdict: Verdict,
    epoch: i64,
    observed_at: i64,
    detail: &Value,
) -> Result<i64, CcbdError> {
    let _ = (conn, agent_id, layer, verdict, epoch, observed_at, detail);
    todo!(
        "g1-m1: C2 emit_perception_event_sync — see src/db/perception/phase1_acceptance.rs for the contract"
    )
}

/// Read back every perception event for `agent_id`, decoded into [`PerceptionEvent`]s,
/// oldest-first.
pub(crate) fn query_perception_events_sync(
    conn: &Connection,
    agent_id: &str,
) -> Result<Vec<PerceptionEvent>, CcbdError> {
    let _ = (conn, agent_id);
    todo!(
        "g1-m1: C2 query_perception_events_sync — see src/db/perception/phase1_acceptance.rs for the contract"
    )
}
