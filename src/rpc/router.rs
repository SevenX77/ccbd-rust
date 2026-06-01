use crate::error::CcbdError;
use crate::rpc::Ctx;
use crate::rpc::handlers::{
    handle_agent_assert_state, handle_agent_discard_evidence, handle_agent_kill,
    handle_agent_learn_rule, handle_agent_read, handle_agent_realign, handle_agent_resolve_prompt,
    handle_agent_send, handle_agent_spawn, handle_agent_watch, handle_event_subscribe,
    handle_evidence_insert, handle_job_cancel, handle_job_has_evidence,
    handle_job_mark_requires_evidence, handle_job_submit, handle_job_wait, handle_session_create,
    handle_session_kill, handle_session_list, handle_session_realign,
    handle_session_spawn_master_pane, handle_system_dump, handle_system_shutdown,
};
use serde_json::{Value, json};

const METHODS: &[&str] = &[
    "session.create",
    "session.kill",
    "session.spawn_master_pane",
    "session.realign",
    "session.list",
    "agent.spawn",
    "agent.realign",
    "agent.send",
    "agent.read",
    "agent.watch",
    "agent.kill",
    "agent.resolve_prompt",
    "agent.learn_rule",
    "agent.assert_state",
    "agent.discard_evidence",
    "evidence.insert",
    "event.subscribe",
    "job.has_evidence",
    "job.mark_requires_evidence",
    "job.submit",
    "job.wait",
    "job.cancel",
    "system.dump",
    "system.shutdown",
];

pub async fn dispatch(line: &str, ctx: &Ctx) -> String {
    let request: Value = match serde_json::from_str(line) {
        Ok(value) => value,
        Err(err) => {
            return error_response(
                Value::Null,
                CcbdError::IpcInvalidRequest(format!("malformed JSON: {err}")),
            );
        }
    };

    let id = request.get("id").cloned().unwrap_or(Value::Null);

    if request.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return error_response(
            id,
            CcbdError::IpcInvalidRequest("missing or invalid jsonrpc version".into()),
        );
    }

    let Some(method) = request.get("method").and_then(Value::as_str) else {
        return error_response(
            id,
            CcbdError::IpcInvalidRequest("missing field 'method'".into()),
        );
    };

    if !METHODS.contains(&method) {
        return error_response(
            id,
            CcbdError::IpcInvalidRequest(format!("unknown method: {method}")),
        );
    }

    let params = request.get("params").cloned().unwrap_or(Value::Null);
    let result = match method {
        "session.create" => handle_session_create(params, ctx).await,
        "session.kill" => handle_session_kill(params, ctx).await,
        "session.spawn_master_pane" => handle_session_spawn_master_pane(params, ctx).await,
        "session.realign" => handle_session_realign(params, ctx).await,
        "session.list" => handle_session_list(params, ctx).await,
        "agent.spawn" => handle_agent_spawn(params, ctx).await,
        "agent.realign" => handle_agent_realign(params, ctx).await,
        "agent.send" => handle_agent_send(params, ctx).await,
        "agent.read" => handle_agent_read(params, ctx).await,
        "agent.watch" => handle_agent_watch(params, ctx).await,
        "agent.kill" => handle_agent_kill(params, ctx).await,
        "agent.resolve_prompt" => handle_agent_resolve_prompt(params, ctx).await,
        "agent.learn_rule" => handle_agent_learn_rule(params, ctx).await,
        "agent.assert_state" => handle_agent_assert_state(params, ctx).await,
        "agent.discard_evidence" => handle_agent_discard_evidence(params, ctx).await,
        "evidence.insert" => handle_evidence_insert(params, ctx).await,
        "event.subscribe" => handle_event_subscribe(params, ctx).await,
        "job.has_evidence" => handle_job_has_evidence(params, ctx).await,
        "job.mark_requires_evidence" => handle_job_mark_requires_evidence(params, ctx).await,
        "job.submit" => handle_job_submit(params, ctx).await,
        "job.wait" => handle_job_wait(params, ctx).await,
        "job.cancel" => handle_job_cancel(params, ctx).await,
        "system.dump" => handle_system_dump(params, ctx).await,
        "system.shutdown" => handle_system_shutdown(params, ctx).await,
        _ => unreachable!("method whitelist checked above"),
    };

    match result {
        Ok(value) => success_response(id, value),
        Err(CcbdError::IpcInvalidRequest(message))
            if method == "job.mark_requires_evidence" && message.contains("job_id not found") =>
        {
            json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": {
                    "code": "IPC_INVALID_REQUEST",
                    "message": format!("invalid IPC request: {message}"),
                    "data": {"error_code": "IPC_INVALID_REQUEST"}
                }
            })
            .to_string()
        }
        Err(CcbdError::DuplicateRequest { existing_seq_id }) => error_response(
            id,
            CcbdError::IpcInvalidRequest(format!(
                "internal duplicate request sentinel leaked for seq_id={existing_seq_id}"
            )),
        ),
        Err(err) => error_response(id, err),
    }
}

