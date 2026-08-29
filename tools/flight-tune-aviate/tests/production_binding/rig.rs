//! One complete production Aviate binding, small enough to drive in a test.
//!
//! The mapping, the controller, and the adjacency policy are the real
//! production types. What is reduced is the vehicle: the controller holds
//! the profile it was given instead of a flight controller holding it.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use flight_tune::{
    AdapterError, ArtifactIdentity, AttemptRole, Candidate, CandidateLineage, Digest,
    MissionDocument, MissionReference, RunExecutionContext, ScenarioSet, SimulatorCapability,
    SimulatorSessionReceipt, calibration_mission_document, reference_observation_scenario,
};
use flight_tune_aviate::direct_transport::direct_transport_identity;
use flight_tune_aviate::{
    AdjacencyPolicy, AviateFeelController, AviateRuntimeIdentity, AviateVehicleAdapter,
    CandidateFeelMapping, ParameterStepLimit, RuntimeIdentityInputs, TransitionValidator,
};
use pilotage_control_feel::{
    AxisCurve, AxisDynamics, AxisResponse, FeelMode, FlightFeelProfile, NeutralBand,
    ValidatedFlightFeelProfile,
};

/// The parameters the test mapping reads out of one candidate.
pub const DEADZONE: &str = "curve.deadzone";
/// The exponent offset near the centre of the curve.
pub const CENTER_EXPO: &str = "curve.center_expo";
/// The largest demand acceleration while an input is active.
pub const APPLY_ACCEL: &str = "dynamics.apply_accel";

/// What the controller was asked to do.
#[derive(Debug, Default)]
pub struct ControllerLog {
    /// How many complete profiles were written.
    pub applies: usize,
    /// How many readbacks were served.
    pub readbacks: usize,
    /// The profile the controller currently holds.
    pub held: Option<FlightFeelProfile>,
}

/// The controller state, shared with the test that reads it.
#[derive(Clone, Debug, Default)]
pub struct ControllerHandle(pub Rc<RefCell<ControllerLog>>);

impl ControllerHandle {
    /// Creates one controller holding the shaped starting profile.
    #[must_use]
    pub fn new() -> Self {
        let handle = Self::default();
        handle.0.borrow_mut().held = Some(FlightFeelProfile::shaped(FeelMode::Balanced));
        handle
    }

    /// How many complete profiles the controller has been written.
    #[must_use]
    pub fn applies(&self) -> usize {
        self.0.borrow().applies
    }
}

/// The reduced Aviate control law.
#[derive(Clone, Debug)]
pub struct TestController(pub ControllerHandle);

impl AviateFeelController for TestController {
    fn readback_blocking(&mut self) -> Result<ValidatedFlightFeelProfile, AdapterError> {
        let mut log = self.0.0.borrow_mut();
        log.readbacks = log.readbacks.saturating_add(1);
        let held = log
            .held
            .clone()
            .ok_or_else(|| AdapterError::new("the controller holds no profile"))?;
        ValidatedFlightFeelProfile::new(held)
            .map_err(|source| AdapterError::new(source.to_string()))
    }

    fn apply_blocking(&mut self, profile: &ValidatedFlightFeelProfile) -> Result<(), AdapterError> {
        let mut log = self.0.0.borrow_mut();
        log.applies = log.applies.saturating_add(1);
        log.held = Some(profile.profile().clone());
        Ok(())
    }
}

/// Maps one candidate onto the horizontal axis of the shaped profile.
#[derive(Clone, Debug)]
pub struct TestMapping {
    identity: ArtifactIdentity,
}

impl TestMapping {
    /// Creates the mapping with its declared identity.
    #[must_use]
    pub fn new() -> Self {
        Self {
            identity: identity("pilotage-aviate-test-mapping", 0x21),
        }
    }
}

impl CandidateFeelMapping for TestMapping {
    fn mapping_identity(&self) -> &ArtifactIdentity {
        &self.identity
    }

