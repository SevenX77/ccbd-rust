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
    pub now_ts: i64,
}

pub fn health_check_observe(
    _observation: &HealthCheckObservation,
    _stuck_threshold_secs: i64,
) -> HealthCheckResult {
    HealthCheckResult {
        alive: false,
        dead_layers: Vec::new(),
        last_progress_ts: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::{HealthCheckObservation, health_check_observe};

    fn observation() -> HealthCheckObservation {
        HealthCheckObservation {
            agent_id: "dogfood_a1".into(),
            provider: "bash".into(),
            state: "BUSY".into(),
            pane_capture: "$ ".into(),
            pane_capture_ok: true,
            last_output_ts: Some(100),
            last_marker_ts: Some(100),
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
}