fn success_response(id: Value, result: Value) -> String {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": result,
    })
    .to_string()
}

fn error_response(id: Value, err: CcbdError) -> String {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": err.to_rpc_error(),
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::dispatch;
    use crate::db;
    use crate::db::learned_rules::{LearnedRuleCategory, lookup_learned_rules_sync};
    use crate::rpc::Ctx;
    use crate::sandbox::EnvState;
    use crate::tmux::TmuxServer;
    use serde_json::Value;
    use std::sync::Arc;

    fn test_ctx() -> Ctx {
        let file = tempfile::NamedTempFile::new().unwrap();
        let state_dir = tempfile::TempDir::new().unwrap().keep();
        Ctx {
            db: db::init(file.path()).unwrap(),
            state_dir: state_dir.clone(),
            env_state: EnvState {
                systemd_run_available: false,
                unsafe_no_sandbox: true,
                under_systemd: false,
            },
            daemon_unit: None,
            tmux_server: Arc::new(TmuxServer::new(&state_dir)),
        }
    }

    #[tokio::test]
    async fn test_dispatch_invalid_json_returns_ipc_invalid_request() {
        let ctx = test_ctx();
        let response = dispatch("not json", &ctx).await;
        let obj: Value = serde_json::from_str(&response).unwrap();

        assert_eq!(obj["jsonrpc"], "2.0");
        assert_eq!(obj["id"], Value::Null);
        assert_eq!(obj["error"]["code"], -32000);
        assert_eq!(obj["error"]["data"]["error_code"], "IPC_INVALID_REQUEST");
    }

    #[tokio::test]
    async fn test_dispatch_unknown_method_returns_ipc_invalid_request() {
        let ctx = test_ctx();
        let response = dispatch(r#"{"jsonrpc":"2.0","method":"weird.foo","id":1}"#, &ctx).await;
        let obj: Value = serde_json::from_str(&response).unwrap();

        assert_eq!(obj["id"], 1);
        assert_eq!(obj["error"]["code"], -32000);
        assert_eq!(obj["error"]["data"]["error_code"], "IPC_INVALID_REQUEST");
        assert!(
            obj["error"]["message"]
                .as_str()
                .unwrap()
                .contains("weird.foo")
        );
    }

    #[tokio::test]
    async fn test_dispatch_known_method_stub() {
        let ctx = test_ctx();
        let response = dispatch(
            r#"{"jsonrpc":"2.0","method":"agent.spawn","params":{},"id":2}"#,
            &ctx,
        )
        .await;
        let obj: Value = serde_json::from_str(&response).unwrap();

        assert_eq!(obj["id"], 2);
        assert_eq!(obj["error"]["code"], -32000);
        assert_eq!(obj["error"]["data"]["error_code"], "IPC_INVALID_REQUEST");
        assert!(
            obj["error"]["message"]
                .as_str()
                .unwrap()
                .contains("missing or invalid field 'session_id'")
        );
    }

    #[tokio::test]
    async fn test_dispatch_session_kill_method_registered() {
        let ctx = test_ctx();
        let response = dispatch(
            r#"{"jsonrpc":"2.0","method":"session.kill","params":{},"id":6}"#,
            &ctx,
        )
        .await;
        let obj: Value = serde_json::from_str(&response).unwrap();

        assert_eq!(obj["id"], 6);
        assert_eq!(obj["error"]["code"], -32000);
        assert_eq!(obj["error"]["data"]["error_code"], "IPC_INVALID_REQUEST");
        assert!(
            obj["error"]["message"]
                .as_str()
                .unwrap()
                .contains("missing or invalid field 'session_id'")
        );
    }

    #[tokio::test]
    async fn test_dispatch_session_spawn_master_pane_method_registered() {
        let ctx = test_ctx();
        let response = dispatch(
            r#"{"jsonrpc":"2.0","method":"session.spawn_master_pane","params":{},"id":10}"#,
            &ctx,
        )
        .await;
        let obj: Value = serde_json::from_str(&response).unwrap();

        assert_eq!(obj["id"], 10);
        assert_eq!(obj["error"]["code"], -32000);
        assert_eq!(obj["error"]["data"]["error_code"], "IPC_INVALID_REQUEST");
        assert!(
            obj["error"]["message"]
                .as_str()
                .unwrap()
                .contains("missing or invalid field 'session_id'")
        );
    }

    #[tokio::test]
    async fn test_dispatch_job_cancel_method_registered() {
        let ctx = test_ctx();
        let response = dispatch(
            r#"{"jsonrpc":"2.0","method":"job.cancel","params":{},"id":8}"#,
            &ctx,
        )
        .await;
        let obj: Value = serde_json::from_str(&response).unwrap();

        assert_eq!(obj["id"], 8);
        assert_eq!(obj["error"]["code"], -32000);
        assert_eq!(obj["error"]["data"]["error_code"], "IPC_INVALID_REQUEST");
        assert!(
            obj["error"]["message"]
                .as_str()
                .unwrap()
                .contains("missing or invalid field 'job_id'")
        );
    }

    #[tokio::test]
    async fn test_dispatch_session_list_method_registered() {
        let ctx = test_ctx();
        let response = dispatch(r#"{"jsonrpc":"2.0","method":"session.list","id":9}"#, &ctx).await;
        let obj: Value = serde_json::from_str(&response).unwrap();

        assert_eq!(obj["id"], 9);
        assert!(obj["result"]["sessions"].as_array().is_some());
    }

    #[tokio::test]
    async fn test_dispatch_job_submit_method_registered() {
        let ctx = test_ctx();
        let response = dispatch(
            r#"{"jsonrpc":"2.0","method":"job.submit","params":{},"id":3}"#,
            &ctx,
        )
        .await;
        let obj: Value = serde_json::from_str(&response).unwrap();

        assert_eq!(obj["id"], 3);
        assert_eq!(obj["error"]["code"], -32000);
        assert_eq!(obj["error"]["data"]["error_code"], "IPC_INVALID_REQUEST");
        assert!(
            obj["error"]["message"]
                .as_str()
                .unwrap()
                .contains("missing or invalid field 'agent_id'")
        );
    }

    #[tokio::test]
    async fn test_dispatch_job_wait_method_registered() {
        let ctx = test_ctx();
        let response = dispatch(
            r#"{"jsonrpc":"2.0","method":"job.wait","params":{},"id":4}"#,
            &ctx,
        )
        .await;
        let obj: Value = serde_json::from_str(&response).unwrap();

        assert_eq!(obj["id"], 4);
        assert_eq!(obj["error"]["code"], -32000);
        assert_eq!(obj["error"]["data"]["error_code"], "IPC_INVALID_REQUEST");
        assert!(
            obj["error"]["message"]
                .as_str()
                .unwrap()
                .contains("missing or invalid field 'job_id'")
        );
    }

    #[tokio::test]
    async fn test_dispatch_agent_watch_method_registered() {
        let ctx = test_ctx();
        let response = dispatch(
            r#"{"jsonrpc":"2.0","method":"agent.watch","params":{},"id":5}"#,
            &ctx,
        )
        .await;
        let obj: Value = serde_json::from_str(&response).unwrap();

        assert_eq!(obj["id"], 5);
        assert_eq!(obj["error"]["code"], -32000);
        assert_eq!(obj["error"]["data"]["error_code"], "IPC_INVALID_REQUEST");
        assert!(
            obj["error"]["message"]
                .as_str()
                .unwrap()
                .contains("missing or invalid field 'agent_id'")
        );
    }

    #[tokio::test]
    async fn test_dispatch_agent_resolve_prompt_method_registered() {
        let ctx = test_ctx();
        let response = dispatch(
            r#"{"jsonrpc":"2.0","method":"agent.resolve_prompt","params":{},"id":11}"#,
            &ctx,
        )
        .await;
        let obj: Value = serde_json::from_str(&response).unwrap();

        assert_eq!(obj["id"], 11);
        assert_eq!(obj["error"]["code"], -32000);
        assert_eq!(obj["error"]["data"]["error_code"], "IPC_INVALID_REQUEST");
        assert!(
            obj["error"]["message"]
                .as_str()
                .unwrap()
                .contains("missing or invalid field 'agent_id'")
        );
    }

    #[tokio::test]
    async fn test_dispatch_agent_learn_rule_method_registered_for_learned_rules() {
        let ctx = test_ctx();
        let response = dispatch(
            r#"{"jsonrpc":"2.0","method":"agent.learn_rule","params":{"agent_id":"ag1","category":"StartupReadiness","fingerprint":{"type":"regex","pattern":"(?m)^\\s*❯"},"positive_examples":["❯ Try \"fix lint errors\""]},"id":12}"#,
            &ctx,
        )
        .await;
        let obj: Value = serde_json::from_str(&response).unwrap();

        assert_eq!(obj["id"], 12);
        assert_eq!(obj["result"]["status"], "ok");
    }

    #[tokio::test]
    async fn test_dispatch_agent_learn_rule_persists_noisy_positive_rule() {
        let ctx = test_ctx();
        let response = dispatch(
            r#"{"jsonrpc":"2.0","method":"agent.learn_rule","params":{"agent_id":"ag1","provider":"claude","category":"StartupReadiness","fingerprint":{"type":"regex","pattern":"(?m)^\\s*❯"},"positive_examples":["❯ Try \"fix lint errors\"\nOpus 4.8 (1M context)"],"cursor_anchor":{"cursor_row_delta_from_match":0,"cursor_col_delta_from_match_end":1}},"id":13}"#,
            &ctx,
        )
        .await;
        let obj: Value = serde_json::from_str(&response).unwrap();

        assert_eq!(obj["id"], 13);
        assert_eq!(obj["result"]["status"], "ok");

        let found =
            lookup_learned_rules_sync(&ctx.db, "claude", LearnedRuleCategory::StartupReadiness)
                .unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].provider, "claude");
        assert_eq!(
            found[0].positive_examples,
            vec!["❯ Try \"fix lint errors\"\nOpus 4.8 (1M context)".to_string()]
        );
    }

    #[tokio::test]
    async fn test_dispatch_agent_learn_rule_rejects_too_tight_regex_without_persisting() {
        let ctx = test_ctx();
        let response = dispatch(
            r#"{"jsonrpc":"2.0","method":"agent.learn_rule","params":{"agent_id":"ag1","provider":"claude","category":"StartupReadiness","fingerprint":{"type":"regex","pattern":"(?m)^\\s*❯\\s*$"},"positive_examples":["❯ Try \"fix lint errors\""]},"id":14}"#,
            &ctx,
        )
        .await;
        let obj: Value = serde_json::from_str(&response).unwrap();

        assert_eq!(obj["id"], 14);
        assert_eq!(obj["error"]["data"]["error_code"], "IPC_INVALID_REQUEST");
        assert!(
            obj["error"]["message"]
                .as_str()
                .unwrap()
                .contains("positive")
        );
        let found =
            lookup_learned_rules_sync(&ctx.db, "claude", LearnedRuleCategory::StartupReadiness)
                .unwrap();
        assert!(found.is_empty());
    }

    #[tokio::test]
    async fn test_dispatch_agent_learn_rule_rejects_action_for_runtime_marker() {
        let ctx = test_ctx();
        let response = dispatch(
            r#"{"jsonrpc":"2.0","method":"agent.learn_rule","params":{"agent_id":"ag1","provider":"claude","category":"RuntimeMarker","fingerprint":{"type":"regex","pattern":"(?m)^READY"},"positive_examples":["READY"],"action":[{"type":"key","value":"Enter"}]},"id":15}"#,
            &ctx,
        )
        .await;
        let obj: Value = serde_json::from_str(&response).unwrap();

        assert_eq!(obj["id"], 15);
        assert_eq!(obj["error"]["data"]["error_code"], "IPC_INVALID_REQUEST");
        assert!(obj["error"]["message"].as_str().unwrap().contains("action"));
    }

    #[tokio::test]
    async fn test_dispatch_agent_learn_rule_rejects_extraction_for_non_reply_extraction() {
        let ctx = test_ctx();
        let response = dispatch(
            r#"{"jsonrpc":"2.0","method":"agent.learn_rule","params":{"agent_id":"ag1","provider":"claude","category":"StartupReadiness","fingerprint":{"type":"regex","pattern":"(?m)^READY"},"positive_examples":["READY"],"extraction":{"start":{"LastPromptEcho":{"prompt_markers":["> "]}},"end":{"NextRegex":{"pattern":"(?m)^READY"}},"drop_lines":[]}},"id":16}"#,
            &ctx,
        )
        .await;
        let obj: Value = serde_json::from_str(&response).unwrap();

        assert_eq!(obj["id"], 16);
        assert_eq!(obj["error"]["data"]["error_code"], "IPC_INVALID_REQUEST");
        assert!(
            obj["error"]["message"]
                .as_str()
                .unwrap()
                .contains("extraction")
        );
    }
}
