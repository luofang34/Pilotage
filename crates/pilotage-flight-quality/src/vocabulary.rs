//! Every metric name this crate can produce.
//!
//! A tuning policy names the objectives a candidate has to clear. Final
//! qualification requires each named objective to be present in every run, so
//! a policy naming something no measurement produces cannot qualify anything:
//! the campaign runs to the end and fails on a name rather than on a vehicle.
//!
//! Stating the vocabulary here lets a policy be checked against it before a
//! campaign is run instead of after, and gives a new vehicle's policy the same
//! check for free.

/// Metric names produced by [`crate::measure_control`].
pub const CONTROL_METRICS: [&str; 4] = [
    "control.effort_rms",
    "control.integrated_abs_effort_s",
    "control.saturation_fraction",
    "control.longest_saturation_s",
];

/// Metric names produced by [`crate::measure_jerk`].
pub const JERK_METRICS: [&str; 5] = [
    "jerk.peak_acceleration_mps2",
    "jerk.peak_jerk_mps3",
    "jerk.jerk_p95_mps3",
    "jerk.jerk_rms_mps3",
    "jerk.integrated_abs_jerk_mps2",
];

/// Metric names produced by [`crate::measure_release`].
pub const RELEASE_METRICS: [&str; 5] = [
    "release.release_to_stop_s",
    "release.brake_distance_m",
    "release.opposite_velocity_peak_mps",
    "release.return_toward_release_m",
    "release.stop_velocity_delta_mps",
];

/// Metric names produced by [`crate::measure_hold`].
pub const HOLD_METRICS: [&str; 2] = ["hold.rebound_distance_m", "hold.zero_crossings"];

/// Metric names produced by [`crate::measure_step_response`].
pub const RESPONSE_METRICS: [&str; 8] = [
    "response.input_to_command_delay_s",
    "response.input_to_response_delay_s",
    "response.rise_time_s",
    "response.settling_time_s",
    "response.overshoot_fraction",
    "response.undershoot",
    "response.steady_state_error",
    "response.integrated_absolute_error",
];

/// Metric names produced by [`crate::measure_signal`].
pub const SIGNAL_METRICS: [&str; 3] = ["signal.rms", "signal.peak_abs", "signal.p95_abs"];

/// Whether this crate can produce a metric of this name.
///
/// A policy that names anything else states a bar no run can be measured
/// against.
#[must_use]
pub fn is_producible(name: &str) -> bool {
    producible_metrics().any(|known| known == name)
}

/// Every producible metric name, in one sequence.
pub fn producible_metrics() -> impl Iterator<Item = &'static str> {
    CONTROL_METRICS
        .into_iter()
        .chain(JERK_METRICS)
        .chain(RELEASE_METRICS)
        .chain(HOLD_METRICS)
        .chain(RESPONSE_METRICS)
        .chain(SIGNAL_METRICS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_vocabulary_is_a_set_of_family_qualified_names() {
        // What the vocabulary states is checked where it is consumed: the
        // evaluator that produces these values maps each name to a field of a
        // measurement result, so a name no measurement returns does not
        // compile. Textual checks here would compare against validation
        // labels, which are the same strings used for a different purpose.
        let mut seen = std::collections::BTreeSet::new();
        for name in producible_metrics() {
            assert!(seen.insert(name), "{name} is listed twice");
            let (family, rest) = name.split_once('.').unwrap_or(("", ""));
            assert!(
                !family.is_empty() && !rest.is_empty(),
                "{name} is not family-qualified"
            );
        }
        assert!(is_producible("control.effort_rms"));
        assert!(is_producible("hold.zero_crossings"));
        assert!(!is_producible("wind.position_rms_m"));
    }
}
