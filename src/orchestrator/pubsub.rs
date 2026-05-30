use std::sync::LazyLock;
use tokio::sync::broadcast;

#[derive(Debug, Clone, serde::Serialize)]
pub struct EventFrame {
    pub event_id: i64,
    pub kind: String,
    pub agent_id: String,
    pub job_id: Option<String>,
    pub state: Option<String>,
    pub ts_unix_micro: i64,
    pub payload: Option<serde_json::Value>,
}

pub static JOB_UPDATES: LazyLock<broadcast::Sender<String>> = LazyLock::new(|| {
    let (tx, _) = broadcast::channel(1024);
    tx
});

pub static AGENT_OUTPUT: LazyLock<broadcast::Sender<String>> = LazyLock::new(|| {
    let (tx, _) = broadcast::channel(1024);
    tx
});

pub static EVENT_FRAMES: LazyLock<broadcast::Sender<EventFrame>> = LazyLock::new(|| {
    let (tx, _) = broadcast::channel(1024);
    tx
});

pub fn notify_job_update(job_id: &str) {
    let _ = JOB_UPDATES.send(job_id.to_string());
}

pub fn subscribe_job_updates() -> broadcast::Receiver<String> {
    JOB_UPDATES.subscribe()
}

pub fn notify_agent_output(agent_id: &str) {
    let _ = AGENT_OUTPUT.send(agent_id.to_string());
}

pub fn subscribe_agent_output() -> broadcast::Receiver<String> {
    AGENT_OUTPUT.subscribe()
}

pub fn notify_event(frame: EventFrame) {
    let _ = EVENT_FRAMES.send(frame);
}

pub fn subscribe_events() -> broadcast::Receiver<EventFrame> {
    EVENT_FRAMES.subscribe()
}
