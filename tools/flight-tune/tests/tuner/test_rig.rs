use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{SystemTime, UNIX_EPOCH};

use flight_tune::{
    ArtifactIdentity, ControlFamily, Digest, MissionDocument, MissionReference,
    RunExecutionContext, TrialScenario, calibration_mission_document,
    reference_observation_scenario,
};

/// The sample ceiling every fake mission runs under.
pub const FAKE_MAX_SAMPLES: u32 = 8;

const FAKE_RECEIPT_TIMEOUT_NS: u64 = 100_000_000;

/// The retry limit that separates the two stored variants of one trial.
///
/// The variants differ in mission content but not in run behaviour, so a
/// changed document is an identity change and nothing else.
const CHANGED_RETRY_LIMIT: u16 = 1;

/// The trial names that the fake backend stores as mission documents.
pub const FAKE_MISSION_IDS: [&str; 4] = [
    "training-calm",
    "promotion-gust",
    "final-crosswind",
    "training-response",
];

/// Resolves the stored fake mission that one reference names.
///
/// The store is keyed by mission content, so a changed document is a
/// different stored artifact rather than a replacement of the first.
pub fn fake_stored_mission(mission: &MissionReference) -> Option<MissionDocument> {
    FAKE_MISSION_IDS
        .into_iter()
        .flat_map(|id| {
            [
                fake_mission_document(id),
                changed_fake_mission_document(id),
                fake_stimulus_mission_document(id, ControlFamily::OperatorVelocity),
                fake_stimulus_mission_document(id, ControlFamily::DirectAttitudeThrust),
            ]
        })
        .find(|document| {
            Digest::from_bytes(*document.identity.content_digest.as_bytes())
                == mission.content_digest
        })
}

/// Builds the stored mission document that one fake trial name produces.
pub fn fake_mission_document(id: &str) -> MissionDocument {
    fake_mission_document_with_retry(id, 0)
}

/// Builds the second stored mission document for one fake trial name.
pub fn changed_fake_mission_document(id: &str) -> MissionDocument {
    fake_mission_document_with_retry(id, CHANGED_RETRY_LIMIT)
}

fn fake_mission_document_with_retry(id: &str, retry_limit: u16) -> MissionDocument {
    calibration_mission_document(
        &reference_observation_scenario(id, None),
        retry_limit,
        FAKE_RECEIPT_TIMEOUT_NS,
    )
    .expect("fake mission document")
}

/// Builds the stored mission document for one trial name and control family.
///
/// The two family variants differ in the physical command that the stimulus
/// requests, so each variant is a separate stored artifact.
pub fn fake_stimulus_mission_document(id: &str, family: ControlFamily) -> MissionDocument {
    calibration_mission_document(
        &fake_stimulus_scenario(id, family),
        0,
        FAKE_RECEIPT_TIMEOUT_NS,
    )
    .expect("fake stimulus mission document")
}

/// Decodes the authored scenario that one stimulus mission projects.
///
/// The rig writes the scenario as schema bytes rather than as constructed
/// values, so the fixture also pins the encoded stimulus shape. Crates that
/// include this rig reach the scenario codec through `flight-tune`.
/// The identity of the envelope the operator-velocity fake stimulus carries.
///
/// A scoped response target row names the envelope its limits are written
/// for, so a test stage that scopes a stimulus mission has to name this one.
pub fn fake_operator_envelope_digest() -> Digest {
    let document =
        fake_stimulus_mission_document(FAKE_MISSION_IDS[0], ControlFamily::OperatorVelocity);
    for phase in &document.phases {
        if let flight_tune::MissionAction::Trial(flight_tune::TrialAction::Stimulate {
            envelope,
            ..
        }) = &phase.action
        {
            let digest = envelope
                .canonical_digest()
                .expect("fake stimulus envelope digest");
            return Digest::from_bytes(*digest.as_bytes());
        }
    }
    panic!("the fake stimulus mission commands no stimulus");
}

fn fake_stimulus_scenario(id: &str, family: ControlFamily) -> TrialScenario {
    let (capability, mapping, envelope) = match family {
        ControlFamily::OperatorVelocity => (
            "operator_velocity_control",
            "candidate_bound_curve",
            r#"{"id":"fake.operator.roll","revision":1,"unit":"meters_per_second",
                "reference":"zero","negative_endpoint":-3.0,"neutral":0.0,
                "positive_endpoint":3.0}"#,
        ),
        ControlFamily::DirectAttitudeThrust => (
            "direct_attitude_thrust_control",
            "affine_exact",
            r#"{"id":"fake.direct.roll","revision":1,"unit":"radians",
                "reference":"effective_setpoint_at_entry","negative_endpoint":-0.2,
                "neutral":0.0,"positive_endpoint":0.2}"#,
        ),
    };
    let document = format!(
        r#"{{"schema_version":3,"id":"{id}","revision":1,"phases":[
            {{"id":"stimulus","max_sim_time_ns":2000000000,
              "required_capabilities":["simulator_time","{capability}"],
              "entry_conditions":[{{"kind":"always"}}],
              "action":{{"kind":"stimulus","family":"{family_name}","channel":"roll",
                        "mapping":"{mapping}","envelope":{envelope},
                        "waveform":{{"kind":"step","value":0.5}}}},
              "exit_conditions":[{{"kind":"always"}}],"abort_conditions":[]}}]}}"#,
        family_name = family.as_str(),
    );
    TrialScenario::from_json(document.as_bytes()).expect("fake stimulus scenario")
}

