use crate::error::CcbdError;
use crate::rpc::Ctx;
use crate::rpc::handlers::{
    handle_agent_assert_state, handle_agent_discard_evidence, handle_agent_kill, handle_agent_read,
    handle_agent_send, handle_agent_spawn, handle_agent_watch, handle_job_submit, handle_job_wait,
    handle_session_create, handle_session_kill, handle_system_dump,
};
use serde_json::{Value, json};

const METHODS: &[&str] = &[
    "session.create",
    "session.kill",
    "agent.spawn",
    "agent.send",
    "agent.read",
    "agent.watch",
    "agent.kill",
    "agent.assert_state",
    "agent.discard_evidence",
    "job.submit",
    "job.wait",
    "system.dump",
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
        "agent.spawn" => handle_agent_spawn(params, ctx).await,
        "agent.send" => handle_agent_send(params, ctx).await,
        "agent.read" => handle_agent_read(params, ctx).await,
        "agent.watch" => handle_agent_watch(params, ctx).await,
        "agent.kill" => handle_agent_kill(params, ctx).await,
        "agent.assert_state" => handle_agent_assert_state(params, ctx).await,
        "agent.discard_evidence" => handle_agent_discard_evidence(params, ctx).await,
        "job.submit" => handle_job_submit(params, ctx).await,
        "job.wait" => handle_job_wait(params, ctx).await,
        "system.dump" => handle_system_dump(params, ctx).await,
        _ => unreachable!("method whitelist checked above"),
    };

    match result {
        Ok(value) => success_response(id, value),
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
                bwrap_available: false,
                systemd_run_available: false,
                unsafe_no_sandbox: true,
            },
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
}
