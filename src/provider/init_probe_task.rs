//! Async InitGate driver for spawned provider panes.

use crate::db::{self, Db};
use crate::error::CcbdError;
use crate::marker::MarkerMatcher;
use crate::prompt_handler::integration::{
    PromptScanDisposition, PromptScanRequest, is_prompt_handling_provider,
    scan_prompt_and_apply_outcome,
};
use crate::provider::manifest::InitProbeKind;
use crate::tmux::{TmuxPaneId, TmuxServer};
use serde_json::json;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

const POLL_FAST: Duration = Duration::from_millis(200);
const POLL_SLOW: Duration = Duration::from_millis(500);
const POLL_SWITCH: Duration = Duration::from_secs(5);
const STEADY_COUNT: u32 = 2;

pub fn spawn_init_probe_task(
    agent_id: String,
    tmux: Arc<TmuxServer>,
    pane: TmuxPaneId,
    db: Arc<Db>,
    provider: String,
    state_dir: PathBuf,
    marker_matcher: Arc<MarkerMatcher>,
    probe_kind: InitProbeKind,
    deadline: Duration,
    idle_scan_enabled: Arc<AtomicBool>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        if let Err(err) = run_init_probe_task(
            agent_id.clone(),
            tmux,
            pane,
            db,
            provider,
            state_dir,
            marker_matcher,
            probe_kind,
            deadline,
            idle_scan_enabled,
        )
        .await
        {
            tracing::warn!(agent_id = %agent_id, error = %err, "init probe task failed");
        }
    })
}

async fn run_init_probe_task(
    agent_id: String,
    tmux: Arc<TmuxServer>,
    pane: TmuxPaneId,
    db: Arc<Db>,
    provider: String,
    state_dir: PathBuf,
    marker_matcher: Arc<MarkerMatcher>,
    probe_kind: InitProbeKind,
    deadline: Duration,
    idle_scan_enabled: Arc<AtomicBool>,
) -> Result<(), CcbdError> {
    let probe = probe_kind.build();
    let start = tokio::time::Instant::now();
    let deadline_at = start + deadline;
    let mut consecutive = 0_u32;
    let mut last_capture = String::new();

    loop {
        if tokio::time::Instant::now() >= deadline_at {
            mark_unknown_after_timeout(db, agent_id, last_capture).await?;
            return Ok(());
        }

        match capture_visible(tmux.clone(), pane.clone()).await {
            Ok(capture) => {
                if capture.trim().is_empty() {
                    consecutive = 0;
                    tracing::debug!(
                        agent_id = %agent_id,
                        "init probe captured empty spawning pane; retrying"
                    );
                    sleep_init_probe_poll(start).await;
                    continue;
                }

                let detected = probe.detect(&capture);
                last_capture = capture;
                if detected {
                    if is_prompt_handling_provider(&provider) {
                        match scan_startup_prompt(
                            db.clone(),
                            agent_id.clone(),
                            provider.clone(),
                            pane.clone(),
                            tmux.clone(),
                            state_dir.clone(),
                            marker_matcher.clone(),
                        )
                        .await
                        {
                            Ok(StartupPromptScan::ReadyConfirmed) => {
                                consecutive += 1;
                                if consecutive >= STEADY_COUNT {
                                    mark_idle_after_probe(db, agent_id, idle_scan_enabled).await?;
                                    return Ok(());
                                }
                            }
                            Ok(StartupPromptScan::HandledOrClear) => {
                                consecutive = 0;
                            }
                            Ok(StartupPromptScan::Deferred) => {
                                if deferred_can_count_as_ready(probe_kind) {
                                    consecutive += 1;
                                    if consecutive >= STEADY_COUNT {
                                        mark_idle_after_probe(db, agent_id, idle_scan_enabled)
                                            .await?;
                                        return Ok(());
                                    }
                                } else {
                                    consecutive = 0;
                                }
                            }
                            Ok(StartupPromptScan::Pending) => return Ok(()),
                            Err(err) => {
                                consecutive = 0;
                                tracing::debug!(
                                    agent_id = %agent_id,
                                    error = %err,
                                    "startup readiness confirmation failed; init probe will retry until deadline"
                                );
                            }
                        }
                    } else {
                        consecutive += 1;
                        if consecutive >= STEADY_COUNT {
                            mark_idle_after_probe(db, agent_id, idle_scan_enabled).await?;
                            return Ok(());
                        }
                    }
                } else {
                    consecutive = 0;
                    match scan_startup_prompt(
                        db.clone(),
                        agent_id.clone(),
                        provider.clone(),
                        pane.clone(),
                        tmux.clone(),
                        state_dir.clone(),
                        marker_matcher.clone(),
                    )
                    .await
                    {
                        Ok(StartupPromptScan::ReadyConfirmed) => {
                            consecutive += 1;
                            if consecutive >= STEADY_COUNT {
                                mark_idle_after_probe(db, agent_id, idle_scan_enabled).await?;
                                return Ok(());
                            }
                        }
                        Ok(StartupPromptScan::HandledOrClear) => {}
                        Ok(StartupPromptScan::Deferred) => {}
                        Ok(StartupPromptScan::Pending) => return Ok(()),
                        Err(err) => {
                            tracing::debug!(
                                agent_id = %agent_id,
                                error = %err,
                                "startup prompt scan failed; init probe will retry until deadline"
                            );
                        }
                    }
                }
            }
            Err(err) => {
                tracing::debug!(agent_id = %agent_id, error = %err, "init probe capture failed");
                consecutive = 0;
            }
        }

        sleep_init_probe_poll(start).await;
    }
}

