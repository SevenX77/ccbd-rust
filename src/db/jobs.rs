use crate::db::Db;
use crate::db::common::{is_unique_constraint_error, map_db_error, spawn_db};
use crate::db::schema::Job;
use crate::error::CcbdError;
use rusqlite::{Connection, OptionalExtension, Row, TransactionBehavior, params};
use serde_json::Value;

fn row_to_job(row: &Row<'_>) -> rusqlite::Result<Job> {
    Ok(Job {
        id: row.get(0)?,
        agent_id: row.get(1)?,
        request_id: row.get(2)?,
        prompt_text: row.get(3)?,
        reply_text: row.get(4)?,
        status: row.get(5)?,
        error_reason: row.get(6)?,
        created_at: row.get(7)?,
        dispatched_at: row.get(8)?,
        dispatched_at_seq_id: row.get(9)?,
        completed_at: row.get(10)?,
        cancel_requested: row.get::<_, i64>(11)? != 0,
    })
}

pub(crate) fn insert_job_sync(
    conn: &Connection,
    id: &str,
    agent_id: &str,
    request_id: Option<&str>,
    prompt_text: &str,
) -> Result<String, CcbdError> {
    let result = conn.execute(
        "INSERT INTO jobs (id, agent_id, request_id, prompt_text, status) VALUES (?, ?, ?, ?, 'QUEUED')",
        params![id, agent_id, request_id, prompt_text],
    );

    match result {
        Ok(_) => Ok(id.to_string()),
        Err(err) if is_unique_constraint_error(&err) && request_id.is_some() => {
            let existing = query_job_by_request_id_sync(conn, agent_id, request_id.unwrap())?
                .ok_or_else(|| map_db_error("query duplicate job by request_id", err))?;
            Ok(existing.id)
        }
        Err(err) => Err(map_db_error("insert job", err)),
    }
}

pub(crate) fn query_job_sync(conn: &Connection, job_id: &str) -> Result<Option<Job>, CcbdError> {
    conn.query_row(
        "SELECT id, agent_id, request_id, prompt_text, reply_text, status, error_reason, created_at, dispatched_at, dispatched_at_seq_id, completed_at, cancel_requested FROM jobs WHERE id = ?",
        params![job_id],
        row_to_job,
    )
    .optional()
    .map_err(|err| map_db_error("query job", err))
}

pub(crate) fn query_job_by_request_id_sync(
    conn: &Connection,
    agent_id: &str,
    request_id: &str,
) -> Result<Option<Job>, CcbdError> {
    conn.query_row(
        "SELECT id, agent_id, request_id, prompt_text, reply_text, status, error_reason, created_at, dispatched_at, dispatched_at_seq_id, completed_at, cancel_requested FROM jobs WHERE agent_id = ? AND request_id = ? LIMIT 1",
        params![agent_id, request_id],
        row_to_job,
    )
    .optional()
    .map_err(|err| map_db_error("query job by request_id", err))
}

pub(crate) fn claim_next_job_sync(db: &Db, agent_id: &str) -> Result<Option<Job>, CcbdError> {
    let mut conn = db.conn();
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|err| map_db_error("begin claim next job", err))?;

    let candidate_id = tx
        .query_row(
            "SELECT id FROM jobs WHERE agent_id = ? AND status = 'QUEUED' ORDER BY created_at ASC, rowid ASC LIMIT 1",
            params![agent_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|err| map_db_error("query queued job", err))?;

    let Some(job_id) = candidate_id else {
        tx.commit()
            .map_err(|err| map_db_error("commit empty claim next job", err))?;
        return Ok(None);
    };

    let changes = tx
        .execute(
            "UPDATE jobs SET status = 'DISPATCHED', dispatched_at = unixepoch() WHERE id = ? AND status = 'QUEUED'",
            params![job_id],
        )
        .map_err(|err| map_db_error("claim queued job", err))?;

    let job = if changes == 1 {
        tx.query_row(
            "SELECT id, agent_id, request_id, prompt_text, reply_text, status, error_reason, created_at, dispatched_at, dispatched_at_seq_id, completed_at, cancel_requested FROM jobs WHERE id = ?",
            params![job_id],
            row_to_job,
        )
        .optional()
        .map_err(|err| map_db_error("query claimed job", err))?
    } else {
        None
    };

    tx.commit()
        .map_err(|err| map_db_error("commit claim next job", err))?;
    Ok(job)
}

