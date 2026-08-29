use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::identity::digest_bytes;
use crate::{ArtifactIdentity, EvaluatorError};

/// The one hard gate every campaign must evaluate, and evaluate first.
///
/// A run that continued after the vehicle hit something measures the
/// collision. Nothing downstream can tell that measurement apart from a
/// command law that was merely poor, so the crash gate is a floor rather than
/// a choice: a stage cannot drop it, rename it, or put another gate in front
/// of it.
pub const MANDATORY_CRASH_GATE_ID: &str = "flight_quality.crash_or_unexpected_contact";

/// One canonical streaming hard gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FlightQualityGate {
    /// Crash or unexpected-contact detection.
    CrashOrUnexpectedContact,
    /// Presence and finite-value checks for all canonical signals.
    FiniteSignals,
    /// Position-error magnitude limit.
    PositionBound,
    /// Attitude-error magnitude limit.
    AttitudeBound,
    /// Body-rate magnitude limit.
    RateBound,
    /// Load-factor limit.
    LoadBound,
    /// Continuous actuator-saturation duration limit.
    ActuatorSaturationDuration,
    /// Estimator-validity check.
    EstimatorValidity,
    /// Command-link validity check.
    CommandLinkValidity,
    /// Recovery deadline check.
    RecoveryDeadline,
}

impl FlightQualityGate {
    /// Returns the stable gate identifier for [`crate::SearchStage`].
    #[must_use]
    pub const fn id(self) -> &'static str {
        match self {
            Self::CrashOrUnexpectedContact => MANDATORY_CRASH_GATE_ID,
            Self::FiniteSignals => "flight_quality.finite_signals",
            Self::PositionBound => "flight_quality.position_bound",
            Self::AttitudeBound => "flight_quality.attitude_bound",
            Self::RateBound => "flight_quality.rate_bound",
            Self::LoadBound => "flight_quality.load_bound",
            Self::ActuatorSaturationDuration => "flight_quality.saturation_duration",
            Self::EstimatorValidity => "flight_quality.estimator_validity",
            Self::CommandLinkValidity => "flight_quality.command_link_validity",
            Self::RecoveryDeadline => "flight_quality.recovery_deadline",
        }
    }
}

/// Limits and priority order for streaming hard gates.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FlightQualityGateConfig {
    /// Hard gates in stage priority order.
    pub required: Vec<FlightQualityGate>,
    /// Maximum position-error magnitude, in meters.
    pub maximum_position_error_m: f64,
    /// Maximum attitude-error magnitude, in radians.
    pub maximum_attitude_error_rad: f64,
    /// Maximum body-rate magnitude, in radians per second.
    pub maximum_body_rate_rad_s: f64,
    /// Maximum absolute load factor, in g.
    pub maximum_load_factor_g: f64,
    /// Maximum continuous actuator-saturation time, in seconds.
    pub maximum_saturation_s: f64,
    /// Time by which the vehicle must report recovery, in seconds.
    pub recovery_deadline_s: f64,
}

impl FlightQualityGateConfig {
    /// Returns the required gate identifiers in evaluation order.
    #[must_use]
    pub fn required_ids(&self) -> Vec<String> {
        self.required
            .iter()
            .map(|gate| gate.id().to_owned())
            .collect()
    }

    pub(super) fn validate(&self) -> Result<(), EvaluatorError> {
        let unique = self.required.iter().copied().collect::<BTreeSet<_>>();
        if self.required.is_empty() || unique.len() != self.required.len() {
            return Err(invalid("hard gates must be present and unique"));
        }
        if self.required.first() != Some(&FlightQualityGate::CrashOrUnexpectedContact) {
            return Err(invalid(
                "the crash or unexpected contact gate is evaluated first",
            ));
        }
        for value in [
            self.maximum_position_error_m,
            self.maximum_attitude_error_rad,
            self.maximum_body_rate_rad_s,
            self.maximum_load_factor_g,
            self.maximum_saturation_s,
            self.recovery_deadline_s,
        ] {
            if !value.is_finite() || value <= 0.0 {
                return Err(invalid("hard gate limits must be finite and positive"));
            }
        }
        Ok(())
    }
}

/// One planned scalar step.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StepPlan {
    /// Input event time, in seconds.
    pub input_time_s: f64,
    /// Response value before the step.
    pub initial_value: f64,
    /// Requested response value after the step.
    pub target_value: f64,
}

/// One planned release and final-hold event.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReleasePlan {
    /// Input release time, in seconds.
    pub release_time_s: f64,
    /// Start time of the final hold, in seconds.
    pub hold_start_s: f64,
    /// Position of the final hold, in meters.
    pub hold_position_m: f64,
}

/// One planned wind-disturbance interval.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WindPlan {
    /// Minimum wind speed that marks an active disturbance.
    pub minimum_wind_speed_mps: f64,
}

/// Metric events for one scenario identifier.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FlightQualityScenario {
    /// Optional scalar step event.
    pub step: Option<StepPlan>,
    /// Optional release and final-hold event.
    pub release: Option<ReleasePlan>,
    /// Optional wind-disturbance interval.
    pub wind: Option<WindPlan>,
}

/// Normalization scales for the dimensionless loss.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FlightQualityScales {
    /// Time scale, in seconds.
    pub time_s: f64,
    /// Position scale, in meters.
    pub position_m: f64,
    /// Speed scale, in meters per second.
    pub speed_mps: f64,
    /// Jerk scale, in meters per second cubed.
    pub jerk_mps3: f64,
    /// Penalty for a response threshold that the trace does not reach.
    pub unreached_penalty: f64,
}

