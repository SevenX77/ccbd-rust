use crate::db::{self, Db};
use crate::marker::{MarkerMatcher, MatchResult, parser_registry, registry};
use std::fs::File;
#[cfg(unix)]
use std::io::Read;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
#[cfg(unix)]
use tokio::io::unix::AsyncFd;
#[cfg(unix)]
use tokio::time::{Duration, timeout};

#[derive(Clone)]
pub struct ReaderMarkerConfig {
    pub matcher: Arc<MarkerMatcher>,
    pub stability_ms: u64,
    pub idle_scan_enabled: Arc<AtomicBool>,
}

pub fn spawn_agent_io_reader_task(
    agent_id: String,
    fifo: File,
    db: Db,
    parser: Arc<Mutex<vt100::Parser>>,
) -> tokio::task::JoinHandle<()> {
    spawn_agent_io_reader_task_with_config(
        agent_id,
        fifo,
        db,
        parser,
        ReaderMarkerConfig {
            matcher: Arc::new(MarkerMatcher::default()),
            stability_ms: 0,
            idle_scan_enabled: Arc::new(AtomicBool::new(true)),
        },
    )
}

pub fn spawn_agent_io_reader_task_with_config(
    agent_id: String,
    fifo: File,
    db: Db,
    parser: Arc<Mutex<vt100::Parser>>,
    config: ReaderMarkerConfig,
) -> tokio::task::JoinHandle<()> {
    #[cfg(windows)]
    {
        let _ = (fifo, db, parser, config);
        return tokio::spawn(async move {
            tracing::warn!(
                agent_id = %agent_id,
                "Windows agent IO stream reader is not implemented until M2"
            );
        });
    }

    #[cfg(unix)]
    tokio::spawn(async move {
        let async_fifo = match AsyncFd::new(fifo) {
            Ok(fd) => fd,
            Err(err) => {
                tracing::warn!(agent_id = %agent_id, error = %err, "failed to create AsyncFd for fifo");
                return;
            }
        };
        let matcher = config.matcher;
        let stability = Duration::from_millis(config.stability_ms);
        let mut pending_stability_match = false;
        let mut skip_scan_after_stability_noise = false;
        let mut buf = vec![0_u8; 8192];

        loop {
            let mut read_from_pending_stability = false;
            let mut guard = if pending_stability_match && config.stability_ms > 0 {
                match timeout(stability, async_fifo.readable()).await {
                    Err(_) => {
                        match db::state_machine::mark_agent_idle_matched(
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
                                tracing::warn!(agent_id = %agent_id, error = %err, "failed to mark agent IDLE after stable marker match");
                            }
                        }
                        if let Some(handle) = registry::take(&agent_id) {
                            let _ = handle.cancel_tx.send(());
                        }
                        pending_stability_match = false;
                        continue;
                    }
                    Ok(Ok(guard)) => {
                        read_from_pending_stability = true;
                        guard
                    }
                    Ok(Err(err)) => {
                        tracing::warn!(agent_id = %agent_id, error = %err, "fifo readiness wait failed");
                        break;
                    }
                }
            } else {
                match async_fifo.readable().await {
                    Ok(guard) => guard,
                    Err(err) => {
                        tracing::warn!(agent_id = %agent_id, error = %err, "fifo readiness wait failed");
                        break;
                    }
                }
            };

            let read_result = guard.try_io(|inner| {
                let mut file = inner.get_ref();
                file.read(&mut buf)
            });
            let n = match read_result {
                Ok(Ok(0)) => break,
                Ok(Ok(n)) => {
                    if read_from_pending_stability {
                        pending_stability_match = false;
                        skip_scan_after_stability_noise = true;
                    }
                    n
                }
                Ok(Err(err)) => {
                    tracing::warn!(agent_id = %agent_id, error = %err, "fifo read failed");
                    break;
                }
                Err(_would_block) => continue,
            };

            let chunk = String::from_utf8_lossy(&buf[..n]).to_string();
            {
                match parser.lock() {
                    Ok(mut parser) => parser.process(&buf[..n]),
                    Err(err) => {
                        tracing::warn!(agent_id = %agent_id, error = %err, "parser mutex poisoned in fifo reader");
                        continue;
                    }
                }
            }

            let payload = serde_json::json!({ "text": chunk }).to_string();
            if let Err(err) = db::events::insert_event(
                db.clone(),
                agent_id.clone(),
                None,
                "output_chunk".into(),
                payload,
            )
            .await
            {
                tracing::warn!(agent_id = %agent_id, error = %err, "failed to persist fifo output chunk");
            } else {
                crate::orchestrator::pubsub::notify_agent_output(&agent_id);
            }

            let matched = if !config.idle_scan_enabled.load(Ordering::SeqCst) {
                MatchResult::NoMatch
            } else if skip_scan_after_stability_noise {
                skip_scan_after_stability_noise = false;
                MatchResult::NoMatch
            } else {
                match parser.lock() {
                    Ok(parser) => matcher.scan(&parser),
                    Err(err) => {
                        tracing::warn!(agent_id = %agent_id, error = %err, "parser mutex poisoned during marker scan");
                        MatchResult::NoMatch
                    }
                }
            };

            match matched {
                MatchResult::Matched => {
                    if config.stability_ms > 0 {
                        pending_stability_match = true;
                    } else {
                        match db::state_machine::mark_agent_idle_matched(
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
                                if let Some(handle) = registry::take(&agent_id) {
                                    let _ = handle.cancel_tx.send(());
                                }
                            }
                            Ok(_) => {}
                            Err(err) => {
                                tracing::warn!(agent_id = %agent_id, error = %err, "failed to mark agent IDLE after marker match");
                            }
                        }
                    }
                }
                MatchResult::NoMatch => {
                    pending_stability_match = false;
                    registry::reset(&agent_id);
                }
            }
        }

        let _ = parser_registry::remove(&agent_id);
    })
}