pub(crate) fn mark_job_completed_sync(
    db: &Db,
    job_id: &str,
    reply_text: &str,
) -> Result<usize, CcbdError> {
    let conn = db.conn();
    mark_job_completed_conn_sync(&conn, job_id, reply_text)
}

pub(crate) fn mark_job_completed_conn_sync(
    conn: &Connection,
    job_id: &str,
    reply_text: &str,
) -> Result<usize, CcbdError> {
    conn.execute(
        "UPDATE jobs SET status = 'COMPLETED', reply_text = ?, completed_at = unixepoch() WHERE id = ? AND status = 'DISPATCHED'",
        params![reply_text, job_id],
    )
    .map_err(|err| map_db_error("mark job completed", err))
}

pub(crate) fn mark_queued_job_cancelled_sync(db: &Db, job_id: &str) -> Result<usize, CcbdError> {
    let conn = db.conn();
    mark_queued_job_cancelled_conn_sync(&conn, job_id)
}

pub(crate) fn mark_queued_job_cancelled_conn_sync(
    conn: &Connection,
    job_id: &str,
) -> Result<usize, CcbdError> {
    conn.execute(
        "UPDATE jobs SET status = 'CANCELLED', completed_at = unixepoch() WHERE id = ? AND status = 'QUEUED'",
        params![job_id],
    )
    .map_err(|err| map_db_error("mark queued job cancelled", err))
}

pub(crate) fn request_dispatched_job_cancel_sync(
    db: &Db,
    job_id: &str,
) -> Result<usize, CcbdError> {
    let conn = db.conn();
    conn.execute(
        "UPDATE jobs SET cancel_requested = 1 WHERE id = ? AND status = 'DISPATCHED'",
        params![job_id],
    )
    .map_err(|err| map_db_error("request dispatched job cancel", err))
}

pub(crate) fn mark_dispatched_job_cancelled_if_agent_idle_sync(
    db: &Db,
    job_id: &str,
) -> Result<usize, CcbdError> {
    let mut conn = db.conn();
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|err| map_db_error("begin mark dispatched job cancelled if agent idle", err))?;
    let changes = tx
        .execute(
            "UPDATE jobs \
             SET status = 'CANCELLED', completed_at = unixepoch() \
             WHERE id = ? \
               AND status = 'DISPATCHED' \
               AND cancel_requested = 1 \
               AND EXISTS (SELECT 1 FROM agents WHERE agents.id = jobs.agent_id AND agents.state IN ('IDLE', 'UNKNOWN'))",
            params![job_id],
        )
        .map_err(|err| map_db_error("mark dispatched job cancelled if agent idle", err))?;
    tx.commit()
        .map_err(|err| map_db_error("commit mark dispatched job cancelled if agent idle", err))?;
    Ok(changes)
}

pub(crate) fn mark_job_cancelled_conn_sync(
    conn: &Connection,
    job_id: &str,
    reply_text: &str,
) -> Result<usize, CcbdError> {
    conn.execute(
        "UPDATE jobs SET status = 'CANCELLED', reply_text = ?, completed_at = unixepoch() WHERE id = ? AND status = 'DISPATCHED'",
        params![reply_text, job_id],
    )
    .map_err(|err| map_db_error("mark job cancelled", err))
}

pub(crate) fn mark_job_failed_sync(
    db: &Db,
    job_id: &str,
    error_reason: &str,
) -> Result<usize, CcbdError> {
    let conn = db.conn();
    mark_job_failed_conn_sync(&conn, job_id, error_reason)
}

pub(crate) fn mark_job_failed_conn_sync(
    conn: &Connection,
    job_id: &str,
    error_reason: &str,
) -> Result<usize, CcbdError> {
    conn.execute(
        "UPDATE jobs SET status = 'FAILED', error_reason = ?, completed_at = unixepoch() WHERE id = ? AND status IN ('QUEUED', 'DISPATCHED')",
        params![error_reason, job_id],
    )
    .map_err(|err| map_db_error("mark job failed", err))
}

pub(crate) fn mark_dispatched_jobs_failed_for_agent_sync(
    db: &Db,
    agent_id: &str,
    reason: &str,
) -> Result<usize, CcbdError> {
    let conn = db.conn();
    mark_dispatched_jobs_failed_for_agent_conn_sync(&conn, agent_id, reason)
}

