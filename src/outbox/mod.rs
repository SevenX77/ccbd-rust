//! R1 outbox transport — journal-first hook delivery (Completion Protocol Part R1).
//!
//! This module is the transactional-outbox floor the completion protocol rides on
//! (design `research/ab-experiment-r1-outbox/frozen-spec/design.md`, Part R1 + JC-1).
//! It owns four things, and nothing about `agents.state` or `jobs.status` semantics:
//!
//! * **R1-T1 (journal-first write)** — [`journal_record`]: an `ah agent notify` writes a
//!   durable outbox record (`.tmp` + `fsync` + `rename()` + dir `fsync`) *before* any RPC.
//!   Invariant: **exit 0 ⇔ a durable outbox record exists**. A journal failure is loud +
//!   non-zero; an RPC failure is exit-0-safe (the durable file is the guarantee).
//! * **JC-1 (transport dedup ledger)** — [`consume_record`]: the consume boundary does
//!   `INSERT INTO outbox_consumed(event_id) … ON CONFLICT DO NOTHING` **before** routing by
//!   `kind`, dedup + handler-effect in one transaction, so at-least-once redelivery is safe.
//! * **R1-T2/T3 (cold-scan replay + reap + error-book)** — [`cold_scan_dir`] /
//!   [`cold_scan_all_agents`]: on ahd startup, before serving RPC, replay every agent's
//!   outbox through the same idempotent consume path; reap after commit; quarantine
//!   un-applyable records to `outbox/dead/` instead of dropping or hot-looping them.
//! * **R1-T4 (selfcheck reconciliation)** — a reserved `selfcheck:` `event_id` is exempt
//!   from the replay ordering assumption but *still takes a ledger row*, so a
//!   crash-surviving selfcheck file re-scans as a harmless no-op rather than re-running.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

/// Reserved `event_id` prefix for the G4 control-path self-check probe (design R1-Q3/Q4,
/// R1-T4). A record whose `event_id` starts with this prefix is a liveness probe: it is
/// exempt from the `event_id`-ordered replay assumption (its id does not time-sort) and it
/// routes to a no-op sink — but it still takes a JC-1 ledger row so a crash-surviving
/// selfcheck file re-scans as a harmless no-op rather than re-running the probe.
pub const SELFCHECK_PREFIX: &str = "selfcheck:";

/// The kind of an outbox record. The dedup ledger is applied *before* this fork
/// (design R1-Q2: "for every record regardless of `kind`").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutboxKind {
    /// F2 events path: a hook lifecycle event (`stop`/idle-marker). Lands on `events`.
    HookEvent,
    /// F3 job_transitions path: an explicit `ah job done` declaration. R2-owned consumer
    /// is not built yet; routed to a labeled stub sink here (see JC-1 scope).
    JobDone,
    /// F3 job_transitions path: an explicit `ah job fail` declaration. As `JobDone`.
    JobFail,
    /// G4 control-path self-check liveness probe. No-op sink; ledger row only (R1-T4).
    Selfcheck,
}

impl OutboxKind {
    /// Stable wire string used in the ledger `kind` column and diagnostics.
    pub fn as_str(self) -> &'static str {
        match self {
            OutboxKind::HookEvent => "hook_event",
            OutboxKind::JobDone => "job_done",
            OutboxKind::JobFail => "job_fail",
            OutboxKind::Selfcheck => "selfcheck",
        }
    }
}

/// One durable outbox record. A **union** wire form (design CP-PM.1): it carries both the
/// F2 hook-event fields and the F3 declaration fields, mostly-null where a field does not
/// apply to a given `kind`. Absent optional fields are omitted from the JSON on the wire.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutboxRecord {
    /// Producer-minted idempotency key. UUIDv7/ULID (time-prefixed, scan-orderable) for
    /// hook events; the reserved `selfcheck:{agent_id}:{boot_id}` id for probes.
    pub event_id: String,
    /// Discriminant routed *after* the dedup ledger insert.
    pub kind: OutboxKind,
    /// The agent this record belongs to.
    pub agent_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// Hook event name for `HookEvent` records (`stop`, `idle`, …).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event: Option<String>,
    /// The arbiter-Q4 per-dispatch-attempt cookie (`{job_id}:{dispatch_seq}`). R1 reads it,
    /// never mints it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attempt_cookie: Option<String>,
    /// Target job for `JobDone`/`JobFail`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job_id: Option<String>,
    /// The declared result carried inside the declaration (F3), never scraped.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply_text: Option<String>,
    /// Honest-failure reason for `JobFail`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// When the hook fired / declaration was made (unix seconds), producer clock.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hook_fired_at: Option<i64>,
    /// Opaque passthrough payload.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<serde_json::Value>,
}

