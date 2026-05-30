//! PaneDiffWatcher pure logic: sanitize PTY text + detect meaningful change.

use regex::Regex;
use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

const MIN_LENGTH_GROWTH: usize = 8;
pub const DEFAULT_WATCH_INTERVAL: Duration = Duration::from_secs(30);
pub const DEFAULT_STUCK_THRESHOLD: Duration = Duration::from_secs(300);

#[derive(Debug, Clone)]
pub struct AgentDiffState {
    pub last_meaningful_text: String,
    pub last_meaningful_at: Instant,
}

impl AgentDiffState {
    pub fn new(initial_text: String) -> Self {
        Self {
            last_meaningful_text: initial_text,
            last_meaningful_at: Instant::now(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct PaneDiffObservation {
    pub agent_id: String,
    pub text: String,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct PaneDiffTickResult {
    pub stuck_agent_ids: Vec<String>,
}

pub fn process_pane_diff_observations(
    state_map: &mut HashMap<String, AgentDiffState>,
    observations: Vec<PaneDiffObservation>,
    now: Instant,
    stuck_threshold: Duration,
) -> PaneDiffTickResult {
    let mut active_agent_ids = HashSet::new();
    let mut stuck_agent_ids = Vec::new();

    for observation in observations {
        active_agent_ids.insert(observation.agent_id.clone());
        let clean_text = sanitize_for_diff(&observation.text);
        let entry = state_map
            .entry(observation.agent_id.clone())
            .or_insert_with(|| AgentDiffState {
                last_meaningful_text: clean_text.clone(),
                last_meaningful_at: now,
            });

        if is_meaningful_diff(&entry.last_meaningful_text, &observation.text) {
            entry.last_meaningful_text = clean_text;
            entry.last_meaningful_at = now;
        } else if now.duration_since(entry.last_meaningful_at) >= stuck_threshold {
            stuck_agent_ids.push(observation.agent_id);
        }
    }

    for agent_id in &stuck_agent_ids {
        state_map.remove(agent_id);
    }
    state_map.retain(|agent_id, _| active_agent_ids.contains(agent_id));

    PaneDiffTickResult { stuck_agent_ids }
}

pub async fn pane_diff_watcher_loop(
    ctx: crate::rpc::Ctx,
    interval: Duration,
    stuck_threshold: Duration,
) {
    let mut state_map = HashMap::new();
    loop {
        tokio::time::sleep(interval).await;
        if let Err(err) = pane_diff_watcher_tick(&ctx, &mut state_map, stuck_threshold).await {
            tracing::warn!(error = %err, "pane diff watcher tick failed");
        }
    }
}

async fn pane_diff_watcher_tick(
    ctx: &crate::rpc::Ctx,
    state_map: &mut HashMap<String, AgentDiffState>,
    stuck_threshold: Duration,
) -> Result<(), crate::error::CcbdError> {
    let busy_agents =
        crate::db::agents::query_agents_by_state(ctx.db.clone(), "BUSY".to_string()).await?;
    let mut observations = Vec::new();

    for agent in busy_agents {
        let Some(pane_id) = crate::agent_io::pane_id(&agent.id) else {
            continue;
        };
        match ctx.tmux_server.capture_pane(pane_id).await {
            Ok(text) => observations.push(PaneDiffObservation {
                agent_id: agent.id,
                text,
            }),
            Err(err) => {
                tracing::warn!(agent_id = %agent.id, error = %err, "pane diff capture_pane failed");
            }
        }
    }

    let result =
        process_pane_diff_observations(state_map, observations, Instant::now(), stuck_threshold);
    for agent_id in result.stuck_agent_ids {
        match crate::db::state_machine::mark_agent_stuck(ctx.db.clone(), agent_id.clone()).await {
            Ok(changes) if changes > 0 => {
                tracing::warn!(agent_id = %agent_id, "pane diff watcher marked agent STUCK");
            }
            Ok(_) => {}
            Err(err) => {
                tracing::warn!(agent_id = %agent_id, error = %err, "mark_agent_stuck failed");
            }
        }
    }

    Ok(())
}

/// 去掉 spinner Unicode、时间戳、ANSI escape 等“假活”装饰，留下实质文本。
pub fn sanitize_for_diff(raw: &str) -> String {
    let without_ansi = crate::db::jobs::strip_ansi_escapes(raw);
    let mut lines = Vec::new();

    for line in without_ansi.lines() {
        let line = braille_spinner_re().replace_all(line, "");
        let line = ascii_spinner_prefix_re().replace(&line, "");
        let line = line.trim();
        if line.is_empty() || thinking_line_re().is_match(line) {
            continue;
        }

        let line = trailing_time_re().replace(line, "");
        let line = line.trim();
        if !line.is_empty() {
            lines.push(line.to_string());
        }
    }

    lines.join("\n")
}

/// 判断 new_text 相比 old_text 是否含“实质增长”，而不是单纯 spinner 闪烁。
pub fn is_meaningful_diff(old_text: &str, new_text: &str) -> bool {
    let clean_old = sanitize_for_diff(old_text);
    let clean_new = sanitize_for_diff(new_text);

    let old_len = clean_old.chars().count();
    let new_len = clean_new.chars().count();
    if new_len >= old_len + MIN_LENGTH_GROWTH {
        return true;
    }

    if !clean_new.starts_with(&clean_old) && !clean_old.starts_with(&clean_new) {
        return clean_new != clean_old;
    }

    false
}

fn braille_spinner_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"[\u{2800}-\u{28FF}]+").expect("valid braille regex"))
}

fn ascii_spinner_prefix_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^\s*[-\\|/]\s+").expect("valid ascii spinner regex"))
}

fn thinking_line_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?i)^thinking\.\.\.\s*(?:\([^)]*\))?$").expect("valid thinking regex")
    })
}

