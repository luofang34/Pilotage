use std::collections::BTreeMap;

use flight_tune::{
    PROMOTION_POLICY_SCHEMA_VERSION, PromotionPolicy, PromotionSeedPolicy, QualificationPolicy,
};
use pilotage_tuning_feedback::{FeedbackError, RequiredPolicy};

/// The objectives an Alia 250 candidate has to clear, as
/// `(metric, promotion regression limit, final absolute maximum)`.
///
/// The two limits are different quantities. The first bounds how much worse
/// than the frozen baseline a challenger may measure on a paired comparison;
/// the second is an absolute ceiling any single qualifying run must sit under.
/// A candidate can improve against a baseline that was itself poor, which is
/// what the second column is for.
///
/// Every name here is one [`pilotage_flight_quality::is_producible`] knows.
/// Final qualification requires each named objective to be present in every
/// run, so a bar naming a metric nothing measures fails the campaign on the
/// name rather than on the vehicle — the whole campaign runs and proves
/// nothing about the aircraft.
const ALIA250_OBJECTIVE_LIMITS: [(&str, f64, f64); 18] = [
    ("control.effort_rms", 0.075, 0.75),
    ("control.longest_saturation_s", 0.15, 1.5),
    ("control.saturation_fraction", 0.005, 0.05),
    ("hold.position_error_m", 0.05, 0.5),
    ("hold.rebound_distance_m", 0.03, 0.3),
    ("jerk.peak_acceleration_mps2", 0.6, 6.0),
    ("jerk.peak_jerk_mps3", 2.5, 25.0),
    ("jerk.jerk_p95_mps3", 1.5, 15.0),
    ("jerk.jerk_rms_mps3", 1.0, 10.0),
    ("release.brake_distance_m", 0.25, 2.5),
    ("release.opposite_velocity_peak_mps", 0.05, 0.5),
    ("release.return_toward_release_m", 0.03, 0.3),
    ("release.release_to_stop_s", 0.15, 1.5),
    ("response.input_to_command_delay_s", 0.01, 0.1),
    ("response.input_to_response_delay_s", 0.035, 0.35),
    ("response.overshoot_fraction", 0.03, 0.3),
    ("response.rise_time_s", 0.15, 1.5),
    ("response.settling_time_s", 0.3, 3.0),
];

/// The objectives an x500 candidate has to clear.
///
/// The same metrics as the Alia, at limits a small quadrotor is held to: it is
/// lighter and quicker, so it rises and settles sooner and is allowed less
/// absolute overshoot, while its jerk ceilings are higher because there is far
/// less mass behind the same acceleration.
///
/// The point of a second vehicle is that it contributes limits and nothing
/// else. Sharing the metric names is what makes the harness reusable; a
/// vehicle that needed its own metric would be a vehicle the scoring layer had
/// to learn about.
const X500_OBJECTIVE_LIMITS: [(&str, f64, f64); 18] = [
    ("control.effort_rms", 0.08, 0.8),
    ("control.longest_saturation_s", 0.1, 1.0),
    ("control.saturation_fraction", 0.005, 0.05),
    ("hold.position_error_m", 0.03, 0.3),
    ("hold.rebound_distance_m", 0.02, 0.2),
    ("jerk.peak_acceleration_mps2", 1.0, 10.0),
    ("jerk.peak_jerk_mps3", 4.0, 40.0),
    ("jerk.jerk_p95_mps3", 2.5, 25.0),
    ("jerk.jerk_rms_mps3", 1.6, 16.0),
    ("release.brake_distance_m", 0.15, 1.5),
    ("release.opposite_velocity_peak_mps", 0.04, 0.4),
    ("release.return_toward_release_m", 0.02, 0.2),
    ("release.release_to_stop_s", 0.09, 0.9),
    ("response.input_to_command_delay_s", 0.01, 0.1),
    ("response.input_to_response_delay_s", 0.025, 0.25),
    ("response.overshoot_fraction", 0.03, 0.3),
    ("response.rise_time_s", 0.09, 0.9),
    ("response.settling_time_s", 0.2, 2.0),
];

/// Returns the default Alia 250 paired promotion policy.
#[must_use]
pub fn alia250_promotion_policy() -> PromotionPolicy {
    promotion_policy(&ALIA250_OBJECTIVE_LIMITS)
}

/// Returns the default Alia 250 final qualification policy.
#[must_use]
pub fn alia250_qualification_policy() -> QualificationPolicy {
    qualification_policy(&ALIA250_OBJECTIVE_LIMITS, 4.0, 4.0, 0.75)
}

/// Returns the bar an Alia 250 campaign must have run against.
///
/// A verifier reads the policy out of the evidence it is checking, so it can
/// only attest that a campaign is self-consistent under whatever bar its
/// operator wrote. This is the bar itself, stated separately, so a consumer
/// deciding whether a calibration may ship names which one it requires.
///
/// # Errors
///
/// Returns [`FeedbackError`] when a policy cannot be encoded.
pub fn alia250_required_policy() -> Result<RequiredPolicy, FeedbackError> {
    RequiredPolicy::new(&alia250_promotion_policy(), &alia250_qualification_policy())
}

/// Returns the default x500 paired promotion policy.
#[must_use]
pub fn x500_promotion_policy() -> PromotionPolicy {
    promotion_policy(&X500_OBJECTIVE_LIMITS)
}

/// Returns the default x500 final qualification policy.
#[must_use]
pub fn x500_qualification_policy() -> QualificationPolicy {
    qualification_policy(&X500_OBJECTIVE_LIMITS, 3.0, 3.0, 0.8)
}

/// Returns the bar an x500 campaign must have run against.
///
/// # Errors
///
/// Returns [`FeedbackError`] when a policy cannot be encoded.
pub fn x500_required_policy() -> Result<RequiredPolicy, FeedbackError> {
    RequiredPolicy::new(&x500_promotion_policy(), &x500_qualification_policy())
}

/// One paired promotion policy over a vehicle's objective limits.
fn promotion_policy(limits: &[(&str, f64, f64)]) -> PromotionPolicy {
    PromotionPolicy {
        schema_version: PROMOTION_POLICY_SCHEMA_VERSION,
        seed_policy: PromotionSeedPolicy::PairedScenarioDigestV1,
        minimum_loss_improvement: 0.0,
        minimum_relative_loss_improvement: 0.2,
        maximum_control_effort_increase: 0.05,
        objective_regression_upper_95: limits
            .iter()
            .map(|(name, regression, _)| ((*name).to_owned(), *regression))
            .collect(),
    }
}

/// One final qualification policy over a vehicle's objective limits.
fn qualification_policy(
    limits: &[(&str, f64, f64)],
    maximum_loss_confidence_upper: f64,
    maximum_p95_loss: f64,
    maximum_mean_control_effort: f64,
) -> QualificationPolicy {
    QualificationPolicy {
        maximum_loss_confidence_upper,
        maximum_p95_loss,
        maximum_mean_control_effort,
        objective_maxima: limits
            .iter()
            .map(|(name, _, maximum)| ((*name).to_owned(), *maximum))
            .collect::<BTreeMap<_, _>>(),
    }
}

#[cfg(test)]
mod tests;
