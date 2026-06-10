use crate::error::CcbdError;
use crate::marker::{MarkerMatcher, MatchResult, parser_registry, registry};
use crate::pane_diff::is_meaningful_diff;
use rusqlite::OptionalExtension;
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;

pub(crate) const CAPTURE_SEED_POLL_MS: u64 = 50;
pub(crate) const CAPTURE_SEED_STABILITY_MS: u64 = 500;
pub(crate) const ACK_IDLE_SCAN_REOPEN_DELAY_MS: u64 = 2_000;

pub(crate) fn spawn_new_capture_seed(
    db: crate::db::Db,
    tmux: Arc<crate::tmux::TmuxServer>,
    agent_id: String,
    provider: String,
    state_dir: std::path::PathBuf,
    baseline: String,
    matcher: Arc<MarkerMatcher>,
) {
    tokio::spawn(async move {
        let allow_direct_idle =
            matcher.mode() != crate::provider::manifest::IdleDetectionMode::ObservedStability;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        let mut processed_len = 0_usize;
        let stability = Duration::from_millis(CAPTURE_SEED_STABILITY_MS);
        let ack_started_at = tokio::time::Instant::now();
        let mut busy_marked = false;
        let mut last_meaningful_diff_at: Option<tokio::time::Instant> = None;
        while tokio::time::Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(CAPTURE_SEED_POLL_MS)).await;
            let Some(pane_id) = crate::agent_io::pane_id(&agent_id) else {
                if let Err(err) =
                    fallback_ack_to_crashed(db.clone(), &agent_id, "pane_unregistered_during_ack")
                        .await
                {
                    tracing::warn!(agent_id = %agent_id, error = %err, "failed to mark ACK fallback CRASHED after pane unregister");
                }
                return;
            };
            let Some(parser_handle) = parser_registry::get(&agent_id) else {
                if let Err(err) =
                    fallback_ack_to_crashed(db.clone(), &agent_id, "reader_unregistered_during_ack")
                        .await
                {
                    tracing::warn!(agent_id = %agent_id, error = %err, "failed to mark ACK fallback CRASHED after reader unregister");
                }
                return;
            };
            let Ok(capture) = tmux.capture_pane(pane_id.clone()).await else {
                if let Err(err) =
                    fallback_ack_to_stuck(db.clone(), &agent_id, "tmux_capture_failed_during_ack")
                        .await
                {
                    tracing::warn!(agent_id = %agent_id, error = %err, "failed to mark ACK fallback STUCK after capture failure");
                }
                return;
            };
            let now = tokio::time::Instant::now();
            if !is_meaningful_diff(&baseline, &capture) {
                if !busy_marked && now.duration_since(ack_started_at) >= stability {
                    if let Err(err) = crate::db::state_machine::transit_agent_state(
                        db.clone(),
                        agent_id.clone(),
                        vec![crate::db::state_machine::STATE_WAITING_FOR_ACK.to_string()],
                        crate::db::state_machine::STATE_BUSY.to_string(),
                        Some("ACK_STABILITY_WINDOW".to_string()),
                    )
                    .await
                    {
                        tracing::warn!(agent_id = %agent_id, error = %err, "failed to mark agent BUSY after ACK stability window");
                    }
                    busy_marked = true;
                }
                if last_meaningful_diff_at
                    .is_some_and(|last_change| now.duration_since(last_change) >= stability)
                {
                    return;
                }
                continue;
            }
            let first_meaningful_diff = last_meaningful_diff_at.is_none();
            last_meaningful_diff_at = Some(now);
            if first_meaningful_diff && !busy_marked {
                if crate::prompt_handler::integration::is_prompt_handling_provider(&provider) {
                    match crate::prompt_handler::integration::scan_prompt_and_apply_outcome(
                        crate::prompt_handler::integration::PromptScanRequest {
                            db: db.clone(),
                            agent_id: agent_id.clone(),
                            provider: provider.clone(),
                            pane_id: pane_id.clone(),
                            tmux: tmux.clone(),
                            state_dir: state_dir.clone(),
                            marker_matcher: matcher.clone(),
                            max_depth: 3,
                            scan_purpose:
                                crate::prompt_handler::PromptScanPurpose::AckVisualDiff,
                        },
                    )
                    .await
                    {
                        Ok(crate::prompt_handler::integration::PromptScanDisposition::Handled {
                            depth,
                        }) => {
                            tracing::info!(
                                agent_id = %agent_id,
                                depth,
                                "prompt scan auto-handled prompt during ACK visual diff; continuing ACK loop"
                            );
                            processed_len = 0;
                            last_meaningful_diff_at = None;
                            continue;
                        }
                        Ok(crate::prompt_handler::integration::PromptScanDisposition::Pending {
                            depth,
                            block_reason,
                        }) => {
                            tracing::info!(
                                agent_id = %agent_id,
                                depth,
                                block_reason,
                                "prompt scan moved agent to PROMPT_PENDING during ACK visual diff"
                            );
                            if let Some(handle) = registry::take(&agent_id) {
                                let _ = handle.cancel_tx.send(());
                            }
                            crate::orchestrator::wake_up();
                            return;
                        }
                        Ok(crate::prompt_handler::integration::PromptScanDisposition::Deferred {
                            depth,
                            block_reason,
                        }) => {
                            tracing::info!(
                                agent_id = %agent_id,
                                depth,
                                block_reason,
                                "prompt scan deferred during ACK visual diff; resuming ACK handling"
                            );
                        }
                        Ok(crate::prompt_handler::integration::PromptScanDisposition::NoActionNeeded {
                            ..
                        }) => {
                            tracing::info!(
                                agent_id = %agent_id,
                                "prompt scan found no prompt during ACK visual diff; resuming ACK handling"
                            );
                        }
                        Err(err) => {
                            tracing::warn!(
                                agent_id = %agent_id,
                                reason = %err,
                                impact = "prompt scan failed; preserving existing ACK visual diff behavior",
                                "prompt scan failed during ACK visual diff"
                            );
                        }
                    }
                }
                if let Err(err) = crate::db::state_machine::transit_agent_state(
                    db.clone(),
                    agent_id.clone(),
                    vec![crate::db::state_machine::STATE_WAITING_FOR_ACK.to_string()],
                    crate::db::state_machine::STATE_BUSY.to_string(),
                    Some("ACK_VISUAL_DIFF".to_string()),
                )
                .await
                {
                    tracing::warn!(agent_id = %agent_id, error = %err, "failed to mark agent BUSY after ACK visual diff");
                }
                tokio::time::sleep(Duration::from_millis(ACK_IDLE_SCAN_REOPEN_DELAY_MS)).await;
                crate::agent_io::set_idle_scan_enabled(&agent_id, true);
                busy_marked = true;
                let matched_after_reopen = match parser_handle.lock() {
                    Ok(parser) => matcher.scan(&parser),
                    Err(err) => {
                        tracing::warn!(agent_id = %agent_id, error = %err, "parser mutex poisoned while reopening idle scan after ACK");
                        MatchResult::NoMatch
                    }
                };
                if matched_after_reopen == MatchResult::Matched {
                    match crate::db::state_machine::mark_agent_idle_matched(
                        db.clone(),
                        agent_id.clone(),
                    )
                    .await
                    {
                        Ok((changes, affected_job)) if changes > 0 => {
                            // R-2 (mvp12): notify hoisted from state_machine wrapper for clearer dispatcher boundary.
                            if let Some(job_id) = affected_job {
                                crate::orchestrator::pubsub::notify_job_update(&job_id);
                            }
                            crate::orchestrator::wake_up();
                        }
                        Ok(_) => {}
                        Err(err) => {
                            tracing::warn!(agent_id = %agent_id, error = %err, "failed to mark agent IDLE after reopening idle scan");
                        }
                    }
                    if let Some(handle) = registry::take(&agent_id) {
                        let _ = handle.cancel_tx.send(());
                    }
                    return;
                }
                continue;
            }
            if !capture.starts_with(&baseline) && !capture.contains(&baseline) {
                let mut parser = vt100::Parser::new(200, 200, 0);
                parser.process(capture.as_bytes());
                if allow_direct_idle && capture_seed_matches(&parser, &matcher) {
                    match crate::db::state_machine::mark_agent_idle_matched(
                        db.clone(),
                        agent_id.clone(),
                    )
                    .await
                    {
                        Ok((changes, affected_job)) if changes > 0 => {
                            // R-2 (mvp12): notify hoisted from state_machine wrapper for clearer dispatcher boundary.
                            if let Some(job_id) = affected_job {
                                crate::orchestrator::pubsub::notify_job_update(&job_id);
                            }
                            crate::orchestrator::wake_up();
                        }
                        Ok(_) => {}
                        Err(err) => {
                            tracing::warn!(agent_id = %agent_id, error = %err, "failed to mark agent IDLE from replacement tmux capture seed");
                        }
                    }
                    if let Some(handle) = registry::take(&agent_id) {
                        let _ = handle.cancel_tx.send(());
                    }
                    return;
                }
                continue;
            }
            let Some(suffix) = capture.strip_prefix(&baseline) else {
                if let Err(err) = fallback_ack_to_stuck(
                    db.clone(),
                    &agent_id,
                    "capture_baseline_mismatch_during_ack",
                )
                .await
                {
                    tracing::warn!(agent_id = %agent_id, error = %err, "failed to mark ACK fallback STUCK after baseline mismatch");
                }
                return;
            };
            if suffix.len() <= processed_len {
                continue;
            }
            let delta = &suffix[processed_len..];
            processed_len = suffix.len();
            let matched = {
                match parser_handle.lock() {
                    Ok(mut parser) => {
                        parser.process(delta.as_bytes());
                        if allow_direct_idle {
                            matcher.scan(&parser)
                        } else {
                            MatchResult::NoMatch
                        }
                    }
                    Err(err) => {
                        tracing::warn!(agent_id = %agent_id, error = %err, "parser mutex poisoned during new tmux capture seed");
                        MatchResult::NoMatch
                    }
                }
            };
            if matched == MatchResult::Matched {
                match crate::db::state_machine::mark_agent_idle_matched(
                    db.clone(),
                    agent_id.clone(),
                )
                .await
                {
                    Ok((changes, affected_job)) if changes > 0 => {
                        // R-2 (mvp12): notify hoisted from state_machine wrapper for clearer dispatcher boundary.
                        if let Some(job_id) = affected_job {
                            crate::orchestrator::pubsub::notify_job_update(&job_id);
                        }
                        crate::orchestrator::wake_up();
                    }
                    Ok(_) => {}
                    Err(err) => {
                        tracing::warn!(agent_id = %agent_id, error = %err, "failed to mark agent IDLE from new tmux capture seed");
                    }
                }
                if let Some(handle) = registry::take(&agent_id) {
                    let _ = handle.cancel_tx.send(());
                }
                return;
            }
        }
        if let Err(err) = fallback_ack_to_stuck(db, &agent_id, "ack_deadline_timeout").await {
            tracing::warn!(agent_id = %agent_id, error = %err, "failed to mark ACK fallback STUCK after capture seed deadline");
        }
    });
}