    fn map(&self, candidate: &Candidate) -> Result<ValidatedFlightFeelProfile, AdapterError> {
        let read = |name: &str| -> Result<f64, AdapterError> {
            candidate
                .parameters()
                .get(name)
                .copied()
                .filter(|value| value.is_finite())
                .ok_or_else(|| AdapterError::new(format!("the candidate states no {name}")))
        };
        let center_expo = read(CENTER_EXPO)? as f32;
        let apply_accel = read(APPLY_ACCEL)? as f32;
        let mut profile = FlightFeelProfile::shaped(FeelMode::Balanced);
        profile.horizontal = AxisResponse {
            curve: AxisCurve {
                deadzone: read(DEADZONE)? as f32,
                center_expo,
                outer_expo: center_expo,
                outer_start: 0.7,
            },
            neutral: NeutralBand {
                active_enter: 0.035,
                active_exit: 0.022,
                dwell_ms: 90,
            },
            dynamics: AxisDynamics {
                apply_accel,
                apply_jerk: apply_accel * 6.0,
                release_accel: apply_accel,
                release_jerk: apply_accel * 6.0,
                reversal_accel: apply_accel,
                reversal_jerk: apply_accel * 6.0,
            },
        };
        ValidatedFlightFeelProfile::new(profile)
            .map_err(|source| AdapterError::new(source.to_string()))
    }
}

/// One artifact identity with a distinct, non-zero digest.
#[must_use]
pub fn identity(id: &str, fill: u8) -> ArtifactIdentity {
    #[allow(clippy::expect_used)]
    ArtifactIdentity::new(id, Digest::from_bytes([fill; 32])).expect("a named test identity")
}

/// The capability for one validated simulator session.
#[must_use]
pub fn capability(session: u8) -> SimulatorCapability {
    SimulatorCapability::for_test(SimulatorSessionReceipt {
        session_digest: Digest::from_bytes([session; 32]),
        simulator_digest: Digest::from_bytes([0x51; 32]),
        airframe_digest: Digest::from_bytes([0x52; 32]),
    })
}

/// The adjacency policy the test vehicle enforces.
#[must_use]
pub fn policy() -> AdjacencyPolicy {
    let step = ParameterStepLimit {
        absolute: 0.05,
        fraction: 0.25,
    };
    #[allow(clippy::expect_used)]
    AdjacencyPolicy::new(
        "pilotage-aviate-test-adjacency-v1",
        BTreeMap::from([
            (DEADZONE.to_owned(), step),
            (CENTER_EXPO.to_owned(), step),
            (
                APPLY_ACCEL.to_owned(),
                ParameterStepLimit {
                    absolute: 0.5,
                    fraction: 0.25,
                },
            ),
        ]),
    )
    .expect("a valid adjacency policy")
}

/// The transition validator the test vehicle authorizes through.
#[must_use]
pub fn validator() -> TransitionValidator {
    #[allow(clippy::expect_used)]
    TransitionValidator::new(policy()).expect("a valid transition validator")
}

/// The sealed runtime identity, with a named configuration.
#[must_use]
pub fn runtime_identity(configuration: &str) -> AviateRuntimeIdentity {
    #[allow(clippy::expect_used)]
    AviateRuntimeIdentity::seal(&RuntimeIdentityInputs {
        vehicle: identity("pilotage-aviate-test-vehicle", 0x22),
        transition_validator: validator().identity().clone(),
        adjacency_policy_digest: validator().policy_digest(),
        direct_transport: direct_transport_identity().expect("the direct transport identity"),
        configuration: ArtifactIdentity::from_text(
            "pilotage-aviate-test-configuration",
            configuration,
        )
        .expect("a named configuration identity"),
    })
    .expect("a sealed runtime identity")
}

/// One adapter bound to a validated session.
#[must_use]
pub fn adapter(session: u8) -> AviateVehicleAdapter<TestMapping, TestController> {
    let controller = ControllerHandle::new();
    AviateVehicleAdapter::bind(
        TestMapping::new(),
        TestController(controller),
        validator(),
        &capability(session),
    )
}

