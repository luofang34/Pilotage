//! The campaign-facing plumbing: the factory that binds a session, the
//! gates a trial is held to, and the stage and scenarios a campaign
//! names.

use std::collections::BTreeMap;

use flight_tune::{
    AdapterError, ArtifactIdentity, Digest, EvaluatorError, GateEvaluator, GateOutcome,
    ScenarioRef, SimulatorCapability, SimulatorVehicleFactory, TelemetrySample,
    TransitionBindingReceipt, VehicleBinding, VehicleBindingReceipt,
};

use super::parameter;
use crate::scoring::channel;
use pilotage_control_feel::{FeelMode, FlightFeelProfile};

use super::adapter::BenchVehicleAdapter;
use super::{BenchHandle, bench_action_port_identity};

/// Binds the bench vehicle to a validated simulator session.
#[derive(Debug)]
pub struct BenchVehicleFactory {
    handle: BenchHandle,
    identity: ArtifactIdentity,
    action_port_identity: ArtifactIdentity,
    transition_validator: ArtifactIdentity,
    adjacency_policy_digest: Digest,
}

impl BenchVehicleFactory {
    /// Creates one factory for a named vehicle.
    ///
    /// # Errors
    ///
    /// Returns [`AdapterError`] when an identity cannot be built.
    pub fn new(handle: BenchHandle, vehicle_id: &str) -> Result<Self, AdapterError> {
        let text = |value: &str| {
            ArtifactIdentity::from_text(value, value)
                .map_err(|error| AdapterError::new(error.to_string()))
        };
        Ok(Self {
            handle,
            identity: text(vehicle_id)?,
            action_port_identity: bench_action_port_identity()?,
            transition_validator: text("bench-transition-validator")?,
            adjacency_policy_digest: Digest::from_bytes([8; 32]),
        })
    }
}

impl SimulatorVehicleFactory for BenchVehicleFactory {
    type Adapter = BenchVehicleAdapter;

    fn vehicle_identity(&self) -> &ArtifactIdentity {
        &self.identity
    }

    fn scenario_action_port_identity(&self) -> &ArtifactIdentity {
        &self.action_port_identity
    }

    fn transition_validator_identity(&self) -> &ArtifactIdentity {
        &self.transition_validator
    }

    fn adjacency_policy_digest(&self) -> Digest {
        self.adjacency_policy_digest
    }

    fn bind_blocking(
        self,
        capability: &SimulatorCapability,
    ) -> Result<VehicleBinding<Self::Adapter>, AdapterError> {
        let transition = TransitionBindingReceipt::new(
            capability.session_digest(),
            self.transition_validator,
            self.adjacency_policy_digest,
        )?;
        capability.bind_vehicle_with_transition(
            BenchVehicleAdapter::new(self.handle),
            VehicleBindingReceipt {
                session_digest: capability.session_digest(),
                vehicle_digest: self.identity.digest,
                scenario_runtime_digest: flight_tune::scenario_runtime_identity(
                    &self.action_port_identity,
                )
                .map_err(|error| AdapterError::new(error.to_string()))?
                .digest,
            },
            transition,
        )
    }
}

/// The hard gates a bench run must not trip.
///
/// A gate is a refusal, not a score: a run that trips one is not a worse
/// candidate, it is a candidate whose result means nothing. Speed and
/// acceleration ceilings here stand for the envelope a real vehicle would be
/// held inside.
#[derive(Debug)]
pub struct BenchGates {
    identity: ArtifactIdentity,
    maximum_speed_mps: f64,
    maximum_acceleration_mps2: f64,
}

impl BenchGates {
    /// Creates gates at one vehicle's ceilings.
    ///
    /// # Errors
    ///
    /// Returns [`EvaluatorError`] when the identity cannot be built.
    pub fn new(
        maximum_speed_mps: f64,
        maximum_acceleration_mps2: f64,
    ) -> Result<Self, EvaluatorError> {
        let identity = ArtifactIdentity::from_text("bench-envelope-gates", "bench-envelope-gates")
            .map_err(|error| EvaluatorError::new(error.to_string()))?;
        Ok(Self {
            identity,
            maximum_speed_mps,
            maximum_acceleration_mps2,
        })
    }
}

impl GateEvaluator for BenchGates {
    fn identity(&self) -> &ArtifactIdentity {
        &self.identity
    }

    fn begin(&mut self, _scenario: &ScenarioRef) -> Result<(), EvaluatorError> {
        Ok(())
    }

    fn evaluate(&mut self, sample: &TelemetrySample) -> Result<Vec<GateOutcome>, EvaluatorError> {
        // A gate is a refusal: a sample that does not carry a channel,
        // or carries garbage in it, must fail the gate rather than pass
        // as a zero the vehicle never reported.
        let read = |name: &str| {
            sample
                .values
                .get(name)
                .copied()
                .filter(|value| value.is_finite())
        };
        let (Some(speed), Some(acceleration)) = (
            read(channel::VELOCITY_MPS).map(f64::abs),
            read(channel::ACCELERATION_MPS2).map(f64::abs),
        ) else {
            return Ok(vec![GateOutcome::fail(
                "envelope.channels",
                "a gated channel is missing or not finite".to_owned(),
            )]);
        };
        let mut outcomes = Vec::new();
        outcomes.push(if speed <= self.maximum_speed_mps {
            GateOutcome::pass("envelope.speed")
        } else {
            GateOutcome::fail("envelope.speed", format!("{speed:.3} m/s"))
        });
        if !outcomes[0].passed {
            return Ok(outcomes);
        }
        outcomes.push(if acceleration <= self.maximum_acceleration_mps2 {
            GateOutcome::pass("envelope.acceleration")
        } else {
            GateOutcome::fail("envelope.acceleration", format!("{acceleration:.3} m/s2"))
        });
        Ok(outcomes)
    }

