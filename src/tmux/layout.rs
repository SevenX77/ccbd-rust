use crate::error::CcbdError;
use crate::tmux::{TmuxPaneId, TmuxServer};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutKind {
    Single,
    Stack,
    Grid,
}

impl LayoutKind {
    pub fn parse(value: &str) -> Result<Self, CcbdError> {
        match value {
            "single" => Ok(Self::Single),
            "stack" => Ok(Self::Stack),
            "grid" => Ok(Self::Grid),
            other => Err(CcbdError::IpcInvalidRequest(format!(
                "invalid layout: {other}"
            ))),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Single => "single",
            Self::Stack => "stack",
            Self::Grid => "grid",
        }
    }

    fn tmux_layout(self) -> Option<&'static str> {
        match self {
            Self::Single => None,
            Self::Stack => Some("even-vertical"),
            Self::Grid => Some("tiled"),
        }
    }
}

pub async fn apply_layout(
    server: &TmuxServer,
    window_target: String,
    mode: LayoutKind,
) -> Result<(), CcbdError> {
    let Some(tmux_layout) = mode.tmux_layout() else {
        return Ok(());
    };
    server
        .select_layout(window_target, tmux_layout.to_string())
        .await
}

pub async fn list_panes(
    server: &TmuxServer,
    window_target: String,
) -> Result<Vec<TmuxPaneId>, CcbdError> {
    server.list_panes(window_target).await
}

#[cfg(test)]
mod tests {
    use super::LayoutKind;

    #[test]
    fn test_layout_kind_parse() {
        assert_eq!(LayoutKind::parse("single").unwrap(), LayoutKind::Single);
        assert_eq!(LayoutKind::parse("stack").unwrap(), LayoutKind::Stack);
        assert_eq!(LayoutKind::parse("grid").unwrap(), LayoutKind::Grid);
        assert!(LayoutKind::parse("weird").is_err());
    }
}
