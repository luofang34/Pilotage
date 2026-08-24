use crate::flight_quality::config::{FlightQualityGate, FlightQualityGateConfig, gate_identity};
use crate::flight_quality::telemetry::{CanonicalTelemetryKey, finite_value, flag};
use crate::{
    ArtifactIdentity, EvaluatorError, GateEvaluator, GateOutcome, ScenarioRef, TelemetrySample,
};

/// A fail-fast streaming evaluator for canonical flight hard gates.
pub struct FlightQualityGateEvaluator {
    config: FlightQualityGateConfig,
    identity: ArtifactIdentity,
    active: bool,
    saturation: SaturationState,
}

#[derive(Default)]
struct SaturationState {
    prior_time_s: Option<f64>,
    prior_saturated: bool,
    active_s: f64,
}

impl FlightQualityGateEvaluator {
    /// Creates a gate evaluator and hashes its configuration and implementation.
    ///
    /// # Errors
    ///
    /// Returns [`EvaluatorError`] when the configuration is not valid.
    pub fn new(config: FlightQualityGateConfig) -> Result<Self, EvaluatorError> {
        config.validate()?;
        let identity = gate_identity(&config)?;
        Ok(Self {
            config,
            identity,
            active: false,
            saturation: SaturationState::default(),
        })
    }

    fn evaluate_one(&mut self, gate: FlightQualityGate, sample: &TelemetrySample) -> GateOutcome {
        let result = match gate {
            FlightQualityGate::CrashOrUnexpectedContact => crash_or_contact(sample),
            FlightQualityGate::FiniteSignals => finite_signals(sample),
            FlightQualityGate::PositionBound => bound(
                sample,
                CanonicalTelemetryKey::PositionErrorM,
                self.config.maximum_position_error_m,
            ),
            FlightQualityGate::AttitudeBound => bound(
                sample,
                CanonicalTelemetryKey::AttitudeErrorRad,
                self.config.maximum_attitude_error_rad,
            ),
            FlightQualityGate::RateBound => bound(
                sample,
                CanonicalTelemetryKey::BodyRateRadS,
                self.config.maximum_body_rate_rad_s,
            ),
            FlightQualityGate::LoadBound => bound(
                sample,
                CanonicalTelemetryKey::LoadFactorG,
                self.config.maximum_load_factor_g,
            ),
            FlightQualityGate::ActuatorSaturationDuration => self.saturation(sample),
            FlightQualityGate::EstimatorValidity => required_flag(
                sample,
                CanonicalTelemetryKey::EstimatorValid,
                "the estimator is not valid",
            ),
            FlightQualityGate::CommandLinkValidity => required_flag(
                sample,
                CanonicalTelemetryKey::CommandLinkValid,
                "the command link is not valid",
            ),
            FlightQualityGate::RecoveryDeadline => self.recovery(sample),
        };
        to_outcome(gate, result)
    }

    fn saturation(&mut self, sample: &TelemetrySample) -> Result<Option<String>, EvaluatorError> {
        let effort = finite_value(sample, CanonicalTelemetryKey::ActuatorEffort)?;
        if !(-1.0..=1.0).contains(&effort) {
            return Ok(Some(
                "normalized actuator effort is outside minus one to one".to_owned(),
            ));
        }
        let saturated = flag(sample, CanonicalTelemetryKey::ActuatorSaturated)?;
        let time_s = sample.elapsed_ms as f64 / 1_000.0;
        if let Some(prior_time_s) = self.saturation.prior_time_s {
            let interval_s = time_s - prior_time_s;
            if interval_s < 0.0 {
                return Err(EvaluatorError::new("sample time moved backward"));
            }
            if self.saturation.prior_saturated {
                self.saturation.active_s += interval_s;
            } else {
                self.saturation.active_s = 0.0;
            }
        }
        self.saturation.prior_time_s = Some(time_s);
        self.saturation.prior_saturated = saturated;
        if self.saturation.active_s > self.config.maximum_saturation_s {
            Ok(Some(format!(
                "continuous actuator saturation is {} s; limit is {} s",
                self.saturation.active_s, self.config.maximum_saturation_s
            )))
        } else {
            Ok(None)
        }
    }

