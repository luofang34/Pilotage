#![allow(clippy::expect_used)]

use super::{GateOutcome, OnlineStats, paired_stats, student_t_95, validate_gate_outcomes};

#[test]
fn critical_values_do_not_use_the_end_of_a_degree_range() {
    assert_eq!(student_t_95(11), 2.201);
    assert_eq!(student_t_95(31), 2.042);
    assert_eq!(student_t_95(61), 2.000);
}

#[test]
fn statistics_reject_non_finite_inputs() {
    assert!(OnlineStats::from_values([1.0, f64::NAN].into_iter()).is_err());
    assert!(paired_stats([0.0, f64::INFINITY].into_iter()).is_err());
}

#[test]
fn stable_variance_stays_finite_for_a_large_offset() {
    let stats = OnlineStats::from_values([1.0e12, 1.0e12 + 1.0, 1.0e12 + 2.0].into_iter())
        .expect("calculate statistics");

    assert_eq!(stats.sample_variance().expect("calculate variance"), 1.0);
}

#[test]
fn an_ordered_failure_can_end_gate_evaluation_early() {
    let required = vec!["crash".to_owned(), "finite".to_owned()];
    let failure = GateOutcome::fail("crash", "crash detected");

    assert_eq!(
        validate_gate_outcomes(&required, std::slice::from_ref(&failure))
            .expect("validate fail-fast result"),
        Some(failure)
    );
    assert!(validate_gate_outcomes(&required, &[GateOutcome::pass("crash")]).is_err());
    assert!(
        validate_gate_outcomes(
            &required,
            &[
                GateOutcome::fail("crash", "crash detected"),
                GateOutcome::pass("finite"),
            ],
        )
        .is_err()
    );
}
