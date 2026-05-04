//! Startup sequence runner for provider TUI bootstrap interactions.

use crate::db::{self, Db};
use crate::error::CcbdError;
use crate::provider::manifest::{PromptHandler, ProviderManifest, StartupStep};
use crate::tmux::{TmuxPaneId, TmuxServer};
use regex::Regex;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};

const VERIFY_POLL_INTERVAL: Duration = Duration::from_millis(200);

type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

trait StartupPane: Send + Sync {
    fn capture_pane<'a>(&'a self, pane: TmuxPaneId) -> BoxFuture<'a, Result<String, CcbdError>>;
    fn send_keys<'a>(
        &'a self,
        pane: TmuxPaneId,
        keys: String,
    ) -> BoxFuture<'a, Result<(), CcbdError>>;
}

impl StartupPane for TmuxServer {
    fn capture_pane<'a>(&'a self, pane: TmuxPaneId) -> BoxFuture<'a, Result<String, CcbdError>> {
        Box::pin(async move { self.capture_pane(pane).await })
    }

    fn send_keys<'a>(
        &'a self,
        pane: TmuxPaneId,
        keys: String,
    ) -> BoxFuture<'a, Result<(), CcbdError>> {
        Box::pin(async move { self.send_keys_keysym(pane, keys).await })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StartupFailureReason {
    StartupTimeout,
    StartupVerifyFailed,
    ClearLineVerifyFailed,
    PromptFlood,
}

impl StartupFailureReason {
    fn as_str(self) -> &'static str {
        match self {
            Self::StartupTimeout => "STARTUP_TIMEOUT",
            Self::StartupVerifyFailed => "STARTUP_VERIFY_FAILED",
            Self::ClearLineVerifyFailed => "CLEAR_LINE_VERIFY_FAILED",
            Self::PromptFlood => "PROMPT_FLOOD",
        }
    }
}

struct PromptInterceptor {
    handlers: Vec<CompiledPromptHandler>,
    counts: HashMap<String, u32>,
}

struct CompiledPromptHandler {
    pattern: &'static str,
    regex: Regex,
    response_keys: &'static str,
    max_triggers: u32,
}

impl PromptInterceptor {
    fn new(handlers: &[PromptHandler]) -> Result<Self, StartupFailureReason> {
        let handlers = handlers
            .iter()
            .map(|handler| {
                Regex::new(handler.pattern)
                    .map(|regex| CompiledPromptHandler {
                        pattern: handler.pattern,
                        regex,
                        response_keys: handler.response_keys,
                        max_triggers: handler.max_triggers,
                    })
                    .map_err(|_| StartupFailureReason::StartupVerifyFailed)
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            handlers,
            counts: HashMap::new(),
        })
    }

    async fn handle<T: StartupPane>(
        &mut self,
        tmux: &T,
        pane: &TmuxPaneId,
        snapshot: &str,
    ) -> Result<bool, StartupFailureReason> {
        let mut handled = false;
        for handler in &self.handlers {
            if handler.regex.is_match(snapshot) {
                let count = self.counts.entry(handler.pattern.to_string()).or_default();
                *count += 1;
                if *count > handler.max_triggers {
                    return Err(StartupFailureReason::PromptFlood);
                }
                tmux.send_keys(pane.clone(), handler.response_keys.to_string())
                    .await
                    .map_err(|_| StartupFailureReason::StartupVerifyFailed)?;
                handled = true;
            }
        }
        Ok(handled)
    }
}

pub async fn run_startup_sequence(
    agent_id: String,
    tmux: Arc<TmuxServer>,
    pane_id: String,
    manifest: Arc<ProviderManifest>,
    db: Arc<Db>,
) -> Result<(), CcbdError> {
    let pane = TmuxPaneId::parse(&pane_id).map_err(|err| CcbdError::PtyIoError(err.to_string()))?;
    run_startup_sequence_for_pane(agent_id, tmux.as_ref(), pane, manifest, db).await
}