    fn recovery(&self, sample: &TelemetrySample) -> Result<Option<String>, EvaluatorError> {
        let recovered = flag(sample, CanonicalTelemetryKey::Recovered)?;
        let time_s = sample.elapsed_ms as f64 / 1_000.0;
        if time_s >= self.config.recovery_deadline_s && !recovered {
            Ok(Some(format!(
                "the vehicle did not recover by {} s",
                self.config.recovery_deadline_s
            )))
        } else {
            Ok(None)
        }
    }
}

impl GateEvaluator for FlightQualityGateEvaluator {
    fn identity(&self) -> &ArtifactIdentity {
        &self.identity
    }

    fn begin(&mut self, _scenario: &ScenarioRef) -> Result<(), EvaluatorError> {
        if self.active {
            return Err(EvaluatorError::new("a gate run is already active"));
        }
        self.active = true;
        self.saturation = SaturationState::default();
        Ok(())
    }

    fn evaluate(&mut self, sample: &TelemetrySample) -> Result<Vec<GateOutcome>, EvaluatorError> {
        if !self.active {
            return Err(EvaluatorError::new("no gate run is active"));
        }
        let required = self.config.required.clone();
        let mut outcomes = Vec::with_capacity(required.len());
        for gate in required {
            let outcome = self.evaluate_one(gate, sample);
            let failed = !outcome.passed;
            outcomes.push(outcome);
            if failed {
                break;
            }
        }
        Ok(outcomes)
    }

    fn finish(&mut self) -> Result<(), EvaluatorError> {
        if !self.active {
            return Err(EvaluatorError::new("no gate run is active"));
        }
        self.active = false;
        self.saturation = SaturationState::default();
        Ok(())
    }

    fn cancel(&mut self) -> Result<(), EvaluatorError> {
        self.active = false;
        self.saturation = SaturationState::default();
        Ok(())
    }
}

fn crash_or_contact(sample: &TelemetrySample) -> Result<Option<String>, EvaluatorError> {
    if flag(sample, CanonicalTelemetryKey::CrashDetected)? {
        return Ok(Some("the simulator detected a crash".to_owned()));
    }
    if flag(sample, CanonicalTelemetryKey::UnexpectedContact)? {
        return Ok(Some("the simulator detected unexpected contact".to_owned()));
    }
    Ok(None)
}

fn finite_signals(sample: &TelemetrySample) -> Result<Option<String>, EvaluatorError> {
    for key in CanonicalTelemetryKey::ALL {
        finite_value(sample, key)?;
    }
    Ok(None)
}

fn bound(
    sample: &TelemetrySample,
    key: CanonicalTelemetryKey,
    limit: f64,
) -> Result<Option<String>, EvaluatorError> {
    let value = finite_value(sample, key)?;
    if value < 0.0 || value > limit {
        Ok(Some(format!(
            "canonical field {} is {value}; limit is {limit}",
            key.as_str()
        )))
    } else {
        Ok(None)
    }
}

fn required_flag(
    sample: &TelemetrySample,
    key: CanonicalTelemetryKey,
    detail: &'static str,
) -> Result<Option<String>, EvaluatorError> {
    if flag(sample, key)? {
        Ok(None)
    } else {
        Ok(Some(detail.to_owned()))
    }
}

fn to_outcome(
    gate: FlightQualityGate,
    result: Result<Option<String>, EvaluatorError>,
) -> GateOutcome {
    match result {
        Ok(None) => GateOutcome::pass(gate.id()),
        Ok(Some(detail)) => GateOutcome::fail(gate.id(), detail),
        Err(error) => GateOutcome::fail(gate.id(), error.to_string()),
    }
}
