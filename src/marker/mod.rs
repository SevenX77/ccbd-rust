//! VT100 marker matching and timeout registry for MVP3 semantic state changes.

pub mod matcher;
pub mod parser_registry;
pub mod registry;
pub mod timer;

pub use matcher::{MarkerMatcher, MatchResult};
pub use timer::{MarkerTimerHandle, TimerKind, spawn_marker_timer_task};
