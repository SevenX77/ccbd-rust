//! Synchronous Anthropic Haiku client for prompt slow-path classification.

use crate::prompt_handler::matcher::sanitize_pane_text;
use crate::prompt_handler::schema::PromptAction;
use serde::Deserialize;
use serde_json::json;
use std::path::PathBuf;
use thiserror::Error;

const ANTHROPIC_MESSAGES_URL: &str = "https://api.anthropic.com/v1/messages";
const HAIKU_45_MODEL: &str = "claude-haiku-4-5-20251001";
const ANTHROPIC_VERSION: &str = "2023-06-01";
const MAX_CONTEXT_CHARS: usize = 2000;

#[derive(Debug, Clone, PartialEq)]
pub struct LlmOutcome {
    pub category: String,
    pub action: Vec<PromptAction>,
    pub confidence: f64,
    pub safe: bool,
    pub suggested_regex: Option<String>,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum LlmError {
    #[error("missing Anthropic API key")]
    MissingKey,

    #[error("LLM request timed out")]
    Timeout,

    #[error("LLM transport failed: {0}")]
    Transport(String),

    #[error("LLM response JSON invalid: {0}")]
    InvalidResponse(String),

    #[error("LLM output JSON invalid: {0}")]
    InvalidOutput(String),
}

pub trait LlmClassifier: Send + Sync {
    fn classify_prompt(
        &self,
        capture: &str,
        provider: &str,
        agent_state: &str,
    ) -> Result<LlmOutcome, LlmError>;
}

pub trait LlmTransport: Send + Sync {
    fn post_json(&self, url: &str, api_key: &str, body: &str) -> Result<String, LlmError>;
}

pub struct RealHaikuClassifier;

impl LlmClassifier for RealHaikuClassifier {
    fn classify_prompt(
        &self,
        capture: &str,
        provider: &str,
        agent_state: &str,
    ) -> Result<LlmOutcome, LlmError> {
        call_haiku_45_sync(capture, provider, agent_state)
    }
}

pub struct UreqTransport;

impl LlmTransport for UreqTransport {
    fn post_json(&self, url: &str, api_key: &str, body: &str) -> Result<String, LlmError> {
        ureq::post(url)
            .set("x-api-key", api_key)
            .set("anthropic-version", ANTHROPIC_VERSION)
            .set("content-type", "application/json")
            .send_string(body)
            .map_err(map_ureq_error)?
            .into_string()
            .map_err(|err| LlmError::Transport(err.to_string()))
    }
}

fn map_ureq_error(err: ureq::Error) -> LlmError {
    let message = err.to_string();
    if message.to_ascii_lowercase().contains("timeout")
        || message.to_ascii_lowercase().contains("timed out")
    {
        LlmError::Timeout
    } else {
        LlmError::Transport(message)
    }
}

pub fn call_haiku_45_sync(
    capture: &str,
    provider: &str,
    agent_state: &str,
) -> Result<LlmOutcome, LlmError> {
    let api_key = resolve_anthropic_api_key().ok_or(LlmError::MissingKey)?;
    call_haiku_45_with_transport(capture, provider, agent_state, &api_key, &UreqTransport)
}

pub fn call_haiku_45_with_transport(
    capture: &str,
    provider: &str,
    agent_state: &str,
    api_key: &str,
    transport: &dyn LlmTransport,
) -> Result<LlmOutcome, LlmError> {
    let body = build_request_body(capture, provider, agent_state);
    let response = transport.post_json(ANTHROPIC_MESSAGES_URL, api_key, &body)?;
    parse_anthropic_response(&response)
}

fn build_request_body(capture: &str, provider: &str, agent_state: &str) -> String {
    let sanitized = tail_chars(&sanitize_pane_text(capture), MAX_CONTEXT_CHARS);
    json!({
        "model": HAIKU_45_MODEL,
        "max_tokens": 512,
        "temperature": 0,
        "system": "Identify whether a terminal pane shows an interactive CLI prompt. Return only compact JSON with keys: is_interactive_prompt, category, action, confidence, safe, suggested_regex.",
        "messages": [{
            "role": "user",
            "content": format!(
                "provider: {provider}\nagent_state: {agent_state}\npane_snapshot:\n{sanitized}"
            )
        }]
    })
    .to_string()
}

fn parse_anthropic_response(response: &str) -> Result<LlmOutcome, LlmError> {
    let envelope: AnthropicResponse =
        serde_json::from_str(response).map_err(|err| LlmError::InvalidResponse(err.to_string()))?;
    let text = envelope
        .content
        .into_iter()
        .find(|content| content.kind == "text")
        .map(|content| content.text)
        .ok_or_else(|| LlmError::InvalidResponse("missing text content".to_string()))?;
    let output_text = extract_json_object(&text)
        .ok_or_else(|| LlmError::InvalidOutput("missing JSON object".to_string()))?;
    let output: LlmOutput = serde_json::from_str(output_text)
        .map_err(|err| LlmError::InvalidOutput(err.to_string()))?;
    if !output.is_interactive_prompt {
        return Err(LlmError::InvalidOutput(
            "not an interactive prompt".to_string(),
        ));
    }
    Ok(LlmOutcome {
        category: output.category,
        action: output.action,
        confidence: output.confidence,
        safe: output.safe,
        suggested_regex: output.suggested_regex,
    })
}

fn resolve_anthropic_api_key() -> Option<String> {
    std::env::var("ANTHROPIC_API_KEY")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            config_api_key_from_path(std::env::var_os("CCB_CONFIG_PATH").map(PathBuf::from))
        })
        .or_else(|| config_api_key_from_path(find_cwd_config()))
        .or_else(|| config_api_key_from_path(home_config_path()))
}

