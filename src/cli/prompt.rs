//! CLI helpers for manually resolving prompt-handler pending agents.

use crate::cli::rpc_client::{CliError, RpcClient};
use serde_json::{Value, json};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptResolveOptions {
    pub agent_id: String,
    pub action_json: Option<String>,
    pub keys: Option<String>,
    pub save_to_kb: bool,
}

pub async fn run_prompt_resolve(
    client: &impl RpcClient,
    options: PromptResolveOptions,
) -> Result<(), CliError> {
    let action = resolve_action_value(options.action_json.as_deref(), options.keys.as_deref())?;
    let result = client
        .call(
            "agent.resolve_prompt",
            json!({
                "agent_id": options.agent_id,
                "action": action,
                "save_to_kb": options.save_to_kb,
            }),
        )
        .await?;
    print_prompt_resolve_result(&result);
    Ok(())
}

pub fn resolve_action_value(
    action_json: Option<&str>,
    keys: Option<&str>,
) -> Result<Value, CliError> {
    match (action_json, keys) {
        (Some(_), Some(_)) => Err(CliError::Config(
            "use either --action or --keys, not both".to_string(),
        )),
        (Some(raw), None) => serde_json::from_str(raw).map_err(|err| {
            CliError::Config(format!("invalid --action JSON for prompt resolve: {err}"))
        }),
        (None, Some(keys)) => parse_keys(keys),
        (None, None) => Err(CliError::Config(
            "prompt resolve requires --action <json> or --keys <keys>".to_string(),
        )),
    }
}

pub fn parse_keys(keys: &str) -> Result<Value, CliError> {
    let values = keys
        .split_whitespace()
        .map(|value| {
            json!({
                "type": "key",
                "value": value,
            })
        })
        .collect::<Vec<_>>();
    if values.is_empty() {
        return Err(CliError::Config("--keys must not be empty".to_string()));
    }
    Ok(Value::Array(values))
}

fn print_prompt_resolve_result(result: &Value) {
    let state = result
        .get("resolved_state")
        .and_then(Value::as_str)
        .unwrap_or("UNKNOWN");
    let saved = result
        .get("saved_to_kb")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let action_sent = result
        .get("action_sent")
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default();
    println!("state: PROMPT_PENDING -> {state}");
    println!("action sent: {action_sent}");
    match result.get("case_id").and_then(Value::as_str) {
        Some(case_id) if saved => println!("saved to KB: yes (case_id: {case_id})"),
        _ if saved => println!("saved to KB: yes"),
        _ => println!("saved to KB: no"),
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_keys, resolve_action_value};

    #[test]
    fn parse_keys_builds_key_actions() {
        let value = parse_keys("2 Enter").unwrap();

        assert_eq!(value[0]["type"], "key");
        assert_eq!(value[0]["value"], "2");
        assert_eq!(value[1]["value"], "Enter");
    }

    #[test]
    fn action_json_and_keys_are_mutually_exclusive() {
        let err = resolve_action_value(Some("[]"), Some("Enter")).unwrap_err();

        assert!(err.to_string().contains("either --action or --keys"));
    }
}
