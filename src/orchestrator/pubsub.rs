use std::sync::LazyLock;
use tokio::sync::broadcast;

pub static JOB_UPDATES: LazyLock<broadcast::Sender<String>> = LazyLock::new(|| {
    let (tx, _) = broadcast::channel(1024);
    tx
});

pub static AGENT_OUTPUT: LazyLock<broadcast::Sender<String>> = LazyLock::new(|| {
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
