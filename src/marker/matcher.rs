//! Prompt marker matcher over a vt100 screen buffer.

use crate::provider::manifest::{IdleDetectionMode, ProviderManifest};
use regex::Regex;

/// Result of scanning the terminal screen for a prompt marker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchResult {
    Matched,
    NoMatch,
}

/// Regex-backed prompt matcher.
pub struct MarkerMatcher {
    mode: IdleDetectionMode,
    regex: Regex,
    anti_regex: Option<Regex>,
}

impl Default for MarkerMatcher {
    fn default() -> Self {
        Self {
            mode: IdleDetectionMode::LineEndRegex,
            regex: Regex::new(r"[\$#>✦]\s*$").expect("valid prompt regex"),
            anti_regex: None,
        }
    }
}

impl MarkerMatcher {
    /// Create a matcher from a custom regex.
    pub fn new(prompt_regex: Regex) -> Self {
        Self {
            mode: IdleDetectionMode::LineEndRegex,
            regex: prompt_regex,
            anti_regex: None,
        }
    }

    pub fn from_manifest(manifest: &ProviderManifest) -> Self {
        Self {
            mode: manifest.idle_detection_mode,
            regex: Regex::new(prompt_regex_for_provider(manifest.provider_name))
                .expect("valid provider prompt regex"),
            anti_regex: if manifest.idle_anti_pattern.is_empty() {
                None
            } else {
                Some(Regex::new(manifest.idle_anti_pattern).expect("valid idle anti regex"))
            },
        }
    }

    pub fn mode(&self) -> IdleDetectionMode {
        self.mode
    }

    /// Scan a vt100 parser screen for a prompt marker.
    pub fn scan(&self, parser: &vt100::Parser) -> MatchResult {
        let screen = parser.screen();
        let cursor_row = screen.cursor_position().0;
        let contents = screen.contents();
        let prompt_matched = match self.mode {
            IdleDetectionMode::LineEndRegex => self.scan_lines_at_cursor(&contents, cursor_row),
            IdleDetectionMode::ObservedStability => self.regex.is_match(&contents),
        };
        if !prompt_matched {
            return MatchResult::NoMatch;
        }
        if let Some(anti_regex) = &self.anti_regex
            && anti_regex.is_match(&contents)
        {
            return MatchResult::NoMatch;
        }
        MatchResult::Matched
    }

    fn scan_lines_at_cursor(&self, contents: &str, cursor_row: u16) -> bool {
        let lines = contents.lines().collect::<Vec<_>>();
        let cursor_idx = cursor_row as usize;
        [cursor_idx, cursor_idx.saturating_sub(1)]
            .into_iter()
            .filter_map(|idx| lines.get(idx))
            .map(|line| line.trim_end())
            .any(|line| self.regex.is_match(line))
    }
}

fn prompt_regex_for_provider(provider: &str) -> &'static str {
    match provider {
        "codex" => r"(?m)^\s*›(?:\s|$)",
        "gemini" => r"Type your message or @path/to/file",
        "claude" => r"(?m)^\s*❯\s*$",
        _ => r"[\$#>✦]\s*$",
    }
}

#[cfg(test)]
mod tests {
    use super::{MarkerMatcher, MatchResult};
    use crate::provider::manifest::{IdleDetectionMode, get_manifest};
    use regex::Regex;

    fn parser_with(bytes: &[u8]) -> vt100::Parser {
        let mut parser = vt100::Parser::new(200, 200, 0);
        parser.process(bytes);
        parser
    }

    #[test]
    fn test_matches_common_shell_prompts() {
        let matcher = MarkerMatcher::default();
        for prompt in [b"$ ".as_slice(), b"# ".as_slice(), b"> ".as_slice()] {
            let parser = parser_with(prompt);
            assert_eq!(matcher.scan(&parser), MatchResult::Matched);
        }
    }

    #[test]
    fn test_matches_unicode_prompt() {
        let matcher = MarkerMatcher::default();
        let parser = parser_with("✦ ".as_bytes());

        assert_eq!(matcher.scan(&parser), MatchResult::Matched);
    }

    #[test]
    fn test_ansi_colored_prompt_matches_after_vt100_processing() {
        let matcher = MarkerMatcher::default();
        let parser = parser_with(b"\x1b[32m$\x1b[0m ");

        assert_eq!(matcher.scan(&parser), MatchResult::Matched);
    }

    #[test]
    fn test_plain_output_does_not_match() {
        let matcher = MarkerMatcher::default();
        let parser = parser_with(b"hello\nworld\n");

        assert_eq!(matcher.scan(&parser), MatchResult::NoMatch);
    }

    #[test]
    fn test_observed_stability_matches_prompt_anywhere_on_screen() {
        let matcher = MarkerMatcher {
            mode: IdleDetectionMode::ObservedStability,
            regex: Regex::new(r"READY_PROMPT").unwrap(),
            anti_regex: None,
        };
        let parser = parser_with(b"top line\nREADY_PROMPT\nmore output below\n");

        assert_eq!(matcher.scan(&parser), MatchResult::Matched);
    }

    #[test]
    fn test_observed_stability_does_not_match_unrelated_shell_dollar() {
        let matcher = MarkerMatcher {
            mode: IdleDetectionMode::ObservedStability,
            regex: Regex::new(r"READY_PROMPT").unwrap(),
            anti_regex: None,
        };
        let parser = parser_with(b"price is $5\n");

        assert_eq!(matcher.scan(&parser), MatchResult::NoMatch);
    }

    #[test]
    fn test_historical_prompt_away_from_cursor_does_not_match() {
        let matcher = MarkerMatcher::default();
        let parser = parser_with(b"$ first command\noutput1\n$ second command\nrunning...\n");

        assert_eq!(matcher.scan(&parser), MatchResult::NoMatch);
    }

    #[test]
    fn test_prompt_on_previous_cursor_line_matches() {
        let matcher = MarkerMatcher::default();
        let parser = parser_with(b"command output\n$ \n");

        assert_eq!(matcher.scan(&parser), MatchResult::Matched);
    }

    #[test]
    fn test_marker_matcher_codex_suppresses_idle_when_working_spinner_present() {
        let manifest = get_manifest("codex");
        let matcher = MarkerMatcher::from_manifest(&manifest);
        let parser = parser_with("› echo from a1\n◦ Working (2s • esc to interrupt)\n".as_bytes());

        assert_eq!(matcher.scan(&parser), MatchResult::NoMatch);
    }

    #[test]
    fn test_marker_matcher_codex_marks_idle_when_no_spinner() {
        let manifest = get_manifest("codex");
        let matcher = MarkerMatcher::from_manifest(&manifest);
        let parser = parser_with("› \n  gpt-5.5 default · /workspace\n".as_bytes());

        assert_eq!(matcher.scan(&parser), MatchResult::Matched);
    }

    #[test]
    fn test_marker_matcher_gemini_suppresses_idle_when_thinking_spinner() {
        let manifest = get_manifest("gemini");
        let matcher = MarkerMatcher::from_manifest(&manifest);
        let parser =
            parser_with("⠋ Thinking...\n >   Type your message or @path/to/file\n".as_bytes());

        assert_eq!(matcher.scan(&parser), MatchResult::NoMatch);
    }

    #[test]
    fn test_marker_matcher_claude_no_anti_pattern_unchanged() {
        let manifest = get_manifest("claude");
        let matcher = MarkerMatcher::from_manifest(&manifest);
        let parser = parser_with("❯\n".as_bytes());

        assert_eq!(matcher.scan(&parser), MatchResult::Matched);
    }
}
