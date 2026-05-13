//! VT100 marker matching and timeout registry for MVP3 semantic state changes.

pub mod matcher;
pub mod parser_registry;
pub mod registry;
pub mod timer;

pub use matcher::{MarkerMatcher, MatchResult};
pub use timer::{
    MarkerTimerHandle, PromptTimerScanContext, TimerKind, spawn_marker_timer_task,
    spawn_marker_timer_task_with_prompt,
};
