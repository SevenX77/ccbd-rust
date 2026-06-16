pub mod pubsub;

use crate::db;
use crate::error::CcbdError;
use crate::marker::{
    MarkerMatcher, PromptTimerScanContext, TimerKind, parser_registry, registry,
    spawn_marker_timer_task_with_prompt,
};
use crate::pane_diff::{pane_diff_watcher_loop, resolve_stuck_watch_config};
use crate::prompt_handler::integration::{
    PromptScanDisposition, PromptScanRequest, is_prompt_handling_provider,
    prompt_pending_unpark_watcher_loop, scan_prompt_and_apply_outcome,
};
use crate::prompt_handler::matcher::PromptScanPurpose;
use crate::provider::health_check::health_check_watcher_loop;
use crate::provider::manifest::is_recovery_eligible_provider;
use crate::rpc::Ctx;
use crate::rpc::handlers::{RealignAgentParams, spawn_realign_agent};
use crate::tmux::TmuxPaneId;
use crate::db::recovery::RecoveryIntentAction;
use rusqlite::{OptionalExtension, params};
use serde_json::json;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, LazyLock};
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::Notify;

pub static WAKER: LazyLock<Notify> = LazyLock::new(Notify::new);

pub fn wake_up() {
    WAKER.notify_one();
}

pub fn spawn_orchestrator_task(ctx: Ctx) {
    let watcher_ctx = ctx.clone();
    tokio::spawn(async move {
        let (watch_interval, stuck_threshold) = resolve_stuck_watch_config();
        pane_diff_watcher_loop(watcher_ctx, watch_interval, stuck_threshold).await;
    });
    let health_ctx = ctx.clone();
    tokio::spawn(async move {
        let (watch_interval, stuck_threshold) = resolve_stuck_watch_config();
        health_check_watcher_loop(health_ctx, watch_interval, stuck_threshold).await;
    });
    let prompt_pending_ctx = ctx.clone();
    tokio::spawn(async move {
        let (watch_interval, _) = resolve_stuck_watch_config();
        prompt_pending_unpark_watcher_loop(prompt_pending_ctx, watch_interval).await;
    });

    tokio::spawn(async move {
        loop {
            if let Err(err) = run_once(&ctx).await {
                tracing::warn!(error = %err, "orchestrator tick failed");
            }
            WAKER.notified().await;
        }
    });
}

async fn run_once(ctx: &Ctx) -> Result<bool, CcbdError> {
    run_once_with_recovery_respawn(ctx, spawn_realign_agent_for_recovery).await
}

async fn run_once_with_recovery_respawn(
    ctx: &Ctx,
    respawn: RecoveryRespawnFn,
) -> Result<bool, CcbdError> {
    let idle_agents = db::agents::query_agents_by_state(ctx.db.clone(), "IDLE".to_string()).await?;
    let mut did_work = false;

    for agent in idle_agents {
        if !db::jobs::has_queued_job(ctx.db.clone(), agent.id.clone()).await? {
            continue;
        }

        let Some(pane_id) = crate::agent_io::pane_id(&agent.id) else {
            let Some(dispatched) = dispatch_queued_job(ctx, &agent.id).await? else {
                continue;
            };
            did_work = true;
            mark_dispatch_io_failed(ctx, &agent.id, "tmux pane not registered").await;
            let _ = db::jobs::mark_job_failed(
                ctx.db.clone(),
                dispatched.job.id,
                "tmux pane not registered".to_string(),
            )
            .await;
            wake_up();
            continue;
        };

        if parser_registry::get(&agent.id).is_none() {
            let Some(dispatched) = dispatch_queued_job(ctx, &agent.id).await? else {
                continue;
            };
            did_work = true;
            mark_dispatch_io_failed(ctx, &agent.id, "parser not registered").await;
            let _ = db::jobs::mark_job_failed(
                ctx.db.clone(),
                dispatched.job.id,
                "parser not registered".to_string(),
            )
            .await;
            wake_up();
            continue;
        }

        match run_dispatch_guard(ctx, &agent.id, &agent.provider, pane_id.clone()).await? {
            DispatchGuardOutcome::Permit => {}
            DispatchGuardOutcome::Refuse { retry } => {
                did_work = true;
                if retry {
                    wake_up();
                }
                continue;
            }
        }

        let Some(dispatched) = dispatch_queued_job(ctx, &agent.id).await? else {
            continue;
        };
        let job = dispatched.job;
        did_work = true;

        crate::agent_io::set_idle_scan_enabled(&agent.id, false);
        let capture_baseline = ctx.tmux_server.capture_pane(pane_id.clone()).await.ok();
        prepare_log_monitor_before_send(ctx, &agent.session_id, &agent.id, &agent.provider);

        let press_enter_after_paste =
            !(agent.provider == "antigravity" && job.prompt_text.ends_with('\n'));
        let send_result = crate::agent_io::send_text_to_pane_with_options(
            ctx.tmux_server.clone(),
            &agent.id,
            pane_id,
            job.prompt_text.clone(),
            press_enter_after_paste,
        )
        .await;

        if let Err(err) = send_result {
            crate::completion::registry::cancel(&agent.id);
            tracing::warn!(
                agent_id = %agent.id,
                error = %err,
                "send failed; cancelled completion log monitor"
            );
            mark_dispatch_io_failed(ctx, &agent.id, &format!("send failed: {err}")).await;
            let _ =
                db::jobs::mark_job_failed(ctx.db.clone(), job.id, format!("send failed: {err}"))
                    .await;
            wake_up();
            continue;
        }
        spawn_dispatch_ack_stability_busy(ctx.db.clone(), agent.id.clone());

        if let Some(parser_handle) = parser_registry::get(&agent.id) {
            let marker_handle = spawn_marker_timer_task_with_prompt(
                agent.id.clone(),
                TimerKind::Busy,
                Arc::new(ctx.db.clone()),
                parser_handle.clone(),
                Some(PromptTimerScanContext {
                    provider: agent.provider.clone(),
                    state_dir: ctx.state_dir.clone(),
                    tmux: ctx.tmux_server.clone(),
                    matcher: Arc::new(MarkerMatcher::from_manifest(
                        &crate::provider::manifest::get_manifest(&agent.provider),
                    )),
                }),
            );
            registry::register(agent.id.clone(), marker_handle);
            let manifest = crate::provider::manifest::get_manifest(&agent.provider);
            let matcher = MarkerMatcher::from_manifest(&manifest);
            if let Some(capture_baseline) = capture_baseline {
                crate::rpc::handlers::spawn_new_capture_seed(
                    ctx.db.clone(),
                    ctx.tmux_server.clone(),
                    agent.id.clone(),
                    agent.provider.clone(),
                    ctx.state_dir.clone(),
                    capture_baseline,
                    Arc::new(matcher),
                );
            }
            drop(parser_handle);
        }
    }

    if run_recovery_once_with_respawn(ctx, respawn).await? {
        did_work = true;
        wake_up();
    }

    Ok(did_work)
}

#[allow(dead_code)]
async fn run_recovery_once(ctx: &Ctx) -> Result<bool, CcbdError> {
    run_recovery_once_with_respawn(ctx, spawn_realign_agent_for_recovery).await
}

