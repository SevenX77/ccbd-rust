//! Recursive prompt-handler runner that executes known prompt actions.

use crate::error::CcbdError;
use crate::marker::MarkerMatcher;
use crate::prompt_handler::gating::{GateContext, PromptGateDecision, classify_capture};
use crate::prompt_handler::matcher::sanitize_pane_text;
use crate::prompt_handler::schema::{PromptAction, PromptKb, ValidatedAction};
use crate::tmux::{TmuxPaneId, TmuxServer};
use std::time::Duration;

const DEFAULT_ACTION_SETTLE_DELAY: Duration = Duration::from_millis(200);

pub trait PromptIo: Send + Sync {
    fn capture_pane(&self, pane_id: &TmuxPaneId) -> Result<String, CcbdError>;
    fn send_key_literal(&self, pane_id: &TmuxPaneId, value: &str) -> Result<(), CcbdError>;
    fn send_key_keysym(&self, pane_id: &TmuxPaneId, value: &str) -> Result<(), CcbdError>;
}

pub struct TmuxPromptIo {
    tmux: TmuxServer,
}

impl TmuxPromptIo {
    pub fn new(tmux: TmuxServer) -> Self {
        Self { tmux }
    }
}

impl PromptIo for TmuxPromptIo {
    fn capture_pane(&self, pane_id: &TmuxPaneId) -> Result<String, CcbdError> {
        self.tmux.capture_pane_sync(pane_id)
    }

    fn send_key_literal(&self, pane_id: &TmuxPaneId, value: &str) -> Result<(), CcbdError> {
        self.tmux.send_keys_literal_sync(pane_id, value)
    }

    fn send_key_keysym(&self, pane_id: &TmuxPaneId, value: &str) -> Result<(), CcbdError> {
        self.tmux.send_keys_keysym_sync(pane_id, value)
    }
}

pub struct RunnerContext<'a> {
    pub agent_id: &'a str,
    pub pane_id: &'a TmuxPaneId,
    pub provider: &'a str,
    pub tmux_session_handle: &'a dyn PromptIo,
    pub kb_handle: &'a PromptKb,
    pub marker_matcher: Option<&'a MarkerMatcher>,
    pub action_settle_delay: Duration,
}

impl<'a> RunnerContext<'a> {
    pub fn new(
        agent_id: &'a str,
        pane_id: &'a TmuxPaneId,
        provider: &'a str,
        tmux_session_handle: &'a dyn PromptIo,
        kb_handle: &'a PromptKb,
    ) -> Self {
        Self {
            agent_id,
            pane_id,
            provider,
            tmux_session_handle,
            kb_handle,
            marker_matcher: None,
            action_settle_delay: DEFAULT_ACTION_SETTLE_DELAY,
        }
    }

    pub fn with_marker_matcher(mut self, marker_matcher: &'a MarkerMatcher) -> Self {
        self.marker_matcher = Some(marker_matcher);
        self
    }

    pub fn with_action_settle_delay(mut self, delay: Duration) -> Self {
        self.action_settle_delay = delay;
        self
    }
}

