//! Prompt handling primitives for recognizing and resolving interactive CLI prompts.

pub mod gating;
pub mod kb;
pub mod matcher;
pub mod runner;
pub mod schema;
pub mod seeds;

pub use gating::{GateContext, GateSkipReason, PromptGateDecision, classify_capture};
pub use kb::{load_or_bootstrap_kb, save_kb_atomic};
pub use matcher::{MatchOutcome, match_prompt, sanitize_pane_text};
pub use runner::{
    PromptIo, PromptRunOutcome, PromptSnapshot, RunnerContext, TmuxPromptIo, handle_prompt_chain,
};
pub use schema::{
    PromptAction, PromptCase, PromptFingerprint, PromptHandlerError, PromptKb, PromptResult,
    ValidatedAction,
};
pub use seeds::default_cases;
