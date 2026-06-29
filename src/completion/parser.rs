use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LogParseResult {
    TurnComplete {
        turn_id: Option<String>,
        reply: Option<String>,
    },
    UserMessage {
        turn_id: Option<String>,
    },
    NotTerminal,
    UnknownDegrade {
        reason: String,
    },
}

pub fn parse_provider_log_line(provider: &str, line: &str) -> LogParseResult {
    let Ok(value) = serde_json::from_str::<Value>(line) else {
        return LogParseResult::NotTerminal;
    };

    match provider {
        "codex" => parse_codex_log_value(&value),
        "claude" => parse_claude_log_value(&value),
        "antigravity" => parse_antigravity_log_value(&value),
        _ => LogParseResult::NotTerminal,
    }
}

pub fn provider_log_line_has_assistant_progress(provider: &str, line: &str) -> bool {
    let Ok(value) = serde_json::from_str::<Value>(line) else {
        return false;
    };

    match provider {
        "claude" => claude_log_value_has_assistant_progress(&value),
        _ => false,
    }
}

fn parse_codex_log_value(value: &Value) -> LogParseResult {
    if value.get("type").and_then(Value::as_str) != Some("event_msg") {
        return LogParseResult::NotTerminal;
    }
    let Some(payload) = value.get("payload") else {
        return LogParseResult::NotTerminal;
    };
    if payload.get("type").and_then(Value::as_str) != Some("task_complete") {
        return LogParseResult::NotTerminal;
    }

    LogParseResult::TurnComplete {
        turn_id: payload
            .get("turn_id")
            .and_then(Value::as_str)
            .map(str::to_string),
        reply: payload
            .get("last_agent_message")
            .and_then(Value::as_str)
            .map(str::to_string),
    }
}

fn parse_claude_log_value(value: &Value) -> LogParseResult {
    if is_claude_user_entry(value) {
        return LogParseResult::UserMessage {
            turn_id: value
                .get("promptId")
                .and_then(Value::as_str)
                .map(str::to_string),
        };
    }

    if value.get("type").and_then(Value::as_str) != Some("assistant") {
        return LogParseResult::NotTerminal;
    }
    let Some(message) = value.get("message") else {
        return LogParseResult::NotTerminal;
    };
    if message.get("type").and_then(Value::as_str) != Some("message")
        || message.get("role").and_then(Value::as_str) != Some("assistant")
    {
        return LogParseResult::NotTerminal;
    }

    match message.get("stop_reason").and_then(Value::as_str) {
        Some("end_turn" | "stop_sequence" | "max_tokens") => {
            let Some(reply) = claude_text_reply(message) else {
                return LogParseResult::NotTerminal;
            };
            LogParseResult::TurnComplete {
                turn_id: None,
                reply: Some(reply),
            }
        }
        Some("tool_use") => LogParseResult::NotTerminal,
        Some(stop_reason) => {
            tracing::warn!(stop_reason, "unknown Claude stop_reason in completion log");
            LogParseResult::UnknownDegrade {
                reason: "claude_unknown_stop_reason".to_string(),
            }
        }
        None => {
            tracing::warn!("missing Claude stop_reason in completion log");
            LogParseResult::UnknownDegrade {
                reason: "claude_missing_stop_reason".to_string(),
            }
        }
    }
}

fn is_claude_user_entry(value: &Value) -> bool {
    if value.get("type").and_then(Value::as_str) == Some("user") {
        return true;
    }
    value
        .get("message")
        .and_then(|message| message.get("role"))
        .and_then(Value::as_str)
        == Some("user")
}

fn claude_log_value_has_assistant_progress(value: &Value) -> bool {
    if value.get("type").and_then(Value::as_str) != Some("assistant") {
        return false;
    }
    let Some(message) = value.get("message") else {
        return false;
    };
    if message.get("type").and_then(Value::as_str) != Some("message")
        || message.get("role").and_then(Value::as_str) != Some("assistant")
    {
        return false;
    }
    message
        .get("content")
        .and_then(Value::as_array)
        .is_some_and(|content| {
            content.iter().any(|item| {
                matches!(
                    item.get("type").and_then(Value::as_str),
                    Some("text" | "tool_use" | "thinking")
                )
            })
        })
}

