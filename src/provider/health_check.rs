use crate::db::state_machine::{STATE_BUSY, STATE_SPAWNING, STATE_WAITING_FOR_ACK};
use crate::error::CcbdError;
use crate::provider::manifest::get_manifest;
use crate::rpc::Ctx;
use serde_json::{Value, json};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HealthCheckResult {
    pub alive: bool,
    pub dead_layers: Vec<String>,
    pub last_progress_ts: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HealthCheckObservation {
    pub agent_id: String,
    pub provider: String,
    pub state: String,
    pub pane_capture: String,
    pub pane_capture_ok: bool,
    pub last_output_ts: Option<i64>,
    pub last_marker_ts: Option<i64>,
    pub now_ts: i64,
}

pub fn health_check_observe(
    observation: &HealthCheckObservation,
    stuck_threshold_secs: i64,
) -> HealthCheckResult {
    let last_progress_ts = observation
        .last_marker_ts
        .or(observation.last_output_ts)
        .unwrap_or(0);
    let mut dead_layers = Vec::new();

    if !observation.pane_capture_ok {
        dead_layers.push("tmux".to_string());
    }

    if observation.state == STATE_SPAWNING {
        let probe = get_manifest(&observation.provider).init_probe.build();
        if !probe.detect(&observation.pane_capture) {
            dead_layers.push("predicate".to_string());
        }
    }

    if is_working_state(&observation.state)
        && last_progress_ts > 0
        && observation.now_ts.saturating_sub(last_progress_ts) > stuck_threshold_secs
    {
        dead_layers.push("completion".to_string());
    }

    HealthCheckResult {
        alive: dead_layers.is_empty(),
        dead_layers,
        last_progress_ts,
    }
}

pub async fn escalate_health_stuck(
    ctx: &Ctx,
    observation: &HealthCheckObservation,
    stuck_threshold_secs: i64,
) -> Result<usize, CcbdError> {
    let result = health_check_observe(observation, stuck_threshold_secs);
    if result.alive || !is_active_state(&observation.state) {
        return Ok(0);
    }

    let changes =
        crate::db::state_machine::mark_agent_stuck(ctx.db.clone(), observation.agent_id.clone())
            .await?;
    if changes == 0 {
        return Ok(0);
    }

    let job_id = crate::db::jobs::query_dispatched_job_for_agent(
        ctx.db.clone(),
        observation.agent_id.clone(),
    )
    .await?
    .map(|job| job.id);
    let signal_kinds: Vec<String> = result
        .dead_layers
        .iter()
        .map(|layer| format!("health:{layer}"))
        .collect();
    let payload = json!({
        "from": observation.state,
        "to": "STUCK",
        "reason": "HEALTH_CHECK_STUCK",
        "job_id": job_id,
        "signal_kinds": signal_kinds,
        "elapsed_secs": observation.now_ts.saturating_sub(result.last_progress_ts),
    });
    let _ = crate::db::events::insert_event(
        ctx.db.clone(),
        observation.agent_id.clone(),
        None,
        "state_change".into(),
        payload.to_string(),
    )
    .await?;
    crate::orchestrator::pubsub::notify_event(crate::orchestrator::pubsub::EventFrame {
        event_id: 0,
        kind: "stuck".to_string(),
        agent_id: observation.agent_id.clone(),
        job_id: job_id.clone(),
        state: Some("STUCK".to_string()),
        ts_unix_micro: now_unix_micro(),
        payload: Some(json!({
            "job_id": job_id,
            "signal_kinds": signal_kinds,
            "elapsed_secs": observation.now_ts.saturating_sub(result.last_progress_ts),
        })),
    });
    Ok(changes)
}

pub async fn health_check_watcher_loop(ctx: Ctx, interval: Duration, stuck_threshold: Duration) {
    loop {
        tokio::time::sleep(interval).await;
        if let Err(err) = health_check_watcher_tick(&ctx, stuck_threshold).await {
            tracing::warn!(error = %err, "health check watcher tick failed");
        }
    }
}

async fn health_check_watcher_tick(ctx: &Ctx, stuck_threshold: Duration) -> Result<(), CcbdError> {
    for state in [STATE_SPAWNING, STATE_WAITING_FOR_ACK, STATE_BUSY] {
        let agents =
            crate::db::agents::query_agents_by_state(ctx.db.clone(), state.to_string()).await?;
        for agent in agents {
            let (pane_capture_ok, pane_capture) =
                if let Some(pane_id) = crate::agent_io::pane_id(&agent.id) {
                    match ctx.tmux_server.capture_pane(pane_id).await {
                        Ok(capture) => (true, capture),
                        Err(_) => (false, String::new()),
                    }
                } else {
                    (false, String::new())
                };
            let (last_output_ts, last_marker_ts) =
                query_last_progress(ctx.db.clone(), agent.id.clone()).await?;
            let observation = HealthCheckObservation {
                agent_id: agent.id,
                provider: agent.provider,
                state: agent.state,
                pane_capture,
                pane_capture_ok,
                last_output_ts,
                last_marker_ts,
                now_ts: now_unix_secs(),
            };
            let _ =
                escalate_health_stuck(ctx, &observation, stuck_threshold.as_secs() as i64).await?;
        }
    }
    Ok(())
}

async fn query_last_progress(
    db: crate::db::Db,
    agent_id: String,
) -> Result<(Option<i64>, Option<i64>), CcbdError> {
    let events = crate::db::events::query_events_since(db, agent_id, 0).await?;
    let mut last_output = None;
    let mut last_marker = None;
    for event in events {
        if event.event_type == "output_chunk" {
            last_output = Some(event.created_at);
            let payload: Value = serde_json::from_str(&event.payload).unwrap_or_else(|_| json!({}));
            if payload
                .get("text")
                .and_then(Value::as_str)
                .is_some_and(|text| text.contains("<<ah-idle:job-id="))
            {
                last_marker = Some(event.created_at);
            }
        }
    }
    Ok((last_output, last_marker))
}

fn is_active_state(state: &str) -> bool {
    matches!(state, STATE_SPAWNING | STATE_WAITING_FOR_ACK | STATE_BUSY)
}

fn is_working_state(state: &str) -> bool {
    matches!(state, STATE_WAITING_FOR_ACK | STATE_BUSY)
}

fn now_unix_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

fn now_unix_micro() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_micros() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::{HealthCheckObservation, health_check_observe};

    fn observation() -> HealthCheckObservation {
        HealthCheckObservation {
            agent_id: "dogfood_a1".into(),
            provider: "bash".into(),
            state: "BUSY".into(),
            pane_capture: "$ ".into(),
            pane_capture_ok: true,
            last_output_ts: Some(100),
            last_marker_ts: Some(100),
            now_ts: 105,
        }
    }

    #[test]
    fn test_health_check_three_layers_alive() {
        let result = health_check_observe(&observation(), 300);
        assert!(result.alive, "all layers ok should be alive");
        assert!(result.dead_layers.is_empty());
    }

    #[test]
    fn test_health_check_pane_dead() {
        let mut obs = observation();
        obs.pane_capture_ok = false;
        let result = health_check_observe(&obs, 300);
        assert_eq!(result.dead_layers, ["tmux"]);
    }

    #[test]
    fn test_health_check_predicate_dead() {
        let mut obs = observation();
        obs.state = "SPAWNING".into();
        obs.pane_capture = "Welcome to Claude Code\nSetup Wizard".into();
        obs.provider = "claude".into();
        let result = health_check_observe(&obs, 300);
        assert_eq!(result.dead_layers, ["predicate"]);
    }

    #[test]
    fn test_health_check_completion_stale() {
        let mut obs = observation();
        obs.last_output_ts = Some(1);
        obs.last_marker_ts = Some(1);
        obs.now_ts = 400;
        let result = health_check_observe(&obs, 300);
        assert_eq!(result.dead_layers, ["completion"]);
    }
}