async fn run_startup_sequence_for_pane<T: StartupPane>(
    agent_id: String,
    tmux: &T,
    pane: TmuxPaneId,
    manifest: Arc<ProviderManifest>,
    db: Arc<Db>,
) -> Result<(), CcbdError> {
    let timeout_duration = Duration::from_secs(manifest.readiness_timeout_s.into());
    let result = tokio::time::timeout(
        timeout_duration,
        inner_run_startup_sequence(tmux, pane, manifest),
    )
    .await;
    let failure = match result {
        Ok(Ok(())) => return Ok(()),
        Ok(Err(reason)) => reason,
        Err(_) => StartupFailureReason::StartupTimeout,
    };
    mark_startup_crashed(db, agent_id, failure.as_str()).await?;
    Err(CcbdError::PtyIoError(failure.as_str().to_string()))
}

async fn inner_run_startup_sequence<T: StartupPane>(
    tmux: &T,
    pane: TmuxPaneId,
    manifest: Arc<ProviderManifest>,
) -> Result<(), StartupFailureReason> {
    let mut interceptor = PromptInterceptor::new(manifest.interactive_prompt_handlers)?;
    for step in manifest.startup_sequence {
        match step {
            StartupStep::WaitMs(ms) => tokio::time::sleep(Duration::from_millis(*ms)).await,
            StartupStep::SendKeysVerified {
                keys,
                verify_pattern,
                verify_timeout_ms,
                retry_fallback_keys,
            } => {
                send_keys_verified(
                    tmux,
                    &pane,
                    keys,
                    *verify_pattern,
                    Duration::from_millis(*verify_timeout_ms),
                    *retry_fallback_keys,
                    &mut interceptor,
                )
                .await?;
            }
            StartupStep::ClearLine { expected_after } => {
                clear_line(tmux, &pane, *expected_after, &mut interceptor).await?;
            }
        }
    }
    Ok(())
}

async fn send_keys_verified<T: StartupPane>(
    tmux: &T,
    pane: &TmuxPaneId,
    keys: &str,
    verify_pattern: Option<&str>,
    verify_timeout: Duration,
    retry_fallback_keys: Option<&[&str]>,
    interceptor: &mut PromptInterceptor,
) -> Result<(), StartupFailureReason> {
    let verify_regex = compile_optional_regex(verify_pattern)?;
    if try_keys_until_verified(
        tmux,
        pane,
        keys,
        verify_timeout,
        verify_regex.as_ref(),
        interceptor,
    )
    .await?
    {
        return Ok(());
    }
    let Some(fallback_keys) = retry_fallback_keys else {
        return Err(StartupFailureReason::StartupVerifyFailed);
    };
    let fallback_timeout = (verify_timeout / 3).max(VERIFY_POLL_INTERVAL);
    for fallback in fallback_keys {
        if try_keys_until_verified(
            tmux,
            pane,
            fallback,
            fallback_timeout,
            verify_regex.as_ref(),
            interceptor,
        )
        .await?
        {
            return Ok(());
        }
    }
    Err(StartupFailureReason::StartupVerifyFailed)
}

async fn try_keys_until_verified<T: StartupPane>(
    tmux: &T,
    pane: &TmuxPaneId,
    keys: &str,
    verify_timeout: Duration,
    verify_regex: Option<&Regex>,
    interceptor: &mut PromptInterceptor,
) -> Result<bool, StartupFailureReason> {
    let baseline = tmux
        .capture_pane(pane.clone())
        .await
        .map_err(|_| StartupFailureReason::StartupVerifyFailed)?;
    tmux.send_keys(pane.clone(), keys.to_string())
        .await
        .map_err(|_| StartupFailureReason::StartupVerifyFailed)?;
    let deadline = Instant::now() + verify_timeout;
    loop {
        if Instant::now() >= deadline {
            return Ok(false);
        }
        tokio::time::sleep(VERIFY_POLL_INTERVAL).await;
        let snapshot = tmux
            .capture_pane(pane.clone())
            .await
            .map_err(|_| StartupFailureReason::StartupVerifyFailed)?;
        if interceptor.handle(tmux, pane, &snapshot).await? {
            continue;
        }
        if snapshot != baseline || verify_regex.is_some_and(|regex| regex.is_match(&snapshot)) {
            return Ok(true);
        }
    }
}

