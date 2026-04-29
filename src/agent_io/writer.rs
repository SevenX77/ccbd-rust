use crate::error::CcbdError;
use crate::tmux::{TmuxPaneId, TmuxServer};
use std::sync::Arc;

pub async fn send_text_to_pane(
    tmux: Arc<TmuxServer>,
    agent_id: &str,
    pane: TmuxPaneId,
    text: String,
) -> Result<(), CcbdError> {
    let buffer_name = format!("ccbd-buf-{}", sanitize_buffer_name(agent_id));
    let should_submit = text.ends_with('\n');
    tmux.load_buffer(buffer_name.clone(), text).await?;
    let paste_result = tmux.paste_buffer(pane.clone(), buffer_name.clone()).await;

    if let Err(err) = tmux.delete_buffer(buffer_name).await {
        tracing::warn!(agent_id, error = %err, "failed to delete tmux paste buffer");
    }

    paste_result?;
    if should_submit {
        tmux.send_enter(pane).await?;
    }
    Ok(())
}

fn sanitize_buffer_name(agent_id: &str) -> String {
    agent_id
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::sanitize_buffer_name;

    #[test]
    fn test_sanitize_buffer_name_preserves_safe_agent_id_chars() {
        assert_eq!(sanitize_buffer_name("ag_foo-123"), "ag_foo-123");
    }

    #[test]
    fn test_sanitize_buffer_name_replaces_tmux_unsafe_chars() {
        assert_eq!(sanitize_buffer_name("ag/foo:bar"), "ag_foo_bar");
    }
}
