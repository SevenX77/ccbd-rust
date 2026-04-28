use crate::error::CcbdError;
use crate::rpc::Ctx;
use crate::rpc::handlers::{
    handle_agent_kill, handle_agent_read, handle_agent_send, handle_agent_spawn,
    handle_session_create,
};
use serde_json::{Value, json};

const METHODS: &[&str] = &[
    "session.create",
    "agent.spawn",
    "agent.send",
    "agent.read",
    "agent.kill",
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
        "agent.spawn" => handle_agent_spawn(params, ctx).await,
        "agent.send" => handle_agent_send(params, ctx).await,
        "agent.read" => handle_agent_read(params, ctx).await,
        "agent.kill" => handle_agent_kill(params, ctx).await,
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
    use serde_json::Value;

    fn test_ctx() -> Ctx {
        let file = tempfile::NamedTempFile::new().unwrap();
        let state_dir = tempfile::TempDir::new().unwrap().keep();
        Ctx {
            db: db::init(file.path()).unwrap(),
            state_dir,
            env_state: EnvState {
                bwrap_available: false,
                systemd_run_available: false,
                unsafe_no_sandbox: true,
            },
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
}