async fn clear_line<T: StartupPane>(
    tmux: &T,
    pane: &TmuxPaneId,
    expected_after: Option<&str>,
    interceptor: &mut PromptInterceptor,
) -> Result<(), StartupFailureReason> {
    tmux.send_keys(pane.clone(), "Escape".to_string())
        .await
        .map_err(|_| StartupFailureReason::ClearLineVerifyFailed)?;
    tokio::time::sleep(Duration::from_millis(50)).await;
    tmux.send_keys(pane.clone(), "C-u".to_string())
        .await
        .map_err(|_| StartupFailureReason::ClearLineVerifyFailed)?;
    tokio::time::sleep(Duration::from_millis(100)).await;

    let Some(expected_after) = expected_after else {
        return Ok(());
    };
    let expected =
        Regex::new(expected_after).map_err(|_| StartupFailureReason::ClearLineVerifyFailed)?;
    for _ in 0..3 {
        let snapshot = tmux
            .capture_pane(pane.clone())
            .await
            .map_err(|_| StartupFailureReason::ClearLineVerifyFailed)?;
        let _ = interceptor.handle(tmux, pane, &snapshot).await?;
        if expected.is_match(&tail_lines(&snapshot, 2)) {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Err(StartupFailureReason::ClearLineVerifyFailed)
}

fn compile_optional_regex(pattern: Option<&str>) -> Result<Option<Regex>, StartupFailureReason> {
    pattern
        .map(Regex::new)
        .transpose()
        .map_err(|_| StartupFailureReason::StartupVerifyFailed)
}

fn tail_lines(text: &str, count: usize) -> String {
    let mut lines = text.lines().rev().take(count).collect::<Vec<_>>();
    lines.reverse();
    lines.join("\n")
}

async fn mark_startup_crashed(
    db: Arc<Db>,
    agent_id: String,
    reason: &str,
) -> Result<(), CcbdError> {
    db::agents_lifecycle::mark_agent_crashed_with_reason(
        db.as_ref().clone(),
        agent_id,
        None,
        reason.to_string(),
    )
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{StartupPane, run_startup_sequence_for_pane};
    use crate::db;
    use crate::db::agents::insert_agent_sync;
    use crate::db::sessions::insert_session_sync;
    use crate::error::CcbdError;
    use crate::provider::manifest::{
        IdleDetectionMode, PromptHandler, ProviderManifest, StartupStep,
    };
    use crate::tmux::TmuxPaneId;
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    #[derive(Default)]
    struct MockTmux {
        captures: Mutex<VecDeque<String>>,
        last_capture: Mutex<String>,
        sent: Mutex<Vec<String>>,
        on_send_captures: Mutex<HashMap<String, VecDeque<String>>>,
    }

    use std::collections::{HashMap, VecDeque};

    impl MockTmux {
        fn with_captures(captures: &[&str]) -> Self {
            Self {
                captures: Mutex::new(captures.iter().map(|value| (*value).to_string()).collect()),
                last_capture: Mutex::new(String::new()),
                sent: Mutex::new(Vec::new()),
                on_send_captures: Mutex::new(HashMap::new()),
            }
        }

        fn push_on_send(&self, keys: &str, captures: &[&str]) {
            self.on_send_captures.lock().unwrap().insert(
                keys.to_string(),
                captures.iter().map(|value| (*value).to_string()).collect(),
            );
        }

        fn sent_keys(&self) -> Vec<String> {
            self.sent.lock().unwrap().clone()
        }
    }

    impl StartupPane for MockTmux {
        fn capture_pane<'a>(
            &'a self,
            _pane: TmuxPaneId,
        ) -> Pin<Box<dyn Future<Output = Result<String, CcbdError>> + Send + 'a>> {
            Box::pin(async move {
                let mut captures = self.captures.lock().unwrap();
                if let Some(snapshot) = captures.pop_front() {
                    *self.last_capture.lock().unwrap() = snapshot.clone();
                    Ok(snapshot)
                } else {
                    Ok(self.last_capture.lock().unwrap().clone())
                }
            })
        }

        fn send_keys<'a>(
            &'a self,
            _pane: TmuxPaneId,
            keys: String,
        ) -> Pin<Box<dyn Future<Output = Result<(), CcbdError>> + Send + 'a>> {
            Box::pin(async move {
                self.sent.lock().unwrap().push(keys.clone());
                if let Some(mut captures) = self.on_send_captures.lock().unwrap().remove(&keys) {
                    self.captures.lock().unwrap().append(&mut captures);
                }
                Ok(())
            })
        }
    }

    fn manifest(
        readiness_timeout_s: u32,
        startup_sequence: &'static [StartupStep],
        handlers: &'static [PromptHandler],
    ) -> Arc<ProviderManifest> {
        Arc::new(ProviderManifest {
            provider_name: "test",
            auth_mount_paths: vec![],
            command: &["test"],
            env_passthrough: &[],
            injected_env_vars: &[],
            readiness_timeout_s,
            startup_sequence,
            interactive_prompt_handlers: handlers,
            idle_detection_mode: IdleDetectionMode::LineEndRegex,
            marker_pattern: r"$",
            stability_ms: 0,
        })
    }

    fn test_db(agent_id: &str) -> db::Db {
        let file = tempfile::NamedTempFile::new().unwrap();
        let db = db::init(file.path()).unwrap();
        {
            let conn = db.conn();
            insert_session_sync(&conn, "s1", "p1", "/tmp/startup").unwrap();
            insert_agent_sync(&conn, agent_id, "s1", "codex", "SPAWNING", Some(123)).unwrap();
        }
        db
    }

    async fn run(
        tmux: &MockTmux,
        manifest: Arc<ProviderManifest>,
        agent_id: &str,
    ) -> Result<(), CcbdError> {
        run_startup_sequence_for_pane(
            agent_id.to_string(),
            tmux,
            TmuxPaneId("%1".into()),
            manifest,
            Arc::new(test_db(agent_id)),
        )
        .await
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_send_keys_verified_succeeds_on_content_change() {
        static STEPS: &[StartupStep] = &[StartupStep::SendKeysVerified {
            keys: "Enter",
            verify_pattern: None,
            verify_timeout_ms: 500,
            retry_fallback_keys: None,
        }];
        let tmux = MockTmux::with_captures(&["before", "after"]);

        run(&tmux, manifest(1, STEPS, &[]), "a_change")
            .await
            .unwrap();

        assert_eq!(tmux.sent_keys(), ["Enter"]);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_send_keys_verified_succeeds_on_verify_pattern() {
        static STEPS: &[StartupStep] = &[StartupStep::SendKeysVerified {
            keys: "Enter",
            verify_pattern: Some("READY"),
            verify_timeout_ms: 500,
            retry_fallback_keys: None,
        }];
        let tmux = MockTmux::with_captures(&["same", "same READY"]);

        run(&tmux, manifest(1, STEPS, &[]), "a_pattern")
            .await
            .unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_send_keys_verified_fallback_recovers() {
        static FALLBACKS: &[&str] = &["Return", "C-m"];
        static STEPS: &[StartupStep] = &[StartupStep::SendKeysVerified {
            keys: "Enter",
            verify_pattern: Some("READY"),
            verify_timeout_ms: 300,
            retry_fallback_keys: Some(FALLBACKS),
        }];
        let tmux = MockTmux::with_captures(&["same", "same", "same"]);
        tmux.push_on_send("Return", &["same READY"]);

        run(&tmux, manifest(1, STEPS, &[]), "a_fallback")
            .await
            .unwrap();

        assert_eq!(tmux.sent_keys(), ["Enter", "Return"]);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_send_keys_verified_failure_marks_crashed() {
        static FALLBACKS: &[&str] = &["Return"];
        static STEPS: &[StartupStep] = &[StartupStep::SendKeysVerified {
            keys: "Enter",
            verify_pattern: Some("READY"),
            verify_timeout_ms: 220,
            retry_fallback_keys: Some(FALLBACKS),
        }];
        let db = test_db("a_fail");
        let tmux = MockTmux::with_captures(&["same"]);
        let err = run_startup_sequence_for_pane(
            "a_fail".to_string(),
            &tmux,
            TmuxPaneId("%1".into()),
            manifest(1, STEPS, &[]),
            Arc::new(db.clone()),
        )
        .await
        .unwrap_err();

        assert!(err.to_string().contains("STARTUP_VERIFY_FAILED"));
        let (state, error_code): (String, Option<String>) = db
            .conn()
            .query_row(
                "SELECT state, error_code FROM agents WHERE id='a_fail'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(state, "CRASHED");
        assert_eq!(error_code.as_deref(), Some("STARTUP_VERIFY_FAILED"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_prompt_interceptor_sends_response() {
        static HANDLERS: &[PromptHandler] = &[PromptHandler {
            pattern: "Update now",
            response_keys: "Escape",
            max_triggers: 1,
        }];
        static STEPS: &[StartupStep] = &[StartupStep::SendKeysVerified {
            keys: "Enter",
            verify_pattern: Some("READY"),
            verify_timeout_ms: 500,
            retry_fallback_keys: None,
        }];
        let tmux = MockTmux::with_captures(&["same", "Update now?", "READY"]);

        run(&tmux, manifest(1, STEPS, HANDLERS), "a_prompt")
            .await
            .unwrap();

        assert_eq!(tmux.sent_keys(), ["Enter", "Escape"]);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_prompt_flood_marks_crashed() {
        static HANDLERS: &[PromptHandler] = &[PromptHandler {
            pattern: "Update now",
            response_keys: "Escape",
            max_triggers: 1,
        }];
        static STEPS: &[StartupStep] = &[StartupStep::SendKeysVerified {
            keys: "Enter",
            verify_pattern: Some("READY"),
            verify_timeout_ms: 500,
            retry_fallback_keys: None,
        }];
        let db = test_db("a_flood");
        let tmux = MockTmux::with_captures(&["same", "Update now?", "Update now?"]);
        let err = run_startup_sequence_for_pane(
            "a_flood".to_string(),
            &tmux,
            TmuxPaneId("%1".into()),
            manifest(1, STEPS, HANDLERS),
            Arc::new(db.clone()),
        )
        .await
        .unwrap_err();

        assert!(err.to_string().contains("PROMPT_FLOOD"));
        let error_code: Option<String> = db
            .conn()
            .query_row(
                "SELECT error_code FROM agents WHERE id='a_flood'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(error_code.as_deref(), Some("PROMPT_FLOOD"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_clear_line_expected_after_ok() {
        static STEPS: &[StartupStep] = &[StartupStep::ClearLine {
            expected_after: Some("prompt>"),
        }];
        let tmux = MockTmux::with_captures(&["old\nprompt>"]);

        run(&tmux, manifest(1, STEPS, &[]), "a_clear")
            .await
            .unwrap();

        assert_eq!(tmux.sent_keys(), ["Escape", "C-u"]);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_clear_line_expected_after_failure() {
        static STEPS: &[StartupStep] = &[StartupStep::ClearLine {
            expected_after: Some("prompt>"),
        }];
        let db = test_db("a_clear_fail");
        let tmux = MockTmux::with_captures(&["old"]);
        let err = run_startup_sequence_for_pane(
            "a_clear_fail".to_string(),
            &tmux,
            TmuxPaneId("%1".into()),
            manifest(1, STEPS, &[]),
            Arc::new(db.clone()),
        )
        .await
        .unwrap_err();

        assert!(err.to_string().contains("CLEAR_LINE_VERIFY_FAILED"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_top_level_startup_timeout_marks_crashed() {
        static STEPS: &[StartupStep] = &[StartupStep::WaitMs(2_000)];
        let db = test_db("a_timeout");
        let tmux = MockTmux::with_captures(&[]);
        let started = Instant::now();
        let err = run_startup_sequence_for_pane(
            "a_timeout".to_string(),
            &tmux,
            TmuxPaneId("%1".into()),
            manifest(1, STEPS, &[]),
            Arc::new(db.clone()),
        )
        .await
        .unwrap_err();

        assert!(started.elapsed() < Duration::from_millis(1_300));
        assert!(err.to_string().contains("STARTUP_TIMEOUT"));
        let error_code: Option<String> = db
            .conn()
            .query_row(
                "SELECT error_code FROM agents WHERE id='a_timeout'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(error_code.as_deref(), Some("STARTUP_TIMEOUT"));
    }
}