type RecoveryRespawnFuture<'a> = Pin<Box<dyn Future<Output = Result<(), CcbdError>> + Send + 'a>>;
type RecoveryRespawnFn =
    for<'a> fn(&'a Ctx, &'a str, &'a RealignAgentParams, &'a str) -> RecoveryRespawnFuture<'a>;

fn spawn_realign_agent_for_recovery<'a>(
    ctx: &'a Ctx,
    session_id: &'a str,
    agent: &'a RealignAgentParams,
    expected_hash: &'a str,
) -> RecoveryRespawnFuture<'a> {
    Box::pin(
        async move { spawn_realign_agent(ctx, session_id, agent, expected_hash, true, true).await },
    )
}

async fn run_recovery_once_with_respawn(
    ctx: &Ctx,
    respawn: RecoveryRespawnFn,
) -> Result<bool, CcbdError> {
    let now = unix_timestamp();
    let crashed_agents =
        db::agents::query_agents_by_state(ctx.db.clone(), "CRASHED".to_string()).await?;

    for agent in crashed_agents {
        if !is_recovery_eligible_provider(&agent.provider) {
            continue;
        }
        let (retry_exhausted, next_retry_at): (i64, Option<i64>) = {
            let conn = ctx.db.conn();
            conn.query_row(
                "SELECT retry_exhausted, next_retry_at FROM agents WHERE id = ?",
                params![agent.id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(|err| {
                CcbdError::DbConstraintViolation(format!("query recovery candidate: {err}"))
            })?
        };
        if retry_exhausted != 0 || next_retry_at.is_some_and(|retry_at| retry_at > now) {
            continue;
        }

        let intent = {
            let conn = ctx.db.conn();
            crate::db::recovery::query_agent_recovery_intent_sync(&conn, &agent.id)?
        };
        let Some(intent) = intent else {
            tracing::warn!(
                agent_id = %agent.id,
                "skipping crashed worker recovery because recovery intent is missing"
            );
            let already_emitted = {
                let conn = ctx.db.conn();
                self_recovery_event_exists(&conn, &agent.id, "skipped", "missing_recovery_intent")?
            };
            if !already_emitted {
                emit_self_recovery_attempt_event(
                    ctx,
                    &agent.id,
                    "skipped",
                    "missing_recovery_intent",
                    0,
                    None,
                    agent.state_version,
                    None,
                    None,
                )
                .await?;
            }
            continue;
        };
        if intent.action == RecoveryIntentAction::ReapOnly {
            tracing::info!(
                agent_id = %agent.id,
                previous_state = %intent.previous_state,
                "reaping crashed worker without respawn because recovery intent is REAP_ONLY"
            );
            crate::agent_io::cleanup_agent_runtime_resources(&agent.id);
            db::agents::delete_agent(ctx.db.clone(), agent.id.clone()).await?;
            return Ok(true);
        }

        let has_snapshot = {
            let conn = ctx.db.conn();
            crate::db::recovery::query_agent_spawn_spec_sync(&conn, &agent.id)?.is_some()
        };
        if !has_snapshot {
            let already_emitted = {
                let conn = ctx.db.conn();
                self_recovery_event_exists(&conn, &agent.id, "skipped", "missing_spawn_spec")?
            };
            if !already_emitted {
                emit_self_recovery_attempt_event(
                    ctx,
                    &agent.id,
                    "skipped",
                    "missing_spawn_spec",
                    0,
                    None,
                    agent.state_version,
                    None,
                    None,
                )
                .await?;
            }
            continue;
        }

        let claimed = {
            let conn = ctx.db.conn();
            crate::db::recovery::try_claim_agent_recovery_sync(
                &conn,
                &agent.id,
                agent.state_version,
                now,
            )?
        };
        if !claimed {
            tracing::debug!(
                agent_id = %agent.id,
                "recovery CAS lost, external state change"
            );
            continue;
        }

        recover_crashed_agent_from_snapshot_with_respawn_and_intent(
            ctx,
            &agent.id,
            respawn,
            Some(intent),
        )
        .await?;
        return Ok(true);
    }

    Ok(false)
}

#[allow(dead_code)] // direct test seam; production passes captured intent through recovery loop
async fn recover_crashed_agent_from_snapshot_with_respawn(
    ctx: &Ctx,
    agent_id: &str,
    respawn: RecoveryRespawnFn,
) -> Result<(), CcbdError> {
    recover_crashed_agent_from_snapshot_with_respawn_and_intent(ctx, agent_id, respawn, None).await
}

async fn recover_crashed_agent_from_snapshot_with_respawn_and_intent(
    ctx: &Ctx,
    agent_id: &str,
    respawn: RecoveryRespawnFn,
    captured_intent: Option<crate::db::recovery::AgentRecoveryIntent>,
) -> Result<(), CcbdError> {
    let (agent, stored) = {
        let conn = ctx.db.conn();
        let agent = db::agents::query_agent_sync(&conn, agent_id)?
            .ok_or_else(|| CcbdError::AgentNotFound(agent_id.to_string()))?;
        let stored = crate::db::recovery::query_agent_spawn_spec_sync(&conn, agent_id)?
            .ok_or_else(|| {
                CcbdError::DbConstraintViolation(format!("missing spawn spec for {agent_id}"))
            })?;
        (agent, stored)
    };
    let crash_context = CrashContext {
        exit_code: agent.exit_code,
        error_code: agent.error_code.clone(),
    };
    let params = RealignAgentParams {
        agent_id: stored.spec.agent_id.clone(),
        provider: stored.spec.provider.clone(),
        env: stored.spec.env.clone(),
        hooks: stored.spec.hooks.clone(),
        plugins: stored.spec.plugins.clone(),
        sandbox_overrides: stored.spec.sandbox_overrides.clone(),
    };

    db::agents::delete_agent(ctx.db.clone(), agent_id.to_string()).await?;
    let spawn_result = respawn(ctx, &agent.session_id, &params, &stored.config_hash).await;
    if let Err(err) = &spawn_result {
        tracing::error!(
            agent_id,
            error = %err,
            "self recovery respawn failed; restoring crashed agent row and spawn snapshot"
        );
        restore_crashed_agent_after_recovery_spawn_failure(ctx, &agent, &stored)?;
        if let Some(intent) = captured_intent.as_ref() {
            let conn = ctx.db.conn();
            crate::db::recovery::persist_agent_recovery_intent_sync(&conn, intent)?;
        }
    } else if let Some(intent) = captured_intent.as_ref() {
        let conn = ctx.db.conn();
        crate::db::recovery::requeue_interrupted_job_from_captured_intent_sync(&conn, intent)?;
    }
    apply_recovery_spawn_result(
        ctx,
        agent_id,
        spawn_result,
        unix_timestamp(),
        Some(crash_context),
    )
    .await
}

#[derive(Debug, Clone)]
struct CrashContext {
    exit_code: Option<i64>,
    error_code: Option<String>,
}

fn self_recovery_event_exists(
    conn: &rusqlite::Connection,
    agent_id: &str,
    action: &str,
    reason: &str,
) -> Result<bool, CcbdError> {
    conn.query_row(
        "SELECT 1 FROM events \
         WHERE agent_id = ? AND event_type = 'self_recovery_attempt' \
           AND payload LIKE ? AND payload LIKE ? \
         LIMIT 1",
        params![
            agent_id,
            format!("%\"action\":\"{action}\"%"),
            format!("%\"reason\":\"{reason}\"%"),
        ],
        |row| row.get::<_, i64>(0),
    )
    .optional()
    .map(|value| value.is_some())
    .map_err(|err| CcbdError::DbConstraintViolation(format!("query recovery event: {err}")))
}

async fn apply_recovery_spawn_result(
    ctx: &Ctx,
    agent_id: &str,
    spawn_result: Result<(), CcbdError>,
    now: i64,
    crash_context: Option<CrashContext>,
) -> Result<(), CcbdError> {
    enum RecoveryEvent {
        Recovered {
            retry_count: i64,
            next_retry_at: Option<i64>,
            state_version: i64,
            crash_context: Option<CrashContext>,
        },
        Failed {
            retry_count: i64,
            next_retry_at: Option<i64>,
            state_version: i64,
            error: String,
        },
    }

    let event = {
        let conn = ctx.db.conn();
        match spawn_result {
            Ok(()) => {
                crate::db::recovery::clear_recovery_backoff_sync(&conn, agent_id)?;
                let (retry_count, next_retry_at, state_version) =
                    recovery_event_state(&conn, agent_id)?;
                RecoveryEvent::Recovered {
                    retry_count,
                    next_retry_at,
                    state_version,
                    crash_context,
                }
            }
            Err(err) => {
                let stored = crate::db::recovery::query_agent_spawn_spec_sync(&conn, agent_id)?;
                conn.execute(
                    "UPDATE agents \
                     SET state = 'CRASHED', state_version = state_version + 1, updated_at = unixepoch() \
                     WHERE id = ?",
                    params![agent_id],
                )
                .map_err(|err| {
                    CcbdError::DbConstraintViolation(format!(
                        "restore crashed recovery state: {err}"
                    ))
                })?;
                if let Some(stored) = stored {
                    crate::db::recovery::persist_agent_spawn_spec_sync(
                        &conn,
                        &stored.spec,
                        &stored.config_hash,
                    )?;
                }
                let backoff = crate::db::recovery::record_recovery_failure_backoff_sync(
                    &conn, agent_id, now,
                )?;
                let (_, _, state_version) = recovery_event_state(&conn, agent_id)?;
                let error_message = recovery_error_message(&err);
                RecoveryEvent::Failed {
                    retry_count: backoff.retry_count,
                    next_retry_at: backoff.next_retry_at,
                    state_version,
                    error: error_message,
                }
            }
        }
    };

    match event {
        RecoveryEvent::Recovered {
            retry_count,
            next_retry_at,
            state_version,
            crash_context,
        } => {
            emit_self_recovery_attempt_event(
                ctx,
                agent_id,
                "recovered",
                "OOM_RECOVERY",
                retry_count,
                next_retry_at,
                state_version,
                None,
                crash_context.as_ref(),
            )
            .await?;
        }
        RecoveryEvent::Failed {
            retry_count,
            next_retry_at,
            state_version,
            error,
        } => {
            emit_self_recovery_attempt_event(
                ctx,
                agent_id,
                "failed",
                "OOM_RECOVERY",
                retry_count,
                next_retry_at,
                state_version,
                Some(error.as_str()),
                None,
            )
            .await?;
        }
    }
    Ok(())
}

async fn emit_self_recovery_attempt_event(
    ctx: &Ctx,
    agent_id: &str,
    action: &str,
    reason: &str,
    retry_count: i64,
    next_retry_at: Option<i64>,
    state_version: i64,
    error: Option<&str>,
    recovered_from: Option<&CrashContext>,
) -> Result<i64, CcbdError> {
    let mut payload = json!({
        "agent_id": agent_id,
        "reason": reason,
        "action": action,
        "retry_count": retry_count,
        "next_retry_at": next_retry_at,
        "state_version": state_version,
    });
    if let Some(error) = error {
        payload["error"] = json!(error);
    }
    if let Some(recovered_from) = recovered_from {
        if let Some(exit_code) = recovered_from.exit_code {
            payload["recovered_from_exit_code"] = json!(exit_code);
        }
        if let Some(error_code) = recovered_from.error_code.as_deref() {
            payload["recovered_from_error_code"] = json!(error_code);
        }
    }
    db::events::insert_event(
        ctx.db.clone(),
        agent_id.to_string(),
        None,
        "self_recovery_attempt".to_string(),
        payload.to_string(),
    )
    .await
}

fn restore_crashed_agent_after_recovery_spawn_failure(
    ctx: &Ctx,
    agent: &crate::db::schema::Agent,
    stored: &crate::db::recovery::StoredAgentSpawnSpec,
) -> Result<(), CcbdError> {
    let conn = ctx.db.conn();
    let exists: Option<i64> = conn
        .query_row(
            "SELECT 1 FROM agents WHERE id = ? LIMIT 1",
            params![agent.id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|err| CcbdError::DbConstraintViolation(format!("query restored agent: {err}")))?;
    if exists.is_none() {
        db::agents::insert_agent_sync(
            &conn,
            &agent.id,
            &agent.session_id,
            &agent.provider,
            "CRASHED",
            None,
        )?;
        conn.execute(
            "UPDATE agents \
             SET config_hash = ?, state_version = ?, retry_count = 0, next_retry_at = NULL, \
                 retry_exhausted = 0, updated_at = unixepoch() \
             WHERE id = ?",
            params![stored.config_hash, agent.state_version, agent.id],
        )
        .map_err(|err| {
            CcbdError::DbConstraintViolation(format!("restore crashed agent row: {err}"))
        })?;
    }
    crate::db::recovery::persist_agent_spawn_spec_sync(&conn, &stored.spec, &stored.config_hash)
}

fn recovery_event_state(
    conn: &rusqlite::Connection,
    agent_id: &str,
) -> Result<(i64, Option<i64>, i64), CcbdError> {
    conn.query_row(
        "SELECT retry_count, next_retry_at, state_version FROM agents WHERE id = ?",
        params![agent_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )
    .map_err(|err| CcbdError::DbConstraintViolation(format!("query recovery event state: {err}")))
}

fn recovery_error_message(err: &CcbdError) -> String {
    match err {
        CcbdError::PtyIoError(message) => message.clone(),
        CcbdError::PtyOpenFailed(message) => message.clone(),
        CcbdError::EnvironmentNotSupported { details }
        | CcbdError::SandboxUserNsDisabled { details }
        | CcbdError::SandboxMountFailed { details }
        | CcbdError::AgentUnexpectedExit { details }
        | CcbdError::StartupMarkerTimeout { details }
        | CcbdError::PtyMarkerTimeout { details } => details.clone(),
        CcbdError::TmuxCommandFailed { stderr, .. } => stderr.clone(),
        _ => err.to_string(),
    }
}

fn unix_timestamp() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

async fn dispatch_queued_job(
    ctx: &Ctx,
    agent_id: &str,
) -> Result<Option<db::jobs::DispatchedJob>, CcbdError> {
    db::jobs::dispatch_job_to_agent(
        ctx.db.clone(),
        agent_id.to_string(),
        vec![db::state_machine::STATE_IDLE.to_string()],
        db::state_machine::STATE_WAITING_FOR_ACK.to_string(),
        "command_received".to_string(),
        json!({ "status": "SENT" }),
    )
    .await
}

enum DispatchGuardOutcome {
    Permit,
    Refuse { retry: bool },
}

async fn run_dispatch_guard(
    ctx: &Ctx,
    agent_id: &str,
    provider: &str,
    pane_id: TmuxPaneId,
) -> Result<DispatchGuardOutcome, CcbdError> {
    if !is_prompt_handling_provider(provider) {
        return Ok(DispatchGuardOutcome::Permit);
    }

    let manifest = crate::provider::manifest::get_manifest(provider);
    let marker_matcher = Arc::new(MarkerMatcher::from_manifest(&manifest));
    let result = scan_prompt_and_apply_outcome(PromptScanRequest {
        db: ctx.db.clone(),
        agent_id: agent_id.to_string(),
        provider: provider.to_string(),
        pane_id,
        tmux: ctx.tmux_server.clone(),
        state_dir: ctx.state_dir.clone(),
        marker_matcher,
        max_depth: 3,
        scan_purpose: PromptScanPurpose::DispatchGuard,
    })
    .await;

    let retry = matches!(result, Ok(PromptScanDisposition::Handled { .. }));
    if dispatch_guard_scan_permits_dispatch(result) {
        Ok(DispatchGuardOutcome::Permit)
    } else {
        Ok(DispatchGuardOutcome::Refuse { retry })
    }
}

fn dispatch_guard_scan_permits_dispatch(result: Result<PromptScanDisposition, CcbdError>) -> bool {
    match result {
        Ok(PromptScanDisposition::NoActionNeeded { .. }) => true,
        Ok(PromptScanDisposition::Handled { depth }) => {
            tracing::info!(
                depth,
                "dispatch guard handled interstitial before dispatch; leaving queued job for retry"
            );
            false
        }
        Ok(PromptScanDisposition::Deferred {
            depth,
            block_reason,
        })
        | Ok(PromptScanDisposition::Pending {
            depth,
            block_reason,
        }) => {
            tracing::warn!(
                depth,
                block_reason,
                "dispatch guard refused dispatch while prompt scan is unresolved"
            );
            false
        }
        Err(err) => {
            tracing::warn!(
                error = %err,
                "dispatch guard scan failed closed before dispatch"
            );
            false
        }
    }
}

fn prepare_log_monitor_before_send(
    ctx: &Ctx,
    session_id: &str,
    agent_id: &str,
    provider: &str,
) -> bool {
    let root = match crate::completion::log_layout::resolve_agent_log_root(
        &ctx.state_dir,
        session_id,
        agent_id,
        provider,
        ctx.env_state.unsafe_no_sandbox,
    ) {
        crate::completion::log_layout::LogRootResolution::Available(root) => root,
        crate::completion::log_layout::LogRootResolution::Unavailable(unavailable) => {
            tracing::info!(
                agent_id,
                provider,
                reason = unavailable.reason,
                "log signal unavailable, UI-only completion"
            );
            return false;
        }
    };

    let cursors = match crate::completion::reader::collect_provider_log_cursors(provider, &root) {
        Ok(cursors) => cursors,
        Err(err) => {
            tracing::info!(
                agent_id,
                provider,
                root = %root.display(),
                error = %err,
                "log signal cursor baseline unavailable, UI-only completion"
            );
            return false;
        }
    };
    let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel();
    let initial_state = crate::completion::reader::LogReadState::from_cursors(cursors);
    crate::completion::registry::register(
        agent_id.to_string(),
        crate::completion::registry::LogMonitorEntry {
            provider: provider.to_string(),
            log_root: root.clone(),
            state: initial_state.clone(),
            cancel_tx,
        },
    );
    crate::completion::monitor::spawn_log_monitor_task(
        ctx.db.clone(),
        agent_id.to_string(),
        provider.to_string(),
        root,
        initial_state,
        cancel_rx,
    );
    true
}

async fn mark_dispatch_io_failed(ctx: &Ctx, agent_id: &str, reason: &str) {
    if let Err(err) = db::state_machine::transit_agent_state(
        ctx.db.clone(),
        agent_id.to_string(),
        vec![db::state_machine::STATE_WAITING_FOR_ACK.to_string()],
        db::state_machine::STATE_STUCK.to_string(),
        Some(reason.to_string()),
    )
    .await
    {
        tracing::warn!(agent_id = %agent_id, error = %err, "failed to compensate dispatch IO failure");
    }
}

fn spawn_dispatch_ack_stability_busy(db: db::Db, agent_id: String) {
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(
            crate::rpc::handlers::CAPTURE_SEED_STABILITY_MS,
        ))
        .await;
        if let Err(err) = db::state_machine::transit_agent_state(
            db,
            agent_id.clone(),
            vec![db::state_machine::STATE_WAITING_FOR_ACK.to_string()],
            db::state_machine::STATE_BUSY.to_string(),
            Some("DISPATCH_ACK_STABLE".to_string()),
        )
        .await
        {
            tracing::warn!(agent_id = %agent_id, error = %err, "failed to mark agent BUSY after dispatch ACK stability window");
        }
    });
}

#[cfg(test)]
mod tests {
    use super::{
        RecoveryRespawnFuture, apply_recovery_spawn_result, dispatch_guard_scan_permits_dispatch,
        prepare_log_monitor_before_send, recover_crashed_agent_from_snapshot_with_respawn,
        run_once, run_once_with_recovery_respawn, run_recovery_once,
        run_recovery_once_with_respawn, wake_up,
    };
    use crate::db;
    use crate::db::agents::insert_agent_sync;
    use crate::db::jobs::{insert_job_sync, query_job_sync};
    use crate::db::recovery::{
        AgentRecoveryIntent, AgentSpawnSpec, CapturedInterruptedJob, RecoveryIntentAction,
        persist_agent_recovery_intent_sync, persist_agent_spawn_spec_sync,
    };
    use crate::db::sessions::insert_session_sync;
    use crate::error::CcbdError;
    use crate::prompt_handler::integration::{
        PromptScanDisposition, apply_prompt_pending_unpark_outcome_sync,
    };
    use crate::rpc::Ctx;
    use crate::rpc::handlers::RealignAgentParams;
    use crate::sandbox::EnvState;
    use crate::tmux::{TmuxPaneId, TmuxServer};
    use serde_json::Value;
    use std::collections::HashMap;
    use std::sync::atomic::AtomicBool;
    use std::sync::{Arc, Mutex};

    fn test_ctx() -> Ctx {
        let file = tempfile::NamedTempFile::new().unwrap();
        let state_dir = tempfile::TempDir::new().unwrap().keep();
        Ctx {
            db: db::init(file.path()).unwrap(),
            state_dir: state_dir.clone(),
            env_state: EnvState {
                systemd_run_available: false,
                unsafe_no_sandbox: true,
                under_systemd: false,
            },
            daemon_unit: None,
            tmux_server: Arc::new(TmuxServer::new(&state_dir)),
        }
    }

    fn seed_session(conn: &rusqlite::Connection) {
        insert_session_sync(conn, "s1", "p1", "/tmp/foo").unwrap();
    }

    fn sample_spawn_spec(agent_id: &str, provider: &str) -> AgentSpawnSpec {
        AgentSpawnSpec {
            agent_id: agent_id.to_string(),
            provider: provider.to_string(),
            env: HashMap::from([("FOO".to_string(), "bar".to_string())]),
            hooks: HashMap::new(),
            plugins: vec!["github@openai-curated".to_string()],
            sandbox_overrides: Default::default(),
        }
    }

    fn sample_recovery_intent(
        agent_id: &str,
        action: RecoveryIntentAction,
        interrupted_job_id: Option<&str>,
    ) -> AgentRecoveryIntent {
        AgentRecoveryIntent {
            agent_id: agent_id.to_string(),
            session_id: "s1".to_string(),
            provider: "codex".to_string(),
            previous_state: match &action {
                RecoveryIntentAction::Revive => "BUSY",
                RecoveryIntentAction::ReapOnly => "IDLE",
            }
            .to_string(),
            crashed_state_version: 7,
            interrupted_job_id: interrupted_job_id.map(str::to_string),
            interrupted_job_status: interrupted_job_id.map(|_| "DISPATCHED".to_string()),
            interrupted_job: interrupted_job_id.map(|job_id| CapturedInterruptedJob {
                id: job_id.to_string(),
                request_id: None,
                prompt_text: "continue\n".to_string(),
                cancel_requested: false,
                requires_physical_evidence: false,
                requires_test_evidence: false,
            }),
            action,
            reason: "AGENT_UNEXPECTED_EXIT".to_string(),
        }
    }

    fn seed_crashed_agent(
        conn: &rusqlite::Connection,
        agent_id: &str,
        provider: &str,
        with_snapshot: bool,
    ) {
        insert_agent_sync(conn, agent_id, "s1", provider, "CRASHED", None).unwrap();
        conn.execute(
            "UPDATE agents SET config_hash = ?, state_version = 7 WHERE id = ?",
            ("hash1", agent_id),
        )
        .unwrap();
        if with_snapshot {
            persist_agent_spawn_spec_sync(conn, &sample_spawn_spec(agent_id, provider), "hash1")
                .unwrap();
        }
    }

    fn recovery_events(conn: &rusqlite::Connection, agent_id: &str) -> Vec<Value> {
        let mut stmt = conn
            .prepare(
                "SELECT payload FROM events \
                 WHERE agent_id = ? AND event_type = 'self_recovery_attempt' \
                 ORDER BY seq_id ASC",
            )
            .unwrap();
        stmt.query_map([agent_id], |row| row.get::<_, String>(0))
            .unwrap()
            .map(|row| serde_json::from_str(&row.unwrap()).unwrap())
            .collect()
    }

    fn recovery_event_count(conn: &rusqlite::Connection, agent_id: &str) -> i64 {
        conn.query_row(
            "SELECT COUNT(*) FROM events \
             WHERE agent_id = ? AND event_type = 'self_recovery_attempt'",
            [agent_id],
            |row| row.get(0),
        )
        .unwrap()
    }

    fn fake_recovery_respawn_ok<'a>(
        ctx: &'a Ctx,
        session_id: &'a str,
        agent: &'a RealignAgentParams,
        expected_hash: &'a str,
    ) -> RecoveryRespawnFuture<'a> {
        Box::pin(async move {
            let conn = ctx.db.conn();
            insert_agent_sync(
                &conn,
                &agent.agent_id,
                session_id,
                &agent.provider,
                "IDLE",
                None,
            )?;
            crate::db::agents::update_agent_config_hash_sync(
                &conn,
                &agent.agent_id,
                expected_hash,
            )?;
            crate::db::recovery::persist_agent_spawn_spec_sync(
                &conn,
                &AgentSpawnSpec {
                    agent_id: agent.agent_id.clone(),
                    provider: agent.provider.clone(),
                    env: agent.env.clone(),
                    hooks: agent.hooks.clone(),
                    plugins: agent.plugins.clone(),
                    sandbox_overrides: agent.sandbox_overrides.clone(),
                },
                expected_hash,
            )?;
            Ok(())
        })
    }

    fn fake_recovery_respawn_requires_ro_bind<'a>(
        ctx: &'a Ctx,
        session_id: &'a str,
        agent: &'a RealignAgentParams,
        expected_hash: &'a str,
    ) -> RecoveryRespawnFuture<'a> {
        Box::pin(async move {
            assert_eq!(
                agent.sandbox_overrides.extra_ro_binds[0].host_path,
                "/opt/keys"
            );
            assert_eq!(
                agent.sandbox_overrides.extra_ro_binds[0].sandbox_path,
                "/mnt/keys"
            );
            fake_recovery_respawn_ok(ctx, session_id, agent, expected_hash).await
        })
    }

    fn fake_recovery_respawn_err<'a>(
        _ctx: &'a Ctx,
        _session_id: &'a str,
        _agent: &'a RealignAgentParams,
        _expected_hash: &'a str,
    ) -> RecoveryRespawnFuture<'a> {
        Box::pin(async {
            Err(CcbdError::PtyIoError(
                "injected spawn failed after delete".to_string(),
            ))
        })
    }

    #[test]
    fn test_wake_up_is_callable() {
        wake_up();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_run_once_fails_job_without_pane() {
        let ctx = test_ctx();
        let _ = crate::agent_io::remove("orchestrator_no_pane");
        {
            let conn = ctx.db.conn();
            insert_session_sync(&conn, "s1", "p1", "/tmp/foo").unwrap();
            insert_agent_sync(
                &conn,
                "orchestrator_no_pane",
                "s1",
                "bash",
                "IDLE",
                Some(123),
            )
            .unwrap();
            insert_job_sync(&conn, "job_1", "orchestrator_no_pane", None, "echo hi\n").unwrap();
        }

        assert!(
            run_once_with_recovery_respawn(&ctx, fake_recovery_respawn_ok)
                .await
                .unwrap()
        );
        let job = query_job_sync(&ctx.db.conn(), "job_1").unwrap().unwrap();

        assert_eq!(job.status, "FAILED");
        assert_eq!(
            job.error_reason.as_deref(),
            Some("tmux pane not registered")
        );
    }

    #[test]
    fn dispatch_guard_handled_or_error_refuses_before_job_claim() {
        let ctx = test_ctx();
        {
            let conn = ctx.db.conn();
            insert_session_sync(&conn, "s1", "p1", "/tmp/foo").unwrap();
            insert_agent_sync(&conn, "a1", "s1", "codex", "IDLE", Some(123)).unwrap();
            insert_job_sync(&conn, "job_1", "a1", None, "echo hi\n").unwrap();
        }

        assert!(!dispatch_guard_scan_permits_dispatch(Ok(
            PromptScanDisposition::Handled { depth: 1 },
        )));
        assert!(!dispatch_guard_scan_permits_dispatch(Err(
            crate::error::CcbdError::TmuxCommandFailed {
                cmd: "fake capture".to_string(),
                stderr: "capture failed".to_string(),
                exit: 1,
            },
        )));

        let conn = ctx.db.conn();
        let job = query_job_sync(&conn, "job_1").unwrap().unwrap();
        let state: String = conn
            .query_row("SELECT state FROM agents WHERE id = 'a1'", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(job.status, "QUEUED");
        assert_eq!(state, "IDLE");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn monitor_registers_baseline_before_send() {
        let mut ctx = test_ctx();
        ctx.env_state.unsafe_no_sandbox = false;
        let sandbox = ctx.state_dir.join("sandboxes").join("s1").join("a1");
        std::fs::create_dir_all(&sandbox).unwrap();
        let home = crate::provider::home_layout::sandbox_home_for_sandbox_dir(&sandbox).unwrap();
        let sessions = home.join(".codex/sessions/2026/06");
        std::fs::create_dir_all(&sessions).unwrap();
        let log = sessions.join("rollout-session.jsonl");
        std::fs::write(&log, b"{\"old\":true}\n").unwrap();
        let conn = ctx.db.conn();
        insert_session_sync(&conn, "s1", "p1", "/tmp/foo").unwrap();
        insert_agent_sync(&conn, "a1", "s1", "codex", "WAITING_FOR_ACK", Some(123)).unwrap();

        let registered = prepare_log_monitor_before_send(&ctx, "s1", "a1", "codex");

        assert!(registered);
        let cursor = crate::completion::registry::cursor_snapshot("a1").unwrap();
        assert_eq!(cursor.get(&log).copied(), Some(13));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn dispatch_guard_capture_error_keeps_job_queued_before_log_monitor() {
        let agent_id = "orchestrator_guard_capture_err";
        let mut ctx = test_ctx();
        ctx.env_state.unsafe_no_sandbox = false;
        crate::completion::registry::cancel(agent_id);
        let _ = crate::agent_io::remove(agent_id);
        let _ = crate::marker::parser_registry::remove(agent_id);
        let sandbox = ctx.state_dir.join("sandboxes").join("s1").join(agent_id);
        std::fs::create_dir_all(&sandbox).unwrap();
        let home = crate::provider::home_layout::sandbox_home_for_sandbox_dir(&sandbox).unwrap();
        let sessions = home.join(".codex/sessions/2026/06");
        std::fs::create_dir_all(&sessions).unwrap();
        std::fs::write(sessions.join("rollout-session.jsonl"), b"{\"old\":true}\n").unwrap();
        {
            let conn = ctx.db.conn();
            insert_session_sync(&conn, "s1", "p1", "/tmp/foo").unwrap();
            insert_agent_sync(&conn, agent_id, "s1", "codex", "IDLE", Some(123)).unwrap();
            insert_job_sync(&conn, "job_1", agent_id, None, "echo hi\n").unwrap();
        }
        crate::marker::parser_registry::register(
            agent_id.to_string(),
            Arc::new(Mutex::new(vt100::Parser::new(200, 200, 0))),
        );
        let reader_handle = tokio::spawn(async {
            std::future::pending::<()>().await;
        });
        crate::agent_io::register(
            agent_id.to_string(),
            crate::agent_io::AgentIoEntry {
                session_id: "s1".to_string(),
                pane_id: TmuxPaneId("%999999".to_string()),
                reader_handle,
                fifo_path: ctx.state_dir.join("pipes").join(format!("{agent_id}.fifo")),
                socket_name: ctx.tmux_server.socket_name().to_string(),
                idle_scan_enabled: Arc::new(AtomicBool::new(true)),
            },
        );

        assert!(run_once(&ctx).await.unwrap());

        assert!(!crate::completion::registry::contains(agent_id));
        let job = query_job_sync(&ctx.db.conn(), "job_1").unwrap().unwrap();
        assert_eq!(job.status, "QUEUED");
        let state: String = ctx
            .db
            .conn()
            .query_row("SELECT state FROM agents WHERE id = ?", [agent_id], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(state, "IDLE");
        let _ = crate::agent_io::remove(agent_id);
        let _ = crate::marker::parser_registry::remove(agent_id);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn rg_unparked_prompt_pending_job_is_dispatched_by_next_run_once() {
        let ctx = test_ctx();
        let agent_id = "rg_prompt_pending_no_pane";
        let _ = crate::agent_io::remove(agent_id);
        {
            let conn = ctx.db.conn();
            insert_session_sync(&conn, "s1", "p1", "/tmp/foo").unwrap();
            insert_agent_sync(&conn, agent_id, "s1", "codex", "PROMPT_PENDING", Some(123)).unwrap();
            insert_job_sync(&conn, "rg_job", agent_id, None, "echo hi\n").unwrap();
            conn.execute(
                "UPDATE agents SET state_version = 5 WHERE id = ?",
                [agent_id],
            )
            .unwrap();
        }

        assert_eq!(
            apply_prompt_pending_unpark_outcome_sync(
                &ctx.db,
                agent_id,
                5,
                crate::prompt_handler::runner::PromptRunOutcome::NoActionNeeded { depth: 0 },
            )
            .unwrap(),
            crate::prompt_handler::integration::PromptPendingUnparkDisposition::Unparked
        );
        assert!(
            run_once_with_recovery_respawn(&ctx, fake_recovery_respawn_ok)
                .await
                .unwrap()
        );

        let job = query_job_sync(&ctx.db.conn(), "rg_job").unwrap().unwrap();
        assert_eq!(job.status, "FAILED");
        assert_eq!(
            job.error_reason.as_deref(),
            Some("tmux pane not registered")
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn missing_or_unreadable_log_root_switches_to_ui_immediately() {
        let ctx = test_ctx();
        let conn = ctx.db.conn();
        insert_session_sync(&conn, "s1", "p1", "/tmp/foo").unwrap();
        insert_agent_sync(&conn, "a1", "s1", "codex", "WAITING_FOR_ACK", Some(123)).unwrap();

        let registered = prepare_log_monitor_before_send(&ctx, "s1", "a1", "codex");

        assert!(!registered);
        assert!(!crate::completion::registry::contains("a1"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn ra2_run_once_dispatches_idle_job_and_then_attempts_recovery() {
        let ctx = test_ctx();
        let _ = crate::agent_io::remove("ra2_idle_no_pane");
        {
            let conn = ctx.db.conn();
            seed_session(&conn);
            insert_agent_sync(&conn, "ra2_idle_no_pane", "s1", "bash", "IDLE", Some(123)).unwrap();
            insert_job_sync(&conn, "ra2_job", "ra2_idle_no_pane", None, "echo hi\n").unwrap();
            seed_crashed_agent(&conn, "ra2_crashed", "codex", true);
            persist_agent_recovery_intent_sync(
                &conn,
                &sample_recovery_intent("ra2_crashed", RecoveryIntentAction::Revive, None),
            )
            .unwrap();
        }

        assert!(
            run_once_with_recovery_respawn(&ctx, fake_recovery_respawn_ok)
                .await
                .unwrap()
        );
        let conn = ctx.db.conn();
        let job = query_job_sync(&conn, "ra2_job").unwrap().unwrap();
        assert_eq!(job.status, "FAILED");
        let events = recovery_events(&conn, "ra2_crashed");
        assert!(
            events.iter().any(|payload| {
                payload.get("action").and_then(Value::as_str) == Some("recovered")
            }),
            "run_once should append recovery after dispatch guard loop"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn ra2_recovery_loop_recovers_one_due_crashed_agent_from_snapshot() {
        let ctx = test_ctx();
        {
            let conn = ctx.db.conn();
            seed_session(&conn);
            seed_crashed_agent(&conn, "ra2_due", "codex", true);
            persist_agent_recovery_intent_sync(
                &conn,
                &sample_recovery_intent("ra2_due", RecoveryIntentAction::Revive, None),
            )
            .unwrap();
        }

        assert!(
            run_recovery_once_with_respawn(&ctx, fake_recovery_respawn_ok)
                .await
                .unwrap()
        );
        let conn = ctx.db.conn();
        let row: (String, i64, Option<i64>, i64) = conn
            .query_row(
                "SELECT state, retry_count, next_retry_at, retry_exhausted FROM agents WHERE id = 'ra2_due'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(row, ("IDLE".to_string(), 0, None, 0));
        assert!(
            recovery_events(&conn, "ra2_due").iter().any(|payload| {
                payload.get("action").and_then(Value::as_str) == Some("recovered")
            })
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn rr_worker_gate_revive_intent_allows_respawn() {
        let ctx = test_ctx();
        {
            let conn = ctx.db.conn();
            seed_session(&conn);
            seed_crashed_agent(&conn, "rr_revive", "codex", true);
            persist_agent_recovery_intent_sync(
                &conn,
                &sample_recovery_intent("rr_revive", RecoveryIntentAction::Revive, None),
            )
            .unwrap();
        }

        assert!(
            run_recovery_once_with_respawn(&ctx, fake_recovery_respawn_ok)
                .await
                .unwrap()
        );
        let conn = ctx.db.conn();
        let state: String = conn
            .query_row("SELECT state FROM agents WHERE id = 'rr_revive'", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(state, "IDLE");
        assert!(recovery_events(&conn, "rr_revive").iter().any(|payload| {
            payload.get("action").and_then(Value::as_str) == Some("recovered")
        }));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn rr_worker_gate_reap_only_intent_deletes_without_respawn() {
        let ctx = test_ctx();
        {
            let conn = ctx.db.conn();
            seed_session(&conn);
            seed_crashed_agent(&conn, "rr_reap_only", "codex", true);
            persist_agent_recovery_intent_sync(
                &conn,
                &sample_recovery_intent("rr_reap_only", RecoveryIntentAction::ReapOnly, None),
            )
            .unwrap();
        }

        assert!(
            run_recovery_once_with_respawn(&ctx, fake_recovery_respawn_ok)
                .await
                .unwrap()
        );
        let conn = ctx.db.conn();
        let exists: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM agents WHERE id = 'rr_reap_only')",
                [],
                |row| row.get(0),
        )
            .unwrap();
        assert!(!exists);
        assert_eq!(recovery_event_count(&conn, "rr_reap_only"), 0);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn rr_worker_gate_missing_intent_skips_crashed_agent_without_respawn() {
        let ctx = test_ctx();
        {
            let conn = ctx.db.conn();
            seed_session(&conn);
            seed_crashed_agent(&conn, "rr_missing_intent", "codex", true);
        }

        assert!(
            !run_recovery_once_with_respawn(&ctx, fake_recovery_respawn_ok)
                .await
                .unwrap()
        );
        let conn = ctx.db.conn();
        let state: String = conn
            .query_row(
                "SELECT state FROM agents WHERE id = 'rr_missing_intent'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(state, "CRASHED");
        assert!(recovery_events(&conn, "rr_missing_intent").iter().any(|payload| {
            payload.get("action").and_then(Value::as_str) == Some("skipped")
                && payload.get("reason").and_then(Value::as_str)
                    == Some("missing_recovery_intent")
        }));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn ra2_recovery_restores_sandbox_overrides_from_snapshot() {
        let ctx = test_ctx();
        {
            let conn = ctx.db.conn();
            seed_session(&conn);
            seed_crashed_agent(&conn, "ra2_bind_snapshot", "codex", true);
            let mut spec = sample_spawn_spec("ra2_bind_snapshot", "codex");
            spec.sandbox_overrides = crate::sandbox::SandboxOverrides {
                extra_ro_binds: vec![crate::sandbox::ReadOnlyBind {
                    host_path: "/opt/keys".to_string(),
                    sandbox_path: "/mnt/keys".to_string(),
                }],
            };
            persist_agent_spawn_spec_sync(&conn, &spec, "hash1").unwrap();
        }

        recover_crashed_agent_from_snapshot_with_respawn(
            &ctx,
            "ra2_bind_snapshot",
            fake_recovery_respawn_requires_ro_bind,
        )
        .await
        .unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn ra2_recovered_event_includes_crash_forensics_from_deleted_row() {
        let ctx = test_ctx();
        {
            let conn = ctx.db.conn();
            seed_session(&conn);
            seed_crashed_agent(&conn, "ra2_crash_forensics", "codex", true);
            conn.execute(
                "UPDATE agents \
                 SET exit_code = 137, error_code = 'AGENT_UNEXPECTED_EXIT' \
                 WHERE id = 'ra2_crash_forensics'",
                [],
            )
            .unwrap();
        }

        recover_crashed_agent_from_snapshot_with_respawn(
            &ctx,
            "ra2_crash_forensics",
            fake_recovery_respawn_ok,
        )
        .await
        .unwrap();

        let payload = recovery_events(&ctx.db.conn(), "ra2_crash_forensics")
            .into_iter()
            .find(|payload| payload.get("action").and_then(Value::as_str) == Some("recovered"))
            .unwrap();
        assert_eq!(payload["recovered_from_exit_code"], 137);
        assert_eq!(
            payload["recovered_from_error_code"],
            "AGENT_UNEXPECTED_EXIT"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn ra2_recovery_loop_processes_at_most_one_candidate_per_tick() {
        let ctx = test_ctx();
        {
            let conn = ctx.db.conn();
            seed_session(&conn);
            seed_crashed_agent(&conn, "ra2_one", "codex", true);
            seed_crashed_agent(&conn, "ra2_two", "claude", true);
            persist_agent_recovery_intent_sync(
                &conn,
                &sample_recovery_intent("ra2_one", RecoveryIntentAction::Revive, None),
            )
            .unwrap();
            persist_agent_recovery_intent_sync(
                &conn,
                &sample_recovery_intent("ra2_two", RecoveryIntentAction::Revive, None),
            )
            .unwrap();
        }

        assert!(
            run_recovery_once_with_respawn(&ctx, fake_recovery_respawn_ok)
                .await
                .unwrap()
        );
        let conn = ctx.db.conn();
        let recovered_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM events WHERE event_type = 'self_recovery_attempt' AND payload LIKE '%\"action\":\"recovered\"%'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(recovered_count, 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn ra2_missing_snapshot_emits_skipped_without_cas_and_stays_crashed() {
        let ctx = test_ctx();
        {
            let conn = ctx.db.conn();
            seed_session(&conn);
            seed_crashed_agent(&conn, "ra2_missing_snapshot", "codex", false);
            persist_agent_recovery_intent_sync(
                &conn,
                &sample_recovery_intent(
                    "ra2_missing_snapshot",
                    RecoveryIntentAction::Revive,
                    None,
                ),
            )
            .unwrap();
        }

        assert!(!run_recovery_once(&ctx).await.unwrap());
        assert!(!run_recovery_once(&ctx).await.unwrap());
        let conn = ctx.db.conn();
        let row: (String, i64) = conn
            .query_row(
                "SELECT state, state_version FROM agents WHERE id = 'ra2_missing_snapshot'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(row, ("CRASHED".to_string(), 7));
        let events = recovery_events(&conn, "ra2_missing_snapshot");
        assert_eq!(events.len(), 1);
        let payload = &events[0];
        assert_eq!(
            payload.get("action").and_then(Value::as_str),
            Some("skipped")
        );
        assert_eq!(
            payload.get("reason").and_then(Value::as_str),
            Some("missing_spawn_spec")
        );
        assert_eq!(recovery_event_count(&conn, "ra2_missing_snapshot"), 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn ra2_missing_snapshot_does_not_block_later_recoverable_agent() {
        let ctx = test_ctx();
        {
            let conn = ctx.db.conn();
            seed_session(&conn);
            seed_crashed_agent(&conn, "ra2_a_missing_snapshot", "codex", false);
            seed_crashed_agent(&conn, "ra2_z_due", "codex", true);
            persist_agent_recovery_intent_sync(
                &conn,
                &sample_recovery_intent(
                    "ra2_a_missing_snapshot",
                    RecoveryIntentAction::Revive,
                    None,
                ),
            )
            .unwrap();
            persist_agent_recovery_intent_sync(
                &conn,
                &sample_recovery_intent("ra2_z_due", RecoveryIntentAction::Revive, None),
            )
            .unwrap();
        }

        assert!(
            run_recovery_once_with_respawn(&ctx, fake_recovery_respawn_ok)
                .await
                .unwrap()
        );
        let conn = ctx.db.conn();
        let missing_state: (String, i64) = conn
            .query_row(
                "SELECT state, state_version FROM agents WHERE id = 'ra2_a_missing_snapshot'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        let due_state: String = conn
            .query_row(
                "SELECT state FROM agents WHERE id = 'ra2_z_due'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(missing_state, ("CRASHED".to_string(), 7));
        assert_eq!(recovery_event_count(&conn, "ra2_a_missing_snapshot"), 1);
        assert_eq!(due_state, "IDLE");
        assert!(
            recovery_events(&conn, "ra2_z_due").iter().any(|payload| {
                payload.get("action").and_then(Value::as_str) == Some("recovered")
            })
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn ra2_respawn_failure_keeps_crashed_bumps_backoff_and_emits_failed() {
        let ctx = test_ctx();
        {
            let conn = ctx.db.conn();
            seed_session(&conn);
            seed_crashed_agent(&conn, "ra2_respawn_fail", "codex", true);
        }

        apply_recovery_spawn_result(
            &ctx,
            "ra2_respawn_fail",
            Err(CcbdError::PtyIoError("injected spawn failed".to_string())),
            1_000,
            None,
        )
        .await
        .unwrap();
        let conn = ctx.db.conn();
        let row: (String, i64, i64) = conn
            .query_row(
                "SELECT state, retry_count, retry_exhausted FROM agents WHERE id = 'ra2_respawn_fail'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(row, ("CRASHED".to_string(), 1, 0));
        assert!(
            recovery_events(&conn, "ra2_respawn_fail")
                .iter()
                .any(|payload| { payload.get("action").and_then(Value::as_str) == Some("failed") })
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn ra2_retry_exhausted_and_future_backoff_are_not_selected() {
        let ctx = test_ctx();
        {
            let conn = ctx.db.conn();
            seed_session(&conn);
            seed_crashed_agent(&conn, "ra2_exhausted", "codex", true);
            seed_crashed_agent(&conn, "ra2_future", "claude", true);
            conn.execute(
                "UPDATE agents SET retry_exhausted = 1 WHERE id = 'ra2_exhausted'",
                [],
            )
            .unwrap();
            conn.execute(
                "UPDATE agents SET next_retry_at = unixepoch() + 3600 WHERE id = 'ra2_future'",
                [],
            )
            .unwrap();
        }

        assert!(!run_recovery_once(&ctx).await.unwrap());
        let conn = ctx.db.conn();
        assert!(recovery_events(&conn, "ra2_exhausted").is_empty());
        assert!(recovery_events(&conn, "ra2_future").is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn ra2_delete_then_spawn_failure_restores_minimal_crashed_row_and_snapshot() {
        let ctx = test_ctx();
        {
            let conn = ctx.db.conn();
            seed_session(&conn);
            seed_crashed_agent(&conn, "ra2_restore_window", "codex", true);
        }

        recover_crashed_agent_from_snapshot_with_respawn(
            &ctx,
            "ra2_restore_window",
            fake_recovery_respawn_err,
        )
        .await
        .unwrap();
        let conn = ctx.db.conn();
        let state: String = conn
            .query_row(
                "SELECT state FROM agents WHERE id = 'ra2_restore_window'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let spec_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM agent_spawn_specs WHERE agent_id = 'ra2_restore_window'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(state, "CRASHED");
        assert_eq!(spec_count, 1);
        let retry_count: i64 = conn
            .query_row(
                "SELECT retry_count FROM agents WHERE id = 'ra2_restore_window'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(retry_count, 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn ra2_self_recovery_attempt_event_payload_contract() {
        let ctx = test_ctx();
        {
            let conn = ctx.db.conn();
            seed_session(&conn);
            seed_crashed_agent(&conn, "ra2_event", "codex", true);
        }

        apply_recovery_spawn_result(
            &ctx,
            "ra2_event",
            Err(CcbdError::PtyIoError("spawn failed".to_string())),
            1_000,
            None,
        )
        .await
        .unwrap();
        let payload = recovery_events(&ctx.db.conn(), "ra2_event")
            .into_iter()
            .next()
            .unwrap();
        assert_eq!(payload["agent_id"], "ra2_event");
        assert_eq!(payload["reason"], "OOM_RECOVERY");
        assert_eq!(payload["action"], "failed");
        assert_eq!(payload["retry_count"], 1);
        assert_eq!(payload["next_retry_at"], 1001);
        assert_eq!(payload["state_version"], 8);
        assert_eq!(payload["error"], "spawn failed");
    }
}
