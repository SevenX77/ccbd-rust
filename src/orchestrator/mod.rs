pub mod pubsub;

use crate::db;
use crate::error::CcbdError;
use crate::marker::{MarkerMatcher, TimerKind, parser_registry, registry, spawn_marker_timer_task};
use crate::pane_diff::{DEFAULT_STUCK_THRESHOLD, DEFAULT_WATCH_INTERVAL, pane_diff_watcher_loop};
use crate::rpc::Ctx;
use serde_json::json;
use std::sync::{Arc, LazyLock};
use std::time::Duration;
use tokio::sync::Notify;

pub static WAKER: LazyLock<Notify> = LazyLock::new(Notify::new);

pub fn wake_up() {
    WAKER.notify_one();
}

pub fn spawn_orchestrator_task(ctx: Ctx) {
    let watcher_ctx = ctx.clone();
    tokio::spawn(async move {
        pane_diff_watcher_loop(watcher_ctx, DEFAULT_WATCH_INTERVAL, DEFAULT_STUCK_THRESHOLD).await;
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
    let idle_agents = db::agents::query_agents_by_state(ctx.db.clone(), "IDLE".to_string()).await?;
    let mut did_work = false;

    for agent in idle_agents {
        let Some(job) = db::jobs::claim_next_job(ctx.db.clone(), agent.id.clone()).await? else {
            continue;
        };
        did_work = true;

        let Some(pane_id) = crate::agent_io::pane_id(&agent.id) else {
            let _ = db::jobs::mark_job_failed(
                ctx.db.clone(),
                job.id,
                "tmux pane not registered".to_string(),
            )
            .await;
            wake_up();
            continue;
        };

        if parser_registry::get(&agent.id).is_none() {
            let _ = db::jobs::mark_job_failed(
                ctx.db.clone(),
                job.id,
                "parser not registered".to_string(),
            )
            .await;
            wake_up();
            continue;
        }
        let waiting_for_ack = db::state_machine::mark_agent_waiting_for_ack(
            ctx.db.clone(),
            agent.id.clone(),
            agent.state_version,
        )
        .await?;
        if !waiting_for_ack {
            tracing::trace!(agent_id = %agent.id, "dispatch skipped: agent state changed before WAITING_FOR_ACK transition");
            continue;
        }
        crate::agent_io::set_idle_scan_enabled(&agent.id, false);
        let capture_baseline = ctx.tmux_server.capture_pane(pane_id.clone()).await.ok();

        let seq_id = db::events::insert_event(
            ctx.db.clone(),
            agent.id.clone(),
            None,
            "command_received".to_string(),
            json!({ "cmd": job.prompt_text, "status": "SENT", "job_id": job.id }).to_string(),
        )
        .await?;
        let _ = db::jobs::update_dispatched_seq_id(ctx.db.clone(), job.id.clone(), seq_id).await?;

        let send_result = crate::agent_io::send_text_to_pane(
            ctx.tmux_server.clone(),
            &agent.id,
            pane_id,
            job.prompt_text.clone(),
        )
        .await;

        if let Err(err) = send_result {
            let _ =
                db::jobs::mark_job_failed(ctx.db.clone(), job.id, format!("send failed: {err}"))
                    .await;
            wake_up();
            continue;
        }
        spawn_dispatch_ack_stability_busy(ctx.db.clone(), agent.id.clone());

        if let Some(parser_handle) = parser_registry::get(&agent.id) {
            let marker_handle = spawn_marker_timer_task(
                agent.id.clone(),
                TimerKind::Busy,
                Arc::new(ctx.db.clone()),
                parser_handle.clone(),
            );
            registry::register(agent.id.clone(), marker_handle);
            let manifest = crate::provider::manifest::get_manifest(&agent.provider);
            let matcher = MarkerMatcher::from_manifest(&manifest);
            if let Some(capture_baseline) = capture_baseline {
                crate::rpc::handlers::spawn_new_capture_seed(
                    ctx.db.clone(),
                    ctx.tmux_server.clone(),
                    agent.id.clone(),
                    capture_baseline,
                    Arc::new(matcher),
                );
            }
            drop(parser_handle);
        }
    }

    Ok(did_work)
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
    use super::{run_once, wake_up};
    use crate::db;
    use crate::db::agents::insert_agent_sync;
    use crate::db::jobs::{insert_job_sync, query_job_sync};
    use crate::db::sessions::insert_session_sync;
    use crate::rpc::Ctx;
    use crate::sandbox::EnvState;
    use crate::tmux::TmuxServer;
    use std::sync::Arc;

    fn test_ctx() -> Ctx {
        let file = tempfile::NamedTempFile::new().unwrap();
        let state_dir = tempfile::TempDir::new().unwrap().keep();
        Ctx {
            db: db::init(file.path()).unwrap(),
            state_dir: state_dir.clone(),
            env_state: EnvState {
                bwrap_available: false,
                systemd_run_available: false,
                unsafe_no_sandbox: true,
                under_systemd: false,
            },
            tmux_server: Arc::new(TmuxServer::new(&state_dir)),
        }
    }

    #[test]
    fn test_wake_up_is_callable() {
        wake_up();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_run_once_fails_job_without_pane() {
        let ctx = test_ctx();
        {
            let conn = ctx.db.conn();
            insert_session_sync(&conn, "s1", "p1", "/tmp/foo").unwrap();
            insert_agent_sync(&conn, "a1", "s1", "bash", "IDLE", Some(123)).unwrap();
            insert_job_sync(&conn, "job_1", "a1", None, "echo hi\n").unwrap();
        }

        assert!(run_once(&ctx).await.unwrap());
        let job = query_job_sync(&ctx.db.conn(), "job_1").unwrap().unwrap();

        assert_eq!(job.status, "FAILED");
        assert_eq!(
            job.error_reason.as_deref(),
            Some("tmux pane not registered")
        );
    }
}
