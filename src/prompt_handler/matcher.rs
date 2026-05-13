//! Regex matcher and screen sanitization for prompt-handler cases.

use crate::prompt_handler::schema::{PromptAction, PromptCase, PromptFingerprint, PromptKb};
use regex::Regex;
use std::sync::OnceLock;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MatchOutcome {
    Matched {
        case_id: String,
        actions: Vec<PromptAction>,
    },
    NoMatch,
}

pub fn match_prompt(provider: &str, pane_text: &str, kb: &PromptKb) -> MatchOutcome {
    let sanitized = sanitize_pane_text(pane_text);
    tracing::info!(
        provider,
        cases = kb.cases.len(),
        "prompt matcher scan start"
    );

    for case in ordered_cases(kb) {
        if !case_applies(provider, case) {
            tracing::info!(
                provider,
                case_id = %case.id,
                case_provider = ?case.provider,
                "prompt matcher skipped provider mismatch"
            );
            continue;
        }

        let PromptFingerprint::Regex { pattern } = &case.fingerprint;
        let regex = match Regex::new(pattern) {
            Ok(regex) => regex,
            Err(err) => {
                tracing::warn!(
                    provider,
                    case_id = %case.id,
                    pattern,
                    reason = %err,
                    impact = "case skipped; prompt-handler continues scanning remaining cases",
                    "prompt matcher skipped invalid regex"
                );
                continue;
            }
        };

        if !regex.is_match(&sanitized) {
            tracing::info!(
                provider,
                case_id = %case.id,
                "prompt matcher case did not match"
            );
            continue;
        }

        tracing::info!(
            provider,
            case_id = %case.id,
            actions = case.action.len(),
            "prompt matcher matched case"
        );
        return MatchOutcome::Matched {
            case_id: case.id.clone(),
            actions: case.action.clone(),
        };
    }

    tracing::info!(provider, "prompt matcher no match");
    MatchOutcome::NoMatch
}

pub fn sanitize_pane_text(pane_text: &str) -> String {
    let without_ansi = crate::db::jobs::strip_ansi_escapes(pane_text);
    let mut lines = Vec::new();
    for line in without_ansi.lines() {
        let mut current = line.to_string();
        current = braille_spinner_regex()
            .replace_all(&current, "")
            .into_owned();
        current = ascii_spinner_prefix_regex()
            .replace_all(&current, "")
            .into_owned();
        current = elapsed_time_regex().replace_all(&current, "").into_owned();
        current = progress_percent_regex()
            .replace_all(&current, "")
            .into_owned();
        current = progress_bar_regex()
            .replace_all(&current, "[]")
            .into_owned();
        let trimmed = current.trim_end();
        if !trimmed.is_empty() {
            lines.push(trimmed.to_string());
        }
    }
    collapse_blankish_lines(lines.join("\n").trim())
}

fn ordered_cases(kb: &PromptKb) -> Vec<&PromptCase> {
    let mut cases = kb.cases.iter().collect::<Vec<_>>();
    cases.sort_by_key(|case| is_default_case(case));
    cases
}

fn is_default_case(case: &PromptCase) -> bool {
    case.created_by.as_deref() == Some("ccbd-default")
}

fn case_applies(provider: &str, case: &PromptCase) -> bool {
    case.provider
        .as_deref()
        .is_none_or(|case_provider| case_provider == provider)
}

fn collapse_blankish_lines(text: &str) -> String {
    let mut out = Vec::new();
    let mut last_empty = false;
    for line in text.lines() {
        let trimmed = line.trim_end();
        let empty = trimmed.is_empty();
        if empty && last_empty {
            continue;
        }
        out.push(trimmed);
        last_empty = empty;
    }
    out.join("\n")
}

fn braille_spinner_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"[\u{2800}-\u{28FF}]+").expect("valid braille regex"))
}

fn ascii_spinner_prefix_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^\s*[-\\|/]\s+").expect("valid ascii spinner regex"))
}

