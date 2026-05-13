//! Prompt handling primitives for recognizing and resolving interactive CLI prompts.

pub mod kb;
pub mod schema;
pub mod seeds;

pub use kb::{load_or_bootstrap_kb, save_kb_atomic};
pub use schema::{
    PromptAction, PromptCase, PromptFingerprint, PromptHandlerError, PromptKb, PromptResult,
    ValidatedAction,
};
pub use seeds::default_cases;
