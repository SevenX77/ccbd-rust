use ccbd::provider::init_probe::{
    BashInitProbe, ClaudeInitProbe, CodexInitProbe, GeminiInitProbe, InitGateProbe,
};

#[test]
fn codex_probe_matches_visible_ready_state() {
    let probe = CodexInitProbe;

    assert!(!probe.detect("Trust this workspace\n  › ready"));
    assert!(!probe.detect("work\n  › old\n1\n2\n3\n4\n5\n6\nstill loading"));
    assert!(probe.detect("approval mode active\n  › ask anything"));
}

#[test]
fn codex_probe_accepts_v0125_footer_after_prompt() {
    let probe = CodexInitProbe;

    assert!(probe.detect(
        "╭──────────────────────────────────────────────╮\n\
         │ >_ OpenAI Codex (v0.125.0)                   │\n\
         │                                              │\n\
         │ model:       gpt-5.5   /model to change      │\n\
         │ directory:   ~/coding/ccbd-rust/.ah/ccbd │\n\
         │ permissions: YOLO mode                       │\n\
         ╰──────────────────────────────────────────────╯\n\
           Tip: New Use /fast to enable our fastest inference with increased plan usage.\n\
         › Summarize recent commits\n\
           gpt-5.5 default · ~/coding/ccbd-rust/.ah/ccbd"
    ));
}

#[test]
fn gemini_probe_matches_visible_ready_state() {
    let probe = GeminiInitProbe;

    assert!(!probe.detect("Choose an authentication method\n> "));
    assert!(probe.detect("workspace trusted\n> "));
    assert!(probe.detect("workspace trusted\n  ✦ "));
}

#[test]
fn gemini_probe_accepts_v040_status_bar_after_prompt() {
    let probe = GeminiInitProbe;

    assert!(probe.detect(
        "? for shortcuts\n\
         ─────────────────────────────────────────────────────────────────────────────────────────────────────\n\
          Shift+Tab to accept edits                                                         1 GEMINI.md file\n\
         ▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄\n\
         >   Type your message or @path/to/file\n\
         ▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀\n\
          workspace (/directory)        branch    sandbox    /model       quota memory\n\
          ~/.../.ah/ccbd            main      no sandbox gemini-3.1   0%    232.1 MB"
    ));
}

#[test]
fn claude_probe_requires_three_signals() {
    let probe = ClaudeInitProbe;

    assert!(!probe.detect("Trust the files in this folder\nSonnet\n❯ "));
    assert!(!probe.detect("Sonnet\nno prompt"));
    assert!(!probe.detect("❯ "));
    assert!(probe.detect("model: Sonnet\n────────\n❯ "));
}

#[test]
fn bash_probe_accepts_shell_prompt() {
    let probe = BashInitProbe;

    assert!(probe.detect("ready\n$ "));
    assert!(probe.detect("ready\nroot# "));
    assert!(!probe.detect("ready\nplain output"));
}
