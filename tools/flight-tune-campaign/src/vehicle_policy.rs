use std::collections::BTreeSet;

use flight_tune::{
    AdapterError, ExecutionRetryPolicy, PROMOTION_POLICY_SCHEMA_VERSION, PromotionPolicy,
    PromotionSeedPolicy, QualificationPolicy, ResponseTargetTable, TargetAuthorityBand,
};
use pilotage_tuning_feedback::{FeedbackError, RequiredPolicy};

use crate::bench::{BenchVehicle, bench_physical_target, bench_response_targets};

/// The objectives an Alia 250 candidate has to clear, as
/// `(metric, promotion regression limit, final absolute maximum)`.
///
/// The two limits are different quantities. The first bounds how much worse
/// than the frozen baseline a challenger may measure on a paired comparison;
/// the second is an absolute ceiling any single qualifying run must sit under.
/// A candidate can improve against a baseline that was itself poor, which is
/// what the second column is for.
///
/// The ceilings are CALIBRATED, not aspirational: each admits the shipped
/// law's measured value on this bench's trial with margin for the search's
/// neighborhood, and refuses serious degradation. The ignored bench probe
/// prints the measured values when the trial or the models change.
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
    ("hold.zero_crossings", 0.5, 4.0),
    ("hold.rebound_distance_m", 0.03, 0.3),
    ("jerk.peak_acceleration_mps2", 0.6, 6.0),
    ("jerk.peak_jerk_mps3", 3.5, 35.0),
    ("jerk.jerk_p95_mps3", 1.5, 15.0),
    ("jerk.jerk_rms_mps3", 1.0, 10.0),
    ("release.brake_distance_m", 0.25, 2.5),
    ("release.opposite_velocity_peak_mps", 0.05, 0.5),
    ("release.return_toward_release_m", 0.03, 0.3),
    ("release.release_to_stop_s", 0.25, 2.5),
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
    ("hold.zero_crossings", 0.4, 3.0),
    ("hold.rebound_distance_m", 0.02, 0.2),
    ("jerk.peak_acceleration_mps2", 1.6, 16.0),
    ("jerk.peak_jerk_mps3", 12.0, 120.0),
    ("jerk.jerk_p95_mps3", 5.0, 50.0),
    ("jerk.jerk_rms_mps3", 1.6, 16.0),
    ("release.brake_distance_m", 0.15, 1.5),
    ("release.opposite_velocity_peak_mps", 0.04, 0.4),
    ("release.return_toward_release_m", 0.02, 0.2),
    ("release.release_to_stop_s", 0.12, 1.2),
    ("response.input_to_command_delay_s", 0.01, 0.1),
    ("response.input_to_response_delay_s", 0.025, 0.25),
    ("response.overshoot_fraction", 0.03, 0.3),
    ("response.rise_time_s", 0.09, 0.9),
    ("response.settling_time_s", 0.2, 2.0),
];

/// The share of the requested physical target an operator input must keep.
///
/// A candidate cannot improve a normalized response metric by asking the
/// vehicle for less: a larger expo lowers the physical target for the same
/// stick, so the vehicle reaches it sooner and overshoots it less, and every
/// normalized measurement reads that as a better command law. This fraction is
/// the authority the operator keeps, and it is checked against the target the
/// candidate resolved on the run rather than the one the scenario requested.
///
/// The value admits the whole shaped neighborhood the search may reach and
/// refuses the corner of the allowlist where the curve gives most of the stick
/// away.
const MINIMUM_OPERATOR_AUTHORITY: f64 = 0.86;

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

/// Returns the exact scoped response limits for an Alia 250 bench campaign.
///
/// # Errors
///
/// Returns [`AdapterError`] when a scenario identity or the table is not
/// valid.
pub fn alia250_response_targets() -> Result<ResponseTargetTable, AdapterError> {
    response_targets(BenchVehicle::alia250(), &ALIA250_OBJECTIVE_LIMITS)
}

/// Returns the exact scoped response limits for an x500 bench campaign.
///
/// # Errors
///
/// Returns [`AdapterError`] when a scenario identity or the table is not
/// valid.
pub fn x500_response_targets() -> Result<ResponseTargetTable, AdapterError> {
    response_targets(BenchVehicle::x500(), &X500_OBJECTIVE_LIMITS)
}

/// One scoped table over a vehicle's own scenarios and limits.
fn response_targets(
    model: BenchVehicle,
    limits: &[(&str, f64, f64)],
) -> Result<ResponseTargetTable, AdapterError> {
    let promotion = limits
        .iter()
        .map(|(name, regression, _)| (*name, *regression))
        .collect::<Vec<_>>();
    let qualification = limits
        .iter()
        .map(|(name, _, maximum)| (*name, *maximum))
        .collect::<Vec<_>>();
    let target = bench_physical_target(model);
    bench_response_targets(
        model,
        &promotion,
        &qualification,
        TargetAuthorityBand {
            minimum: target * MINIMUM_OPERATOR_AUTHORITY,
            maximum: target,
        },
    )
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
    RequiredPolicy::new(
        &alia250_promotion_policy(),
        &alia250_qualification_policy(),
        &execution_retry_policy(),
        &alia250_response_targets().map_err(|error| FeedbackError::Invalid {
            detail: error.to_string(),
        })?,
    )
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
    RequiredPolicy::new(
        &x500_promotion_policy(),
        &x500_qualification_policy(),
        &execution_retry_policy(),
        &x500_response_targets().map_err(|error| FeedbackError::Invalid {
            detail: error.to_string(),
        })?,
    )
}

/// The execution retry limit every shipped vehicle campaign runs against.
///
/// A replacement execution is an execution the operator discarded. Neither
/// vehicle bar admits one, so a campaign that authorized replacements does
/// not clear the bar this consumer states.
fn execution_retry_policy() -> ExecutionRetryPolicy {
    ExecutionRetryPolicy::none()
}

/// One paired promotion policy over a vehicle's declared objectives.
///
/// The policy names the objectives; the scoped table states each limit. A
/// single number here would bound every scenario the same way, which is what
/// lets a limit written for one physical response decide another.
fn promotion_policy(limits: &[(&str, f64, f64)]) -> PromotionPolicy {
    PromotionPolicy {
        schema_version: PROMOTION_POLICY_SCHEMA_VERSION,
        seed_policy: PromotionSeedPolicy::PairedScenarioDigestV1,
        minimum_loss_improvement: 0.0,
        minimum_relative_loss_improvement: 0.2,
        maximum_control_effort_increase: 0.05,
        objectives: objective_names(limits),
    }
}

/// One final qualification policy over a vehicle's declared objectives.
///
/// The three scalar ceilings bound the campaign-wide loss distribution rather
/// than any one response, so they stay here. Every per-objective maximum is a
/// scoped row.
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
        objectives: objective_names(limits),
    }
}

fn objective_names(limits: &[(&str, f64, f64)]) -> BTreeSet<String> {
    limits
        .iter()
        .map(|(name, _, _)| (*name).to_owned())
        .collect()
}

#[cfg(test)]
mod tests;
