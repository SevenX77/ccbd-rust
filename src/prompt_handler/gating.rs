//! Four-level hash gate for prompt-handler capture classification.

use crate::marker::{MarkerMatcher, MatchResult};
use crate::prompt_handler::matcher::{MatchOutcome, match_prompt, sanitize_pane_text};
use crate::prompt_handler::schema::{PromptAction, PromptKb};
use sha2::{Digest, Sha256};

#[derive(Clone, Copy)]
pub struct GateContext<'a> {
    pub provider: &'a str,
    pub kb: &'a PromptKb,
    pub marker_matcher: Option<&'a MarkerMatcher>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PromptGateDecision {
    Skip {
        reason: GateSkipReason,
        sanitized_hash: [u8; 32],
    },
    KnownAction {
        case_id: String,
        actions: Vec<PromptAction>,
        sanitized_hash: [u8; 32],
    },
    Unknown {
        sanitized_hash: [u8; 32],
        sanitized_text: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateSkipReason {
    SameHash,
    IdleMarker,
}

pub fn classify_capture(
    ctx: GateContext<'_>,
    prev_hash: Option<[u8; 32]>,
    capture: &str,
) -> PromptGateDecision {
    let sanitized_text = sanitize_pane_text(capture);
    let sanitized_hash = hash_sanitized_text(&sanitized_text);
    tracing::info!(
        provider = ctx.provider,
        has_prev_hash = prev_hash.is_some(),
        "prompt gate hash computed"
    );

    if prev_hash.is_some_and(|previous| previous == sanitized_hash) {
        tracing::info!(
            provider = ctx.provider,
            reason = "same sanitized capture hash",
            "prompt gate skipped capture"
        );
        return PromptGateDecision::Skip {
            reason: GateSkipReason::SameHash,
            sanitized_hash,
        };
    }

    if marker_matches(ctx.marker_matcher, capture) {
        tracing::info!(
            provider = ctx.provider,
            reason = "idle marker matched",
            "prompt gate skipped capture"
        );
        return PromptGateDecision::Skip {
            reason: GateSkipReason::IdleMarker,
            sanitized_hash,
        };
    }

    match match_prompt(ctx.provider, &sanitized_text, ctx.kb) {
        MatchOutcome::Matched { case_id, actions } => {
            tracing::info!(
                provider = ctx.provider,
                case_id,
                action_count = actions.len(),
                "prompt gate classified known prompt"
            );
            PromptGateDecision::KnownAction {
                case_id,
                actions,
                sanitized_hash,
            }
        }
        MatchOutcome::NoMatch => {
            tracing::info!(
                provider = ctx.provider,
                impact =
                    "caller should move agent to PROMPT_PENDING and emit UNKNOWN_PROMPT_DETECTED",
                "prompt gate classified unknown prompt"
            );
            PromptGateDecision::Unknown {
                sanitized_hash,
                sanitized_text,
            }
        }
    }
}

pub fn hash_sanitized_text(text: &str) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(text.as_bytes());
    hasher.finalize().into()
}

fn marker_matches(marker_matcher: Option<&MarkerMatcher>, capture: &str) -> bool {
    let Some(marker_matcher) = marker_matcher else {
        return false;
    };
    let mut parser = vt100::Parser::new(200, 200, 0);
    parser.process(capture.as_bytes());
    marker_matcher.scan(&parser) == MatchResult::Matched
}

#[cfg(test)]
mod tests {
    use super::{GateContext, GateSkipReason, PromptGateDecision, classify_capture};
    use crate::marker::MarkerMatcher;
    use crate::prompt_handler::gating::hash_sanitized_text;
    use crate::prompt_handler::matcher::sanitize_pane_text;
    use crate::prompt_handler::schema::{PromptAction, PromptKb};
    use crate::prompt_handler::seeds::default_cases;

    fn ctx<'a>(kb: &'a PromptKb) -> GateContext<'a> {
        GateContext {
            provider: "codex",
            kb,
            marker_matcher: None,
        }
    }

    #[test]
    fn same_hash_skips_capture() {
        let kb = PromptKb::new(default_cases());
        let capture = "Working(1s)\nNo prompt here";
        let hash = hash_sanitized_text(&sanitize_pane_text(capture));

        let decision = classify_capture(ctx(&kb), Some(hash), capture);

        assert!(matches!(
            decision,
            PromptGateDecision::Skip {
                reason: GateSkipReason::SameHash,
                ..
            }
        ));
    }

    #[test]
    fn idle_marker_skips_capture() {
        let kb = PromptKb::new(default_cases());
        let marker = MarkerMatcher::default();
        let ctx = GateContext {
            provider: "bash",
            kb: &kb,
            marker_matcher: Some(&marker),
        };

        let decision = classify_capture(ctx, None, "ready\n$ ");

        assert!(matches!(
            decision,
            PromptGateDecision::Skip {
                reason: GateSkipReason::IdleMarker,
                ..
            }
        ));
    }

    #[test]
    fn regex_match_returns_known_action() {
        let kb = PromptKb::new(default_cases());

        let decision = classify_capture(
            ctx(&kb),
            None,
            "Update available! Run `npm install -g @openai/codex`",
        );

        assert_eq!(
            decision,
            PromptGateDecision::KnownAction {
                case_id: "codex_update_01".into(),
                actions: vec![
                    PromptAction::Key { value: "2".into() },
                    PromptAction::Key {
                        value: "Enter".into()
                    },
                ],
                sanitized_hash: hash_sanitized_text(&sanitize_pane_text(
                    "Update available! Run `npm install -g @openai/codex`"
                )),
            }
        );
    }

    #[test]
    fn unknown_returns_pending_candidate() {
        let kb = PromptKb::new(default_cases());

        let decision = classify_capture(ctx(&kb), None, "Completely new approval screen");

        match decision {
            PromptGateDecision::Unknown {
                sanitized_hash,
                sanitized_text,
            } => {
                assert_eq!(sanitized_text, "Completely new approval screen");
                assert_eq!(sanitized_hash, hash_sanitized_text(&sanitized_text));
            }
            other => panic!("expected unknown, got {other:?}"),
        }
    }

    #[test]
    fn marker_like_text_does_not_become_unknown_when_marker_matches() {
        let kb = PromptKb::new(default_cases());
        let marker = MarkerMatcher::default();
        let ctx = GateContext {
            provider: "bash",
            kb: &kb,
            marker_matcher: Some(&marker),
        };

        let decision = classify_capture(ctx, None, "plain shell\n> ");

        assert!(matches!(
            decision,
            PromptGateDecision::Skip {
                reason: GateSkipReason::IdleMarker,
                ..
            }
        ));
    }
}