#[derive(Debug)]
pub enum PromptRunOutcome {
    NoActionNeeded {
        depth: usize,
    },
    Pending {
        snapshot: PromptSnapshot,
        depth: usize,
    },
    DepthExceeded {
        snapshot: PromptSnapshot,
        depth: usize,
    },
    ExecutorFailed {
        error: CcbdError,
        depth: usize,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptSnapshot {
    pub sanitized_hash: [u8; 32],
    pub sanitized_text: String,
}

pub fn handle_prompt_chain(ctx: RunnerContext<'_>, max_depth: usize) -> PromptRunOutcome {
    let mut depth = 0usize;
    let mut previous_hash: Option<[u8; 32]> = None;

    loop {
        tracing::info!(
            agent_id = ctx.agent_id,
            pane_id = ctx.pane_id.0,
            depth,
            max_depth,
            "prompt runner capturing pane"
        );
        let capture = match ctx.tmux_session_handle.capture_pane(ctx.pane_id) {
            Ok(capture) => capture,
            Err(error) => {
                tracing::error!(
                    agent_id = ctx.agent_id,
                    depth,
                    reason = %error,
                    impact = "prompt-handler cannot classify pane; caller should keep existing fallback",
                    "prompt runner capture failed"
                );
                return PromptRunOutcome::ExecutorFailed { error, depth };
            }
        };

        let decision = classify_capture(
            GateContext {
                provider: ctx.provider,
                kb: ctx.kb_handle,
                marker_matcher: ctx.marker_matcher,
            },
            previous_hash,
            &capture,
        );

        match decision {
            PromptGateDecision::Skip {
                sanitized_hash: _,
                reason,
            } => {
                tracing::info!(
                    agent_id = ctx.agent_id,
                    depth,
                    ?reason,
                    "prompt runner finished without action"
                );
                return PromptRunOutcome::NoActionNeeded { depth };
            }
            PromptGateDecision::KnownAction {
                case_id,
                actions,
                sanitized_hash,
            } => {
                if depth >= max_depth {
                    tracing::warn!(
                        agent_id = ctx.agent_id,
                        depth,
                        max_depth,
                        case_id,
                        reason = "max prompt recursion depth reached",
                        impact = "caller should move agent to PROMPT_PENDING for manual resolution",
                        "prompt runner depth exceeded"
                    );
                    return PromptRunOutcome::DepthExceeded {
                        snapshot: PromptSnapshot {
                            sanitized_hash,
                            sanitized_text: sanitize_pane_text(&capture),
                        },
                        depth,
                    };
                }

                tracing::info!(
                    agent_id = ctx.agent_id,
                    depth,
                    case_id,
                    action_count = actions.len(),
                    "prompt runner executing known prompt actions"
                );
                if let Err(error) = execute_actions(&ctx, &case_id, &actions, depth) {
                    return PromptRunOutcome::ExecutorFailed { error, depth };
                }
                depth += 1;
                previous_hash = Some(sanitized_hash);

                // TODO(Phase 2/3): replace this fixed settle delay with capture-hash
                // stability detection so slow terminal refreshes do not look unchanged.
                if !ctx.action_settle_delay.is_zero() {
                    std::thread::sleep(ctx.action_settle_delay);
                }
            }
            PromptGateDecision::Unknown {
                sanitized_hash,
                sanitized_text,
            } => {
                tracing::info!(
                    agent_id = ctx.agent_id,
                    depth,
                    impact =
                        "caller should move agent to PROMPT_PENDING and emit unknown prompt event",
                    "prompt runner found unknown prompt"
                );
                return PromptRunOutcome::Pending {
                    snapshot: PromptSnapshot {
                        sanitized_hash,
                        sanitized_text,
                    },
                    depth,
                };
            }
        }
    }
}

fn execute_actions(
    ctx: &RunnerContext<'_>,
    case_id: &str,
    actions: &[PromptAction],
    depth: usize,
) -> Result<(), CcbdError> {
    for action in actions {
        match action.validate() {
            Ok(ValidatedAction::Key(keysym)) => {
                tracing::info!(
                    agent_id = ctx.agent_id,
                    depth,
                    case_id,
                    keysym,
                    "prompt runner sending keysym"
                );
                ctx.tmux_session_handle
                    .send_key_keysym(ctx.pane_id, &keysym)
                    .map_err(|error| log_action_error(ctx, depth, case_id, error))?;
            }
            Ok(ValidatedAction::Literal(literal)) => {
                tracing::info!(
                    agent_id = ctx.agent_id,
                    depth,
                    case_id,
                    literal,
                    "prompt runner sending literal"
                );
                ctx.tmux_session_handle
                    .send_key_literal(ctx.pane_id, &literal)
                    .map_err(|error| log_action_error(ctx, depth, case_id, error))?;
            }
            Err(error) => {
                tracing::warn!(
                    agent_id = ctx.agent_id,
                    depth,
                    case_id,
                    reason = %error,
                    impact = "invalid prompt action rejected before terminal input",
                    "prompt runner rejected prompt action"
                );
                return Err(CcbdError::from(error));
            }
        }
    }
    Ok(())
}

fn log_action_error(
    ctx: &RunnerContext<'_>,
    depth: usize,
    case_id: &str,
    error: CcbdError,
) -> CcbdError {
    tracing::error!(
        agent_id = ctx.agent_id,
        depth,
        case_id,
        reason = %error,
        impact = "prompt-handler stopped before continuing recursive prompt handling",
        "prompt runner action execution failed"
    );
    error
}

#[cfg(test)]
mod tests {
    use super::{PromptIo, PromptRunOutcome, RunnerContext, handle_prompt_chain};
    use crate::error::CcbdError;
    use crate::marker::MarkerMatcher;
    use crate::prompt_handler::schema::{PromptAction, PromptCase, PromptFingerprint, PromptKb};
    use crate::prompt_handler::seeds::default_cases;
    use crate::tmux::TmuxPaneId;
    use std::sync::Mutex;
    use std::time::Duration;

    #[derive(Default)]
    struct FakePromptIo {
        captures: Mutex<Vec<String>>,
        sent: Mutex<Vec<String>>,
        fail_send: bool,
    }

    impl FakePromptIo {
        fn new(captures: &[&str]) -> Self {
            Self {
                captures: Mutex::new(captures.iter().map(|value| value.to_string()).collect()),
                sent: Mutex::new(Vec::new()),
                fail_send: false,
            }
        }

        fn with_send_failure(captures: &[&str]) -> Self {
            Self {
                fail_send: true,
                ..Self::new(captures)
            }
        }

        fn sent(&self) -> Vec<String> {
            self.sent.lock().unwrap().clone()
        }
    }

    impl PromptIo for FakePromptIo {
        fn capture_pane(&self, _pane_id: &TmuxPaneId) -> Result<String, CcbdError> {
            if self.captures.lock().unwrap().is_empty() {
                return Err(CcbdError::TmuxCommandFailed {
                    cmd: "fake capture".into(),
                    stderr: "no scripted capture".into(),
                    exit: 1,
                });
            }
            Ok(self.captures.lock().unwrap().remove(0))
        }

        fn send_key_literal(&self, _pane_id: &TmuxPaneId, value: &str) -> Result<(), CcbdError> {
            if self.fail_send {
                return Err(CcbdError::TmuxCommandFailed {
                    cmd: "fake send literal".into(),
                    stderr: "scripted send failure".into(),
                    exit: 1,
                });
            }
            self.sent.lock().unwrap().push(format!("literal:{value}"));
            Ok(())
        }

        fn send_key_keysym(&self, _pane_id: &TmuxPaneId, value: &str) -> Result<(), CcbdError> {
            if self.fail_send {
                return Err(CcbdError::TmuxCommandFailed {
                    cmd: "fake send keysym".into(),
                    stderr: "scripted send failure".into(),
                    exit: 1,
                });
            }
            self.sent.lock().unwrap().push(format!("key:{value}"));
            Ok(())
        }
    }

    fn pane() -> TmuxPaneId {
        TmuxPaneId("%1".to_string())
    }

    fn ctx<'a>(
        io: &'a dyn PromptIo,
        pane: &'a TmuxPaneId,
        kb: &'a PromptKb,
        marker: &'a MarkerMatcher,
    ) -> RunnerContext<'a> {
        RunnerContext::new("ag_test", pane, "codex", io, kb)
            .with_marker_matcher(marker)
            .with_action_settle_delay(Duration::ZERO)
    }