#[doc(hidden)]
pub async fn fallback_ack_to_stuck(
    db: crate::db::Db,
    agent_id: &str,
    reason: &str,
) -> Result<usize, CcbdError> {
    let agent_id = agent_id.to_string();
    let reason = reason.to_string();
    crate::db::common::spawn_db("handlers::fallback_ack_to_stuck", move || {
        let mut conn = db.conn();
        let tx = conn
            .transaction()
            .map_err(|err| crate::db::common::map_db_error("begin ACK stuck fallback", err))?;
        let current = tx
            .query_row(
                "SELECT state, state_version FROM agents WHERE id = ?",
                rusqlite::params![agent_id.as_str()],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
            )
            .optional()
            .map_err(|err| crate::db::common::map_db_error("query ACK fallback state", err))?;
        let Some((current_state, state_version)) = current else {
            tx.rollback()
                .map_err(|err| crate::db::common::map_db_error("rollback missing ACK fallback", err))?;
            return Ok(0);
        };
        if current_state != crate::db::state_machine::STATE_WAITING_FOR_ACK {
            tx.rollback()
                .map_err(|err| crate::db::common::map_db_error("rollback ignored ACK fallback", err))?;
            return Ok(0);
        }

        let changes = tx
            .execute(
                "UPDATE agents SET state = 'STUCK', state_version = state_version + 1, updated_at = unixepoch() WHERE id = ? AND state = 'WAITING_FOR_ACK' AND state_version = ?",
                rusqlite::params![agent_id.as_str(), state_version],
            )
            .map_err(|err| crate::db::common::map_db_error("mark ACK fallback stuck", err))?;
        if changes == 1 {
            let payload = json!({
                "from": current_state,
                "to": crate::db::state_machine::STATE_STUCK,
                "reason": reason,
            })
            .to_string();
            tx.execute(
                "INSERT INTO events (agent_id, request_id, event_type, payload) VALUES (?, NULL, 'state_change', ?)",
                rusqlite::params![agent_id.as_str(), payload],
            )
            .map_err(|err| crate::db::common::map_db_error("insert ACK fallback stuck state_change", err))?;
        }
        tx.commit()
            .map_err(|err| crate::db::common::map_db_error("commit ACK stuck fallback", err))?;
        Ok(changes)
    })
    .await
}

#[doc(hidden)]
pub async fn fallback_ack_to_crashed(
    db: crate::db::Db,
    agent_id: &str,
    reason: &str,
) -> Result<usize, CcbdError> {
    let state = crate::db::agents::query_agent_state(db.clone(), agent_id.to_string()).await?;
    if state
        .as_ref()
        .is_none_or(|(state, _)| state != crate::db::state_machine::STATE_WAITING_FOR_ACK)
    {
        return Ok(0);
    }
    crate::db::agents_lifecycle::mark_agent_crashed_with_reason(
        db,
        agent_id.to_string(),
        None,
        reason.to_string(),
    )
    .await
}

pub(super) fn capture_seed_matches(parser: &vt100::Parser, matcher: &MarkerMatcher) -> bool {
    if matcher.mode() == crate::provider::manifest::IdleDetectionMode::ObservedStability {
        return false;
    }
    matcher.scan(parser) == MatchResult::Matched
}
