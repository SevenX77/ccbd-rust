use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::completion::parser::{LogParseResult, parse_provider_log_line};

pub type LogCursorMap = BTreeMap<PathBuf, u64>;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LogReadState {
    pub cursors: LogCursorMap,
    pub claude_user_entry_seen_paths: BTreeSet<PathBuf>,
}

impl LogReadState {
    pub fn from_cursors(cursors: LogCursorMap) -> Self {
        Self {
            cursors,
            claude_user_entry_seen_paths: BTreeSet::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogCompletion {
    pub parsed: LogParseResult,
    pub raw_path: PathBuf,
    pub raw_offset: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogTailReadResult {
    pub completions: Vec<LogCompletion>,
    pub cursors: LogCursorMap,
    pub state: LogReadState,
}

pub fn collect_provider_log_cursors(provider: &str, log_root: &Path) -> io::Result<LogCursorMap> {
    let mut files = Vec::new();
    collect_provider_log_files(provider, log_root, &mut files)?;
    files.sort();

    let mut cursors = LogCursorMap::new();
    for path in files {
        let len = fs::metadata(&path)?.len();
        cursors.insert(path, len);
    }
    Ok(cursors)
}

pub fn read_provider_log_tail(
    provider: &str,
    log_root: &Path,
    cursors: &LogCursorMap,
) -> io::Result<LogTailReadResult> {
    read_provider_log_tail_with_state(
        provider,
        log_root,
        &LogReadState::from_cursors(cursors.clone()),
    )
}

pub fn read_provider_log_tail_with_state(
    provider: &str,
    log_root: &Path,
    state: &LogReadState,
) -> io::Result<LogTailReadResult> {
    let mut completions = Vec::new();
    let mut updated_state = state.clone();
    let mut files = Vec::new();
    collect_provider_log_files(provider, log_root, &mut files)?;
    files.sort();

    for path in files {
        let bytes = fs::read(&path)?;
        let consumed_offset = state.cursors.get(&path).copied().unwrap_or(0);
        let start = parse_start_offset(&bytes, consumed_offset);
        if provider == "claude" && consumed_offset > bytes.len() as u64 {
            updated_state.claude_user_entry_seen_paths.remove(&path);
        }
        let mut claude_seen_user_entry =
            provider == "claude" && updated_state.claude_user_entry_seen_paths.contains(&path);

        let mut line_start = start;
        while line_start < bytes.len() {
            let relative_end = bytes[line_start..]
                .iter()
                .position(|byte| *byte == b'\n')
                .unwrap_or(bytes.len() - line_start);
            let line_end = line_start + relative_end;
            let next_line_start = if line_end < bytes.len() && bytes[line_end] == b'\n' {
                line_end + 1
            } else {
                line_end
            };
            let line = &bytes[line_start..line_end];
            if line.is_empty() {
                line_start = next_line_start;
                continue;
            }
            let line = String::from_utf8_lossy(line);
            let parsed = parse_provider_log_line(provider, line.as_ref());
            match parsed {
                LogParseResult::UserMessage { .. } if provider == "claude" => {
                    claude_seen_user_entry = true;
                    updated_state
                        .claude_user_entry_seen_paths
                        .insert(path.clone());
                }
                LogParseResult::TurnComplete { .. }
                    if provider != "claude" || claude_seen_user_entry =>
                {
                    completions.push(LogCompletion {
                        parsed,
                        raw_path: path.clone(),
                        raw_offset: next_line_start as u64,
                    });
                    if provider == "claude" {
                        claude_seen_user_entry = false;
                        updated_state.claude_user_entry_seen_paths.remove(&path);
                    }
                }
                _ => {}
            }
            if next_line_start == line_start {
                break;
            }
            line_start = next_line_start;
        }

        updated_state.cursors.insert(path, bytes.len() as u64);
    }

    Ok(LogTailReadResult {
        completions,
        cursors: updated_state.cursors.clone(),
        state: updated_state,
    })
}

fn parse_start_offset(bytes: &[u8], consumed_offset: u64) -> usize {
    let mut start = consumed_offset.min(bytes.len() as u64) as usize;
    if start > 0 && start < bytes.len() && bytes[start - 1] != b'\n' {
        start = bytes[start..]
            .iter()
            .position(|byte| *byte == b'\n')
            .map(|position| start + position + 1)
            .unwrap_or(bytes.len());
    }
    start
}

fn collect_provider_log_files(
    provider: &str,
    root: &Path,
    files: &mut Vec<PathBuf>,
) -> io::Result<()> {
    if !root.exists() {
        return Ok(());
    }

    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            collect_provider_log_files(provider, &path, files)?;
        } else if file_type.is_file() && provider_log_file_matches(provider, &path) {
            files.push(path);
        }
    }

    Ok(())
}

fn provider_log_file_matches(provider: &str, path: &Path) -> bool {
    match provider {
        "codex" => path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("rollout-") && name.ends_with(".jsonl")),
        "claude" => path.extension().and_then(|extension| extension.to_str()) == Some("jsonl"),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::fs;

    use crate::completion::parser::LogParseResult;

    use super::{
        LogCompletion, LogCursorMap, LogReadState, read_provider_log_tail,
        read_provider_log_tail_with_state,
    };

    fn codex_complete(turn_id: &str, reply: &str) -> String {
        format!(
            r#"{{"type":"event_msg","payload":{{"type":"task_complete","turn_id":"{turn_id}","last_agent_message":"{reply}"}}}}"#
        )
    }

    fn claude_user(content: &str) -> String {
        format!(r#"{{"type":"user","message":{{"role":"user","content":"{content}"}}}}"#)
    }

    fn claude_end_turn(reply: &str) -> String {
        format!(
            r#"{{"type":"assistant","message":{{"type":"message","role":"assistant","content":[{{"type":"text","text":"{reply}"}}],"stop_reason":"end_turn"}}}}"#
        )
    }

    #[test]
    fn cursor_mid_line_drops_partial_first_line() {
        let temp = tempfile::TempDir::new().unwrap();
        let file = temp.path().join("2026/06/rollout-session.jsonl");
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        let old = codex_complete("old-turn", "OLD");
        let new = codex_complete("new-turn", "NEW");
        fs::write(&file, format!("{old}\n{new}\n")).unwrap();

        let mut cursors = BTreeMap::new();
        cursors.insert(file.clone(), (old.len() / 2) as u64);

        let result = read_provider_log_tail("codex", temp.path(), &cursors).unwrap();

        assert_eq!(
            result.completions,
            vec![LogCompletion {
                parsed: LogParseResult::TurnComplete {
                    turn_id: Some("new-turn".to_string()),
                    reply: Some("NEW".to_string()),
                },
                raw_path: file.clone(),
                raw_offset: fs::metadata(&file).unwrap().len(),
            }]
        );
        assert_eq!(
            result.cursors.get(&file).copied(),
            Some(fs::metadata(&file).unwrap().len())
        );
    }

    #[test]
    fn dynamic_reglob_reads_file_created_after_dispatch() {
        let temp = tempfile::TempDir::new().unwrap();
        let file = temp.path().join("nested/rollout-new.jsonl");
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(&file, format!("{}\n", codex_complete("turn-1", "PONG"))).unwrap();

        let cursors = LogCursorMap::new();

        let result = read_provider_log_tail("codex", temp.path(), &cursors).unwrap();

        assert_eq!(
            result.completions,
            vec![LogCompletion {
                parsed: LogParseResult::TurnComplete {
                    turn_id: Some("turn-1".to_string()),
                    reply: Some("PONG".to_string()),
                },
                raw_path: file.clone(),
                raw_offset: fs::metadata(&file).unwrap().len(),
            }]
        );
        assert_eq!(
            result.cursors.get(&file).copied(),
            Some(fs::metadata(&file).unwrap().len())
        );
    }

    #[test]
    fn complete_before_cursor_is_ignored() {
        let temp = tempfile::TempDir::new().unwrap();
        let file = temp.path().join("rollout-session.jsonl");
        let old = codex_complete("old-turn", "OLD");
        let new = codex_complete("new-turn", "NEW");
        fs::write(&file, format!("{old}\n{new}\n")).unwrap();

        let mut cursors = LogCursorMap::new();
        cursors.insert(file.clone(), (old.len() + 1) as u64);

        let result = read_provider_log_tail("codex", temp.path(), &cursors).unwrap();

        assert_eq!(
            result.completions,
            vec![LogCompletion {
                parsed: LogParseResult::TurnComplete {
                    turn_id: Some("new-turn".to_string()),
                    reply: Some("NEW".to_string()),
                },
                raw_path: file,
                raw_offset: fs::metadata(temp.path().join("rollout-session.jsonl"))
                    .unwrap()
                    .len(),
            }]
        );
    }

    #[test]
    fn complete_event_consumed_once_by_path_offset() {
        let temp = tempfile::TempDir::new().unwrap();
        let file = temp.path().join("rollout-session.jsonl");
        fs::write(&file, format!("{}\n", codex_complete("turn-1", "PONG"))).unwrap();

        let cursors = LogCursorMap::new();
        let first = read_provider_log_tail("codex", temp.path(), &cursors).unwrap();
        let second = read_provider_log_tail("codex", temp.path(), &first.cursors).unwrap();

        assert_eq!(
            first.completions,
            vec![LogCompletion {
                parsed: LogParseResult::TurnComplete {
                    turn_id: Some("turn-1".to_string()),
                    reply: Some("PONG".to_string()),
                },
                raw_path: file.clone(),
                raw_offset: fs::metadata(&file).unwrap().len(),
            }]
        );
        assert!(second.completions.is_empty());
        assert_eq!(
            second.cursors.get(&file).copied(),
            Some(fs::metadata(&file).unwrap().len())
        );
    }

    #[test]
    fn claude_end_turn_without_prior_user_entry_after_cursor_is_not_completed() {
        let temp = tempfile::TempDir::new().unwrap();
        let file = temp.path().join("project/session.jsonl");
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(&file, format!("{}\n", claude_end_turn("STALE"))).unwrap();

        let result = read_provider_log_tail("claude", temp.path(), &LogCursorMap::new()).unwrap();

        assert!(result.completions.is_empty());
        assert_eq!(
            result.cursors.get(&file).copied(),
            Some(fs::metadata(&file).unwrap().len())
        );
    }

    #[test]
    fn claude_user_entry_then_end_turn_after_cursor_completes() {
        let temp = tempfile::TempDir::new().unwrap();
        let file = temp.path().join("project/session.jsonl");
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(
            &file,
            format!(
                "{}\n{}\n",
                claude_user("echo PONG"),
                claude_end_turn("PONG")
            ),
        )
        .unwrap();

        let result = read_provider_log_tail("claude", temp.path(), &LogCursorMap::new()).unwrap();

        assert_eq!(
            result.completions,
            vec![LogCompletion {
                parsed: LogParseResult::TurnComplete {
                    turn_id: None,
                    reply: Some("PONG".to_string()),
                },
                raw_path: file,
                raw_offset: fs::metadata(temp.path().join("project/session.jsonl"))
                    .unwrap()
                    .len(),
            }]
        );
    }

    #[test]
    fn claude_user_entry_then_end_turn_across_separate_reads_completes() {
        let temp = tempfile::TempDir::new().unwrap();
        let file = temp.path().join("project/session.jsonl");
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(&file, format!("{}\n", claude_user("echo PONG"))).unwrap();

        let first =
            read_provider_log_tail_with_state("claude", temp.path(), &LogReadState::default())
                .unwrap();
        assert!(first.completions.is_empty());
        fs::write(
            &file,
            format!(
                "{}\n{}\n",
                claude_user("echo PONG"),
                claude_end_turn("PONG")
            ),
        )
        .unwrap();

        let second =
            read_provider_log_tail_with_state("claude", temp.path(), &first.state).unwrap();

        assert_eq!(
            second.completions,
            vec![LogCompletion {
                parsed: LogParseResult::TurnComplete {
                    turn_id: None,
                    reply: Some("PONG".to_string()),
                },
                raw_path: file,
                raw_offset: fs::metadata(temp.path().join("project/session.jsonl"))
                    .unwrap()
                    .len(),
            }]
        );
    }

    #[test]
    fn claude_stale_end_turn_across_ticks_without_user_entry_is_rejected() {
        let temp = tempfile::TempDir::new().unwrap();
        let file = temp.path().join("project/session.jsonl");
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(&file, format!("{}\n", claude_end_turn("STALE-1"))).unwrap();

        let first =
            read_provider_log_tail_with_state("claude", temp.path(), &LogReadState::default())
                .unwrap();
        assert!(first.completions.is_empty());
        fs::write(
            &file,
            format!(
                "{}\n{}\n",
                claude_end_turn("STALE-1"),
                claude_end_turn("STALE-2")
            ),
        )
        .unwrap();

        let second =
            read_provider_log_tail_with_state("claude", temp.path(), &first.state).unwrap();

        assert!(second.completions.is_empty());
        assert_eq!(
            second.cursors.get(&file).copied(),
            Some(
                fs::metadata(temp.path().join("project/session.jsonl"))
                    .unwrap()
                    .len()
            )
        );
    }

    #[test]
    fn codex_task_complete_after_cursor_completes_without_user_entry() {
        let temp = tempfile::TempDir::new().unwrap();
        let file = temp.path().join("rollout-session.jsonl");
        fs::write(&file, format!("{}\n", codex_complete("turn-1", "PONG"))).unwrap();

        let result = read_provider_log_tail("codex", temp.path(), &LogCursorMap::new()).unwrap();

        assert_eq!(
            result.completions,
            vec![LogCompletion {
                parsed: LogParseResult::TurnComplete {
                    turn_id: Some("turn-1".to_string()),
                    reply: Some("PONG".to_string()),
                },
                raw_path: file,
                raw_offset: fs::metadata(temp.path().join("rollout-session.jsonl"))
                    .unwrap()
                    .len(),
            }]
        );
    }
}