fn claude_text_reply(message: &Value) -> Option<String> {
    let content = message.get("content")?.as_array()?;
    let text_parts = content
        .iter()
        .filter_map(|item| {
            if item.get("type").and_then(Value::as_str) == Some("text") {
                item.get("text").and_then(Value::as_str)
            } else {
                None
            }
        })
        .collect::<Vec<_>>();

    if text_parts.is_empty() {
        None
    } else {
        Some(text_parts.join(""))
    }
}

fn parse_antigravity_log_value(value: &Value) -> LogParseResult {
    if value.get("source").and_then(Value::as_str) == Some("USER_EXPLICIT")
        && value.get("type").and_then(Value::as_str) == Some("USER_INPUT")
    {
        return LogParseResult::UserMessage { turn_id: None };
    }

    if value.get("source").and_then(Value::as_str) != Some("MODEL")
        || value.get("type").and_then(Value::as_str) != Some("PLANNER_RESPONSE")
        || value.get("status").and_then(Value::as_str) != Some("DONE")
    {
        return LogParseResult::NotTerminal;
    }

    if value
        .get("tool_calls")
        .and_then(Value::as_array)
        .is_some_and(|tool_calls| !tool_calls.is_empty())
    {
        return LogParseResult::NotTerminal;
    }

    LogParseResult::TurnComplete {
        turn_id: None,
        reply: value
            .get("content")
            .and_then(Value::as_str)
            .map(str::to_string),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        LogParseResult, parse_provider_log_line, provider_log_line_has_assistant_progress,
    };

    #[test]
    fn codex_task_complete_emits_turn_complete() {
        let line = r#"{"timestamp":"2026-06-02T02:03:43.476Z","type":"event_msg","payload":{"type":"task_complete","turn_id":"turn-1","last_agent_message":"PONG"}}"#;

        let parsed = parse_provider_log_line("codex", line);

        assert_eq!(
            parsed,
            LogParseResult::TurnComplete {
                turn_id: Some("turn-1".to_string()),
                reply: Some("PONG".to_string()),
            }
        );
    }

    #[test]
    fn codex_agent_message_without_task_complete_is_not_terminal() {
        let line = r#"{"type":"event_msg","payload":{"type":"agent_message","message":"PONG","phase":"final_answer"}}"#;

        let parsed = parse_provider_log_line("codex", line);

        assert_eq!(parsed, LogParseResult::NotTerminal);
    }

    #[test]
    fn claude_end_turn_emits_turn_complete() {
        let line = r#"{"type":"assistant","message":{"model":"claude-opus-4-8","type":"message","role":"assistant","content":[{"type":"text","text":"PONG"}],"stop_reason":"end_turn"}}"#;

        let parsed = parse_provider_log_line("claude", line);

        assert_eq!(
            parsed,
            LogParseResult::TurnComplete {
                turn_id: None,
                reply: Some("PONG".to_string()),
            }
        );
    }

    #[test]
    fn claude_assistant_progress_does_not_require_end_turn() {
        let progress = r#"{"type":"assistant","message":{"type":"message","role":"assistant","content":[{"type":"text","text":"working"}]}}"#;
        let user = r#"{"type":"user","message":{"role":"user","content":"继续"}}"#;
        let pane_echo = "继续";

        assert!(provider_log_line_has_assistant_progress("claude", progress));
        assert!(!provider_log_line_has_assistant_progress("claude", user));
        assert!(!provider_log_line_has_assistant_progress(
            "claude", pane_echo
        ));
    }

    #[test]
    fn claude_thinking_endturn_line_then_text_line() {
        let thinking_line = r#"{"type":"assistant","message":{"type":"message","role":"assistant","content":[{"type":"thinking","thinking":"Working through it"}],"stop_reason":"end_turn"}}"#;
        let text_line = r#"{"type":"assistant","message":{"type":"message","role":"assistant","content":[{"type":"text","text":"ANSWER"}],"stop_reason":"end_turn"}}"#;

        let thinking_parsed = parse_provider_log_line("claude", thinking_line);
        let text_parsed = parse_provider_log_line("claude", text_line);

        assert_eq!(thinking_parsed, LogParseResult::NotTerminal);
        assert_eq!(
            text_parsed,
            LogParseResult::TurnComplete {
                turn_id: None,
                reply: Some("ANSWER".to_string()),
            }
        );
    }

    #[test]
    fn claude_text_only_endturn_still_completes() {
        let line = r#"{"type":"assistant","message":{"type":"message","role":"assistant","content":[{"type":"text","text":"X"}],"stop_reason":"end_turn"}}"#;

        let parsed = parse_provider_log_line("claude", line);

        assert_eq!(
            parsed,
            LogParseResult::TurnComplete {
                turn_id: None,
                reply: Some("X".to_string()),
            }
        );
    }

    #[test]
    fn claude_tool_use_is_busy_not_terminal() {
        let line = r#"{"type":"assistant","message":{"type":"message","role":"assistant","content":[{"type":"tool_use","id":"toolu_1","name":"Bash","input":{"command":"echo PONG"}}],"stop_reason":"tool_use"}}"#;

        let parsed = parse_provider_log_line("claude", line);

        assert_eq!(parsed, LogParseResult::NotTerminal);
    }

    #[test]
    fn claude_unknown_stop_reason_warns_and_falls_back() {
        let line = r#"{"type":"assistant","message":{"type":"message","role":"assistant","content":[{"type":"text","text":"PONG"}],"stop_reason":"paused_for_unknown_reason"}}"#;

        let parsed = parse_provider_log_line("claude", line);

        assert_eq!(
            parsed,
            LogParseResult::UnknownDegrade {
                reason: "claude_unknown_stop_reason".to_string(),
            }
        );
    }

    #[test]
    fn antigravity_user_input_emits_user_message_baseline() {
        let line = include_str!("../../tests/fixtures/antigravity_log/final_reply.jsonl")
            .lines()
            .next()
            .unwrap();

        let parsed = parse_provider_log_line("antigravity", line);

        assert_eq!(parsed, LogParseResult::UserMessage { turn_id: None });
    }

    #[test]
    fn antigravity_final_planner_response_emits_turn_complete() {
        let line = include_str!("../../tests/fixtures/antigravity_log/final_reply.jsonl")
            .lines()
            .nth(1)
            .unwrap();

        let parsed = parse_provider_log_line("antigravity", line);

        assert_eq!(
            parsed,
            LogParseResult::TurnComplete {
                turn_id: None,
                reply: Some("The requested summary is complete.".to_string()),
            }
        );
    }

    #[test]
    fn antigravity_planner_response_with_tool_call_is_not_terminal() {
        let line = include_str!("../../tests/fixtures/antigravity_log/tool_call_in_progress.jsonl")
            .lines()
            .nth(1)
            .unwrap();

        let parsed = parse_provider_log_line("antigravity", line);

        assert_eq!(parsed, LogParseResult::NotTerminal);
    }

    #[test]
    fn antigravity_cancelled_planner_response_is_not_terminal() {
        let line = include_str!("../../tests/fixtures/antigravity_log/cancelled_no_final.jsonl")
            .lines()
            .nth(1)
            .unwrap();

        let parsed = parse_provider_log_line("antigravity", line);

        assert_eq!(parsed, LogParseResult::NotTerminal);
    }

    #[test]
    fn bad_json_line_is_not_terminal_without_panic() {
        let parsed = parse_provider_log_line("codex", "{not json");

        assert_eq!(parsed, LogParseResult::NotTerminal);
    }

    #[test]
    fn missing_payload_or_message_fields_do_not_panic() {
        assert_eq!(
            parse_provider_log_line("codex", r#"{"type":"event_msg"}"#),
            LogParseResult::NotTerminal
        );
        assert_eq!(
            parse_provider_log_line("claude", r#"{"type":"assistant"}"#),
            LogParseResult::NotTerminal
        );
    }
}
