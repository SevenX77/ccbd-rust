//! Prompt handling primitives for recognizing and resolving interactive CLI prompts.

pub mod events;
pub mod gating;
pub mod integration;
pub mod kb;
pub mod llm_client;
pub mod matcher;
pub mod resolve;
pub mod runner;
pub mod schema;
pub mod seeds;

pub use events::{UNKNOWN_PROMPT_DETECTED, UnknownPromptPayload, emit_unknown_prompt_detected};
pub use gating::{GateContext, GateSkipReason, PromptGateDecision, classify_capture};
pub use integration::{PromptScanDisposition, PromptScanRequest, scan_prompt_and_apply_outcome};
pub use kb::{load_or_bootstrap_kb, save_kb_atomic};
pub use matcher::{MatchOutcome, PromptScanPurpose, match_prompt, sanitize_pane_text};
pub use resolve::{
    ResolvePromptRequest, ResolvePromptResult, normalize_action_value, resolve_prompt_with_io,
};
pub use runner::{
    PromptIo, PromptRunOutcome, PromptSnapshot, RunnerContext, TmuxPromptIo, handle_prompt_chain,
};
pub use schema::{
    PromptAction, PromptCase, PromptFingerprint, PromptHandlerError, PromptKb, PromptResult,
    ValidatedAction,
};
pub use seeds::default_cases;
