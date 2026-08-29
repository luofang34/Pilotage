#![allow(clippy::expect_used, clippy::panic)]

use crate::ExecutionRetryPolicy;

use super::super::training_suite::tests::stage_for_budget;

#[test]
fn a_prepared_campaign_states_a_finite_bound() {
    let stage = stage_for_budget();
    let bound = stage.run_bound(4).expect("a bounded campaign");

    // Nine training attempts: one starting baseline, then one fresh suite
    // baseline and one challenger for each of four proposals. The widest
    // suite runs four times. Promotion runs one pair and final runs once.
    assert_eq!(bound.maximum_runs, 9 * 4 + 2 * 2 + 2);
    assert!(bound.maximum_duration_ns > 0);
}

#[test]
fn a_replacement_allowance_multiplies_the_bound() {
    let mut stage = stage_for_budget();
    let first = stage.run_bound(4).expect("a bounded campaign");
    stage.execution_retry = ExecutionRetryPolicy::with_limit(2).expect("a supported limit");
    let second = stage.run_bound(4).expect("a bounded campaign with retries");

    assert_eq!(second.maximum_runs, first.maximum_runs * 3);
    assert_eq!(second.maximum_duration_ns, first.maximum_duration_ns * 3);
}

#[test]
fn an_invalid_stage_states_no_bound() {
    let mut stage = stage_for_budget();
    stage.training_suites.clear();

    assert!(stage.run_bound(1).is_err());
}