fn trailing_time_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?:\s+\(\d+(?:\.\d+)?[smh](?:\s+\d+(?:\.\d+)?[smh])?\)|\s+\d+:\d+:\d+|\s+\d+m\s+\d+s)$",
        )
        .expect("valid trailing time regex")
    })
}

#[cfg(test)]
mod tests {
    use super::{
        AgentDiffState, PaneDiffObservation, compute_content_hash, detect_thinking_spinner,
        is_meaningful_diff, process_pane_diff_observations, query_log_mtime, sanitize_for_diff,
    };
    use std::collections::HashMap;
    use std::fs;
    use std::time::{Duration, Instant};

    #[test]
    fn test_sanitize_strips_ansi_escapes() {
        assert_eq!(sanitize_for_diff("\u{1b}[31mhello\u{1b}[0m"), "hello");
    }

    #[test]
    fn test_sanitize_strips_braille_spinner_chars() {
        assert_eq!(sanitize_for_diff("⠋ Thinking..."), "");
    }

    #[test]
    fn test_sanitize_strips_thinking_with_timestamp() {
        assert_eq!(sanitize_for_diff("Thinking... (3m 24s)\nactual"), "actual");
    }

    #[test]
    fn test_sanitize_strips_trailing_time_seconds() {
        assert_eq!(sanitize_for_diff("Some output (12.3s)"), "Some output");
    }

    #[test]
    fn test_sanitize_collapses_blank_lines() {
        assert_eq!(sanitize_for_diff("one\n\n\n  two  \n\n"), "one\ntwo");
    }

    #[test]
    fn test_is_meaningful_diff_detects_length_growth() {
        assert!(is_meaningful_diff("abc", "abc + new tool call: read_file"));
    }

    #[test]
    fn test_is_meaningful_diff_ignores_spinner_only_change() {
        assert!(!is_meaningful_diff(
            "Thinking... (1m)",
            "Thinking... (1m 5s)"
        ));
    }

    #[test]
    fn test_is_meaningful_diff_detects_total_text_replacement() {
        assert!(is_meaningful_diff("hello world", "goodbye now"));
    }

    #[test]
    fn test_agent_diff_state_initial() {
        let before = Instant::now();
        let state = AgentDiffState::new("initial".to_string());
        let after = Instant::now();

        assert_eq!(state.last_meaningful_text, "initial");
        assert!(state.last_meaningful_at >= before);
        assert!(state.last_meaningful_at <= after + Duration::from_millis(1));
    }

