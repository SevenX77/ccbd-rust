//! R2 ACK visual-diff tests.
//! T2.2.1 keeps this at the diff predicate layer; T2.2.2 wires ACK -> BUSY.

use ccbd::pane_diff::is_meaningful_diff;

#[test]
fn capture_seed_ignores_spinner_only_output() {
    assert!(!is_meaningful_diff(
        "Thinking... (1m)\n",
        "⠹ Thinking... (1m 5s)\n"
    ));
}

#[test]
fn capture_seed_detects_real_token_output() {
    assert!(is_meaningful_diff(
        "ready>\n",
        "ready>\nLoading file abc.py...\n"
    ));
}