    #[test]
    fn single_layer_known_prompt_then_idle_returns_no_action_needed() {
        let kb = PromptKb::new(default_cases());
        let pane = pane();
        let marker = MarkerMatcher::default();
        let io = FakePromptIo::new(&[
            "Update available! Run `npm install -g @openai/codex`",
            "ready\n$ ",
        ]);

        let outcome = handle_prompt_chain(ctx(&io, &pane, &kb, &marker), 3);

        assert!(matches!(
            outcome,
            PromptRunOutcome::NoActionNeeded { depth: 1 }
        ));
        assert_eq!(io.sent(), ["key:2", "key:Enter"]);
    }

    #[test]
    fn two_layer_prompt_chain_executes_both_known_cases() {
        let kb = PromptKb::new(default_cases());
        let pane = pane();
        let marker = MarkerMatcher::default();
        let io = FakePromptIo::new(&[
            "Do you trust this directory?\n1) Yes\n2) No",
            "Update available! Run `npm install -g @openai/codex`",
            "ready\n$ ",
        ]);

        let outcome = handle_prompt_chain(ctx(&io, &pane, &kb, &marker), 3);

        assert!(matches!(
            outcome,
            PromptRunOutcome::NoActionNeeded { depth: 2 }
        ));
        assert_eq!(io.sent(), ["key:1", "key:Enter", "key:2", "key:Enter"]);
    }