async fn sleep_init_probe_poll(start: tokio::time::Instant) {
    let elapsed = tokio::time::Instant::now().saturating_duration_since(start);
    let period = if elapsed < POLL_SWITCH {
        POLL_FAST
    } else {
        POLL_SLOW
    };
    tokio::time::sleep(period).await;
}

fn deferred_can_count_as_ready(probe_kind: InitProbeKind) -> bool {
    matches!(
        probe_kind,
        InitProbeKind::Codex | InitProbeKind::Gemini | InitProbeKind::Claude
    )
}

enum StartupPromptScan {
    ReadyConfirmed,
    HandledOrClear,
    Deferred,
    Pending,
}

async fn scan_startup_prompt(
    db: Arc<Db>,
    agent_id: String,
    provider: String,
    pane: TmuxPaneId,
    tmux: Arc<TmuxServer>,
    state_dir: PathBuf,
    marker_matcher: Arc<MarkerMatcher>,
) -> Result<StartupPromptScan, CcbdError> {
    if !is_prompt_handling_provider(&provider) {
        return Ok(StartupPromptScan::HandledOrClear);
    }

    let disposition = scan_prompt_and_apply_outcome(PromptScanRequest {
        db: db.as_ref().clone(),
        agent_id,
        provider,
        pane_id: pane,
        tmux,
        state_dir,
        marker_matcher,
        max_depth: 3,
    })
    .await?;

    match disposition {
        PromptScanDisposition::Deferred { .. } => Ok(StartupPromptScan::Deferred),
        PromptScanDisposition::Pending { .. } => Ok(StartupPromptScan::Pending),
        PromptScanDisposition::NoActionNeeded { .. } => Ok(StartupPromptScan::ReadyConfirmed),
        PromptScanDisposition::Handled { .. } => Ok(StartupPromptScan::HandledOrClear),
    }
}

async fn capture_visible(tmux: Arc<TmuxServer>, pane: TmuxPaneId) -> Result<String, CcbdError> {
    crate::db::common::spawn_db("tmux::capture_visible_pane", move || {
        let args = [
            "-L",
            tmux.socket_name(),
            "capture-pane",
            "-p",
            "-t",
            &pane.0,
        ];
        let output = Command::new("tmux").args(args).output().map_err(|err| {
            CcbdError::EnvironmentNotSupported {
                details: format!("run tmux capture-pane: {err}"),
            }
        })?;
        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).to_string())
        } else {
            Err(CcbdError::TmuxCommandFailed {
                cmd: format!("tmux {}", args.join(" ")),
                stderr: String::from_utf8_lossy(&output.stderr).to_string(),
                exit: output.status.code().unwrap_or(-1),
            })
        }
    })
    .await
}

async fn mark_idle_after_probe(
    db: Arc<Db>,
    agent_id: String,
    idle_scan_enabled: Arc<AtomicBool>,
) -> Result<(), CcbdError> {
    match db::state_machine::mark_agent_idle_matched(db.as_ref().clone(), agent_id.clone()).await {
        Ok((changes, affected_job)) if changes > 0 => {
            if let Some(job_id) = affected_job {
                crate::orchestrator::pubsub::notify_job_update(&job_id);
            }
            crate::orchestrator::wake_up();
            idle_scan_enabled.store(true, Ordering::SeqCst);
            if let Some(handle) = crate::marker::registry::take(&agent_id) {
                let _ = handle.cancel_tx.send(());
            }
        }
        Ok(_) => {}
        Err(err) => return Err(err),
    }
    Ok(())
}

async fn mark_unknown_after_timeout(
    db: Arc<Db>,
    agent_id: String,
    capture: String,
) -> Result<(), CcbdError> {
    let failed_rules = json!(["init_probe"]);
    match db::state_machine::mark_agent_unknown(
        db.as_ref().clone(),
        agent_id.clone(),
        "INIT_PROBE_TIMEOUT".to_string(),
        capture.into_bytes(),
        failed_rules,
    )
    .await
    {
        Ok((changes, affected_job)) if changes > 0 => {
            if let Some(job_id) = affected_job {
                crate::orchestrator::pubsub::notify_job_update(&job_id);
            }
            crate::orchestrator::wake_up();
        }
        Ok(_) => {}
        Err(err) => return Err(err),
    }
    Ok(())
}
