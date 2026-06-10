pub mod pubsub;

use crate::db;
use crate::error::CcbdError;
use crate::marker::{
    MarkerMatcher, PromptTimerScanContext, TimerKind, parser_registry, registry,
    spawn_marker_timer_task_with_prompt,
};
use crate::pane_diff::{pane_diff_watcher_loop, resolve_stuck_watch_config};
use crate::provider::health_check::health_check_watcher_loop;
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
        let (watch_interval, stuck_threshold) = resolve_stuck_watch_config();
        pane_diff_watcher_loop(watcher_ctx, watch_interval, stuck_threshold).await;
    });
    let health_ctx = ctx.clone();
    tokio::spawn(async move {
        let (watch_interval, stuck_threshold) = resolve_stuck_watch_config();
        health_check_watcher_loop(health_ctx, watch_interval, stuck_threshold).await;
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
        let Some(dispatched) = db::jobs::dispatch_job_to_agent(
            ctx.db.clone(),
            agent.id.clone(),
            vec![db::state_machine::STATE_IDLE.to_string()],
            db::state_machine::STATE_WAITING_FOR_ACK.to_string(),
            "command_received".to_string(),
            json!({ "status": "SENT" }),
        )
        .await?
        else {
            continue;
        };
        let job = dispatched.job;
        did_work = true;

        let Some(pane_id) = crate::agent_io::pane_id(&agent.id) else {
            mark_dispatch_io_failed(ctx, &agent.id, "tmux pane not registered").await;
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
            mark_dispatch_io_failed(ctx, &agent.id, "parser not registered").await;
            let _ = db::jobs::mark_job_failed(
                ctx.db.clone(),
                job.id,
                "parser not registered".to_string(),
            )
            .await;
            wake_up();
            continue;
        }
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

    Ok(did_work)
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
    use super::{prepare_log_monitor_before_send, run_once, wake_up};
    use crate::db;
    use crate::db::agents::insert_agent_sync;
    use crate::db::jobs::{insert_job_sync, query_job_sync};
    use crate::db::sessions::insert_session_sync;
    use crate::rpc::Ctx;
    use crate::sandbox::EnvState;
    use crate::tmux::{TmuxPaneId, TmuxServer};
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
    async fn send_failure_cancels_registered_log_monitor() {
        let mut ctx = test_ctx();
        ctx.env_state.unsafe_no_sandbox = false;
        let sandbox = ctx.state_dir.join("sandboxes").join("s1").join("a1");
        std::fs::create_dir_all(&sandbox).unwrap();
        let home = crate::provider::home_layout::sandbox_home_for_sandbox_dir(&sandbox).unwrap();
        let sessions = home.join(".codex/sessions/2026/06");
        std::fs::create_dir_all(&sessions).unwrap();
        std::fs::write(sessions.join("rollout-session.jsonl"), b"{\"old\":true}\n").unwrap();
        {
            let conn = ctx.db.conn();
            insert_session_sync(&conn, "s1", "p1", "/tmp/foo").unwrap();
            insert_agent_sync(&conn, "a1", "s1", "codex", "IDLE", Some(123)).unwrap();
            insert_job_sync(&conn, "job_1", "a1", None, "echo hi\n").unwrap();
        }
        crate::marker::parser_registry::register(
            "a1".to_string(),
            Arc::new(Mutex::new(vt100::Parser::new(200, 200, 0))),
        );
        let reader_handle = tokio::spawn(async {
            std::future::pending::<()>().await;
        });
        crate::agent_io::register(
            "a1".to_string(),
            crate::agent_io::AgentIoEntry {
                session_id: "s1".to_string(),
                pane_id: TmuxPaneId("%999999".to_string()),
                reader_handle,
                fifo_path: ctx.state_dir.join("pipes").join("a1.fifo"),
                socket_name: ctx.tmux_server.socket_name().to_string(),
                idle_scan_enabled: Arc::new(AtomicBool::new(true)),
            },
        );

        assert!(run_once(&ctx).await.unwrap());

        assert!(!crate::completion::registry::contains("a1"));
        let job = query_job_sync(&ctx.db.conn(), "job_1").unwrap().unwrap();
        assert_eq!(job.status, "FAILED");
        assert!(
            job.error_reason
                .as_deref()
                .is_some_and(|reason| { reason.starts_with("send failed:") })
        );
        let _ = crate::agent_io::remove("a1");
        let _ = crate::marker::parser_registry::remove("a1");
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
}
