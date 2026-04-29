use crate::tmux::TmuxError;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct TmuxPaneId(pub String);

impl TmuxPaneId {
    pub fn parse(value: &str) -> Result<Self, TmuxError> {
        if value.starts_with('%') {
            Ok(Self(value.to_string()))
        } else {
            Err(TmuxError::ParsePaneId(value.to_string()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::TmuxPaneId;

    #[test]
    fn test_pane_id_parse_ok() {
        assert_eq!(TmuxPaneId::parse("%1").unwrap(), TmuxPaneId("%1".into()));
    }

    #[test]
    fn test_pane_id_parse_err() {
        assert!(TmuxPaneId::parse("abc").is_err());
    }
}