    #[test]
    fn fourth_known_prompt_exceeds_depth_three() {
        let kb = PromptKb::new(default_cases());
        let pane = pane();
        let marker = MarkerMatcher::default();
        let io = FakePromptIo::new(&[
            "Do you trust this directory?\n1) Yes\n2) No",
            "Update available! Run `npm install -g @openai/codex`",
            "Do you trust this workspace?\n1) Yes\n2) No",
            "Update available! Run `npm install -g @openai/codex`",
        ]);

        let outcome = handle_prompt_chain(ctx(&io, &pane, &kb, &marker), 3);

        assert!(matches!(
            outcome,
            PromptRunOutcome::DepthExceeded { depth: 3, .. }
        ));
        assert_eq!(
            io.sent(),
            [
                "key:1",
                "key:Enter",
                "key:2",
                "key:Enter",
                "key:1",
                "key:Enter"
            ]
        );
    }

    #[test]
    fn unknown_prompt_returns_pending_without_actions() {
        let kb = PromptKb::new(default_cases());
        let pane = pane();
        let marker = MarkerMatcher::default();
        let io = FakePromptIo::new(&["A new prompt with no known answer"]);

        let outcome = handle_prompt_chain(ctx(&io, &pane, &kb, &marker), 3);

        match outcome {
            PromptRunOutcome::Pending { snapshot, depth } => {
                assert_eq!(depth, 0);
                assert!(snapshot.sanitized_text.contains("new prompt"));
            }
            other => panic!("expected pending outcome, got {other:?}"),
        }
        assert!(io.sent().is_empty());
    }

    #[test]
    fn invalid_action_is_rejected_without_terminal_input() {
        let kb = PromptKb::new(vec![PromptCase {
            id: "bad_action".into(),
            provider: Some("codex".into()),
            fingerprint: PromptFingerprint::Regex {
                pattern: "Unsafe approval".into(),
            },
            action: vec![PromptAction::Literal {
                value: "yes && rm -rf /".into(),
            }],
            category: "auto-accept".into(),
            description: None,
            confidence_threshold: None,
            used_count: 0,
            created_at: None,
            last_used_at: None,
            created_by: Some("test".into()),
            regex_flags: Vec::new(),
            trigger_state: None,
        }]);
        let pane = pane();
        let marker = MarkerMatcher::default();
        let io = FakePromptIo::new(&["Unsafe approval"]);

        let outcome = handle_prompt_chain(ctx(&io, &pane, &kb, &marker), 3);

        assert!(matches!(
            outcome,
            PromptRunOutcome::ExecutorFailed { depth: 0, .. }
        ));
        assert!(io.sent().is_empty());
    }

    #[test]
    fn send_failure_returns_executor_failed() {
        let kb = PromptKb::new(default_cases());
        let pane = pane();
        let marker = MarkerMatcher::default();
        let io = FakePromptIo::with_send_failure(&[
            "Update available! Run `npm install -g @openai/codex`",
        ]);

        let outcome = handle_prompt_chain(ctx(&io, &pane, &kb, &marker), 3);

        assert!(matches!(
            outcome,
            PromptRunOutcome::ExecutorFailed { depth: 0, .. }
        ));
    }
}