impl OutboxRecord {
    /// A minimal `HookEvent` record with a freshly-minted time-ordered `event_id`.
    pub fn hook_event(agent_id: impl Into<String>, event: impl Into<String>) -> Self {
        OutboxRecord {
            event_id: new_event_id(),
            kind: OutboxKind::HookEvent,
            agent_id: agent_id.into(),
            provider: None,
            event: Some(event.into()),
            attempt_cookie: None,
            job_id: None,
            reply_text: None,
            reason: None,
            hook_fired_at: None,
            payload: None,
        }
    }

    /// True if this record's `event_id` is the reserved selfcheck probe id (R1-T4).
    pub fn is_reserved_selfcheck(&self) -> bool {
        is_reserved_selfcheck_id(&self.event_id)
    }
}

/// True if an `event_id` is the reserved G4 selfcheck probe id (R1-T4).
pub fn is_reserved_selfcheck_id(event_id: &str) -> bool {
    event_id.starts_with(SELFCHECK_PREFIX)
}

/// Mint a fresh, time-ordered (UUIDv7) `event_id`. Time-prefixed so a filesystem/`event_id`
/// sort approximates fire order for replay (design R1-Q3).
pub fn new_event_id() -> String {
    uuid::Uuid::now_v7().to_string()
}

/// Host-visible outbox root under a state dir: `{state_dir}/outbox/`.
pub fn outbox_root(state_dir: &Path) -> PathBuf {
    state_dir.join("outbox")
}

/// Per-agent outbox directory: `{state_dir}/outbox/{sanitized_agent_id}/`. The sanitization
/// is deterministic and identical on the writer (sandbox `ah agent notify`) and scanner
/// (host `ahd`) sides so the writer glob and scanner glob agree (F-7).
pub fn outbox_dir_for_agent(state_dir: &Path, agent_id: &str) -> PathBuf {
    outbox_root(state_dir).join(sanitize_path_component(agent_id))
}

/// Replace filesystem-unsafe characters with `_` so an `agent_id`/`event_id` is a safe
/// single path component. Deterministic (writer == scanner).
fn sanitize_path_component(raw: &str) -> String {
    raw.chars()
        .map(|ch| {
            if ch == '/' || ch == '\\' || ch == '\0' || ch.is_control() {
                '_'
            } else {
                ch
            }
        })
        .collect()
}

/// The `.json` filename a record is durably stored under. The scanner globs `*.json` and
/// must never observe a `.tmp` — one naming convention, writer glob == scanner glob (F-7).
fn record_json_filename(event_id: &str) -> String {
    format!("{}.json", sanitize_path_component(event_id))
}

