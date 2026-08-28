#![allow(clippy::expect_used)]

use pilotage_mission_core::{
    Digest as MissionDigest, EngineStart, ExecutionTarget, MissionCapability, MissionDirective,
    NavigationDataIdentity, ReceiptResult, WallDeadline,
};
use pilotage_trial::{
    BackendCapability, Phase, PhaseAction, PhaseCondition, SCENARIO_SCHEMA_VERSION, Scenario,
};

use super::*;
use flight_tune::{
    ArtifactIdentity, AttemptRole, CampaignMissionRuntime, Digest, RunExecutionContext,
    ScenarioObservationReceipt, ScenarioRef, ScenarioRuntime, ScenarioRuntimeError, ScenarioSet,
    ScenarioStopContext, mission_document_from_scenario,
};

struct RecordingRuntime {
    identity: ArtifactIdentity,
}

#[derive(Default)]
struct RecordingSimulatorActions {
    actions: Vec<XPlaneSimulatorAction>,
}

impl XPlaneSimulatorActionDriver for RecordingSimulatorActions {
    fn capabilities(&self) -> &[MissionCapability] {
        &[MissionCapability::Reset]
    }

    fn execute_blocking(
        &mut self,
        _frame: &ScenarioFrame,
        action: &XPlaneSimulatorAction,
    ) -> Result<ReceiptResult, ScenarioRuntimeError> {
        self.actions.push(action.clone());
        Ok(ReceiptResult::Succeeded {})
    }
}

impl ScenarioRuntime for RecordingRuntime {
    fn identity(&self) -> &ArtifactIdentity {
        &self.identity
    }

    fn capabilities(&self) -> &[MissionCapability] {
        &[MissionCapability::SimulatorTime]
    }

    fn prepare_blocking(
        &mut self,
        _document: &pilotage_mission_core::MissionDocument,
        _context: &RunExecutionContext,
    ) -> Result<(), ScenarioRuntimeError> {
        Ok(())
    }

    fn start_blocking(&mut self) -> Result<(), ScenarioRuntimeError> {
        Ok(())
    }

    fn observe_blocking(
        &mut self,
        frame: &ScenarioFrame,
        directive: Option<&MissionDirective>,
    ) -> Result<ScenarioObservationReceipt, ScenarioRuntimeError> {
        Ok(ScenarioObservationReceipt {
            source_sequence: frame.source_sequence,
            action_result: directive.map(|_| ReceiptResult::Succeeded {}),
        })
    }

    fn stop_blocking(
        &mut self,
        _context: &mut ScenarioStopContext,
    ) -> Result<(), ScenarioRuntimeError> {
        Ok(())
    }

    fn cleanup_blocking(&mut self) -> Result<(), ScenarioRuntimeError> {
        Ok(())
    }
}

#[test]
fn reference_and_xplane_frames_produce_the_same_directives() {
    let scenario = Scenario {
        schema_version: SCENARIO_SCHEMA_VERSION,
        id: "projection-conformance".to_owned(),
        revision: 1,
        phases: vec![Phase {
            id: "observe".to_owned(),
            max_sim_time_ns: 2_000_000_000,
            required_capabilities: vec![BackendCapability::SimulatorTime],
            entry_conditions: vec![PhaseCondition::Always],
            action: PhaseAction::Observe,
            exit_conditions: vec![PhaseCondition::Always],
            abort_conditions: Vec::new(),
        }],
    };
    let document = mission_document_from_scenario(&scenario, navigation(), 0, 1_000_000_000)
        .expect("mission document");
    let identity = ArtifactIdentity::new(
        "conformance-runtime",
        flight_tune::Digest::from_bytes([7; 32]),
    )
    .expect("runtime identity");
    let mut reference_port = RecordingRuntime {
        identity: identity.clone(),
    };
    let mut reference = CampaignMissionRuntime::start_blocking(
        document.clone(),
        start(&document),
        &identity,
        &mut reference_port,
        &context(),
    )
    .expect("reference runtime");
    let mut xplane_port = RecordingRuntime {
        identity: identity.clone(),
    };
    let mut xplane = CampaignMissionRuntime::start_blocking(
        document.clone(),
        start(&document),
        &identity,
        &mut xplane_port,
        &context(),
    )
    .expect("X-Plane runtime");
    let sample = sample(0);
    let mut projection = XPlaneFrameProjection::new();
    let projected = projection
        .project(&sample, VehicleFrameValues::default())
        .expect("project frame");
    let direct = reference_frame(&sample);

    let reference_output = reference
        .advance_blocking(&mut reference_port, &direct, 1)
        .expect("reference tick");
    let xplane_output = xplane
        .advance_blocking(&mut xplane_port, &projected, 1)
        .expect("X-Plane tick");

    assert_eq!(reference_output, xplane_output);
    assert_eq!(reference_output.directives.len(), 1);
}

