//! R2 WAITING_FOR_ACK 集成测试.
//! T2.1.1: 常量 + helper 的最小 sanity. 后续 task (T2.1.2 / T2.2.x) 会扩展真实 ACK e2e flow.

use ccbd::db::state_machine::{
    STATE_BUSY, STATE_IDLE, STATE_WAITING_FOR_ACK, is_active_state, is_waiting_for_ack,
};

#[test]
fn waiting_for_ack_constant_visible_at_crate_boundary() {
    assert_eq!(STATE_WAITING_FOR_ACK, "WAITING_FOR_ACK");
    assert!(is_active_state(STATE_WAITING_FOR_ACK));
    assert!(is_waiting_for_ack(STATE_WAITING_FOR_ACK));
    assert!(!is_waiting_for_ack(STATE_IDLE));
    assert!(!is_waiting_for_ack(STATE_BUSY));
}