pub(crate) fn mark_dispatched_jobs_failed_for_agent_conn_sync(
    conn: &Connection,
    agent_id: &str,
    reason: &str,
) -> Result<usize, CcbdError> {
    conn.execute(
        "UPDATE jobs SET status = 'FAILED', error_reason = ?, completed_at = unixepoch() WHERE agent_id = ? AND status = 'DISPATCHED'",
        params![reason, agent_id],
    )
    .map_err(|err| map_db_error("mark dispatched jobs failed for agent", err))
}

pub(crate) fn query_dispatched_job_for_agent_sync(
    conn: &Connection,
    agent_id: &str,
) -> Result<Option<Job>, CcbdError> {
    conn.query_row(
        "SELECT id, agent_id, request_id, prompt_text, reply_text, status, error_reason, created_at, dispatched_at, dispatched_at_seq_id, completed_at, cancel_requested FROM jobs WHERE agent_id = ? AND status = 'DISPATCHED' ORDER BY dispatched_at ASC, id ASC LIMIT 1",
        params![agent_id],
        row_to_job,
    )
    .optional()
    .map_err(|err| map_db_error("query dispatched job for agent", err))
}

pub(crate) fn update_dispatched_seq_id_sync(
    conn: &Connection,
    job_id: &str,
    seq_id: i64,
) -> Result<usize, CcbdError> {
    conn.execute(
        "UPDATE jobs SET dispatched_at_seq_id = ? WHERE id = ? AND status = 'DISPATCHED'",
        params![seq_id, job_id],
    )
    .map_err(|err| map_db_error("update job dispatched seq_id", err))
}

pub(crate) fn collect_reply_for_dispatched_job_sync(
    conn: &Connection,
    agent_id: &str,
    dispatched_at_seq_id: Option<i64>,
) -> Result<String, CcbdError> {
    let Some(dispatched_at_seq_id) = dispatched_at_seq_id else {
        return Ok(String::new());
    };
    let mut stmt = conn
        .prepare(
            "SELECT payload FROM events WHERE agent_id = ? AND event_type = 'output_chunk' AND seq_id > ? ORDER BY seq_id ASC",
        )
        .map_err(|err| map_db_error("prepare collect job reply", err))?;
    let rows = stmt
        .query_map(params![agent_id, dispatched_at_seq_id], |row| {
            row.get::<_, String>(0)
        })
        .map_err(|err| map_db_error("query job reply events", err))?;

    let mut reply = String::new();
    for payload in rows {
        let payload = payload.map_err(|err| map_db_error("collect job reply payload", err))?;
        let value: Value = serde_json::from_str(&payload).map_err(|err| {
            CcbdError::DbConstraintViolation(format!("parse output_chunk payload: {err}"))
        })?;
        if let Some(text) = value.get("text").and_then(Value::as_str) {
            reply.push_str(text);
        }
    }
    Ok(reply)
}

pub async fn insert_job(
    db: Db,
    id: String,
    agent_id: String,
    request_id: Option<String>,
    prompt_text: String,
) -> Result<String, CcbdError> {
    spawn_db("jobs::insert_job", move || {
        let conn = db.conn();
        insert_job_sync(&conn, &id, &agent_id, request_id.as_deref(), &prompt_text)
    })
    .await
}

pub async fn query_job(db: Db, job_id: String) -> Result<Option<Job>, CcbdError> {
    spawn_db("jobs::query_job", move || {
        let conn = db.conn();
        query_job_sync(&conn, &job_id)
    })
    .await
}

pub async fn query_job_by_request_id(
    db: Db,
    agent_id: String,
    request_id: String,
) -> Result<Option<Job>, CcbdError> {
    spawn_db("jobs::query_job_by_request_id", move || {
        let conn = db.conn();
        query_job_by_request_id_sync(&conn, &agent_id, &request_id)
    })
    .await
}

pub async fn claim_next_job(db: Db, agent_id: String) -> Result<Option<Job>, CcbdError> {
    spawn_db("jobs::claim_next_job", move || {
        claim_next_job_sync(&db, &agent_id)
    })
    .await
}

pub async fn mark_job_completed(
    db: Db,
    job_id: String,
    reply_text: String,
) -> Result<usize, CcbdError> {
    let notify_job_id = job_id.clone();
    spawn_db("jobs::mark_job_completed", move || {
        mark_job_completed_sync(&db, &job_id, &reply_text)
    })
    .await
    .inspect(|changes| {
        if *changes > 0 {
            crate::orchestrator::pubsub::notify_job_update(&notify_job_id);
        }
    })
}

