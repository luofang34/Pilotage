//! The campaign-facing plumbing: the factory that binds a session, the
//! gates a trial is held to, and the stage and scenarios a campaign
//! names.

use std::collections::{BTreeMap, BTreeSet};

use flight_tune::{
    AdapterError, ArtifactIdentity, Digest, EvaluatorError, GateEvaluator, GateOutcome,
    MissionReference, SimulatorCapability, SimulatorVehicleFactory, TelemetrySample,
    TransitionBindingReceipt, VehicleBinding, VehicleBindingReceipt,
};

use super::parameter;
use crate::scoring::channel;
use pilotage_control_feel::{FeelMode, FlightFeelProfile};

use super::adapter::BenchVehicleAdapter;
use super::{BenchHandle, BenchVehicle, bench_action_port_identity};

mod trial;

use trial::BENCH_TRIAL_IDS;
pub use trial::{
    BENCH_FINAL_TRIAL_ID, BENCH_PROMOTION_TRIAL_ID, bench_mission_revision_id,
    bench_physical_target, bench_response_targets, bench_scenario, bench_stored_mission,
};

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

    fn begin(&mut self, _scenario: &MissionReference) -> Result<(), EvaluatorError> {
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
        let mut outcomes = vec![crash_outcome(
            read(channel::CRASHED),
            read(channel::GROUND_CONTACT),
        )?];
        if !outcomes[0].passed {
            return Ok(outcomes);
        }
        let (Some(speed), Some(acceleration)) = (
            read(channel::VELOCITY_MPS).map(f64::abs),
            read(channel::ACCELERATION_MPS2).map(f64::abs),
        ) else {
            outcomes.push(GateOutcome::fail(
                "envelope.channels",
                "a gated channel is missing or not finite".to_owned(),
            ));
            return Ok(outcomes);
        };
        outcomes.push(if speed <= self.maximum_speed_mps {
            GateOutcome::pass("envelope.speed")
        } else {
            GateOutcome::fail("envelope.speed", format!("{speed:.3} m/s"))
        });
        if !outcomes[1].passed {
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

/// The crash gate over one sample's contact signals.
///
/// An absent contact value refuses the execution rather than scoring it. A
/// missing signal is not evidence that nothing was hit, and reading it as a
/// passed gate would let a run whose vehicle hit something be measured, while
/// reading it as a crash would blame the candidate for the harness.
fn crash_outcome(
    crashed: Option<f64>,
    ground_contact: Option<f64>,
) -> Result<GateOutcome, EvaluatorError> {
    let (Some(crashed), Some(ground_contact)) = (crashed, ground_contact) else {
        return Err(EvaluatorError::new(
            "a sample states no crash or contact value",
        ));
    };
    if crashed != 0.0 {
        return Ok(GateOutcome::fail(
            flight_tune::MANDATORY_CRASH_GATE_ID,
            "the bench detected a crash".to_owned(),
        ));
    }
    if ground_contact != 0.0 {
        return Ok(GateOutcome::fail(
            flight_tune::MANDATORY_CRASH_GATE_ID,
            "the bench detected unexpected contact".to_owned(),
        ));
    }
    Ok(GateOutcome::pass(flight_tune::MANDATORY_CRASH_GATE_ID))
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
/// Returns an error when a mission identity cannot be calculated.
pub fn bench_stage(
    id: &str,
    model: BenchVehicle,
    promotion: flight_tune::PromotionPolicy,
    qualification: flight_tune::QualificationPolicy,
    response_targets: flight_tune::ResponseTargetTable,
) -> Result<flight_tune::SearchStage, AdapterError> {
    use flight_tune::ParameterBounds;
    let bounds = |minimum: f64, maximum: f64| ParameterBounds { minimum, maximum };
    let direct = bench_scenario(BENCH_TRIAL_IDS[0], model)?;
    let operator = bench_scenario(BENCH_TRIAL_IDS[1], model)?;
    let promotion_scenario = bench_scenario(BENCH_PROMOTION_TRIAL_ID, model)?;
    let final_scenario = bench_scenario(BENCH_FINAL_TRIAL_ID, model)?;
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
        // The crash gate is the floor of every campaign and is evaluated
        // before any limit. A run that hit something has no measurement worth
        // comparing against an envelope.
        required_hard_gates: vec![
            flight_tune::MANDATORY_CRASH_GATE_ID.to_owned(),
            "envelope.speed".to_owned(),
            "envelope.acceleration".to_owned(),
        ],
        // Training, promotion and final qualification fly separate scenarios.
        // The bar that decides what ships is measured on runs the search never
        // saw, so a candidate cannot be fitted to the scenario that judges it.
        training_scenarios: vec![direct.clone(), operator.clone()],
        training_suites: vec![
            direct_response_suite(&direct),
            operator_feel_suite(
                &operator,
                &direct,
                &response_targets,
                &promotion_scenario.revision_id,
            )?,
        ],
        search_groups: search_groups(),
        promotion_scenarios: vec![promotion_scenario],
        final_qualification_scenarios: vec![final_scenario],
        repetitions: 2,
        promotion,
        qualification,
        // The bench plant is deterministic and runs in process, so no
        // execution it starts can fail for a reason a replacement would
        // answer. Authorizing replacements that can never be needed would
        // state a weaker bar than the one this campaign actually clears.
        execution_retry: flight_tune::ExecutionRetryPolicy::none(),
        response_targets,
    })
}

/// The objectives a change to the command shape may not degrade.
///
/// Both are direct response measurements. A shaping change that improves the
/// operator trial while it slows the response or spends more actuator is a
/// change the operator trial alone would accept.
const GUARD_OBJECTIVES: [&str; 2] = ["response.overshoot_fraction", "control.effort_rms"];

/// The suite that answers a change to the demand dynamics.
///
/// The demand rate limits shape the closed-loop response, so the direct step
/// trial is the evidence that answers them.
fn direct_response_suite(direct: &MissionReference) -> flight_tune::TrainingSuite {
    flight_tune::TrainingSuite {
        schema_version: flight_tune::TRAINING_SUITE_SCHEMA_VERSION,
        id: "direct-response".to_owned(),
        primary_scenarios: vec![direct.clone()],
        guard_scenarios: Vec::new(),
        guard_regression_limits: BTreeMap::new(),
        repetitions: 2,
    }
}

/// The suite that answers a change to the operator command shape.
///
/// The operator trial carries the loss and the direct step trial guards it.
/// The guard limits are the vehicle's own promotion regression limits for the
/// promotion scenario, so one bar answers both the hidden decision and the
/// search.
fn operator_feel_suite(
    operator: &MissionReference,
    direct: &MissionReference,
    response_targets: &flight_tune::ResponseTargetTable,
    promotion_mission_id: &str,
) -> Result<flight_tune::TrainingSuite, AdapterError> {
    let mut guard_regression_limits = BTreeMap::new();
    for name in GUARD_OBJECTIVES {
        let limit = response_targets
            .target(promotion_mission_id, name)
            .map_err(|error| AdapterError::new(error.to_string()))?
            .limit;
        guard_regression_limits.insert(name.to_owned(), limit);
    }
    Ok(flight_tune::TrainingSuite {
        schema_version: flight_tune::TRAINING_SUITE_SCHEMA_VERSION,
        id: "operator-feel".to_owned(),
        primary_scenarios: vec![operator.clone()],
        guard_scenarios: vec![direct.clone()],
        guard_regression_limits,
        repetitions: 2,
    })
}

/// The two parameter groups and the suite each one is answered by.
///
/// The demand rate limits and the stick shape are separate response families.
/// A search that compared one against evidence produced under the other would
/// compare two different questions.
fn search_groups() -> Vec<flight_tune::SearchGroup> {
    vec![
        flight_tune::SearchGroup {
            id: "command-dynamics".to_owned(),
            kind: flight_tune::SearchGroupKind::Controller,
            parameters: BTreeSet::from([
                parameter::APPLY_ACCEL.to_owned(),
                parameter::APPLY_JERK.to_owned(),
                parameter::RELEASE_FACTOR.to_owned(),
            ]),
            suite_id: "direct-response".to_owned(),
        },
        flight_tune::SearchGroup {
            id: "command-shape".to_owned(),
            kind: flight_tune::SearchGroupKind::OperatorFeel,
            parameters: BTreeSet::from([
                parameter::DEADZONE.to_owned(),
                parameter::CENTER_EXPO.to_owned(),
                parameter::OUTER_EXPO.to_owned(),
                parameter::NEUTRAL_ENTER.to_owned(),
                parameter::NEUTRAL_DWELL_MS.to_owned(),
            ]),
            suite_id: "operator-feel".to_owned(),
        },
    ]
}
