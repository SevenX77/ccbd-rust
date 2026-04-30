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
}

impl Default for MarkerMatcher {
    fn default() -> Self {
        Self {
            mode: IdleDetectionMode::LineEndRegex,
            regex: Regex::new(r"[\$#>✦]\s*$").expect("valid prompt regex"),
        }
    }
}

impl MarkerMatcher {
    /// Create a matcher from a custom regex.
    pub fn new(prompt_regex: Regex) -> Self {
        Self {
            mode: IdleDetectionMode::LineEndRegex,
            regex: prompt_regex,
        }
    }

    pub fn from_manifest(manifest: &ProviderManifest) -> Self {
        Self {
            mode: manifest.idle_detection_mode,
            regex: Regex::new(manifest.marker_pattern).expect("valid provider marker regex"),
        }
    }

    pub fn mode(&self) -> IdleDetectionMode {
        self.mode
    }

    /// Scan the bottom of a vt100 parser screen for a prompt marker.
    pub fn scan(&self, parser: &vt100::Parser) -> MatchResult {
        match self.mode {
            IdleDetectionMode::LineEndRegex => {
                let contents = parser.screen().contents();
                if self.scan_lines(contents.lines().rev().take(5)) {
                    return MatchResult::Matched;
                }
                if self.scan_lines(contents.lines().rev().take(20)) {
                    return MatchResult::Matched;
                }
                MatchResult::NoMatch
            }
            IdleDetectionMode::ObservedStability => {
                if self.regex.is_match(&parser.screen().contents()) {
                    MatchResult::Matched
                } else {
                    MatchResult::NoMatch
                }
            }
        }
    }

    fn scan_lines<'a>(&self, lines: impl Iterator<Item = &'a str>) -> bool {
        lines
            .map(str::trim_end)
            .any(|line| self.regex.is_match(line))
    }
}

#[cfg(test)]
mod tests {
    use crate::provider::manifest::{IdleDetectionMode, ProviderManifest};

    use super::{MarkerMatcher, MatchResult};

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
        let manifest = ProviderManifest {
            provider_name: "fake",
            auth_mount_paths: vec![],
            idle_detection_mode: IdleDetectionMode::ObservedStability,
            marker_pattern: r"READY_PROMPT",
            stability_ms: 300,
        };
        let matcher = MarkerMatcher::from_manifest(&manifest);
        let parser = parser_with(b"top line\nREADY_PROMPT\nmore output below\n");

        assert_eq!(matcher.scan(&parser), MatchResult::Matched);
    }

    #[test]
    fn test_observed_stability_does_not_match_unrelated_shell_dollar() {
        let manifest = ProviderManifest {
            provider_name: "fake",
            auth_mount_paths: vec![],
            idle_detection_mode: IdleDetectionMode::ObservedStability,
            marker_pattern: r"READY_PROMPT",
            stability_ms: 300,
        };
        let matcher = MarkerMatcher::from_manifest(&manifest);
        let parser = parser_with(b"price is $5\n");

        assert_eq!(matcher.scan(&parser), MatchResult::NoMatch);
    }
}
