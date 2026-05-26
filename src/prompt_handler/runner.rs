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
    pub current_state: &'a str,
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
            current_state: crate::db::state_machine::STATE_BUSY,
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

    pub fn with_current_state(mut self, current_state: &'a str) -> Self {
        self.current_state = current_state;
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
    let mut same_hash_skips = 0usize;

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
                current_state: ctx.current_state,
                kb: ctx.kb_handle,
                marker_matcher: ctx.marker_matcher,
            },
            previous_hash,
            &capture,
        );

        match decision {
            PromptGateDecision::Skip {
                sanitized_hash,
                reason,
            } => {
                if reason == crate::prompt_handler::gating::GateSkipReason::SameHash {
                    same_hash_skips += 1;
                    if same_hash_skips <= max_depth {
                        tracing::info!(
                            agent_id = ctx.agent_id,
                            depth,
                            same_hash_skips,
                            "prompt runner waiting for post-action screen refresh"
                        );
                        if !ctx.action_settle_delay.is_zero() {
                            std::thread::sleep(ctx.action_settle_delay);
                        }
                        continue;
                    }
                    tracing::warn!(
                        agent_id = ctx.agent_id,
                        depth,
                        same_hash_skips,
                        max_depth,
                        reason = "same sanitized capture hash persisted after action",
                        impact = "caller should move agent to PROMPT_PENDING for manual resolution",
                        "prompt runner same-hash stuck"
                    );
                    return PromptRunOutcome::Pending {
                        snapshot: PromptSnapshot {
                            sanitized_hash,
                            sanitized_text: sanitize_pane_text(&capture),
                        },
                        depth,
                    };
                }

                if reason == crate::prompt_handler::gating::GateSkipReason::IdleMarker {
                    match confirm_can_input(&ctx, &capture, depth) {
                        CanInputProbe::Confirmed => {
                            tracing::info!(
                                agent_id = ctx.agent_id,
                                depth,
                                "prompt runner confirmed provider can accept input"
                            );
                            return PromptRunOutcome::NoActionNeeded { depth };
                        }
                        CanInputProbe::NotCandidate => {}
                        CanInputProbe::Failed(error) => {
                            return PromptRunOutcome::ExecutorFailed { error, depth };
                        }
                    }
                }

                tracing::info!(
                    agent_id = ctx.agent_id,
                    depth,
                    ?reason,
                    "prompt runner finished without action"
                );
                if reason == crate::prompt_handler::gating::GateSkipReason::IdleMarker
                    && crate::prompt_handler::integration::is_prompt_handling_provider(ctx.provider)
                {
                    return PromptRunOutcome::Pending {
                        snapshot: PromptSnapshot {
                            sanitized_hash,
                            sanitized_text: sanitize_pane_text(&capture),
                        },
                        depth,
                    };
                }
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
                same_hash_skips = 0;

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

enum CanInputProbe {
    Confirmed,
    NotCandidate,
    Failed(CcbdError),
}

fn confirm_can_input(ctx: &RunnerContext<'_>, capture: &str, depth: usize) -> CanInputProbe {
    let probe = input_probe_literal(ctx.provider);
    if probe.is_empty() || !is_input_candidate(ctx.provider, capture) {
        return CanInputProbe::NotCandidate;
    }

    if let Err(error) = ctx
        .tmux_session_handle
        .send_key_literal(ctx.pane_id, probe)
        .map_err(|error| log_probe_error(ctx, depth, "send probe literal", error))
    {
        return CanInputProbe::Failed(error);
    }
    settle_after_probe_action(ctx);

    let echoed = match ctx.tmux_session_handle.capture_pane(ctx.pane_id) {
        Ok(capture) => capture,
        Err(error) => {
            return CanInputProbe::Failed(log_probe_error(ctx, depth, "capture probe echo", error));
        }
    };
    if !probe_echoed(ctx.provider, &echoed, probe) {
        if let Err(error) = ctx
            .tmux_session_handle
            .send_key_keysym(ctx.pane_id, "BSpace")
            .map_err(|error| log_probe_error(ctx, depth, "cleanup unconfirmed probe", error))
        {
            return CanInputProbe::Failed(error);
        }
        settle_after_probe_action(ctx);
        return CanInputProbe::NotCandidate;
    }

    if let Err(error) = ctx
        .tmux_session_handle
        .send_key_keysym(ctx.pane_id, "BSpace")
        .map_err(|error| log_probe_error(ctx, depth, "cleanup probe literal", error))
    {
        return CanInputProbe::Failed(error);
    }
    settle_after_probe_action(ctx);

    let cleaned = match ctx.tmux_session_handle.capture_pane(ctx.pane_id) {
        Ok(capture) => capture,
        Err(error) => {
            return CanInputProbe::Failed(log_probe_error(
                ctx,
                depth,
                "capture probe cleanup",
                error,
            ));
        }
    };
    if probe_echoed(ctx.provider, &cleaned, probe) {
        return CanInputProbe::NotCandidate;
    }

    CanInputProbe::Confirmed
}

fn settle_after_probe_action(ctx: &RunnerContext<'_>) {
    if !ctx.action_settle_delay.is_zero() {
        std::thread::sleep(ctx.action_settle_delay);
    }
}

fn input_probe_literal(provider: &str) -> &'static str {
    match provider {
        "codex" | "gemini" | "claude" => "x",
        _ => "",
    }
}

fn is_input_candidate(provider: &str, capture: &str) -> bool {
    match provider {
        "codex" => capture
            .lines()
            .any(|line| line.trim_start().starts_with('›')),
        "gemini" => capture.lines().any(is_gemini_input_placeholder_line),
        "claude" => {
            capture_contains_claude_model_marker(capture)
                && capture.lines().any(is_claude_empty_input_line)
        }
        _ => false,
    }
}

fn probe_echoed(provider: &str, capture: &str, probe: &str) -> bool {
    match provider {
        "codex" => capture
            .lines()
            .any(|line| line.trim_start().starts_with(&format!("› {probe}"))),
        "gemini" => capture
            .lines()
            .any(|line| gemini_input_line_payload(line).is_some_and(|payload| payload == probe)),
        "claude" => capture
            .lines()
            .any(|line| claude_input_line_payload(line).is_some_and(|payload| payload == probe)),
        _ => false,
    }
}

fn is_gemini_input_placeholder_line(line: &str) -> bool {
    let trimmed = line.trim_start();
    let Some(payload) = trimmed
        .strip_prefix('*')
        .or_else(|| trimmed.strip_prefix('>'))
    else {
        return false;
    };
    payload
        .trim_start()
        .starts_with("Type your message or @path/to/file")
}

fn gemini_input_line_payload(line: &str) -> Option<&str> {
    let trimmed = line.trim_start();
    let payload = trimmed
        .strip_prefix('*')
        .or_else(|| trimmed.strip_prefix('>'))?
        .trim();
    if payload.starts_with("Type your message or @path/to/file") {
        return None;
    }
    (!payload.is_empty()).then_some(payload)
}

fn capture_contains_claude_model_marker(capture: &str) -> bool {
    capture
        .split(|ch: char| !ch.is_alphanumeric())
        .any(|token| matches!(token, "Sonnet" | "Opus" | "Haiku"))
}

fn is_claude_empty_input_line(line: &str) -> bool {
    claude_input_line_payload(line).is_none() && line.trim_start().starts_with('❯')
}

fn claude_input_line_payload(line: &str) -> Option<&str> {
    let payload = line.trim_start().strip_prefix('❯')?.trim();
    (!payload.is_empty()).then_some(payload)
}

fn log_probe_error(
    ctx: &RunnerContext<'_>,
    depth: usize,
    step: &'static str,
    error: CcbdError,
) -> CcbdError {
    tracing::error!(
        agent_id = ctx.agent_id,
        depth,
        step,
        reason = %error,
        impact = "can-input confirmation failed before marking provider ready",
        "prompt runner can-input probe failed"
    );
    error
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
    use crate::provider::manifest::get_manifest;
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
        let marker = MarkerMatcher::from_manifest(&get_manifest("codex"));
        let io = FakePromptIo::new(&[
            "Update available! Run `npm install -g @openai/codex`",
            "ready\n  › ",
            "ready\n  › x",
            "ready\n  › ",
        ]);

        let outcome = handle_prompt_chain(ctx(&io, &pane, &kb, &marker), 3);

        assert!(matches!(
            outcome,
            PromptRunOutcome::NoActionNeeded { depth: 1 }
        ));
        assert_eq!(io.sent(), ["key:2", "key:Enter", "literal:x", "key:BSpace"]);
    }

    #[test]
    fn two_layer_prompt_chain_executes_both_known_cases() {
        let kb = PromptKb::new(default_cases());
        let pane = pane();
        let marker = MarkerMatcher::from_manifest(&get_manifest("codex"));
        let io = FakePromptIo::new(&[
            "Do you trust this directory?\n1) Yes\n2) No",
            "Update available! Run `npm install -g @openai/codex`",
            "ready\n  › ",
            "ready\n  › x",
            "ready\n  › ",
        ]);

        let outcome = handle_prompt_chain(ctx(&io, &pane, &kb, &marker), 3);

        assert!(matches!(
            outcome,
            PromptRunOutcome::NoActionNeeded { depth: 2 }
        ));
        assert_eq!(
            io.sent(),
            [
                "key:1",
                "key:Enter",
                "key:2",
                "key:Enter",
                "literal:x",
                "key:BSpace"
            ]
        );
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
    fn same_hash_stuck_after_action_returns_pending() {
        let kb = PromptKb::new(default_cases());
        let pane = pane();
        let marker = MarkerMatcher::default();
        let io = FakePromptIo::new(&[
            "Update available! Run `npm install -g @openai/codex`",
            "Update available! Run `npm install -g @openai/codex`",
            "Update available! Run `npm install -g @openai/codex`",
        ]);

        let outcome = handle_prompt_chain(ctx(&io, &pane, &kb, &marker), 1);

        assert!(matches!(
            outcome,
            PromptRunOutcome::Pending { depth: 1, .. }
        ));
        assert_eq!(io.sent(), ["key:2", "key:Enter"]);
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
