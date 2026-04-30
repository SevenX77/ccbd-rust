use crate::cli::output::parse_event_payload;
use crate::cli::rpc_client::{CliError, RpcClient};
use serde_json::{Value, json};
use std::io::Write;

pub async fn fetch_logs(
    client: &impl RpcClient,
    agent_id: &str,
    since_event_id: i64,
) -> Result<Vec<Value>, CliError> {
    let result = client
        .call(
            "agent.read",
            json!({
                "agent_id": agent_id,
                "since_event_id": since_event_id,
            }),
        )
        .await?;
    result
        .get("events")
        .and_then(Value::as_array)
        .cloned()
        .ok_or_else(|| CliError::InvalidResponse("agent.read missing events array".into()))
}

pub fn format_logs(events: &[Value]) -> Result<String, CliError> {
    let mut output = String::new();
    for event in events {
        match event.get("event_type").and_then(Value::as_str) {
            Some("output_chunk") => {
                let payload = parse_event_payload(event)?;
                if let Some(text) = payload.get("text").and_then(Value::as_str) {
                    output.push_str(text);
                }
            }
            Some("state_change") => {
                let payload = parse_event_payload(event)?;
                output.push_str(&format!("\n--- state_change {} ---\n", payload));
            }
            _ => {}
        }
    }
    Ok(output)
}

pub async fn run_logs(
    client: &impl RpcClient,
    agent_id: &str,
    since_event_id: i64,
) -> Result<(), CliError> {
    let events = fetch_logs(client, agent_id, since_event_id).await?;
    print!("{}", format_logs(&events)?);
    std::io::stdout().flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::format_logs;
    use serde_json::json;

    #[test]
    fn test_format_logs_prints_output_chunks() {
        let events = vec![json!({
            "event_type": "output_chunk",
            "payload": "{\"text\":\"hello\\n\"}"
        })];
        assert_eq!(format_logs(&events).unwrap(), "hello\n");
    }

    #[test]
    fn test_format_logs_includes_state_change() {
        let events = vec![json!({
            "event_type": "state_change",
            "payload": "{\"to\":\"IDLE\"}"
        })];
        assert!(format_logs(&events).unwrap().contains("state_change"));
    }
}