#[path = "test_rig/backend.rs"]
mod backend;
#[path = "test_rig/cleanup_fault.rs"]
mod cleanup_fault;
#[path = "test_rig/scoring.rs"]
mod scoring;
#[path = "test_rig/terminal.rs"]
mod terminal;
#[path = "test_rig/terminal_head_poison.rs"]
mod terminal_head_poison;
#[path = "test_rig/terminal_state.rs"]
mod terminal_state;
#[path = "test_rig/vehicle.rs"]
mod vehicle;
#[path = "test_rig/vehicle_state.rs"]
mod vehicle_state;

pub use backend::FakeBackend;
pub use cleanup_fault::FakeCleanupFault;
#[allow(unused_imports)]
pub use scoring::{
    EnvelopeGates, ObservedViews, ParameterSequenceStrategy, QuadraticMetric, SequenceStrategy,
    assert_receipt_error, candidate, stage, stage_with_changed_suite,
    stage_with_changed_training_mission, stage_with_execution_retry_limit,
    stage_with_stimulus_family, two_group_candidate, two_group_stage,
};
#[allow(unused_imports)]
pub use terminal_head_poison::TerminalExternalAction;
#[allow(unused_imports)]
pub use terminal_state::{FakeTerminalReadbackFault, FakeTerminalSealFault, FakeTerminalState};
#[allow(unused_imports)]
pub use vehicle::{FakeFactory, FakeVehicle, FakeVehicleRollback};
pub use vehicle_state::FakeVehicleState;

/// The outer runtime lease an operator holds across every open attempt.
///
/// The lease stands for the ownership proof that lives above the open
/// transaction. A cleanup that reached past its own acquisition would
/// release it, and the next open would then have no runtime to talk to.
#[derive(Debug, Default)]
pub struct FakeRuntimeLease {
    pub acquisitions: usize,
    pub held: bool,
}

impl FakeRuntimeLease {
    pub fn acquired() -> Self {
        Self {
            acquisitions: 1,
            held: true,
        }
    }
}

#[derive(Debug, Default)]
pub struct FakeState {
    pub vehicle: FakeVehicleState,
    pub terminal: FakeTerminalState,
    /// Every acquisition and release of the open path, in order.
    pub open_order: Vec<String>,
    /// Whether the simulator holds a session the backend has to close.
    pub session_open: bool,
    /// Every session close the backend has answered.
    pub session_close_count: usize,
    /// The session close at this one-based count fails.
    pub fail_session_close_on: Option<usize>,
    /// The session receipt names another airframe.
    pub bad_session_receipt: bool,
    /// The operator-owned runtime lease, when a test arms one.
    pub runtime_lease: Option<FakeRuntimeLease>,
    pub open_session_count: usize,
    pub prepare_count: usize,
    pub start_count: usize,
    pub sample_count: usize,
    pub sample_poll_count: usize,
    pub stop_count: usize,
    pub cleanup_count: usize,
    pub scenario_action_stop_count: usize,
    pub scenario_action_cleanup_count: usize,
    pub scenario_action_start_count: usize,
    pub scenario_action_observe_count: usize,
    pub metric_observe_count: usize,
    pub gate_begin_count: usize,
    pub gate_evaluate_count: usize,
    pub gate_finish_count: usize,
    pub gate_cancel_count: usize,
    pub metric_begin_count: usize,
    pub metric_finish_count: usize,
    pub metric_cancel_count: usize,
    pub scenario_runs: Vec<(String, u64, f64)>,
    pub transition: FakeTransitionState,
    pub lifecycle: Vec<String>,
    pub current_scenario: Option<MissionReference>,
    pub current_seed: u64,
    pub next_sequence: u64,
    pub panic_on_prepare: Option<usize>,
    pub panic_on_start: Option<usize>,
    /// Every simulator start at or below this one-based count fails execution.
    pub fail_starts_through: usize,
    pub expected_head_event_on_stop: Option<(PathBuf, String)>,
    pub cleanup_fault: FakeCleanupFault,
    pub change_head_on_prepare: Option<PathBuf>,
    pub change_head_on_action_prepare: Option<PathBuf>,
    pub change_head_on_sample: Option<PathBuf>,
    pub bad_scenario_readback: bool,
    pub bad_mission_content: bool,
    pub bad_mission_revision: bool,
    pub timeout_next_sample: bool,
    pub complete_without_sample: bool,
}

#[derive(Debug, Default)]
pub struct FakeTransitionState {
    pub authorization_count: usize,
    pub checks: Vec<(f64, f64)>,
    pub maximum_delta: Option<f64>,
    pub prepared_contexts: Vec<RunExecutionContext>,
    pub started_contexts: Vec<RunExecutionContext>,
    pub vehicle_contexts: Vec<RunExecutionContext>,
    pub bad_preparation_intent: bool,
    pub bad_start_intent: bool,
    pub bad_vehicle_intent: bool,
}

#[derive(Clone)]
pub struct FakeHandle(pub Rc<RefCell<FakeState>>);

impl FakeHandle {
    pub fn new() -> Self {
        Self(Rc::new(RefCell::new(FakeState::default())))
    }
}

fn identity(id: &str, content: &str) -> ArtifactIdentity {
    ArtifactIdentity::from_text(id, content).expect("artifact identity")
}
pub struct TestDirectory {
    path: PathBuf,
}

impl TestDirectory {
    pub fn new(label: &str) -> Self {
        let time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos();
        let path = test_root().join(format!("flight-tune-{label}-{}-{time}", std::process::id()));
        Self { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

fn test_root() -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        PathBuf::from("/private/tmp")
    }
    #[cfg(not(target_os = "macos"))]
    {
        std::env::temp_dir()
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.path).ok();
    }
}
