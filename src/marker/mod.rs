//! VT100 marker matching and timeout registry for MVP3 semantic state changes.

pub mod matcher;
pub mod parser_registry;
pub mod perception_stream;
pub mod registry;
pub mod timer;

pub use matcher::{MarkerMatcher, MatchResult};
pub use perception_stream::{PerceptionStreamConfig, spawn_perception_stream_processor_task};
pub use timer::{
    MarkerTimerHandle, PromptTimerScanContext, TimerKind, spawn_marker_timer_task,
    spawn_marker_timer_task_with_prompt,
};