/// Journal-first durable write of an outbox record (R1-T1 / CP-R1.1).
///
/// Writes `{outbox_dir}/{event_id}.tmp`, `fsync`s it, `rename()`s to `{event_id}.json`, then
/// `fsync`s the directory. The `rename()` is the durability commit point (POSIX guarantees
/// the scanner never observes a partial file under the `.json` name). Every step is checked;
/// on any failure the partial `.tmp` is best-effort removed and the error is returned so the
/// caller can exit **loud + non-zero** — never exit 0 on a failed journal.
///
/// Returns the path of the durable `.json` on success.
pub fn journal_record(outbox_dir: &Path, record: &OutboxRecord) -> io::Result<PathBuf> {
    use std::io::Write;

    // The outbox dir must exist and be durable before we place a record in it. A failure
    // here (unwritable/read-only/parent-is-a-file) means nothing durable can land → Err.
    fs::create_dir_all(outbox_dir)?;

    let filename = record_json_filename(&record.event_id);
    let final_path = outbox_dir.join(&filename);
    let tmp_path = outbox_dir.join(format!("{}.tmp", sanitize_path_component(&record.event_id)));

    // Serialize before touching the filesystem so a bad record never leaves a stray tmp.
    let bytes = serde_json::to_vec(record)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;

    // Write + fsync the tmp, rename to the durable name, fsync the dir. Every step checked;
    // on any error best-effort remove the partial tmp and propagate — never a silent Ok.
    let commit = (|| -> io::Result<()> {
        let mut file = fs::File::create(&tmp_path)?;
        file.write_all(&bytes)?;
        file.sync_all()?; // fsync the file: bytes are on disk before the rename
        drop(file);
        fs::rename(&tmp_path, &final_path)?; // atomic durability commit point
        fsync_dir(outbox_dir)?; // fsync the dir: the rename itself is durable
        Ok(())
    })();

    if let Err(err) = commit {
        let _ = fs::remove_file(&tmp_path); // best-effort cleanup of the partial tmp
        return Err(err);
    }
    Ok(final_path)
}

/// `fsync` a directory so a rename into it survives a crash (design R1-Q1). On platforms
/// where opening a directory for fsync is unsupported, a `NotFound`/`PermissionDenied` on
/// the *dir handle* is not swallowed — only the fsync-not-supported case degrades quietly.
fn fsync_dir(dir: &Path) -> io::Result<()> {
    let dir_file = fs::File::open(dir)?;
    match dir_file.sync_all() {
        Ok(()) => Ok(()),
        // Some filesystems reject fsync on a directory fd; the rename is still durable there.
        Err(err) if err.raw_os_error() == Some(libc::EINVAL) => Ok(()),
        Err(err) => Err(err),
    }
}

// ===========================================================================
// JC-1 — transport dedup ledger + consume funnel (design R1-Q2 / CP-R1.2)
// ===========================================================================

/// Outcome of feeding one record through the consume boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsumeOutcome {
    /// The record's `event_id` was not in the ledger; the per-kind effect was applied and
    /// committed in the same transaction. The file may now be reaped.
    FirstSeen,
    /// The record's `event_id` was already in the ledger; no effect was applied
    /// (at-least-once redelivery deduped). The file may be reaped.
    Duplicate,
}

/// Why a record could not be consumed. Distinguishes a transient/effect failure (retry then
/// quarantine) from a malformed record (quarantine immediately) so cold-scan can route it.
#[derive(Debug)]
pub enum ConsumeError {
    /// The database rejected the dedup insert or the handler effect (e.g. a hook_event whose
    /// agent no longer exists → FK violation). The tx is rolled back, so no ledger row is
    /// left behind and the record can be retried on a later cold-scan before quarantine.
    Db(rusqlite::Error),
    /// The record deserialized but is semantically incomplete (e.g. a `job_done` with no
    /// `job_id`) — it can never apply, so cold-scan quarantines it.
    Malformed(String),
}

impl std::fmt::Display for ConsumeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConsumeError::Db(err) => write!(f, "db error consuming outbox record: {err}"),
            ConsumeError::Malformed(msg) => write!(f, "malformed outbox record: {msg}"),
        }
    }
}

impl std::error::Error for ConsumeError {}

impl From<rusqlite::Error> for ConsumeError {
    fn from(err: rusqlite::Error) -> Self {
        ConsumeError::Db(err)
    }
}

