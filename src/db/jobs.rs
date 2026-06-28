use crate::db::Db;
use crate::db::common::{is_unique_constraint_error, map_db_error, spawn_db};
use crate::db::schema::Job;
use crate::db::state_machine::{STATE_IDLE, transit_agent_state_conn_sync};
use crate::error::CcbdError;
use rusqlite::{Connection, OptionalExtension, Row, TransactionBehavior, params};
use serde_json::{Map, Value};

pub(crate) const RECOVERY_REQUEUED_ERROR_REASON_PREFIX: &str = "RECOVERY_REQUEUED:";
pub(crate) const RECOVERY_REQUEUED_MAX_STALE_PANE_RETRIES: u32 = 1;

#[derive(Debug, Clone)]
pub struct DispatchedJob {
    pub job: Job,
    pub seq_id: i64,
    pub job_payload: Value,
}

pub(crate) fn recovery_requeued_error_reason(attempt: u32) -> String {
    format!("{RECOVERY_REQUEUED_ERROR_REASON_PREFIX}{attempt}")
}

pub(crate) fn recovery_requeued_attempt(error_reason: Option<&str>) -> Option<u32> {
    error_reason?
        .strip_prefix(RECOVERY_REQUEUED_ERROR_REASON_PREFIX)?
        .parse()
        .ok()
}

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
        requires_physical_evidence: row.get::<_, i64>(12)? != 0,
        requires_test_evidence: row.get::<_, i64>(13)? != 0,
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

pub(crate) fn insert_recovered_queued_job_sync(
    conn: &Connection,
    id: &str,
    agent_id: &str,
    request_id: Option<&str>,
    prompt_text: &str,
    cancel_requested: bool,
    requires_physical_evidence: bool,
    requires_test_evidence: bool,
    error_reason: &str,
) -> Result<String, CcbdError> {
    let result = conn.execute(
        "INSERT INTO jobs (
             id, agent_id, request_id, prompt_text, status, error_reason,
             cancel_requested, requires_physical_evidence, requires_test_evidence
         )
         VALUES (?, ?, ?, ?, 'QUEUED', ?, ?, ?, ?)",
        params![
            id,
            agent_id,
            request_id,
            prompt_text,
            error_reason,
            i64::from(cancel_requested),
            i64::from(requires_physical_evidence),
            i64::from(requires_test_evidence),
        ],
    );

    match result {
        Ok(_) => Ok(id.to_string()),
        Err(err) if is_unique_constraint_error(&err) && request_id.is_some() => {
            let existing = query_job_by_request_id_sync(conn, agent_id, request_id.unwrap())?
                .ok_or_else(|| map_db_error("query duplicate recovered job by request_id", err))?;
            Ok(existing.id)
        }
        Err(err) => Err(map_db_error("insert recovered queued job", err)),
    }
}

pub(crate) fn query_job_sync(conn: &Connection, job_id: &str) -> Result<Option<Job>, CcbdError> {
    conn.query_row(
        "SELECT id, agent_id, request_id, prompt_text, reply_text, status, error_reason, created_at, dispatched_at, dispatched_at_seq_id, completed_at, cancel_requested, requires_physical_evidence, requires_test_evidence FROM jobs WHERE id = ?",
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
        "SELECT id, agent_id, request_id, prompt_text, reply_text, status, error_reason, created_at, dispatched_at, dispatched_at_seq_id, completed_at, cancel_requested, requires_physical_evidence, requires_test_evidence FROM jobs WHERE agent_id = ? AND request_id = ? LIMIT 1",
        params![agent_id, request_id],
        row_to_job,
    )
    .optional()
    .map_err(|err| map_db_error("query job by request_id", err))
}

pub(crate) fn has_queued_job_sync(conn: &Connection, agent_id: &str) -> Result<bool, CcbdError> {
    conn.query_row(
        "SELECT 1 FROM jobs WHERE agent_id = ? AND status = 'QUEUED' ORDER BY created_at ASC, rowid ASC LIMIT 1",
        params![agent_id],
        |_| Ok(true),
    )
    .optional()
    .map(|value| value.unwrap_or(false))
    .map_err(|err| map_db_error("query queued job exists", err))
}

