//! Pinned contract types for the perception channel (C2 event shape / C3 tri-state).
//!
//! These are the interface the acceptance tests bind to. Their *representation* on the
//! wire (how the enums serialize into the `events.payload` JSON) is g1-m1's choice; the
//! tests assert observable values, not a specific spelling.

use serde_json::Value;

/// Signal-producing layer (requirements C2: `os` | `log` | `hook`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PerceptionLayer {
    Os,
    Log,
    Hook,
}

/// Tri-state verdict (requirements C2/C3: `True` | `False` | `Unknown`).
///
/// `Unknown` is a first-class, explicitly-emitted value — never "absence collapsed to
/// False". The budget/emit-on-first-observation semantics that give `Unknown` its teeth
/// are Phase 3; here it is only the carried value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Verdict {
    True,
    False,
    Unknown,
}

/// A perception event as read back from the channel (requirements C2 carried fields).
///
/// `agent_id` lives in the `events` table column; the rest live in the `events.payload`
/// JSON — this struct is the decoded view over one such row.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PerceptionEvent {
    pub(crate) agent_id: String,
    pub(crate) layer: PerceptionLayer,
    pub(crate) verdict: Verdict,
    pub(crate) epoch: i64,
    pub(crate) observed_at: i64,
    pub(crate) detail: Value,
}