    fn finish(&mut self) -> Result<(), EvaluatorError> {
        Ok(())
    }

    fn cancel(&mut self) -> Result<(), EvaluatorError> {
        Ok(())
    }
}

/// The starting command law a search departs from, as candidate parameters.
///
/// This is the shaped mode the vehicle would otherwise ship with, expressed in
/// the parameters the search may move. Starting a search from a law that is
/// already safe is what keeps the first trials from being wasted on finding
/// one.
#[must_use]
pub fn warm_start_parameters(mode: FeelMode) -> BTreeMap<String, f64> {
    let profile = FlightFeelProfile::shaped(mode);
    let axis = profile.horizontal;
    BTreeMap::from([
        (
            parameter::DEADZONE.to_owned(),
            f64::from(axis.curve.deadzone),
        ),
        (
            parameter::CENTER_EXPO.to_owned(),
            f64::from(axis.curve.center_expo),
        ),
        (
            parameter::OUTER_EXPO.to_owned(),
            f64::from(axis.curve.outer_expo),
        ),
        (
            parameter::APPLY_ACCEL.to_owned(),
            f64::from(axis.dynamics.apply_accel),
        ),
        (
            parameter::APPLY_JERK.to_owned(),
            f64::from(axis.dynamics.apply_jerk),
        ),
        (
            parameter::RELEASE_FACTOR.to_owned(),
            f64::from(axis.dynamics.release_accel / axis.dynamics.apply_accel),
        ),
        (
            parameter::NEUTRAL_ENTER.to_owned(),
            f64::from(axis.neutral.active_enter),
        ),
        (
            parameter::NEUTRAL_DWELL_MS.to_owned(),
            f64::from(axis.neutral.dwell_ms),
        ),
    ])
}

/// The stage one vehicle is tuned over.
///
/// The allowlist is the command law's own numbers, bounded around the shaped
/// starting point rather than around zero: a search that may propose any value
/// spends its first trials rediscovering that a control has to be stable,
/// which is what a warm start exists to avoid.
///
/// # Errors
///
/// Returns an error when a scenario identity cannot be calculated.
pub fn bench_stage(
    id: &str,
    promotion: flight_tune::PromotionPolicy,
    qualification: flight_tune::QualificationPolicy,
) -> Result<flight_tune::SearchStage, AdapterError> {
    use flight_tune::ParameterBounds;
    let bounds = |minimum: f64, maximum: f64| ParameterBounds { minimum, maximum };
    Ok(flight_tune::SearchStage {
        id: id.to_owned(),
        allowlist: BTreeMap::from([
            (parameter::DEADZONE.to_owned(), bounds(0.02, 0.12)),
            (parameter::CENTER_EXPO.to_owned(), bounds(0.0, 0.7)),
            (parameter::OUTER_EXPO.to_owned(), bounds(0.0, 0.7)),
            (parameter::APPLY_ACCEL.to_owned(), bounds(1.0, 10.0)),
            (parameter::APPLY_JERK.to_owned(), bounds(5.0, 80.0)),
            (parameter::RELEASE_FACTOR.to_owned(), bounds(1.0, 2.0)),
            (parameter::NEUTRAL_ENTER.to_owned(), bounds(0.01, 0.06)),
            (parameter::NEUTRAL_DWELL_MS.to_owned(), bounds(20.0, 200.0)),
        ]),
        fixed_parameters: BTreeMap::new(),
        required_hard_gates: vec![
            "envelope.speed".to_owned(),
            "envelope.acceleration".to_owned(),
        ],
        // Training, promotion and final qualification fly separate scenarios.
        // The bar that decides what ships is measured on runs the search never
        // saw, so a candidate cannot be fitted to the scenario that judges it.
        training_scenarios: vec![bench_scenario("training-step", 21)?],
        promotion_scenarios: vec![bench_scenario("promotion-step", 22)?],
        final_qualification_scenarios: vec![bench_scenario("final-step", 23)?],
        repetitions: 2,
        promotion,
        qualification,
    })
}

/// One trial of the bench, as a scenario reference.
///
/// # Errors
///
/// Returns an error when the canonical scenario identity cannot be calculated.
pub fn bench_scenario(id: &str, digest_byte: u8) -> Result<ScenarioRef, AdapterError> {
    let mut reference = ScenarioRef {
        id: id.to_owned(),
        digest: Digest::from_bytes([digest_byte; 32]),
        // The ceiling covers the whole trial at the bench's sample rate
        // with room for the completion event.
        max_samples: 700,
        sample_timeout_ms: 200,
    };
    reference.digest =
        flight_tune::reference_observation_scenario(&reference, Some(10_480_000_000))
            .canonical_digest()
            .map_err(|error| AdapterError::new(error.to_string()))?;
    Ok(reference)
}