/// One adapter, with the controller log the test reads.
#[must_use]
pub fn adapter_with_log(
    session: u8,
) -> (
    AviateVehicleAdapter<TestMapping, TestController>,
    ControllerHandle,
) {
    let controller = ControllerHandle::new();
    let adapter = AviateVehicleAdapter::bind(
        TestMapping::new(),
        TestController(controller.clone()),
        validator(),
        &capability(session),
    );
    (adapter, controller)
}

/// One candidate whose complete mapped profile is valid.
#[must_use]
pub fn candidate(deadzone: f64, center_expo: f64, apply_accel: f64) -> Candidate {
    #[allow(clippy::expect_used)]
    Candidate::new(
        CandidateLineage {
            schema: "pilotage-aviate-test-candidate-v1".to_owned(),
            base_preset_digest: Digest::from_bytes([0x31; 32]),
            plant_digest: Digest::from_bytes([0x32; 32]),
        },
        BTreeMap::from([
            (DEADZONE.to_owned(), deadzone),
            (CENTER_EXPO.to_owned(), center_expo),
            (APPLY_ACCEL.to_owned(), apply_accel),
        ]),
    )
    .expect("a valid test candidate")
}

/// The exact identity of one candidate, as the harness calculates it.
#[must_use]
pub fn candidate_digest(candidate: &Candidate) -> Digest {
    #[allow(clippy::expect_used)]
    let bytes = serde_json::to_vec(candidate).expect("encode a candidate");
    flight_tune::Digest::from_bytes(sha256(&bytes))
}

fn sha256(bytes: &[u8]) -> [u8; 32] {
    use sha2::{Digest as _, Sha256};

    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher.finalize().into()
}

/// One stored calibration mission the runtime can admit.
#[must_use]
pub fn mission_document(id: &str) -> MissionDocument {
    #[allow(clippy::expect_used)]
    calibration_mission_document(
        &reference_observation_scenario(id, Some(2_000_000_000)),
        2,
        1_000_000_000,
    )
    .expect("a valid calibration mission")
}

/// One reference to a stored calibration mission.
#[must_use]
pub fn mission_reference(document: &MissionDocument) -> MissionReference {
    #[allow(clippy::expect_used)]
    MissionReference::from_document(document, 512).expect("a valid mission reference")
}

/// One complete run identity for a training baseline.
#[must_use]
pub fn run_context(
    session: u8,
    document: &MissionDocument,
    candidate_digest: Digest,
    seed: u64,
) -> RunExecutionContext {
    #[allow(clippy::expect_used)]
    RunExecutionContext::new(
        Digest::from_bytes([session; 32]),
        7,
        AttemptRole::TrainingBaseline,
        candidate_digest,
        None,
        ScenarioSet::Training,
        &mission_reference(document),
        0,
        seed,
    )
    .expect("a valid run execution context")
}

/// One supervised launch request with no run intent bound yet.
///
/// The paths never reach a launch: the test only reads the run intent the
/// request carries, which is what binds a process to the run it flies.
#[must_use]
pub fn supervised_request() -> flight_tune_aviate::SupervisedProcessRequest {
    use std::path::PathBuf;
    use std::time::Duration;

    flight_tune_aviate::SupervisedProcessRequest {
        supervisor_executable: PathBuf::from("/nonexistent/supervisor"),
        supervisor_executable_digest: Digest::from_bytes([0x71; 32]),
        target_executable: PathBuf::from("/nonexistent/aviate"),
        target_executable_digest: Digest::from_bytes([0x72; 32]),
        target_arguments: Vec::new(),
        target_environment: BTreeMap::new(),
        target_process_contract: flight_tune_aviate::TargetProcessContract::RetainProcessGroup,
        target_current_directory: PathBuf::from("/nonexistent"),
        storage_root: PathBuf::from("/nonexistent/storage"),
        runtime_root: PathBuf::from("/nonexistent/runtime"),
        artifact_root: PathBuf::from("/nonexistent/artifact"),
        run_intent_digest: Digest::from_bytes([0; 32]),
        startup_timeout: Duration::from_secs(5),
        cleanup_timeout: Duration::from_secs(5),
    }
}