pub(crate) fn claim_next_job_sync(db: &Db, agent_id: &str) -> Result<Option<Job>, CcbdError> {
    let mut conn = db.conn();
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|err| map_db_error("begin claim next job", err))?;

    let agent_state = tx
        .query_row(
            "SELECT state FROM agents WHERE id = ?",
            params![agent_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|err| map_db_error("query agent state before claim next job", err))?
        .unwrap_or_default();
    if agent_state != STATE_IDLE {
        tracing::info!(
            agent_id,
            state = %agent_state,
            reason = "agent is not IDLE",
            impact = "queued jobs remain queued and dispatch is blocked",
            "claim next job skipped"
        );
        tx.commit()
            .map_err(|err| map_db_error("commit skipped claim next job", err))?;
        return Ok(None);
    }

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
            "SELECT id, agent_id, request_id, prompt_text, reply_text, status, error_reason, created_at, dispatched_at, dispatched_at_seq_id, completed_at, cancel_requested, requires_physical_evidence, requires_test_evidence FROM jobs WHERE id = ?",
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

pub fn dispatch_job_to_agent_sync(
    conn: &mut Connection,
    agent_id: &str,
    expected_from_state: &[&str],
    new_state: &str,
    event_kind: &str,
    event_payload: &Value,
) -> Result<Option<DispatchedJob>, CcbdError> {
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|err| map_db_error("begin dispatch job to agent", err))?;

    let current_state = tx
        .query_row(
            "SELECT state FROM agents WHERE id = ?",
            params![agent_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|err| map_db_error("query agent state before dispatch", err))?;
    let Some(current_state) = current_state else {
        return Err(CcbdError::AgentNotFound(agent_id.to_string()));
    };
    if !expected_from_state.is_empty() && !expected_from_state.contains(&current_state.as_str()) {
        tracing::info!(
            agent_id,
            state = %current_state,
            expected = ?expected_from_state,
            reason = "agent state is not dispatchable",
            impact = "queued jobs remain queued and dispatch is blocked",
            "dispatch job to agent rejected"
        );
        return Err(CcbdError::AgentWrongState { current_state });
    }

    let candidate_id = tx
        .query_row(
            "SELECT id FROM jobs WHERE agent_id = ? AND status = 'QUEUED' ORDER BY created_at ASC, rowid ASC LIMIT 1",
            params![agent_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|err| map_db_error("query queued job for dispatch", err))?;

    let Some(job_id) = candidate_id else {
        tx.commit()
            .map_err(|err| map_db_error("commit empty dispatch job to agent", err))?;
        return Ok(None);
    };
    let changes = tx
        .execute(
            "UPDATE jobs SET status = 'DISPATCHED', dispatched_at = unixepoch() WHERE id = ? AND status = 'QUEUED'",
            params![job_id],
        )
        .map_err(|err| map_db_error("mark job dispatched", err))?;
    if changes != 1 {
        return Err(map_db_error(
            "mark job dispatched",
            rusqlite::Error::QueryReturnedNoRows,
        ));
    }

    transit_agent_state_conn_sync(
        &tx,
        agent_id,
        expected_from_state,
        new_state,
        Some("dispatched"),
    )?;

    let job = tx
        .query_row(
            "SELECT id, agent_id, request_id, prompt_text, reply_text, status, error_reason, created_at, dispatched_at, dispatched_at_seq_id, completed_at, cancel_requested, requires_physical_evidence, requires_test_evidence FROM jobs WHERE id = ?",
            params![job_id],
            row_to_job,
        )
        .map_err(|err| map_db_error("query dispatched job", err))?;
    let job_payload = dispatch_event_payload(event_payload, &job);
    tx.execute(
        "INSERT INTO events (agent_id, request_id, event_type, payload) VALUES (?, NULL, ?, ?)",
        params![agent_id, event_kind, job_payload.to_string()],
    )
    .map_err(|err| map_db_error("insert dispatch event", err))?;
    let seq_id = tx.last_insert_rowid();

    tx.execute(
        "UPDATE jobs SET dispatched_at_seq_id = ? WHERE id = ? AND status = 'DISPATCHED'",
        params![seq_id, job.id],
    )
    .map_err(|err| map_db_error("update dispatched seq_id during dispatch", err))?;

    let job = tx
        .query_row(
            "SELECT id, agent_id, request_id, prompt_text, reply_text, status, error_reason, created_at, dispatched_at, dispatched_at_seq_id, completed_at, cancel_requested, requires_physical_evidence, requires_test_evidence FROM jobs WHERE id = ?",
            params![job.id],
            row_to_job,
        )
        .map_err(|err| map_db_error("query dispatched job with seq_id", err))?;

    tx.commit()
        .map_err(|err| map_db_error("commit dispatch job to agent", err))?;
    Ok(Some(DispatchedJob {
        job,
        seq_id,
        job_payload,
    }))
}

fn dispatch_event_payload(base: &Value, job: &Job) -> Value {
    let mut payload = match base {
        Value::Object(map) => map.clone(),
        _ => Map::new(),
    };
    payload
        .entry("cmd".to_string())
        .or_insert_with(|| Value::String(job.prompt_text.clone()));
    payload
        .entry("job_id".to_string())
        .or_insert_with(|| Value::String(job.id.clone()));
    payload
        .entry("status".to_string())
        .or_insert_with(|| Value::String("SENT".to_string()));
    let env = payload
        .entry("env".to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    if let Value::Object(env) = env {
        env.entry("CCB_JOB_ID".to_string())
            .or_insert_with(|| Value::String(job.id.clone()));
    }
    Value::Object(payload)
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

pub(crate) fn set_job_evidence_requirements_sync(
    conn: &Connection,
    job_id: &str,
    requires_physical_evidence: bool,
    requires_test_evidence: bool,
) -> Result<usize, CcbdError> {
    conn.execute(
        "UPDATE jobs SET requires_physical_evidence = ?, requires_test_evidence = ? WHERE id = ?",
        params![
            i64::from(requires_physical_evidence),
            i64::from(requires_test_evidence),
            job_id
        ],
    )
    .map_err(|err| map_db_error("set job evidence requirements", err))
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

pub(crate) fn requeue_recovered_dispatch_io_failure_sync(
    conn: &mut Connection,
    job_id: &str,
    agent_id: &str,
    next_attempt: u32,
    reason: &str,
) -> Result<usize, CcbdError> {
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|err| map_db_error("begin requeue recovered dispatch failure", err))?;
    let changed = tx
        .execute(
            "UPDATE jobs
             SET status = 'QUEUED',
                 dispatched_at = NULL,
                 dispatched_at_seq_id = NULL,
                 completed_at = NULL,
                 error_reason = ?
             WHERE id = ?
               AND agent_id = ?
               AND status = 'DISPATCHED'
               AND error_reason LIKE 'RECOVERY_REQUEUED:%'",
            params![
                recovery_requeued_error_reason(next_attempt),
                job_id,
                agent_id
            ],
        )
        .map_err(|err| map_db_error("requeue recovered dispatch failure job", err))?;
    if changed == 1 {
        tx.execute(
            "UPDATE agents
             SET state = 'IDLE',
                 state_version = state_version + 1,
                 updated_at = unixepoch()
             WHERE id = ?
               AND state IN ('WAITING_FOR_ACK', 'STUCK')",
            params![agent_id],
        )
        .map_err(|err| map_db_error("restore recovered dispatch failure agent idle", err))?;
        tx.execute(
            "INSERT INTO events (agent_id, request_id, event_type, payload)
             VALUES (?, NULL, 'dispatch_requeued', ?)",
            params![
                agent_id,
                serde_json::json!({
                    "job_id": job_id,
                    "reason": reason,
                    "attempt": next_attempt,
                    "source": "recovered_stale_pane"
                })
                .to_string()
            ],
        )
        .map_err(|err| map_db_error("insert recovered dispatch requeue event", err))?;
    }
    tx.commit()
        .map_err(|err| map_db_error("commit requeue recovered dispatch failure", err))?;
    Ok(changed)
}

pub(crate) fn requeue_dispatched_job_before_send_sync(
    conn: &mut Connection,
    job_id: &str,
    agent_id: &str,
    reason: &str,
) -> Result<usize, CcbdError> {
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|err| map_db_error("begin requeue pre-send dispatch", err))?;
    let changed = tx
        .execute(
            "UPDATE jobs
             SET status = 'QUEUED',
                 dispatched_at = NULL,
                 dispatched_at_seq_id = NULL,
                 completed_at = NULL
             WHERE id = ?
               AND agent_id = ?
               AND status = 'DISPATCHED'",
            params![job_id, agent_id],
        )
        .map_err(|err| map_db_error("requeue pre-send dispatch job", err))?;
    if changed == 1 {
        tx.execute(
            "UPDATE agents
             SET state = 'IDLE',
                 state_version = state_version + 1,
                 updated_at = unixepoch()
             WHERE id = ?
               AND state = 'WAITING_FOR_ACK'",
            params![agent_id],
        )
        .map_err(|err| map_db_error("restore pre-send dispatch agent idle", err))?;
        tx.execute(
            "INSERT INTO events (agent_id, request_id, event_type, payload)
             VALUES (?, NULL, 'dispatch_requeued', ?)",
            params![
                agent_id,
                serde_json::json!({
                    "job_id": job_id,
                    "reason": reason,
                    "source": "pre_send_readiness_recheck"
                })
                .to_string()
            ],
        )
        .map_err(|err| map_db_error("insert pre-send dispatch requeue event", err))?;
    }
    tx.commit()
        .map_err(|err| map_db_error("commit requeue pre-send dispatch", err))?;
    Ok(changed)
}

pub(crate) fn clear_recovered_dispatch_marker_sync(
    db: &Db,
    job_id: &str,
) -> Result<usize, CcbdError> {
    let conn = db.conn();
    conn.execute(
        "UPDATE jobs
         SET error_reason = NULL
         WHERE id = ?
           AND status IN ('DISPATCHED', 'COMPLETED')
           AND error_reason LIKE 'RECOVERY_REQUEUED:%'",
        params![job_id],
    )
    .map_err(|err| map_db_error("clear recovered dispatch marker", err))
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
        "SELECT id, agent_id, request_id, prompt_text, reply_text, status, error_reason, created_at, dispatched_at, dispatched_at_seq_id, completed_at, cancel_requested, requires_physical_evidence, requires_test_evidence FROM jobs WHERE agent_id = ? AND status = 'DISPATCHED' ORDER BY dispatched_at ASC, id ASC LIMIT 1",
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
    prompt_text: &str,
) -> Result<String, CcbdError> {
    let Some(dispatched_at_seq_id) = dispatched_at_seq_id else {
        tracing::warn!(
            agent_id,
            "collect_reply: dispatched_at_seq_id is None, returning empty"
        );
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

    let mut parser = vt100::Parser::new(500, 250, 5000);
    let mut chunk_count = 0u32;
    let mut raw_bytes_total = 0usize;
    let mut raw_concat = String::new();
    for payload in rows {
        let payload = payload.map_err(|err| map_db_error("collect job reply payload", err))?;
        let value: Value = serde_json::from_str(&payload).map_err(|err| {
            CcbdError::DbConstraintViolation(format!("parse output_chunk payload: {err}"))
        })?;
        if let Some(text) = value.get("text").and_then(Value::as_str) {
            chunk_count += 1;
            raw_bytes_total += text.len();
            raw_concat.push_str(text);
            parser.process(text.as_bytes());
        }
    }
    let screen_text = parser.screen().contents();
    let reply = distill_reply(&screen_text, prompt_text);

    let reply = if reply.is_empty() && raw_bytes_total > 0 {
        let fallback = distill_reply(&strip_ansi_escapes(&raw_concat), prompt_text);
        tracing::warn!(
            agent_id,
            raw_bytes_total,
            fallback_len = fallback.len(),
            "collect_reply: vt100 screen empty, using strip_ansi fallback"
        );
        fallback
    } else {
        tracing::info!(
            agent_id,
            chunk_count,
            raw_bytes_total,
            screen_text_len = screen_text.len(),
            reply_len = reply.len(),
            "collect_reply complete"
        );
        reply
    };
    Ok(reply)
}

pub(crate) fn distill_reply(raw: &str, prompt_text: &str) -> String {
    let without_ansi = strip_ansi_escapes(raw);
    let current_turn = if let Some(end) = prompt_match_end_byte(&without_ansi, prompt_text) {
        &without_ansi[end..]
    } else {
        without_ansi.as_str()
    };
    let mut lines: Vec<&str> = Vec::new();
    for line in current_turn.lines() {
        let trimmed = line.trim();
        if is_prompt_echo_line(trimmed, prompt_text) {
            continue;
        }
        if trimmed.eq_ignore_ascii_case("thinking...")
            || trimmed.starts_with("Thinking...")
            || trimmed.starts_with("Working(")
            || trimmed.starts_with("• Working(")
            || is_reply_chrome_line(trimmed)
        {
            continue;
        }
        lines.push(line);
    }
    let mut text = lines.join("\n");
    if !prompt_text.is_empty()
        && let Some(start) = text.find(prompt_text)
    {
        let end = start + prompt_text.len();
        text.replace_range(start..end, "");
    }
    text.trim().to_string()
}

pub(crate) fn contains_prompt_text(raw: &str, prompt_text: &str) -> bool {
    if prompt_text.trim().is_empty() {
        return false;
    }
    let without_ansi = strip_ansi_escapes(raw);
    prompt_match_end_byte(&without_ansi, prompt_text).is_some()
}

fn prompt_match_end_byte(raw_without_ansi: &str, prompt_text: &str) -> Option<usize> {
    let (normalized_raw, raw_end_offsets) = normalize_whitespace_with_end_offsets(raw_without_ansi);
    let (normalized_prompt, _) = normalize_whitespace_with_end_offsets(prompt_text);
    if normalized_prompt.is_empty() {
        return None;
    }
    let start = normalized_raw.find(&normalized_prompt)?;
    let end = start + normalized_prompt.len();
    let end_chars = normalized_raw[..end].chars().count();
    raw_end_offsets.get(end_chars.checked_sub(1)?).copied()
}

fn normalize_whitespace_with_end_offsets(input: &str) -> (String, Vec<usize>) {
    let mut normalized = String::new();
    let mut end_offsets = Vec::new();
    let mut in_whitespace = false;
    for (idx, ch) in input.char_indices() {
        let end = idx + ch.len_utf8();
        if ch.is_whitespace() {
            if !normalized.is_empty() && !in_whitespace {
                normalized.push(' ');
                end_offsets.push(end);
                in_whitespace = true;
            }
            continue;
        }
        normalized.push(ch);
        end_offsets.push(end);
        in_whitespace = false;
    }
    if normalized.ends_with(' ') {
        normalized.pop();
        end_offsets.pop();
    }
    (normalized, end_offsets)
}

fn is_prompt_echo_line(trimmed: &str, prompt_text: &str) -> bool {
    if prompt_text.is_empty() || trimmed.is_empty() {
        return false;
    }
    if trimmed == prompt_text {
        return true;
    }
    let Some(without_marker) = strip_prompt_marker(trimmed) else {
        return false;
    };
    let without_marker = without_marker.trim();
    !without_marker.is_empty()
        && (without_marker == prompt_text || prompt_text.starts_with(without_marker))
}

fn strip_prompt_marker(line: &str) -> Option<&str> {
    ["> ", "❯ ", "✦ ", "* "]
        .into_iter()
        .find_map(|marker| line.strip_prefix(marker))
}

fn is_reply_chrome_line(trimmed: &str) -> bool {
    if trimmed.is_empty() {
        return false;
    }
    if is_box_drawing_line(trimmed) {
        return true;
    }
    if matches!(trimmed, ">" | "❯" | "✦") {
        return true;
    }
    if trimmed.starts_with("▸ Thought") || trimmed.starts_with("▸ Thinking") {
        return true;
    }
    trimmed.starts_with("? for shortcuts") || trimmed.contains("esc to cancel")
}

fn is_box_drawing_line(trimmed: &str) -> bool {
    trimmed
        .chars()
        .all(|ch| ('\u{2500}'..='\u{257f}').contains(&ch))
}

pub(crate) fn strip_ansi_escapes(input: &str) -> String {
    let mut out = String::new();
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '\u{1b}' {
            out.push(ch);
            continue;
        }

        let Some(&next) = chars.peek() else {
            break;
        };
        match next {
            '[' => {
                let _ = chars.next();
                for c in chars.by_ref() {
                    if ('@'..='~').contains(&c) {
                        break;
                    }
                }
            }
            ']' => {
                let _ = chars.next();
                while let Some(&c) = chars.peek() {
                    if c == '\u{07}' {
                        let _ = chars.next();
                        break;
                    }
                    if c == '\u{1b}' {
                        let _ = chars.next();
                        if chars.peek() == Some(&'\\') {
                            let _ = chars.next();
                            break;
                        }
                    } else {
                        let _ = chars.next();
                    }
                }
            }
            'P' | '_' | '^' | 'X' => {
                let _ = chars.next();
                while let Some(&c) = chars.peek() {
                    if c == '\u{1b}' {
                        let _ = chars.next();
                        if chars.peek() == Some(&'\\') {
                            let _ = chars.next();
                            break;
                        }
                    } else {
                        let _ = chars.next();
                    }
                }
            }
            _ => {
                let _ = chars.next();
            }
        }
    }
    out
}

#[allow(dead_code)]
pub(crate) fn strip_ansi_csi(input: &str) -> String {
    strip_ansi_escapes(input)
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

pub async fn has_queued_job(db: Db, agent_id: String) -> Result<bool, CcbdError> {
    spawn_db("jobs::has_queued_job", move || {
        let conn = db.conn();
        has_queued_job_sync(&conn, &agent_id)
    })
    .await
}

pub async fn dispatch_job_to_agent(
    db: Db,
    agent_id: String,
    expected_from_state: Vec<String>,
    new_state: String,
    event_kind: String,
    event_payload: Value,
) -> Result<Option<DispatchedJob>, CcbdError> {
    spawn_db("jobs::dispatch_job_to_agent", move || {
        let mut fresh_conn;
        let mut locked_conn;
        let conn = if let Some(conn) = db.try_conn()? {
            locked_conn = conn;
            &mut *locked_conn
        } else {
            fresh_conn = db.fresh_conn()?;
            &mut fresh_conn
        };
        let from_states = expected_from_state
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>();
        dispatch_job_to_agent_sync(
            conn,
            &agent_id,
            &from_states,
            &new_state,
            &event_kind,
            &event_payload,
        )
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

pub(crate) async fn requeue_recovered_dispatch_io_failure(
    db: Db,
    job_id: String,
    agent_id: String,
    next_attempt: u32,
    reason: String,
) -> Result<usize, CcbdError> {
    let notify_job_id = job_id.clone();
    spawn_db("jobs::requeue_recovered_dispatch_io_failure", move || {
        let mut fresh_conn;
        let mut locked_conn;
        let conn = if let Some(conn) = db.try_conn()? {
            locked_conn = conn;
            &mut *locked_conn
        } else {
            fresh_conn = db.fresh_conn()?;
            &mut fresh_conn
        };
        requeue_recovered_dispatch_io_failure_sync(conn, &job_id, &agent_id, next_attempt, &reason)
    })
    .await
    .inspect(|changes| {
        if *changes > 0 {
            crate::orchestrator::pubsub::notify_job_update(&notify_job_id);
            crate::orchestrator::wake_up();
        }
    })
}

pub(crate) async fn requeue_dispatched_job_before_send(
    db: Db,
    job_id: String,
    agent_id: String,
    reason: String,
) -> Result<usize, CcbdError> {
    let notify_job_id = job_id.clone();
    spawn_db("jobs::requeue_dispatched_job_before_send", move || {
        let mut fresh_conn;
        let mut locked_conn;
        let conn = if let Some(conn) = db.try_conn()? {
            locked_conn = conn;
            &mut *locked_conn
        } else {
            fresh_conn = db.fresh_conn()?;
            &mut fresh_conn
        };
        requeue_dispatched_job_before_send_sync(conn, &job_id, &agent_id, &reason)
    })
    .await
    .inspect(|changes| {
        if *changes > 0 {
            crate::orchestrator::pubsub::notify_job_update(&notify_job_id);
            crate::orchestrator::wake_up();
        }
    })
}

pub(crate) async fn clear_recovered_dispatch_marker(
    db: Db,
    job_id: String,
) -> Result<usize, CcbdError> {
    spawn_db("jobs::clear_recovered_dispatch_marker", move || {
        clear_recovered_dispatch_marker_sync(&db, &job_id)
    })
    .await
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
    prompt_text: String,
) -> Result<String, CcbdError> {
    spawn_db("jobs::collect_reply_for_dispatched_job", move || {
        let conn = db.conn();
        collect_reply_for_dispatched_job_sync(&conn, &agent_id, dispatched_at_seq_id, &prompt_text)
    })
    .await
}

pub async fn set_job_evidence_requirements(
    db: Db,
    job_id: String,
    requires_physical_evidence: bool,
    requires_test_evidence: bool,
) -> Result<usize, CcbdError> {
    spawn_db("jobs::set_job_evidence_requirements", move || {
        let conn = db.conn();
        set_job_evidence_requirements_sync(
            &conn,
            &job_id,
            requires_physical_evidence,
            requires_test_evidence,
        )
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::{
        claim_next_job_sync, collect_reply_for_dispatched_job_sync, dispatch_job_to_agent_sync,
        distill_reply, insert_job_sync, mark_dispatched_job_cancelled_if_agent_idle_sync,
        mark_dispatched_jobs_failed_for_agent_sync, mark_job_completed_sync, mark_job_failed_sync,
        mark_queued_job_cancelled_sync, query_dispatched_job_for_agent_sync,
        query_job_by_request_id_sync, query_job_sync, request_dispatched_job_cancel_sync,
        set_job_evidence_requirements_sync, strip_ansi_csi, update_dispatched_seq_id_sync,
    };
    use crate::db::agents::insert_agent_sync;
    use crate::db::events::insert_event_sync;
    use crate::db::sessions::insert_session_sync;
    use crate::db::state_machine::{
        STATE_BUSY, STATE_IDLE, STATE_PROMPT_PENDING, STATE_WAITING_FOR_ACK,
    };
    use crate::db::{Db, init};
    use crate::error::CcbdError;

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
    fn test_distill_reply_strips_ansi_escapes() {
        let raw = "\u{1b}[31mactual reply\u{1b}[0m";

        assert_eq!(distill_reply(raw, ""), "actual reply");
    }

    #[test]
    fn test_distill_reply_removes_prompt_echo() {
        let raw = "echo prompt\nactual reply";

        assert_eq!(distill_reply(raw, "echo prompt"), "actual reply");
    }

    #[test]
    fn test_distill_reply_removes_prompt_marker_echo_line() {
        let raw = "> echo prompt\nactual reply";

        assert_eq!(distill_reply(raw, "echo prompt"), "actual reply");
    }

    #[test]
    fn test_distill_reply_trims_whitespace() {
        let raw = "\n  actual reply \n\n";

        assert_eq!(distill_reply(raw, ""), "actual reply");
    }

    #[test]
    fn test_distill_reply_removes_antigravity_chrome() {
        let raw = "reply PONG\n\
                   > \n\n\
                     PONG\n\n\
                   ────────────────────────────────────────────────\n\
                   >\n\
                   ────────────────────────────────────────────────\n\
                   ? for shortcuts                          Gemini 3.5 Flash (High)\n";

        assert_eq!(distill_reply(raw, "reply PONG"), "PONG");
    }

    #[test]
    fn test_distill_reply_removes_antigravity_prompt_echo_and_thought_summary() {
        let prompt = "In two sentences, what is X?";
        let raw = "> In two sentences, what is X?\n\n\
                   ▸ Thought for 1s, 306 tokens\n\
                     Defining X\n\
                     X is a thing.\n\
                   ────────────────────────────────────────────────\n\
                   >\n\
                   ? for shortcuts                          Gemini 3.5 Flash (High)\n";

        assert_eq!(distill_reply(raw, prompt), "Defining X\nX is a thing.");
    }

    #[test]
    fn test_distill_reply_anchors_soft_wrapped_antigravity_prompt() {
        let prompt = "Please reply with exactly one single word and nothing else, no punctuation no explanation no commentary whatsoever, and the one word you must reply with is: delta";
        let raw = include_str!(
            "../../.kiro/specs/ah-hook-push-completion/REAL-a3-idle-longprompt-wrapped.txt"
        );

        assert_eq!(distill_reply(raw, prompt), "delta");
    }

    #[test]
    fn test_distill_reply_keeps_claude_content() {
        let raw = "summarize\nHere is the result:\n- alpha\n- beta\n❯";

        assert_eq!(
            distill_reply(raw, "summarize"),
            "Here is the result:\n- alpha\n- beta"
        );
    }

    #[test]
    fn test_strip_ansi_escapes_handles_osc_window_title() {
        let raw = "\u{1b}]0;some title\u{07}content";

        assert_eq!(strip_ansi_csi(raw), "content");
    }

    #[test]
    fn test_strip_ansi_escapes_handles_osc_with_st_terminator() {
        let raw = "\u{1b}]0;title\u{1b}\\content";

        assert_eq!(strip_ansi_csi(raw), "content");
    }

    #[test]
    fn test_strip_ansi_escapes_handles_simple_esc() {
        let raw = "\u{1b}=hello";

        assert_eq!(strip_ansi_csi(raw), "hello");
    }

    #[test]
    fn test_strip_ansi_escapes_real_codex_status_bar() {
        let raw = "Use /skills to list available skills\n\
                   \u{1b}]0;⠴ ccbd-rust•Working(0s • esc to interrupt)\u{07}\
                   › echo from a1\n\
                   \u{1b}]0;⠦ ccbd-rust\u{1b}\\clean reply";

        let stripped = strip_ansi_csi(raw);

        assert!(!stripped.contains("]0;"));
        assert!(!stripped.contains("ccbd-rust•Working"));
        assert!(stripped.contains("Use /skills to list available skills"));
        assert!(stripped.contains("› echo from a1"));
        assert!(stripped.contains("clean reply"));
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
    fn test_claim_next_job_skips_stuck_agent() {
        with_test_db(|db| {
            {
                let conn = db.conn();
                conn.execute("UPDATE agents SET state = 'STUCK' WHERE id = 'a1'", [])
                    .unwrap();
                insert_job_sync(&conn, "job_stuck", "a1", None, "one").unwrap();
            }

            let claimed = claim_next_job_sync(db, "a1").unwrap();
            let job = query_job_sync(&db.conn(), "job_stuck").unwrap().unwrap();

            assert!(claimed.is_none());
            assert_eq!(job.status, "QUEUED");
        });
    }

    #[test]
    fn test_claim_next_job_skips_waiting_for_ack_agent() {
        with_test_db(|db| {
            {
                let conn = db.conn();
                conn.execute(
                    "UPDATE agents SET state = ? WHERE id = 'a1'",
                    [STATE_WAITING_FOR_ACK],
                )
                .unwrap();
                insert_job_sync(&conn, "job_waiting", "a1", None, "one").unwrap();
            }

            let claimed = claim_next_job_sync(db, "a1").unwrap();
            let job = query_job_sync(&db.conn(), "job_waiting").unwrap().unwrap();

            assert!(claimed.is_none());
            assert_eq!(job.status, "QUEUED");
        });
    }

    #[test]
    fn test_claim_next_job_skips_prompt_pending_agent() {
        with_test_db(|db| {
            {
                let conn = db.conn();
                conn.execute(
                    "UPDATE agents SET state = ? WHERE id = 'a1'",
                    [STATE_PROMPT_PENDING],
                )
                .unwrap();
                insert_job_sync(&conn, "job_prompt_pending", "a1", None, "one").unwrap();
            }

            let claimed = claim_next_job_sync(db, "a1").unwrap();
            let job = query_job_sync(&db.conn(), "job_prompt_pending")
                .unwrap()
                .unwrap();

            assert!(claimed.is_none());
            assert_eq!(job.status, "QUEUED");
        });
    }

    #[test]
    fn test_dispatch_job_to_agent_sync_updates_metadata_atomically() {
        with_test_db(|db| {
            {
                let conn = db.conn();
                insert_job_sync(&conn, "job_dispatch", "a1", None, "ask").unwrap();
            }

            let mut conn = db.conn();
            let dispatched = dispatch_job_to_agent_sync(
                &mut conn,
                "a1",
                &[STATE_IDLE],
                STATE_WAITING_FOR_ACK,
                "command_received",
                &serde_json::json!({ "status": "SENT" }),
            )
            .unwrap()
            .unwrap();

            assert_eq!(dispatched.job.id, "job_dispatch");
            assert_eq!(dispatched.job.status, "DISPATCHED");
            assert_eq!(dispatched.job.dispatched_at_seq_id, Some(dispatched.seq_id));
            assert_eq!(dispatched.job_payload["cmd"], "ask");
            assert_eq!(dispatched.job_payload["job_id"], "job_dispatch");
            let (agent_state, job_status, seq_id, command_events, state_events): (
                String,
                String,
                Option<i64>,
                i64,
                i64,
            ) = conn
                .query_row(
                    "SELECT agents.state, jobs.status, jobs.dispatched_at_seq_id, \
                            (SELECT COUNT(*) FROM events WHERE agent_id = 'a1' AND event_type = 'command_received'), \
                            (SELECT COUNT(*) FROM events WHERE agent_id = 'a1' AND event_type = 'state_change') \
                     FROM agents JOIN jobs ON jobs.agent_id = agents.id WHERE jobs.id = 'job_dispatch'",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
                )
                .unwrap();

            assert_eq!(agent_state, STATE_WAITING_FOR_ACK);
            assert_eq!(job_status, "DISPATCHED");
            assert_eq!(seq_id, Some(dispatched.seq_id));
            assert_eq!(command_events, 1);
            assert_eq!(state_events, 1);
        });
    }

    #[test]
    fn test_dispatch_job_to_agent_sync_rejects_wrong_state_and_rolls_back() {
        with_test_db(|db| {
            {
                let conn = db.conn();
                conn.execute("UPDATE agents SET state = ? WHERE id = 'a1'", [STATE_BUSY])
                    .unwrap();
                insert_job_sync(&conn, "job_busy", "a1", None, "ask").unwrap();
            }

            let mut conn = db.conn();
            let err = dispatch_job_to_agent_sync(
                &mut conn,
                "a1",
                &[STATE_IDLE],
                STATE_WAITING_FOR_ACK,
                "command_received",
                &serde_json::json!({ "status": "SENT" }),
            )
            .unwrap_err();
            let (agent_state, job_status, event_count): (String, String, i64) = conn
                .query_row(
                    "SELECT agents.state, jobs.status, COUNT(events.seq_id) \
                     FROM agents \
                     JOIN jobs ON jobs.agent_id = agents.id \
                     LEFT JOIN events ON events.agent_id = agents.id \
                     WHERE jobs.id = 'job_busy' \
                     GROUP BY agents.id, jobs.id",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .unwrap();

            assert!(matches!(err, CcbdError::AgentWrongState { .. }));
            assert_eq!(agent_state, STATE_BUSY);
            assert_eq!(job_status, "QUEUED");
            assert_eq!(event_count, 0);
        });
    }

    #[test]
    fn test_dispatch_job_to_agent_sync_rejects_prompt_pending_and_keeps_job_queued() {
        with_test_db(|db| {
            {
                let conn = db.conn();
                conn.execute(
                    "UPDATE agents SET state = ? WHERE id = 'a1'",
                    [STATE_PROMPT_PENDING],
                )
                .unwrap();
                insert_job_sync(&conn, "job_pending_dispatch", "a1", None, "hello").unwrap();
            }

            let mut conn = db.conn();
            let err = dispatch_job_to_agent_sync(
                &mut conn,
                "a1",
                &[STATE_IDLE],
                STATE_WAITING_FOR_ACK,
                "command_received",
                &serde_json::json!({ "status": "SENT" }),
            )
            .unwrap_err();
            let (agent_state, job_status, event_count): (String, String, i64) = conn
                .query_row(
                    "SELECT agents.state, jobs.status, COUNT(events.seq_id) \
                     FROM agents \
                     JOIN jobs ON jobs.agent_id = agents.id \
                     LEFT JOIN events ON events.agent_id = agents.id \
                     WHERE agents.id = 'a1'",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .unwrap();

            assert!(matches!(err, CcbdError::AgentWrongState { .. }));
            assert_eq!(agent_state, STATE_PROMPT_PENDING);
            assert_eq!(job_status, "QUEUED");
            assert_eq!(event_count, 0);
        });
    }

    #[test]
    fn test_dispatch_job_to_agent_sync_missing_agent_errors_without_side_effects() {
        with_test_db(|db| {
            let mut conn = db.conn();
            let err = dispatch_job_to_agent_sync(
                &mut conn,
                "missing",
                &[STATE_IDLE],
                STATE_WAITING_FOR_ACK,
                "command_received",
                &serde_json::json!({ "status": "SENT" }),
            )
            .unwrap_err();

            assert!(matches!(err, CcbdError::AgentNotFound(agent) if agent == "missing"));
        });
    }

    #[test]
    fn test_dispatch_job_to_agent_sync_no_job_has_no_side_effects() {
        with_test_db(|db| {
            let mut conn = db.conn();
            let dispatched = dispatch_job_to_agent_sync(
                &mut conn,
                "a1",
                &[STATE_IDLE],
                STATE_WAITING_FOR_ACK,
                "command_received",
                &serde_json::json!({ "status": "SENT" }),
            )
            .unwrap();
            let (agent_state, event_count): (String, i64) = conn
                .query_row(
                    "SELECT state, (SELECT COUNT(*) FROM events WHERE agent_id = 'a1') FROM agents WHERE id = 'a1'",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .unwrap();

            assert!(dispatched.is_none());
            assert_eq!(agent_state, STATE_IDLE);
            assert_eq!(event_count, 0);
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
            assert!(!job.requires_physical_evidence);
            assert!(!job.requires_test_evidence);
        });
    }

    #[test]
    fn test_set_job_evidence_requirements() {
        with_test_db(|db| {
            {
                let conn = db.conn();
                insert_job_sync(&conn, "job_1", "a1", None, "one").unwrap();
                set_job_evidence_requirements_sync(&conn, "job_1", true, true).unwrap();
            }

            let conn = db.conn();
            let job = query_job_sync(&conn, "job_1").unwrap().unwrap();
            assert!(job.requires_physical_evidence);
            assert!(job.requires_test_evidence);
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
            let reply =
                collect_reply_for_dispatched_job_sync(&conn, "a1", Some(before), "").unwrap();

            assert!(after > before);
            assert_eq!(reply, "new");
        });
    }

    #[test]
    fn test_collect_reply_uses_vt100_to_handle_cursor_reposition() {
        with_test_db(|db| {
            let before = {
                let conn = db.conn();
                insert_job_sync(&conn, "job_1", "a1", None, "echo from a1").unwrap();
                let before =
                    insert_event_sync(&conn, "a1", None, "command_received", r#"{"cmd":"echo"}"#)
                        .unwrap();
                insert_event_sync(
                    &conn,
                    "a1",
                    None,
                    "output_chunk",
                    "{\"text\":\"clean reply\\n\\u001b[10;1HWorking(0s)\\u001b[10;1H           \"}",
                )
                .unwrap();
                before
            };

            let reply = collect_reply_for_dispatched_job_sync(
                &db.conn(),
                "a1",
                Some(before),
                "echo from a1",
            )
            .unwrap();

            assert_eq!(reply, "clean reply");
        });
    }

    #[test]
    fn test_collect_reply_handles_status_bar_overwrite() {
        with_test_db(|db| {
            let before = {
                let conn = db.conn();
                insert_job_sync(&conn, "job_1", "a1", None, "summarize").unwrap();
                let before =
                    insert_event_sync(&conn, "a1", None, "command_received", r#"{"cmd":"ask"}"#)
                        .unwrap();
                insert_event_sync(
                    &conn,
                    "a1",
                    None,
                    "output_chunk",
                    "{\"text\":\"\\u001b[1;1H•Working(0s • esc to interrupt)\\u001b[1;1H\\u001b[2KFinal answer\"}",
                )
                .unwrap();
                before
            };

            let reply =
                collect_reply_for_dispatched_job_sync(&db.conn(), "a1", Some(before), "summarize")
                    .unwrap();

            assert_eq!(reply, "Final answer");
            assert!(!reply.contains("Working"));
        });
    }

    #[test]
    fn test_collect_reply_handles_long_output_beyond_50_lines() {
        with_test_db(|db| {
            let before = {
                let conn = db.conn();
                insert_job_sync(&conn, "job_1", "a1", None, "generate").unwrap();
                let before =
                    insert_event_sync(&conn, "a1", None, "command_received", r#"{"cmd":"gen"}"#)
                        .unwrap();
                let mut long_text = String::new();
                for i in 1..=100 {
                    long_text.push_str(&format!("Line {i}: content here\n"));
                }
                let payload = serde_json::json!({ "text": long_text }).to_string();
                insert_event_sync(&conn, "a1", None, "output_chunk", &payload).unwrap();
                before
            };

            let reply =
                collect_reply_for_dispatched_job_sync(&db.conn(), "a1", Some(before), "generate")
                    .unwrap();

            assert!(reply.contains("Line 1: content here"));
            assert!(reply.contains("Line 100: content here"));
        });
    }

    #[test]
    fn test_collect_reply_fallback_when_vt100_screen_empty() {
        with_test_db(|db| {
            let before = {
                let conn = db.conn();
                insert_job_sync(&conn, "job_1", "a1", None, "ask something").unwrap();
                let before =
                    insert_event_sync(&conn, "a1", None, "command_received", r#"{"cmd":"ask"}"#)
                        .unwrap();
                // Simulate output that only uses cursor positioning to clear itself,
                // leaving vt100 screen empty but raw text containing real content.
                // In practice this can happen with certain TUI frameworks that reset screen.
                let payload = serde_json::json!({
                    "text": "\x1b[2J\x1b[HReal answer from agent"
                })
                .to_string();
                insert_event_sync(&conn, "a1", None, "output_chunk", &payload).unwrap();
                before
            };

            let reply = collect_reply_for_dispatched_job_sync(
                &db.conn(),
                "a1",
                Some(before),
                "ask something",
            )
            .unwrap();

            assert!(reply.contains("Real answer from agent"));
        });
    }
}