pub async fn mark_queued_job_cancelled(db: Db, job_id: String) -> Result<usize, CcbdError> {
    let notify_job_id = job_id.clone();
    spawn_db("jobs::mark_queued_job_cancelled", move || {
        mark_queued_job_cancelled_sync(&db, &job_id)
    })
    .await
    .inspect(|changes| {
        if *changes > 0 {
            crate::orchestrator::pubsub::notify_job_update(&notify_job_id);
            crate::orchestrator::wake_up();
        }
    })
}

pub async fn request_dispatched_job_cancel(db: Db, job_id: String) -> Result<usize, CcbdError> {
    spawn_db("jobs::request_dispatched_job_cancel", move || {
        request_dispatched_job_cancel_sync(&db, &job_id)
    })
    .await
}

pub async fn mark_dispatched_job_cancelled_if_agent_idle(
    db: Db,
    job_id: String,
) -> Result<usize, CcbdError> {
    let notify_job_id = job_id.clone();
    spawn_db(
        "jobs::mark_dispatched_job_cancelled_if_agent_idle",
        move || mark_dispatched_job_cancelled_if_agent_idle_sync(&db, &job_id),
    )
    .await
    .inspect(|changes| {
        if *changes > 0 {
            crate::orchestrator::pubsub::notify_job_update(&notify_job_id);
            crate::orchestrator::wake_up();
        }
    })
}

pub async fn mark_job_failed(
    db: Db,
    job_id: String,
    error_reason: String,
) -> Result<usize, CcbdError> {
    let notify_job_id = job_id.clone();
    spawn_db("jobs::mark_job_failed", move || {
        mark_job_failed_sync(&db, &job_id, &error_reason)
    })
    .await
    .inspect(|changes| {
        if *changes > 0 {
            crate::orchestrator::pubsub::notify_job_update(&notify_job_id);
        }
    })
}

pub async fn mark_dispatched_jobs_failed_for_agent(
    db: Db,
    agent_id: String,
    reason: String,
) -> Result<usize, CcbdError> {
    let affected_job = query_dispatched_job_for_agent(db.clone(), agent_id.clone())
        .await?
        .map(|job| job.id);
    spawn_db("jobs::mark_dispatched_jobs_failed_for_agent", move || {
        mark_dispatched_jobs_failed_for_agent_sync(&db, &agent_id, &reason)
    })
    .await
    .inspect(|changes| {
        if *changes > 0
            && let Some(job_id) = &affected_job
        {
            crate::orchestrator::pubsub::notify_job_update(job_id);
        }
    })
}

pub async fn query_dispatched_job_for_agent(
    db: Db,
    agent_id: String,
) -> Result<Option<Job>, CcbdError> {
    spawn_db("jobs::query_dispatched_job_for_agent", move || {
        let conn = db.conn();
        query_dispatched_job_for_agent_sync(&conn, &agent_id)
    })
    .await
}

pub async fn update_dispatched_seq_id(
    db: Db,
    job_id: String,
    seq_id: i64,
) -> Result<usize, CcbdError> {
    spawn_db("jobs::update_dispatched_seq_id", move || {
        let conn = db.conn();
        update_dispatched_seq_id_sync(&conn, &job_id, seq_id)
    })
    .await
}

