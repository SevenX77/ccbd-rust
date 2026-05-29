#![allow(dead_code)]

// PR-3 Grand Tour: cargo test --test ah_full_e2e_realign_extra -- --include-ignored --test-threads=1

mod common;

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn grand_tour_realign_extra_matrix() {
    panic!("red: PR-3 ORPHAN + BUSY + ERROR matrix not implemented");
}

// TODO PR-3 T4: case_06_orphan_audit_only
// TODO PR-3 T5: case_07_orphan_force_cleanup
// TODO PR-3 T6: case_08_busy_skip
// TODO PR-3 T7: case_09_busy_force_realign
// TODO PR-3 T8: case_10_error_crash_detection
// TODO PR-3 T9: case_11_error_recovery_known_gap
