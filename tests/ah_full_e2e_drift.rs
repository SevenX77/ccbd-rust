// PR-2 Grand Tour: cargo test --test ah_full_e2e_drift -- --include-ignored --test-threads=1

mod common;

// TODO PR-2 T4: case_01_env_drift
// TODO PR-2 T5: case_02_hooks_drift
// TODO PR-2 T6: case_03_plugins_drift
// TODO PR-2 T7: case_04_no_change
// TODO PR-2 T8: case_05_new_agent

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn grand_tour_drift_new_matrix() {
    panic!("red: PR-2 DRIFT + NEW matrix not implemented");
}