pub async fn collect_reply_for_dispatched_job(
    db: Db,
    agent_id: String,
    dispatched_at_seq_id: Option<i64>,
) -> Result<String, CcbdError> {
    spawn_db("jobs::collect_reply_for_dispatched_job", move || {
        let conn = db.conn();
        collect_reply_for_dispatched_job_sync(&conn, &agent_id, dispatched_at_seq_id)
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::{
        claim_next_job_sync, collect_reply_for_dispatched_job_sync, insert_job_sync,
        mark_dispatched_job_cancelled_if_agent_idle_sync,
        mark_dispatched_jobs_failed_for_agent_sync, mark_job_completed_sync, mark_job_failed_sync,
        mark_queued_job_cancelled_sync, query_dispatched_job_for_agent_sync,
        query_job_by_request_id_sync, query_job_sync, request_dispatched_job_cancel_sync,
        update_dispatched_seq_id_sync,
    };
    use crate::db::agents::insert_agent_sync;
    use crate::db::events::insert_event_sync;
    use crate::db::sessions::insert_session_sync;
    use crate::db::{Db, init};

    fn with_test_db<T>(test: impl FnOnce(&Db) -> T) -> T {
        let file = tempfile::NamedTempFile::new().unwrap();
        let db = init(file.path()).unwrap();
        {
            let conn = db.conn();
            insert_session_sync(&conn, "s1", "p1", "/tmp/foo").unwrap();
            insert_agent_sync(&conn, "a1", "s1", "bash", "IDLE", Some(123)).unwrap();
        }
        test(&db)
    }

    #[test]
    fn test_insert_and_query_job() {
        with_test_db(|db| {
            let conn = db.conn();
            let id = insert_job_sync(&conn, "job_1", "a1", None, "hello").unwrap();
            let job = query_job_sync(&conn, &id).unwrap().unwrap();

            assert_eq!(id, "job_1");
            assert_eq!(job.agent_id, "a1");
            assert_eq!(job.prompt_text, "hello");
            assert_eq!(job.status, "QUEUED");
            assert_eq!(job.dispatched_at_seq_id, None);
        });
    }

    #[test]
    fn test_insert_job_idempotent_by_request_id() {
        with_test_db(|db| {
            let conn = db.conn();
            let first = insert_job_sync(&conn, "job_1", "a1", Some("req-1"), "hello").unwrap();
            let second = insert_job_sync(&conn, "job_2", "a1", Some("req-1"), "other").unwrap();
            let by_req = query_job_by_request_id_sync(&conn, "a1", "req-1")
                .unwrap()
                .unwrap();

            assert_eq!(first, "job_1");
            assert_eq!(second, "job_1");
            assert_eq!(by_req.prompt_text, "hello");
        });
    }

    #[test]
    fn test_query_missing_job_returns_none() {
        with_test_db(|db| {
            let conn = db.conn();
            assert!(query_job_sync(&conn, "job_missing").unwrap().is_none());
        });
    }

    #[test]
    fn test_claim_next_job_fifo_and_single_claim() {
        with_test_db(|db| {
            {
                let conn = db.conn();
                insert_job_sync(&conn, "job_1", "a1", None, "one").unwrap();
                insert_job_sync(&conn, "job_2", "a1", None, "two").unwrap();
            }

            let first = claim_next_job_sync(db, "a1").unwrap().unwrap();
            let second = claim_next_job_sync(db, "a1").unwrap().unwrap();
            let third = claim_next_job_sync(db, "a1").unwrap();

            assert_eq!(first.id, "job_1");
            assert_eq!(first.status, "DISPATCHED");
            assert!(first.dispatched_at.is_some());
            assert_eq!(second.id, "job_2");
            assert!(third.is_none());
        });
    }

    #[test]
    fn test_mark_job_completed_only_dispatched() {
        with_test_db(|db| {
            {
                let conn = db.conn();
                insert_job_sync(&conn, "job_1", "a1", None, "one").unwrap();
            }

            assert_eq!(mark_job_completed_sync(db, "job_1", "reply").unwrap(), 0);
            claim_next_job_sync(db, "a1").unwrap().unwrap();
            assert_eq!(mark_job_completed_sync(db, "job_1", "reply").unwrap(), 1);

            let conn = db.conn();
            let job = query_job_sync(&conn, "job_1").unwrap().unwrap();
            assert_eq!(job.status, "COMPLETED");
            assert_eq!(job.reply_text.as_deref(), Some("reply"));
            assert!(job.completed_at.is_some());
        });
    }

    #[test]
    fn test_mark_job_failed_from_queued_or_dispatched() {
        with_test_db(|db| {
            {
                let conn = db.conn();
                insert_job_sync(&conn, "job_1", "a1", None, "one").unwrap();
                insert_job_sync(&conn, "job_2", "a1", None, "two").unwrap();
            }
            claim_next_job_sync(db, "a1").unwrap().unwrap();

            assert_eq!(mark_job_failed_sync(db, "job_1", "boom").unwrap(), 1);
            assert_eq!(mark_job_failed_sync(db, "job_2", "skip").unwrap(), 1);

            let conn = db.conn();
            let job_1 = query_job_sync(&conn, "job_1").unwrap().unwrap();
            let job_2 = query_job_sync(&conn, "job_2").unwrap().unwrap();
            assert_eq!(job_1.status, "FAILED");
            assert_eq!(job_1.error_reason.as_deref(), Some("boom"));
            assert_eq!(job_2.status, "FAILED");
            assert_eq!(job_2.error_reason.as_deref(), Some("skip"));
        });
    }

    #[test]
    fn test_mark_queued_job_cancelled() {
        with_test_db(|db| {
            {
                let conn = db.conn();
                insert_job_sync(&conn, "job_1", "a1", None, "one").unwrap();
            }

            assert_eq!(mark_queued_job_cancelled_sync(db, "job_1").unwrap(), 1);
            assert_eq!(mark_queued_job_cancelled_sync(db, "job_1").unwrap(), 0);
            let job = query_job_sync(&db.conn(), "job_1").unwrap().unwrap();
            assert_eq!(job.status, "CANCELLED");
            assert!(job.completed_at.is_some());
        });
    }

    #[test]
    fn test_request_dispatched_job_cancel() {
        with_test_db(|db| {
            {
                let conn = db.conn();
                insert_job_sync(&conn, "job_1", "a1", None, "one").unwrap();
            }
            claim_next_job_sync(db, "a1").unwrap().unwrap();

            assert_eq!(request_dispatched_job_cancel_sync(db, "job_1").unwrap(), 1);
            let job = query_job_sync(&db.conn(), "job_1").unwrap().unwrap();
            assert_eq!(job.status, "DISPATCHED");
            assert!(job.cancel_requested);
        });
    }

    #[test]
    fn test_mark_dispatched_cancelled_if_agent_already_idle() {
        with_test_db(|db| {
            {
                let conn = db.conn();
                insert_job_sync(&conn, "job_1", "a1", None, "one").unwrap();
            }
            claim_next_job_sync(db, "a1").unwrap().unwrap();
            request_dispatched_job_cancel_sync(db, "job_1").unwrap();

            assert_eq!(
                mark_dispatched_job_cancelled_if_agent_idle_sync(db, "job_1").unwrap(),
                1
            );
            let job = query_job_sync(&db.conn(), "job_1").unwrap().unwrap();
            assert_eq!(job.status, "CANCELLED");
        });
    }

    #[test]
    fn test_mark_dispatched_jobs_failed_for_agent() {
        with_test_db(|db| {
            {
                let conn = db.conn();
                insert_job_sync(&conn, "job_1", "a1", None, "one").unwrap();
                insert_job_sync(&conn, "job_2", "a1", None, "two").unwrap();
            }
            claim_next_job_sync(db, "a1").unwrap().unwrap();

            assert_eq!(
                mark_dispatched_jobs_failed_for_agent_sync(db, "a1", "crashed").unwrap(),
                1
            );

            let conn = db.conn();
            let job_1 = query_job_sync(&conn, "job_1").unwrap().unwrap();
            let job_2 = query_job_sync(&conn, "job_2").unwrap().unwrap();
            assert_eq!(job_1.status, "FAILED");
            assert_eq!(job_2.status, "QUEUED");
        });
    }

    #[test]
    fn test_query_dispatched_job_for_agent() {
        with_test_db(|db| {
            {
                let conn = db.conn();
                insert_job_sync(&conn, "job_1", "a1", None, "one").unwrap();
            }
            assert!(
                query_dispatched_job_for_agent_sync(&db.conn(), "a1")
                    .unwrap()
                    .is_none()
            );
            claim_next_job_sync(db, "a1").unwrap().unwrap();
            let dispatched = query_dispatched_job_for_agent_sync(&db.conn(), "a1")
                .unwrap()
                .unwrap();
            assert_eq!(dispatched.id, "job_1");
        });
    }

    #[test]
    fn test_update_dispatched_seq_id() {
        with_test_db(|db| {
            {
                let conn = db.conn();
                insert_job_sync(&conn, "job_1", "a1", None, "one").unwrap();
            }
            claim_next_job_sync(db, "a1").unwrap().unwrap();
            {
                let conn = db.conn();
                assert_eq!(
                    update_dispatched_seq_id_sync(&conn, "job_1", 42).unwrap(),
                    1
                );
                let job = query_job_sync(&conn, "job_1").unwrap().unwrap();
                assert_eq!(job.dispatched_at_seq_id, Some(42));
            }
        });
    }

    #[test]
    fn test_collect_reply_for_dispatched_job_uses_seq_id_boundary() {
        with_test_db(|db| {
            let (before, after) = {
                let conn = db.conn();
                insert_job_sync(&conn, "job_1", "a1", None, "one").unwrap();
                let before =
                    insert_event_sync(&conn, "a1", None, "output_chunk", r#"{"text":"old"}"#)
                        .unwrap();
                let after =
                    insert_event_sync(&conn, "a1", None, "output_chunk", r#"{"text":"new"}"#)
                        .unwrap();
                (before, after)
            };
            let conn = db.conn();
            let reply = collect_reply_for_dispatched_job_sync(&conn, "a1", Some(before)).unwrap();

            assert!(after > before);
            assert_eq!(reply, "new");
        });
    }
}
