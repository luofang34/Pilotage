use std::collections::BTreeMap;

use flight_tune::{
    PROMOTION_POLICY_SCHEMA_VERSION, PromotionPolicy, PromotionSeedPolicy, QualificationPolicy,
};
use pilotage_tuning_feedback::{FeedbackError, RequiredPolicy};

const ALIA250_OBJECTIVE_LIMITS: [(&str, f64, f64); 22] = [
    ("control.effort_rms", 0.075, 0.75),
    ("control.longest_saturation_s", 0.15, 1.5),
    ("control.saturation_fraction", 0.005, 0.05),
    ("hold.rebound_distance_m", 0.03, 0.3),
    ("hold.zero_crossings", 0.2, 2.0),
    ("jerk.peak_acceleration_mps2", 0.6, 6.0),
    ("jerk.peak_mps3", 2.5, 25.0),
    ("jerk.p95_mps3", 1.5, 15.0),
    ("jerk.rms_mps3", 1.0, 10.0),
    ("release.brake_distance_m", 0.25, 2.5),
    ("release.opposite_velocity_peak_mps", 0.05, 0.5),
    ("release.return_toward_release_m", 0.03, 0.3),
    ("release.stop_time_s", 0.15, 1.5),
    ("step.command_delay_s", 0.01, 0.1),
    ("step.overshoot_fraction", 0.02, 0.3),
    ("step.response_delay_s", 0.035, 0.35),
    ("step.rise_time_s", 0.15, 1.5),
    ("step.settling_time_s", 0.3, 3.0),
    ("step.undershoot", 0.03, 0.3),
    ("wind.position_peak_m", 0.4, 4.0),
    ("wind.position_p95_m", 0.25, 2.5),
    ("wind.position_rms_m", 0.2, 2.0),
];

/// Returns the default Alia 250 paired promotion policy.
#[must_use]
pub fn alia250_promotion_policy() -> PromotionPolicy {
    PromotionPolicy {
        schema_version: PROMOTION_POLICY_SCHEMA_VERSION,
        seed_policy: PromotionSeedPolicy::PairedScenarioDigestV1,
        minimum_loss_improvement: 0.0,
        minimum_relative_loss_improvement: 0.2,
        maximum_control_effort_increase: 0.05,
        objective_regression_upper_95: ALIA250_OBJECTIVE_LIMITS
            .iter()
            .map(|(name, maximum, _)| ((*name).to_owned(), *maximum))
            .collect(),
    }
}

/// Returns the default Alia 250 final qualification policy.
#[must_use]
pub fn alia250_qualification_policy() -> QualificationPolicy {
    QualificationPolicy {
        maximum_loss_confidence_upper: 4.0,
        maximum_p95_loss: 4.0,
        maximum_mean_control_effort: 0.75,
        objective_maxima: ALIA250_OBJECTIVE_LIMITS
            .iter()
            .map(|(name, _, maximum)| ((*name).to_owned(), *maximum))
            .collect::<BTreeMap<_, _>>(),
    }
}

#[cfg(test)]
#[path = "alia_policy/tests.rs"]
mod tests;

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
