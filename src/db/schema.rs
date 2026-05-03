pub const SCHEMA_DDL: &str = r#"
CREATE TABLE IF NOT EXISTS projects (
    id TEXT PRIMARY KEY,
    absolute_path TEXT NOT NULL UNIQUE,
    created_at INTEGER NOT NULL DEFAULT (unixepoch())
) STRICT;

CREATE TABLE IF NOT EXISTS sessions (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    master_pid INTEGER NOT NULL,
    status TEXT NOT NULL DEFAULT 'ACTIVE',
    created_at INTEGER NOT NULL DEFAULT (unixepoch())
) STRICT;

CREATE TABLE IF NOT EXISTS agents (
    id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    provider TEXT NOT NULL,
    state TEXT NOT NULL,
    state_version INTEGER NOT NULL DEFAULT 1,
    pid INTEGER,
    exit_code INTEGER,
    error_code TEXT,
    created_at INTEGER NOT NULL DEFAULT (unixepoch()),
    sub_state TEXT,
    updated_at INTEGER NOT NULL DEFAULT (unixepoch())
) STRICT;

CREATE INDEX IF NOT EXISTS idx_agents_active ON agents(state) WHERE state NOT IN ('CRASHED');

CREATE TABLE IF NOT EXISTS events (
    seq_id INTEGER PRIMARY KEY AUTOINCREMENT,
    agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    request_id TEXT,
    event_type TEXT NOT NULL,
    payload TEXT NOT NULL,
    created_at INTEGER NOT NULL DEFAULT (unixepoch())
) STRICT;

CREATE INDEX IF NOT EXISTS idx_events_agent_seq ON events(agent_id, seq_id);
CREATE UNIQUE INDEX IF NOT EXISTS idx_events_idempotent ON events(agent_id, request_id) WHERE request_id IS NOT NULL;

CREATE TABLE IF NOT EXISTS evidence (
    id TEXT PRIMARY KEY,
    agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    event_seq_id INTEGER NOT NULL REFERENCES events(seq_id),
    pane_bytes BLOB NOT NULL,
    failed_rules TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'PENDING',
    l3_asserted_state TEXT,
    created_at INTEGER NOT NULL DEFAULT (unixepoch())
) STRICT;

CREATE INDEX IF NOT EXISTS idx_evidence_pending ON evidence(status, created_at) WHERE status = 'PENDING';
CREATE INDEX IF NOT EXISTS idx_evidence_agent ON evidence(agent_id);

CREATE TABLE IF NOT EXISTS jobs (
    id TEXT PRIMARY KEY,
    agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    request_id TEXT,
    prompt_text TEXT NOT NULL,
    reply_text TEXT,
    status TEXT NOT NULL DEFAULT 'QUEUED',
    error_reason TEXT,
    created_at INTEGER NOT NULL DEFAULT (unixepoch()),
    dispatched_at INTEGER,
    dispatched_at_seq_id INTEGER,
    completed_at INTEGER,
    cancel_requested INTEGER NOT NULL DEFAULT 0
) STRICT;

CREATE INDEX IF NOT EXISTS idx_jobs_queue ON jobs(agent_id, status, created_at) WHERE status IN ('QUEUED', 'DISPATCHED');
CREATE UNIQUE INDEX IF NOT EXISTS idx_jobs_idempotent ON jobs(agent_id, request_id) WHERE request_id IS NOT NULL;
"#;

#[derive(Debug, Clone)]
pub struct Project {
    pub id: String,
    pub absolute_path: String,
    pub created_at: i64,
}

#[derive(Debug, Clone)]
pub struct Session {
    pub id: String,
    pub project_id: String,
    pub status: String,
    pub created_at: i64,
}

#[derive(Debug, Clone)]
pub struct Agent {
    pub id: String,
    pub session_id: String,
    pub provider: String,
    pub state: String,
    pub state_version: i64,
    pub pid: Option<i64>,
    pub exit_code: Option<i64>,
    pub error_code: Option<String>,
    pub sub_state: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone)]
pub struct Event {
    pub seq_id: i64,
    pub agent_id: String,
    pub request_id: Option<String>,
    pub event_type: String,
    pub payload: String,
    pub created_at: i64,
}

#[derive(Debug, Clone)]
pub struct Evidence {
    pub id: String,
    pub agent_id: String,
    pub event_seq_id: i64,
    pub status: String,
    pub created_at: i64,
}

#[derive(Debug, Clone)]
pub struct Job {
    pub id: String,
    pub agent_id: String,
    pub request_id: Option<String>,
    pub prompt_text: String,
    pub reply_text: Option<String>,
    pub status: String,
    pub error_reason: Option<String>,
    pub created_at: i64,
    pub dispatched_at: Option<i64>,
    pub dispatched_at_seq_id: Option<i64>,
    pub completed_at: Option<i64>,
    pub cancel_requested: bool,
}
