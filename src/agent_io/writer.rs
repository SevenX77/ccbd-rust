use crate::error::CcbdError;
use crate::tmux::{TmuxPaneId, TmuxServer};
use std::sync::Arc;

pub async fn send_text_to_pane(
    tmux: Arc<TmuxServer>,
    pane: TmuxPaneId,
    text: String,
) -> Result<(), CcbdError> {
    let parts = text.split('\n').collect::<Vec<_>>();

    for (idx, segment) in parts.iter().enumerate() {
        if !segment.is_empty() {
            tmux.send_keys_literal(pane.clone(), (*segment).to_string())
                .await?;
        }
        if idx < parts.len() - 1 {
            tmux.send_keys_keysym(pane.clone(), "Enter".into()).await?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_split_keeps_trailing_empty_segment() {
        let parts = "echo one\necho two\n".split('\n').collect::<Vec<_>>();

        assert_eq!(parts, vec!["echo one", "echo two", ""]);
    }
}