    #[test]
    fn test_pane_diff_watcher_marks_stuck_after_threshold() {
        let mut state_map = HashMap::new();
        let start = Instant::now();
        let observations = vec![PaneDiffObservation {
            agent_id: "a1".to_string(),
            text: "same output".to_string(),
        }];

        let first = process_pane_diff_observations(
            &mut state_map,
            observations.clone(),
            start,
            Duration::from_secs(300),
        );
        let second = process_pane_diff_observations(
            &mut state_map,
            observations,
            start + Duration::from_secs(301),
            Duration::from_secs(300),
        );

        assert!(first.stuck_agent_ids.is_empty());
        assert_eq!(second.stuck_agent_ids, ["a1"]);
        assert!(!state_map.contains_key("a1"));
    }

    #[test]
    fn test_pane_diff_watcher_resets_timer_on_meaningful_change() {
        let mut state_map = HashMap::new();
        let start = Instant::now();

        process_pane_diff_observations(
            &mut state_map,
            vec![PaneDiffObservation {
                agent_id: "a1".to_string(),
                text: "short".to_string(),
            }],
            start,
            Duration::from_secs(300),
        );
        let result = process_pane_diff_observations(
            &mut state_map,
            vec![PaneDiffObservation {
                agent_id: "a1".to_string(),
                text: "short plus meaningful new content".to_string(),
            }],
            start + Duration::from_secs(301),
            Duration::from_secs(300),
        );

        let state = state_map.get("a1").unwrap();
        assert!(result.stuck_agent_ids.is_empty());
        assert_eq!(
            state.last_meaningful_text,
            "short plus meaningful new content"
        );
        assert_eq!(state.last_meaningful_at, start + Duration::from_secs(301));
    }

    #[test]
    fn test_pane_diff_watcher_cleans_up_state_when_agent_no_longer_busy() {
        let mut state_map = HashMap::new();
        let start = Instant::now();

        process_pane_diff_observations(
            &mut state_map,
            vec![PaneDiffObservation {
                agent_id: "a1".to_string(),
                text: "initial".to_string(),
            }],
            start,
            Duration::from_secs(300),
        );
        process_pane_diff_observations(
            &mut state_map,
            Vec::new(),
            start + Duration::from_secs(30),
            Duration::from_secs(300),
        );

        assert!(state_map.is_empty());
    }

    #[test]
    fn test_compute_content_hash_changes_reset_timer() {
        let same_a = compute_content_hash("Thinking...\nunchanged body");
        let same_b = compute_content_hash("Thinking...\nunchanged body");
        let changed = compute_content_hash("Thinking...\nnew body");

        assert_eq!(same_a, same_b);
        assert_ne!(
            same_a, changed,
            "content hash change should reset stuck timer"
        );
    }

    #[test]
    fn test_query_log_mtime_changes_reset_timer() {
        let dir = tempfile::TempDir::new().unwrap();
        let log_path = dir.path().join("agent.log");
        fs::write(&log_path, "first\n").unwrap();
        let first = query_log_mtime(&log_path).expect("first mtime");

        std::thread::sleep(Duration::from_millis(5));
        fs::write(&log_path, "second\n").unwrap();
        let second = query_log_mtime(&log_path).expect("second mtime");

        assert!(second > first, "mtime change should reset stuck timer");
    }

    #[test]
    fn test_detect_thinking_spinner_match() {
        assert!(detect_thinking_spinner("Thinking..."));
        assert!(detect_thinking_spinner("Spinner still running"));
        assert!(detect_thinking_spinner("Working on your request"));
        assert!(!detect_thinking_spinner("final answer text"));
    }

    #[test]
    fn test_three_signals_static_marks_stuck() {
        let mut state_map = HashMap::new();
        let start = Instant::now();
        let observations = vec![PaneDiffObservation {
            agent_id: "a1".to_string(),
            text: "Thinking...".to_string(),
        }];

        let first = process_pane_diff_observations(
            &mut state_map,
            observations.clone(),
            start,
            Duration::from_secs(5),
        );
        let second = process_pane_diff_observations(
            &mut state_map,
            observations,
            start + Duration::from_secs(6),
            Duration::from_secs(5),
        );

        assert!(first.stuck_agent_ids.is_empty());
        assert_eq!(second.stuck_agent_ids, ["a1"]);
    }
}
