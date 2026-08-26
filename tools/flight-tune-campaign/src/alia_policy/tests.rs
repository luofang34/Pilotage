#![allow(clippy::expect_used, clippy::panic)]

use std::collections::BTreeSet;

use super::{alia250_promotion_policy, alia250_qualification_policy};

const FLIGHT_QUALITY_OBJECTIVES: [&str; 22] = [
    "control.effort_rms",
    "control.longest_saturation_s",
    "control.saturation_fraction",
    "hold.rebound_distance_m",
    "hold.zero_crossings",
    "jerk.peak_acceleration_mps2",
    "jerk.peak_mps3",
    "jerk.p95_mps3",
    "jerk.rms_mps3",
    "release.brake_distance_m",
    "release.opposite_velocity_peak_mps",
    "release.return_toward_release_m",
    "release.stop_time_s",
    "step.command_delay_s",
    "step.overshoot_fraction",
    "step.response_delay_s",
    "step.rise_time_s",
    "step.settling_time_s",
    "step.undershoot",
    "wind.position_peak_m",
    "wind.position_p95_m",
    "wind.position_rms_m",
];

#[test]
fn alia_policies_cover_each_current_flight_quality_objective() {
    let promotion = alia250_promotion_policy();
    let qualification = alia250_qualification_policy();
    let expected = FLIGHT_QUALITY_OBJECTIVES
        .into_iter()
        .collect::<BTreeSet<_>>();
    let promotion_keys = promotion
        .objective_regression_upper_95
        .keys()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let qualification_keys = qualification
        .objective_maxima
        .keys()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();

    assert_eq!(promotion_keys, expected);
    assert_eq!(qualification_keys, expected);
}

#[test]
fn alia_policy_limits_are_finite_and_nonnegative() {
    let promotion = alia250_promotion_policy();
    let qualification = alia250_qualification_policy();

    promotion.validate().expect("validate promotion policy");
    assert!(
        promotion
            .objective_regression_upper_95
            .keys()
            .eq(qualification.objective_maxima.keys())
    );
    assert!(qualification.maximum_loss_confidence_upper.is_finite());
    assert!(qualification.maximum_loss_confidence_upper >= 0.0);
    assert!(qualification.maximum_p95_loss.is_finite());
    assert!(qualification.maximum_p95_loss >= 0.0);
    assert!(qualification.maximum_mean_control_effort.is_finite());
    assert!(qualification.maximum_mean_control_effort >= 0.0);
    assert!(
        qualification
            .objective_maxima
            .values()
            .all(|limit| limit.is_finite() && *limit >= 0.0)
    );
}
