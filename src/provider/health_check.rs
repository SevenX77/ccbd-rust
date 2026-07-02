use crate::db::state_machine::{STATE_BUSY, STATE_SPAWNING, STATE_WAITING_FOR_ACK};
use crate::error::CcbdError;
use crate::marker::matcher::{MarkerMatcher, MatchResult};
use crate::provider::manifest::{CompletionSignalKind, get_manifest};
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
    /// Unix-seconds dispatch time of the agent's current DISPATCHED job, if any.
    /// The completion-staleness window is floored to this so an agent that sat
    /// idle for a long time before being re-dispatched is judged only on
    /// stagnation *since the current task started* — not on pre-dispatch idle.
    pub dispatched_at: Option<i64>,
    pub now_ts: i64,
}

pub fn health_check_observe(
    observation: &HealthCheckObservation,
    stuck_threshold_secs: i64,
) -> HealthCheckResult {
    health_check_observe_with_completion_signal(observation, stuck_threshold_secs, |provider| {
        get_manifest(provider).completion_signal
    })
}

fn health_check_observe_with_completion_signal(
    observation: &HealthCheckObservation,
    stuck_threshold_secs: i64,
    completion_signal: impl FnOnce(&str) -> CompletionSignalKind,
) -> HealthCheckResult {
    // Floor the progress baseline to the current job's dispatch time. Without this,
    // an agent that sat idle for a long time and is then freshly re-dispatched would
    // be judged "completion layer dead" the instant it goes BUSY — because its last
    // output/marker predates the new task by more than the threshold (G0 false-STUCK).
    // The staleness window must only count stagnation *since the current task started*.
    let last_progress_ts = observation
        .last_marker_ts
        .or(observation.last_output_ts)
        .unwrap_or(0)
        .max(observation.dispatched_at.unwrap_or(0));
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
        && completion_signal(&observation.provider) != CompletionSignalKind::UiOnly
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

    if result.dead_layers.iter().any(|layer| layer == "completion")
        && observation.pane_capture_ok
        && get_manifest(&observation.provider).completion_signal == CompletionSignalKind::LogAndUi
    {
        let manifest = get_manifest(&observation.provider);
        let matcher = MarkerMatcher::from_manifest(&manifest);
        let mut parser = vt100::Parser::new(200, 200, 0);
        parser.process(observation.pane_capture.as_bytes());
        let pane_matches_idle = matcher.scan(&parser) == MatchResult::Matched;
        tracing::info!(
            agent_id = %observation.agent_id,
            provider = %observation.provider,
            dispatched_at = observation.dispatched_at,
            last_output_ts = observation.last_output_ts,
            last_marker_ts = observation.last_marker_ts,
            pane_matches_idle,
            "health completion stale candidate checked pane recapture before STUCK"
        );
        if pane_matches_idle {
            let recapture =
                crate::db::state_machine::mark_agent_idle_recaptured_health_check_with_pane(
                    ctx.db.clone(),
                    observation.agent_id.clone(),
                    observation.pane_capture.clone(),
                )
                .await?;
            if recapture.changes > 0 {
                tracing::info!(
                    agent_id = %observation.agent_id,
                    provider = %observation.provider,
                    disposition = ?recapture.disposition,
                    affected_job = ?recapture.affected_job,
                    "health completion stale candidate resolved by pane recapture"
                );
                return Ok(0);
            }
        }
    }

    let changes =
        crate::db::state_machine::mark_agent_stuck(ctx.db.clone(), observation.agent_id.clone())
            .await?;
    if changes == 0 {
        return Ok(0);
    }
    tracing::info!(
        agent_id = %observation.agent_id,
        provider = %observation.provider,
        from = %observation.state,
        dead_layers = ?result.dead_layers,
        elapsed_secs = observation.now_ts.saturating_sub(result.last_progress_ts),
        dispatched_at = observation.dispatched_at,
        last_output_ts = observation.last_output_ts,
        last_marker_ts = observation.last_marker_ts,
        "health check marked agent STUCK"
    );

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
            let dispatched_at =
                crate::db::jobs::query_dispatched_job_for_agent(ctx.db.clone(), agent.id.clone())
                    .await?
                    .and_then(|job| job.dispatched_at);
            let observation = HealthCheckObservation {
                agent_id: agent.id,
                provider: agent.provider,
                state: agent.state,
                pane_capture,
                pane_capture_ok,
                last_output_ts,
                last_marker_ts,
                dispatched_at,
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
    let last_output = crate::db::events::query_last_event_of_type(
        db.clone(),
        agent_id.clone(),
        "output_chunk".to_string(),
    )
    .await?
    .map(|event| event.created_at);

    let marker_like = format!("%{}%", crate::db::events::AH_IDLE_MARKER_PREFIX);
    let mut before_seq_id = None;
    let last_marker = loop {
        let Some(event) = crate::db::events::query_last_event_of_type_matching_payload(
            db.clone(),
            agent_id.clone(),
            "output_chunk".to_string(),
            marker_like.clone(),
            before_seq_id,
        )
        .await?
        else {
            break None;
        };
        before_seq_id = Some(event.seq_id);
        let payload: Value = serde_json::from_str(&event.payload).unwrap_or_else(|_| json!({}));
        if payload
            .get("text")
            .and_then(Value::as_str)
            .is_some_and(|text| text.contains(crate::db::events::AH_IDLE_MARKER_PREFIX))
        {
            break Some(event.created_at);
        }
    };
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
    use super::{
        HealthCheckObservation, escalate_health_stuck, health_check_observe,
        health_check_observe_with_completion_signal, query_last_progress,
    };
    use crate::db::agents::insert_agent_sync;
    use crate::db::events::insert_event_sync;
    use crate::db::init;
    use crate::db::jobs::{insert_job_sync, query_job_sync, update_dispatched_seq_id_sync};
    use crate::db::sessions::insert_session_sync;
    use crate::provider::manifest::CompletionSignalKind;
    use crate::rpc::Ctx;
    use crate::sandbox::EnvState;
    use crate::tmux::TmuxServer;
    use rusqlite::params;
    use std::sync::Arc;
    use std::time::Instant;

    fn observation() -> HealthCheckObservation {
        HealthCheckObservation {
            agent_id: "dogfood_a1".into(),
            provider: "bash".into(),
            state: "BUSY".into(),
            pane_capture: "$ ".into(),
            pane_capture_ok: true,
            last_output_ts: Some(100),
            last_marker_ts: Some(100),
            dispatched_at: None,
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

    #[test]
    fn test_health_check_does_not_false_stuck_on_recent_redispatch_despite_stale_progress() {
        // G0 regression: a LogAndUi agent that sat idle for a long time (very old
        // last_output/last_marker) and was then freshly re-dispatched must NOT be
        // marked completion-layer dead. The staleness window is floored to the
        // current job's dispatch time, so only post-dispatch stagnation counts.
        let mut obs = observation();
        obs.provider = "codex".into();
        obs.last_output_ts = Some(1); // last progress ages ago (pre-dispatch idle)
        obs.last_marker_ts = Some(1);
        obs.dispatched_at = Some(390); // but the current job was dispatched just now
        obs.now_ts = 400; // only 10s since dispatch — well under threshold
        let result = health_check_observe(&obs, 300);
        assert!(
            result.dead_layers.is_empty(),
            "freshly re-dispatched agent must not be STUCK from pre-dispatch idle (G0); got {:?}",
            result.dead_layers
        );
    }

    #[test]
    fn test_health_check_completion_stale_when_dispatched_long_ago_without_progress() {
        // Genuine stuck must still fire: dispatched long ago AND no progress since.
        let mut obs = observation();
        obs.provider = "codex".into();
        obs.last_output_ts = Some(1);
        obs.last_marker_ts = Some(1);
        obs.dispatched_at = Some(1); // dispatched long ago too
        obs.now_ts = 400;
        let result = health_check_observe(&obs, 300);
        assert_eq!(
            result.dead_layers,
            ["completion"],
            "genuine post-dispatch stagnation must still mark completion layer dead"
        );
    }

    #[test]
    fn test_health_check_does_not_completion_stale_ui_only_provider() {
        let mut obs = observation();
        obs.provider = "synthetic-ui-only".into();
        obs.last_output_ts = Some(1);
        obs.last_marker_ts = Some(1);
        obs.now_ts = 400;
        let result = health_check_observe_with_completion_signal(&obs, 300, |_| {
            CompletionSignalKind::UiOnly
        });
        assert!(
            result.dead_layers.is_empty(),
            "UiOnly providers must leave stale output completion judgment to pane_diff recapture"
        );
    }

    fn seed_agent(conn: &rusqlite::Connection, agent_id: &str) {
        insert_agent_sync(conn, agent_id, "s1", "bash", "BUSY", Some(123)).unwrap();
    }

    fn insert_output_at(conn: &rusqlite::Connection, agent_id: &str, text: &str, ts: i64) -> i64 {
        let seq_id = insert_event_sync(
            conn,
            agent_id,
            None,
            "output_chunk",
            &serde_json::json!({ "text": text }).to_string(),
        )
        .unwrap();
        conn.execute(
            "UPDATE events SET created_at = ? WHERE seq_id = ?",
            params![ts, seq_id],
        )
        .unwrap();
        seq_id
    }

    fn test_ctx(db: crate::db::Db) -> Ctx {
        let state_dir = tempfile::TempDir::new().unwrap().keep();
        Ctx {
            db,
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

    fn seed_codex_dispatched_job(db: &crate::db::Db, agent_id: &str, job_id: &str) {
        let conn = db.conn();
        insert_session_sync(
            &conn,
            "s_codex_health",
            "p_codex_health",
            "/tmp/codex-health",
        )
        .unwrap();
        insert_agent_sync(
            &conn,
            agent_id,
            "s_codex_health",
            "codex",
            "BUSY",
            Some(123),
        )
        .unwrap();
        insert_job_sync(&conn, job_id, agent_id, None, "say alpha\n").unwrap();
        conn.execute(
            "UPDATE jobs SET status = 'DISPATCHED', dispatched_at = 10 WHERE id = ?",
            [job_id],
        )
        .unwrap();
        let dispatch_seq = insert_event_sync(
            &conn,
            agent_id,
            None,
            "command_received",
            r#"{"cmd":"say alpha\n","status":"SENT"}"#,
        )
        .unwrap();
        update_dispatched_seq_id_sync(&conn, job_id, dispatch_seq).unwrap();
        insert_output_at(&conn, agent_id, "say alpha\nalpha\n", 11);
    }

    #[tokio::test]
    async fn test_query_last_progress_has_no_events() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let db = init(file.path()).unwrap();
        {
            let conn = db.conn();
            insert_session_sync(&conn, "s1", "p1", "/tmp/p1").unwrap();
            seed_agent(&conn, "a1");
        }

        let progress = query_last_progress(db, "a1".to_string()).await.unwrap();
        assert_eq!(progress, (None, None));
    }

    #[tokio::test]
    async fn test_query_last_progress_tracks_last_output_and_last_marker_separately() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let db = init(file.path()).unwrap();
        {
            let conn = db.conn();
            insert_session_sync(&conn, "s1", "p1", "/tmp/p1").unwrap();
            seed_agent(&conn, "a1");
            seed_agent(&conn, "a2");
            insert_output_at(&conn, "a1", "plain output", 10);
            insert_output_at(&conn, "a1", "done <<ah-idle:job-id=j1>>", 20);
            insert_output_at(&conn, "a1", "final output", 30);
            insert_output_at(&conn, "a2", "done <<ah-idle:job-id=other>>", 40);
        }

        let progress = query_last_progress(db.clone(), "a1".to_string())
            .await
            .unwrap();
        assert_eq!(progress, (Some(30), Some(20)));

        let isolated = query_last_progress(db, "a2".to_string()).await.unwrap();
        assert_eq!(isolated, (Some(40), Some(40)));
    }

    #[tokio::test]
    async fn test_query_last_progress_ignores_output_without_marker() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let db = init(file.path()).unwrap();
        {
            let conn = db.conn();
            insert_session_sync(&conn, "s1", "p1", "/tmp/p1").unwrap();
            seed_agent(&conn, "a1");
            insert_output_at(&conn, "a1", "plain output", 10);
        }

        let progress = query_last_progress(db, "a1".to_string()).await.unwrap();
        assert_eq!(progress, (Some(10), None));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn health_completion_stale_codex_recaptures_idle_pane_before_stuck_even_with_active_registry()
     {
        let file = tempfile::NamedTempFile::new().unwrap();
        let db = init(file.path()).unwrap();
        let agent_id = "a_health_codex_recapture";
        let job_id = "job_health_codex_recapture";
        seed_codex_dispatched_job(&db, agent_id, job_id);
        let (cancel_tx, _cancel_rx) = tokio::sync::oneshot::channel();
        crate::completion::registry::register(
            agent_id.to_string(),
            crate::completion::registry::LogMonitorEntry {
                provider: "codex".to_string(),
                log_root: std::path::PathBuf::from("/tmp/codex-log-root"),
                state: crate::completion::reader::LogReadState::default(),
                cancel_tx,
            },
        );
        let ctx = test_ctx(db.clone());
        let observation = HealthCheckObservation {
            agent_id: agent_id.to_string(),
            provider: "codex".to_string(),
            state: "BUSY".to_string(),
            pane_capture: "say alpha\nalpha\n› \n  gpt-5.5 default · /workspace\n".to_string(),
            pane_capture_ok: true,
            last_output_ts: Some(11),
            last_marker_ts: None,
            dispatched_at: Some(10),
            now_ts: 400,
        };

        let changes = escalate_health_stuck(&ctx, &observation, 300)
            .await
            .unwrap();
        let (state, status, reply, payload): (String, String, String, String) = db
            .conn()
            .query_row(
                "SELECT agents.state, jobs.status, jobs.reply_text, events.payload \
                 FROM agents \
                 JOIN jobs ON jobs.agent_id = agents.id \
                 JOIN events ON events.agent_id = agents.id AND events.event_type = 'state_change' \
                 WHERE agents.id = ? \
                 ORDER BY events.seq_id DESC LIMIT 1",
                [agent_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        let payload: serde_json::Value = serde_json::from_str(&payload).unwrap();
        crate::completion::registry::cancel(agent_id);

        assert_eq!(
            changes, 0,
            "health check should not mark STUCK after recapture"
        );
        assert_eq!(state, "IDLE");
        assert_eq!(status, "COMPLETED");
        assert_eq!(reply, "alpha");
        assert_eq!(payload["reason"], "UI_COMPLETION_RECAPTURE_MATCHED");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn health_completion_stale_prompt_only_pane_still_marks_stuck() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let db = init(file.path()).unwrap();
        let agent_id = "a_health_codex_prompt_only";
        let job_id = "job_health_codex_prompt_only";
        seed_codex_dispatched_job(&db, agent_id, job_id);
        db.conn()
            .execute(
                "UPDATE events SET payload = ? WHERE agent_id = ? AND event_type = 'output_chunk'",
                [
                    serde_json::json!({ "text": "say alpha\n" }).to_string(),
                    agent_id.to_string(),
                ],
            )
            .unwrap();
        let ctx = test_ctx(db.clone());
        let observation = HealthCheckObservation {
            agent_id: agent_id.to_string(),
            provider: "codex".to_string(),
            state: "BUSY".to_string(),
            pane_capture: "say alpha\n› \n  gpt-5.5 default · /workspace\n".to_string(),
            pane_capture_ok: true,
            last_output_ts: Some(11),
            last_marker_ts: None,
            dispatched_at: Some(10),
            now_ts: 400,
        };

        let changes = escalate_health_stuck(&ctx, &observation, 300)
            .await
            .unwrap();
        let state: String = db
            .conn()
            .query_row("SELECT state FROM agents WHERE id = ?", [agent_id], |row| {
                row.get(0)
            })
            .unwrap();
        let job = query_job_sync(&db.conn(), job_id).unwrap().unwrap();

        assert_eq!(
            changes, 0,
            "prompt-only recapture marks STUCK before health writes a duplicate STUCK"
        );
        assert_eq!(state, "STUCK");
        assert_eq!(job.status, "DISPATCHED");
    }

    #[tokio::test]
    #[ignore = "manual perf guard for #94; main validation runs this serially"]
    async fn test_query_last_progress_100k_events_is_bounded() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let db = init(file.path()).unwrap();
        {
            let conn = db.conn();
            insert_session_sync(&conn, "s1", "p1", "/tmp/p1").unwrap();
            seed_agent(&conn, "a1");
            for idx in 0..100_000 {
                insert_output_at(&conn, "a1", &format!("chunk {idx}"), idx);
            }
            insert_output_at(&conn, "a1", "done <<ah-idle:job-id=j1>>", 100_001);
            insert_output_at(&conn, "a1", "final output", 100_002);
        }

        let started = Instant::now();
        let progress = query_last_progress(db, "a1".to_string()).await.unwrap();
        assert_eq!(progress, (Some(100_002), Some(100_001)));
        assert!(
            started.elapsed().as_millis() < 50,
            "bounded query should not materialize 100k events"
        );
    }
}