fn elapsed_time_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\b\d+(?:\.\d+)?s\b").expect("valid elapsed time regex"))
}

fn progress_percent_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\b\d{1,3}%").expect("valid progress percent regex"))
}

fn progress_bar_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\[[=>#\-\s]{3,}\]").expect("valid progress bar regex"))
}

#[cfg(test)]
mod tests {
    use super::{MatchOutcome, match_prompt, sanitize_pane_text};
    use crate::prompt_handler::schema::{PromptAction, PromptCase, PromptFingerprint, PromptKb};
    use crate::prompt_handler::seeds::default_cases;

    fn user_case(id: &str, pattern: &str, action_value: &str) -> PromptCase {
        PromptCase {
            id: id.to_string(),
            provider: Some("codex".to_string()),
            fingerprint: PromptFingerprint::Regex {
                pattern: pattern.to_string(),
            },
            action: vec![PromptAction::Key {
                value: action_value.to_string(),
            }],
            category: "auto-skip".to_string(),
            description: None,
            confidence_threshold: Some(0.9),
            used_count: 0,
            created_at: None,
            last_used_at: None,
            created_by: Some("master-manual".to_string()),
            regex_flags: Vec::new(),
            trigger_state: None,
        }
    }

    #[test]
    fn codex_update_matches_builtin_skip_action() {
        let kb = PromptKb::new(default_cases());
        let pane = "Update available! 0.129 -> 0.130\nRun `npm install -g @openai/codex`";

        let outcome = match_prompt("codex", pane, &kb);

        assert_eq!(
            outcome,
            MatchOutcome::Matched {
                case_id: "codex_update_01".to_string(),
                actions: vec![
                    PromptAction::Key { value: "2".into() },
                    PromptAction::Key {
                        value: "Enter".into()
                    },
                ],
            }
        );
    }

    #[test]
    fn trust_path_matches_builtin_accept_action() {
        let kb = PromptKb::new(default_cases());
        let pane = "Do you trust this directory?\n1) Yes\n2) No";

        let outcome = match_prompt("gemini", pane, &kb);

        assert_eq!(
            outcome,
            MatchOutcome::Matched {
                case_id: "trust_path_01".to_string(),
                actions: vec![
                    PromptAction::Key { value: "1".into() },
                    PromptAction::Key {
                        value: "Enter".into()
                    },
                ],
            }
        );
    }

    #[test]
    fn sanitization_removes_dynamic_noise() {
        let first = "\u{1b}[31m⠴ Working(1s) [==>   ] 20%\u{1b}[0m\nUpdate available!";
        let second = "\u{1b}[32m⠦ Working(9s) [====>] 80%\u{1b}[0m\nUpdate available!";

        assert_eq!(sanitize_pane_text(first), sanitize_pane_text(second));
    }

    #[test]
    fn user_case_has_priority_over_default_case() {
        let mut cases = default_cases();
        cases.push(user_case(
            "user_codex_update",
            r"(?is)update available!?.*npm install -g @openai/codex",
            "Esc",
        ));
        let kb = PromptKb::new(cases);

        let outcome = match_prompt(
            "codex",
            "Update available! Run `npm install -g @openai/codex`",
            &kb,
        );

        assert_eq!(
            outcome,
            MatchOutcome::Matched {
                case_id: "user_codex_update".to_string(),
                actions: vec![PromptAction::Key {
                    value: "Esc".into()
                }],
            }
        );
    }

    #[test]
    fn invalid_regex_is_skipped_without_blocking_next_case() {
        let kb = PromptKb::new(vec![
            user_case("bad_regex", "(", "Esc"),
            user_case("good_regex", "safe prompt", "Enter"),
        ]);

        let outcome = match_prompt("codex", "safe prompt", &kb);

        assert_eq!(
            outcome,
            MatchOutcome::Matched {
                case_id: "good_regex".to_string(),
                actions: vec![PromptAction::Key {
                    value: "Enter".into()
                }],
            }
        );
    }
}