#[test]
fn xplane_dispatches_reset_without_sending_it_to_the_vehicle_port() {
    let identity =
        ArtifactIdentity::new("runtime", Digest::from_bytes([7; 32])).expect("runtime identity");
    let vehicle = RecordingRuntime { identity };
    let mut runtime = XPlaneScenarioRuntime::new(RecordingSimulatorActions::default(), vehicle);
    let directive: MissionDirective = serde_json::from_value(serde_json::json!({
        "lane": "trial",
        "directive": {
            "context": {
                "action_id": 1,
                "phase_index": 0,
                "phase_id": "reset",
                "attempt": 1,
                "purpose": { "purpose": "phase_action" }
            },
            "action": { "kind": "reset" }
        }
    }))
    .expect("reset directive");

    let receipt = runtime
        .observe_blocking(&reference_frame(&sample(0)), Some(&directive))
        .expect("dispatch reset");
    let (simulator, _vehicle) = runtime.into_inner();

    assert_eq!(receipt.action_result, Some(ReceiptResult::Succeeded {}));
    assert_eq!(simulator.actions, vec![XPlaneSimulatorAction::Reset]);
}

#[test]
fn projection_latches_the_first_truth_position_as_the_run_origin() {
    let mut projection = XPlaneFrameProjection::new();
    let first = projection
        .project(&sample(0), VehicleFrameValues::default())
        .expect("first frame");
    let second = projection
        .project(&sample(1), VehicleFrameValues::default())
        .expect("second frame");

    assert_eq!(first.truth.position_ned_m, [0.0; 3]);
    assert_eq!(second.truth.position_ned_m, [-1.0, 1.0, -1.0]);
}

#[test]
fn rejected_frame_does_not_latch_the_run_origin() {
    let mut projection = XPlaneFrameProjection::new();
    let mut invalid = sample(0);
    invalid.local_velocity_mps[0] = f64::NAN;

    assert!(matches!(
        projection.project(&invalid, VehicleFrameValues::default()),
        Err(XPlaneProjectionError::InvalidKinematics)
    ));
    let accepted = projection
        .project(&sample(1), VehicleFrameValues::default())
        .expect("valid frame");

    assert_eq!(accepted.truth.position_ned_m, [0.0; 3]);
}

fn sample(sequence: u64) -> XPlaneTruthSample {
    XPlaneTruthSample {
        generation: 1,
        sequence,
        sim_time_s: 1.0 + sequence as f64 * 0.02,
        trial_time_s: sequence as f64 * 0.02,
        reset_generation: 1,
        local_position_m: [
            2.0 + sequence as f64,
            3.0 + sequence as f64,
            5.0 + sequence as f64,
        ],
        local_velocity_mps: [0.0; 3],
        local_acceleration_mps2: [0.0; 3],
        body_specific_force_g: [0.0; 3],
        quaternion: [1.0, 0.0, 0.0, 0.0],
        body_rates_rps: [0.0; 3],
        on_ground: Some(false),
        crashed: Some(false),
        wind_speed_mps: 0.0,
        wind_direction_deg: 0.0,
    }
}

fn navigation() -> NavigationDataIdentity {
    NavigationDataIdentity {
        cycle: "none".to_owned(),
        snapshot_id: "calibration".to_owned(),
        snapshot_digest: MissionDigest::from_bytes([4; 32]),
    }
}

fn reference_frame(sample: &XPlaneTruthSample) -> ScenarioFrame {
    ScenarioFrame {
        source_sequence: sample.sequence,
        simulator_time_ns: 1_000_000_000,
        trial_time_ns: 0,
        lifecycle: None,
        ground_contact: Some(false),
        crashed: Some(false),
        link_valid: None,
        estimator_valid: None,
        truth: KinematicTruth {
            position_ned_m: [0.0; 3],
            velocity_ned_mps: [0.0; 3],
            acceleration_ned_mps2: [0.0; 3],
            attitude_wxyz: sample.quaternion,
            body_rates_rps: sample.body_rates_rps,
        },
        applied_conditions: BTreeMap::new(),
        canonical_signals: Vec::new(),
    }
}

fn context() -> RunExecutionContext {
    RunExecutionContext::new(
        Digest::from_bytes([1; 32]),
        1,
        AttemptRole::TrainingBaseline,
        Digest::from_bytes([2; 32]),
        None,
        ScenarioSet::Training,
        &ScenarioRef {
            id: "projection-conformance".to_owned(),
            digest: Digest::from_bytes([3; 32]),
            max_samples: 2,
            sample_timeout_ms: 10,
        },
        0,
        4,
    )
    .expect("run context")
}

fn start(document: &pilotage_mission_core::MissionDocument) -> EngineStart {
    EngineStart {
        host_target: ExecutionTarget::Simulator,
        simulator_time_ns: 0,
        wall_time_ns: 0,
        wall_deadline: WallDeadline {
            mission_content_digest: document.identity.content_digest,
            expires_at_ns: 10_000_000_000,
        },
    }
}