fn config_api_key_from_path(path: Option<PathBuf>) -> Option<String> {
    let path = path?;
    let raw = std::fs::read_to_string(path).ok()?;
    api_key_from_toml(&raw)
}

fn api_key_from_toml(raw: &str) -> Option<String> {
    raw.parse::<toml::Value>()
        .ok()?
        .get("auth")?
        .get("anthropic")?
        .get("api_key")?
        .as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn find_cwd_config() -> Option<PathBuf> {
    let mut current = std::env::current_dir().ok()?;
    loop {
        let candidate = current.join("ccb.toml");
        if candidate.is_file() {
            return Some(candidate);
        }
        if !current.pop() {
            return None;
        }
    }
}

fn home_config_path() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .map(|home| home.join(".ccb").join("config.toml"))
        .filter(|path| path.is_file())
}

fn tail_chars(text: &str, max_chars: usize) -> String {
    let mut chars = text.chars().rev().take(max_chars).collect::<Vec<_>>();
    chars.reverse();
    chars.into_iter().collect()
}

fn extract_json_object(text: &str) -> Option<&str> {
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    (start <= end).then_some(&text[start..=end])
}

#[derive(Debug, Deserialize)]
struct AnthropicResponse {
    content: Vec<AnthropicContent>,
}

#[derive(Debug, Deserialize)]
struct AnthropicContent {
    #[serde(rename = "type")]
    kind: String,
    text: String,
}

#[derive(Debug, Deserialize)]
struct LlmOutput {
    is_interactive_prompt: bool,
    category: String,
    action: Vec<PromptAction>,
    confidence: f64,
    safe: bool,
    suggested_regex: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::{
        LlmError, LlmTransport, api_key_from_toml, call_haiku_45_with_transport,
        parse_anthropic_response,
    };
    use crate::prompt_handler::schema::PromptAction;
    use std::sync::Mutex;

    struct FakeTransport {
        response: Result<String, LlmError>,
        requests: Mutex<Vec<String>>,
    }

    impl FakeTransport {
        fn success(response: &str) -> Self {
            Self {
                response: Ok(response.to_string()),
                requests: Mutex::new(Vec::new()),
            }
        }

        fn failure(error: LlmError) -> Self {
            Self {
                response: Err(error),
                requests: Mutex::new(Vec::new()),
            }
        }
    }

    impl LlmTransport for FakeTransport {
        fn post_json(&self, _url: &str, _api_key: &str, body: &str) -> Result<String, LlmError> {
            self.requests.lock().unwrap().push(body.to_string());
            self.response.clone()
        }
    }

    fn anthropic_response(output_json: &str) -> String {
        json_string(&format!(
            r#"{{
                "content": [{{
                    "type": "text",
                    "text": {output}
                }}]
            }}"#,
            output = serde_json::to_string(output_json).unwrap()
        ))
    }

    fn json_string(value: &str) -> String {
        serde_json::from_str::<serde_json::Value>(value)
            .unwrap()
            .to_string()
    }

    #[test]
    fn parses_haiku_prompt_json_from_text_content() {
        let response = anthropic_response(
            r#"{"is_interactive_prompt":true,"category":"auto-accept","action":[{"type":"key","value":"Enter"}],"confidence":0.91,"safe":true,"suggested_regex":"Accept terms"}"#,
        );

        let outcome = parse_anthropic_response(&response).unwrap();

        assert_eq!(outcome.category, "auto-accept");
        assert_eq!(
            outcome.action,
            vec![PromptAction::Key {
                value: "Enter".into()
            }]
        );
        assert_eq!(outcome.confidence, 0.91);
        assert!(outcome.safe);
        assert_eq!(outcome.suggested_regex.as_deref(), Some("Accept terms"));
    }

    #[test]
    fn mock_transport_receives_sanitized_context_and_returns_outcome() {
        let response = anthropic_response(
            r#"{"is_interactive_prompt":true,"category":"auto-skip","action":[{"type":"key","value":"2"}],"confidence":0.93,"safe":true}"#,
        );
        let transport = FakeTransport::success(&response);

        let outcome = call_haiku_45_with_transport(
            "\u{1b}[31mProvider prompt\u{1b}[0m\nAccept?",
            "codex",
            "BUSY",
            "test-key",
            &transport,
        )
        .unwrap();

        assert_eq!(outcome.category, "auto-skip");
        assert_eq!(
            outcome.action,
            vec![PromptAction::Key { value: "2".into() }]
        );
        let request = transport.requests.lock().unwrap().pop().unwrap();
        assert!(request.contains("provider: codex"));
        assert!(request.contains("agent_state: BUSY"));
        assert!(request.contains("Provider prompt"));
        assert!(!request.contains("\u{1b}[31m"));
    }

    #[test]
    fn mock_transport_error_maps_to_llm_error() {
        let transport = FakeTransport::failure(LlmError::Transport("timeout".into()));

        let err = call_haiku_45_with_transport("prompt", "codex", "BUSY", "test-key", &transport)
            .unwrap_err();

        assert_eq!(err, LlmError::Transport("timeout".into()));
    }

    #[test]
    fn config_api_key_reads_auth_anthropic_table() {
        let key = api_key_from_toml(
            r#"
            [auth.anthropic]
            api_key = "sk-test"
            "#,
        );

        assert_eq!(key.as_deref(), Some("sk-test"));
    }
}