/// The JC-1 consume boundary: dedup on `event_id` **before** the `kind` fork, then apply the
/// per-kind effect in the **same transaction** (design R1-Q2). For *every* record regardless
/// of kind, the first step is `INSERT INTO outbox_consumed(event_id) … ON CONFLICT DO NOTHING`;
/// zero rows affected ⇒ already applied ⇒ drop without dispatching. Only a first-seen
/// `event_id` proceeds to routing, and the ledger row + handler effect commit atomically, so
/// a crash between apply and reap can neither double-apply nor lose the record.
pub fn consume_record(
    conn: &mut Connection,
    record: &OutboxRecord,
) -> Result<ConsumeOutcome, ConsumeError> {
    // A job declaration that names no job can never apply — reject before opening a tx so it
    // is quarantined by cold-scan rather than retried forever.
    if matches!(record.kind, OutboxKind::JobDone | OutboxKind::JobFail) && record.job_id.is_none() {
        return Err(ConsumeError::Malformed(format!(
            "{} record {} has no job_id",
            record.kind.as_str(),
            record.event_id
        )));
    }

    let tx = conn.transaction()?;

    // JC-1: the FIRST step for every record, regardless of kind, is the dedup insert.
    let inserted = tx.execute(
        "INSERT INTO outbox_consumed(event_id, kind) VALUES (?1, ?2) ON CONFLICT(event_id) DO NOTHING",
        params![record.event_id, record.kind.as_str()],
    )?;
    if inserted == 0 {
        // Already consumed under at-least-once redelivery — drop without dispatching.
        return Ok(ConsumeOutcome::Duplicate);
    }

    // First-seen: apply the per-kind effect in THIS SAME transaction (atomic with the ledger).
    match record.kind {
        OutboxKind::HookEvent => apply_hook_event(&tx, record)?,
        OutboxKind::JobDone | OutboxKind::JobFail => apply_job_declaration_stub(&tx, record)?,
        // R1-T4: a selfcheck probe routes to a no-op sink — it takes the ledger row above
        // (so a crash-surviving selfcheck re-scans as a harmless no-op) but has no effect.
        OutboxKind::Selfcheck => {}
    }

    tx.commit()?;
    Ok(ConsumeOutcome::FirstSeen)
}

/// F2 events-path effect: record the delivered hook event durably on the `events` spine.
///
/// Scope note: R1 is the transport floor. This lands the durable, replay-safe event record;
/// it deliberately does **not** run the R2-owned idle-marker / completion logic
/// (`mark_agent_idle_hook_event_sync` and its `mark_job_completed_conn_sync` entanglement).
/// R2-T2 is the load-bearing refactor that reroutes the state effect through this same
/// deduped boundary. The FK to `agents` means a record for a vanished agent fails here and is
/// error-booked by cold-scan (an "un-applyable record", design R1-Q3).
fn apply_hook_event(tx: &rusqlite::Transaction<'_>, record: &OutboxRecord) -> Result<(), ConsumeError> {
    let payload = serde_json::json!({
        "source": "outbox",
        "hook_event": record.event,
        "provider": record.provider,
        "event_id": record.event_id,
        "attempt_cookie": record.attempt_cookie,
        "schema_version": 1,
    })
    .to_string();
    tx.execute(
        "INSERT INTO events (agent_id, request_id, event_type, payload) VALUES (?1, NULL, 'hook_event', ?2)",
        params![record.agent_id, payload],
    )?;
    Ok(())
}

/// F3 job_transitions-path effect — **STUB** (JC-1 scope: the F3 consumer is not built yet).
///
/// R2-T2 owns the real `apply_job_done_declaration_sync` that writes `job_transitions`
/// (`DISPATCHED→COMPLETED/FAILED`, `reason=explicit_done|explicit_fail`). This stub records
/// the declaration to a labeled sink so the JC-1 dedup gate has an observable F3-side effect
/// under test; it is repointed at `job_transitions` and this sink dropped when R2 lands.
fn apply_job_declaration_stub(
    tx: &rusqlite::Transaction<'_>,
    record: &OutboxRecord,
) -> Result<(), ConsumeError> {
    let job_id = record
        .job_id
        .as_deref()
        .ok_or_else(|| ConsumeError::Malformed("job declaration missing job_id".to_string()))?;
    tx.execute(
        "INSERT INTO outbox_job_declaration_stub(event_id, job_id, kind, attempt_cookie, reply_text, reason) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            record.event_id,
            job_id,
            record.kind.as_str(),
            record.attempt_cookie,
            record.reply_text,
            record.reason,
        ],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests;
