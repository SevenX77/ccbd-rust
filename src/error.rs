use thiserror::Error;

#[derive(Error, Debug)]
pub enum CcbdError {
    #[error("database constraint violation: {0}")]
    DbConstraintViolation(String),

    #[error("agent not found: {0}")]
    AgentNotFound(String),

    #[error("agent already exists: {0}")]
    AgentAlreadyExists(String),

    #[error("pty open failed: {0}")]
    PtyOpenFailed(String),

    #[error("pty I/O error: {0}")]
    PtyIoError(String),

    #[error("invalid IPC request: {0}")]
    IpcInvalidRequest(String),

    #[error("duplicate request for existing event seq_id={existing_seq_id}")]
    DuplicateRequest { existing_seq_id: i64 },
}

impl CcbdError {
    pub fn to_rpc_error(&self) -> serde_json::Value {
        let error_code = match self {
            Self::DbConstraintViolation(_) => "DB_CONSTRAINT_VIOLATION",
            Self::AgentNotFound(_) => "AGENT_NOT_FOUND",
            Self::AgentAlreadyExists(_) => "AGENT_ALREADY_EXISTS",
            Self::PtyOpenFailed(_) => "PTY_OPEN_FAILED",
            Self::PtyIoError(_) => "PTY_IO_ERROR",
            Self::IpcInvalidRequest(_) => "IPC_INVALID_REQUEST",
            Self::DuplicateRequest { .. } => {
                unreachable!("DuplicateRequest is an internal DB sentinel, not an RPC error")
            }
        };

        serde_json::json!({
            "code": -32000,
            "message": self.to_string(),
            "data": {
                "error_code": error_code,
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::CcbdError;

    fn assert_rpc_error(error: CcbdError, expected_code: &str, expected_message: &str) {
        let obj = error.to_rpc_error();

        assert_eq!(obj["code"], -32000);
        assert_eq!(obj["data"]["error_code"], expected_code);
        assert!(obj["message"].as_str().unwrap().contains(expected_message));
    }

    #[test]
    fn test_agent_not_found_to_rpc_error() {
        let obj = CcbdError::AgentNotFound("ag_1".into()).to_rpc_error();

        assert_eq!(obj["code"], -32000);
        assert_eq!(obj["data"]["error_code"], "AGENT_NOT_FOUND");
        assert!(obj["message"].as_str().unwrap().contains("ag_1"));
    }

    #[test]
    fn test_ipc_invalid_request_to_rpc_error() {
        let obj = CcbdError::IpcInvalidRequest("missing field 'method'".into()).to_rpc_error();

        assert_eq!(obj["code"], -32000);
        assert_eq!(obj["data"]["error_code"], "IPC_INVALID_REQUEST");
        assert!(
            obj["message"]
                .as_str()
                .unwrap()
                .contains("missing field 'method'")
        );
    }

    #[test]
    fn test_db_constraint_violation_round_trip() {
        assert_rpc_error(
            CcbdError::DbConstraintViolation("foreign key failed".into()),
            "DB_CONSTRAINT_VIOLATION",
            "foreign key failed",
        );
    }

    #[test]
    fn test_agent_not_found_round_trip() {
        assert_rpc_error(
            CcbdError::AgentNotFound("ag_missing".into()),
            "AGENT_NOT_FOUND",
            "ag_missing",
        );
    }

    #[test]
    fn test_agent_already_exists_round_trip() {
        assert_rpc_error(
            CcbdError::AgentAlreadyExists("ag_1".into()),
            "AGENT_ALREADY_EXISTS",
            "ag_1",
        );
    }

    #[test]
    fn test_pty_open_failed_round_trip() {
        assert_rpc_error(
            CcbdError::PtyOpenFailed("openpty failed".into()),
            "PTY_OPEN_FAILED",
            "openpty failed",
        );
    }

    #[test]
    fn test_pty_io_error_round_trip() {
        assert_rpc_error(
            CcbdError::PtyIoError("broken pipe".into()),
            "PTY_IO_ERROR",
            "broken pipe",
        );
    }

    #[test]
    fn test_ipc_invalid_request_round_trip() {
        assert_rpc_error(
            CcbdError::IpcInvalidRequest("bad json".into()),
            "IPC_INVALID_REQUEST",
            "bad json",
        );
    }
}
