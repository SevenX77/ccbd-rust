//! # Perception Arbiter — Phase 1 foundation (write-gate + event channel)
//!
//! Perception-arbiter design, Phase 1.
//!
//! This module is the **foundation** slice: it builds the single-write gate (C1),
//! the `perception_events` channel (C2), and is guarded by a CI grep rule. It does
//! **not** migrate any producer (Phase 2) nor host the arbiter's tri-state / budget /
//! epoch logic (Phase 3). Those land later and depend on this slice existing first.
//!
//! ## Contract handoff (g1 gatekeeper -> g1-m1 code-monkey)
//!
//! The RED acceptance tests in [`phase1_acceptance`] were authored **first** by g1 from
//! the spec contract, without an implementation in view. g1-m1 makes them GREEN by filling
//! the `todo!()` bodies below — **without editing `phase1_acceptance.rs`**. Signature or
//! interface disputes go to g1 via a `.lane-question` (g1 has final say in-lane).
//!
//! The pinned contract the tests bind to:
//!
//! - **C1 gate** ([`gate::transit_agent_perception_state_sync`]): the *single* function that
//!   may mutate `agents.state`. CAS on `expected_version`; from-state guard; on a successful
//!   write it must persist the **caller-supplied `reason` verbatim** (no hardcoded string —
//!   this is the structural kill of the `PANE_DIFF_STUCK`-for-everything bug) via a
//!   `state_change` event carrying `{from,to,reason}` (existing events-payload convention,
//!   design.md line 119).
//! - **C2 channel** ([`events::emit_perception_event_sync`] / [`events::query_perception_events_sync`]):
//!   perception events are a **typed convention over the existing `events` table**, *not* a new
//!   dedicated table (master decision citing design.md 118-119 / requirements line 35). The
//!   `events.agent_id` column carries the agent; the `events.payload` JSON carries
//!   `layer` / `verdict` / `epoch` / `observed_at` / `detail` (requirements C2 field list).
//!   Append-only.
//! - **CI grep rule**: `scripts/ci/check_state_write_gate.sh [ROOT]` — flags any
//!   `UPDATE agents SET state` (and `... SET status`) outside the gate file
//!   `db/perception/gate.rs`; exempts the gate; must pass on the Phase-1 tree (existing
//!   pre-migration call sites baselined — Phase 2 shrinks the baseline). With no `ROOT`
//!   arg it scans the repo `src/` resolved relative to the script's own location. Wire it
//!   into `.github/workflows/ci.yml` (a step running it on the real tree); the hermetic
//!   discrimination test here already gates it under `cargo test --all-targets`.
//!
//! Everything is `pub(crate)` so producers/arbiter (later phases) and the in-crate
//! acceptance tests can reach it; the gate is never re-exported publicly.

// Phase 1 foundation: the gate + channel exist before any producer is migrated onto them
// (Phase 2) or the arbiter consumes them (Phase 3), so in a non-test build they have no
// caller yet. Scoped allow keeps `cargo check`/clippy clean until Phase 2 wires producers.
#![allow(dead_code)]

pub(crate) mod events;
pub(crate) mod gate;
pub(crate) mod types;

#[cfg(test)]
mod phase1_acceptance;