/// Group weights for the dimensionless loss.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FlightQualityWeights {
    /// Step-response metric weight.
    pub step_response: f64,
    /// Release and rebound metric weight.
    pub release: f64,
    /// Jerk metric weight.
    pub jerk: f64,
    /// Wind-position metric weight.
    pub wind_position: f64,
}

/// Scenario plans and loss settings for the metric evaluator.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FlightQualityMetricConfig {
    /// Scenario plans indexed by exact scenario identifier.
    pub scenarios: BTreeMap<String, FlightQualityScenario>,
    /// Normalization scales.
    pub scales: FlightQualityScales,
    /// Loss group weights.
    pub weights: FlightQualityWeights,
}

impl FlightQualityMetricConfig {
    pub(super) fn validate(&self) -> Result<(), EvaluatorError> {
        if self.scenarios.is_empty() {
            return Err(invalid("metric configuration needs a scenario plan"));
        }
        validate_scales(self.scales)?;
        validate_weights(self.weights)?;
        for (id, scenario) in &self.scenarios {
            if id.trim().is_empty() || id.len() > 128 {
                return Err(invalid("scenario identifiers must have 1 to 128 bytes"));
            }
            validate_scenario(scenario)?;
            if active_weight(self.weights, scenario) <= 0.0 {
                return Err(invalid(
                    "each scenario needs a positive weight for an active metric group",
                ));
            }
        }
        Ok(())
    }
}

pub(super) fn gate_identity(
    config: &FlightQualityGateConfig,
) -> Result<ArtifactIdentity, EvaluatorError> {
    implementation_identity(
        "pilotage-flight-quality-streaming-gates",
        config,
        &[
            include_str!("config.rs"),
            include_str!("gates.rs"),
            include_str!("telemetry.rs"),
        ],
    )
}

pub(super) fn metric_identity(
    config: &FlightQualityMetricConfig,
) -> Result<ArtifactIdentity, EvaluatorError> {
    implementation_identity(
        "pilotage-flight-quality-streaming-metrics",
        config,
        &[
            include_str!("config.rs"),
            include_str!("metrics.rs"),
            include_str!("telemetry.rs"),
            include_str!("../../../../crates/pilotage-flight-quality/src/control.rs"),
            include_str!("../../../../crates/pilotage-flight-quality/src/release.rs"),
            include_str!("../../../../crates/pilotage-flight-quality/src/response.rs"),
            include_str!("../../../../crates/pilotage-flight-quality/src/signal.rs"),
            include_str!("../../../../crates/pilotage-flight-quality/src/series.rs"),
            include_str!("../../../../crates/pilotage-flight-quality/src/sample.rs"),
            include_str!("../../../../crates/pilotage-flight-quality/src/error.rs"),
        ],
    )
}

fn implementation_identity<T: Serialize>(
    id: &'static str,
    config: &T,
    sources: &[&str],
) -> Result<ArtifactIdentity, EvaluatorError> {
    let config = serde_json::to_vec(config)
        .map_err(|error| invalid(format!("cannot encode evaluator configuration: {error}")))?;
    let mut bytes = Vec::with_capacity(config.len());
    append_document(&mut bytes, &config);
    for source in sources {
        append_document(&mut bytes, source.as_bytes());
    }
    ArtifactIdentity::new(id, digest_bytes(&bytes)).map_err(|error| invalid(error.to_string()))
}

fn append_document(output: &mut Vec<u8>, document: &[u8]) {
    output.extend_from_slice(&(document.len() as u64).to_le_bytes());
    output.extend_from_slice(document);
}

fn active_weight(weights: FlightQualityWeights, scenario: &FlightQualityScenario) -> f64 {
    weights.jerk
        + scenario.step.map_or(0.0, |_| weights.step_response)
        + scenario.release.map_or(0.0, |_| weights.release)
        + scenario.wind.map_or(0.0, |_| weights.wind_position)
}

fn validate_scales(scales: FlightQualityScales) -> Result<(), EvaluatorError> {
    for value in [
        scales.time_s,
        scales.position_m,
        scales.speed_mps,
        scales.jerk_mps3,
        scales.unreached_penalty,
    ] {
        if !value.is_finite() || value <= 0.0 {
            return Err(invalid("metric scales must be finite and positive"));
        }
    }
    Ok(())
}

fn validate_weights(weights: FlightQualityWeights) -> Result<(), EvaluatorError> {
    let values = [
        weights.step_response,
        weights.release,
        weights.jerk,
        weights.wind_position,
    ];
    if values
        .iter()
        .any(|value| !value.is_finite() || *value < 0.0)
        || values.iter().all(|value| *value == 0.0)
    {
        return Err(invalid(
            "metric weights must be finite, nonnegative, and not all zero",
        ));
    }
    Ok(())
}

fn validate_scenario(scenario: &FlightQualityScenario) -> Result<(), EvaluatorError> {
    if let Some(step) = scenario.step
        && (!step.input_time_s.is_finite()
            || step.input_time_s < 0.0
            || !step.initial_value.is_finite()
            || !step.target_value.is_finite()
            || step.initial_value == step.target_value)
    {
        return Err(invalid("step plan values are not valid"));
    }
    if let Some(release) = scenario.release
        && (!release.release_time_s.is_finite()
            || release.release_time_s < 0.0
            || !release.hold_start_s.is_finite()
            || release.hold_start_s <= release.release_time_s
            || !release.hold_position_m.is_finite())
    {
        return Err(invalid("release plan values are not valid"));
    }
    if let Some(wind) = scenario.wind
        && (!wind.minimum_wind_speed_mps.is_finite() || wind.minimum_wind_speed_mps < 0.0)
    {
        return Err(invalid("wind plan values are not valid"));
    }
    Ok(())
}

fn invalid(detail: impl Into<String>) -> EvaluatorError {
    EvaluatorError::new(detail)
}
