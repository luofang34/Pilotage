#![allow(clippy::expect_used)]

use pilotage_mission_core::{
    Digest as MissionDigest, EngineStart, ExecutionTarget, MissionCapability, MissionDirective,
    NavigationDataIdentity, ReceiptResult, WallDeadline,
};
use pilotage_trial::{
    BackendCapability, ControlChannel, ControlFamily, Phase, PhaseAction, PhaseCondition,
    PhysicalUnit, ReferenceRule, SCENARIO_SCHEMA_VERSION, Scenario, StimulusEnvelope, Waveform,
};

use super::*;
use flight_tune::{
    ArtifactIdentity, AttemptRole, CampaignMissionRuntime, Digest, HoverEstimatorMode,
    MissionReference, RunExecutionContext, ScenarioObservationReceipt, ScenarioRuntime,
    ScenarioRuntimeError, ScenarioSet, ScenarioStopContext, mission_document_from_scenario,
};

struct RecordingRuntime {
    identity: ArtifactIdentity,
    capabilities: Vec<MissionCapability>,
    uncertainty: Vec<BackendCapability>,
    hover_estimator_mode: HoverEstimatorMode,
    prepare_count: u32,
}

impl RecordingRuntime {
    fn new(identity: ArtifactIdentity) -> Self {
        Self {
            identity,
            capabilities: vec![MissionCapability::SimulatorTime],
            uncertainty: Vec::new(),
            hover_estimator_mode: HoverEstimatorMode::Online,
            prepare_count: 0,
        }
    }

    fn with_capability(mut self, capability: MissionCapability) -> Self {
        self.capabilities.push(capability);
        self
    }

    fn with_uncertainty(
        mut self,
        capability: BackendCapability,
        hover_estimator_mode: HoverEstimatorMode,
    ) -> Self {
        self.uncertainty.push(capability);
        self.hover_estimator_mode = hover_estimator_mode;
        self
    }
}

#[derive(Default)]
struct RecordingSimulatorActions {
    actions: Vec<XPlaneSimulatorAction>,
}

impl XPlaneSimulatorActionDriver for RecordingSimulatorActions {
    fn capabilities(&self) -> &[MissionCapability] {
        &[MissionCapability::Reset]
    }

    fn uncertainty_capabilities(&self) -> &[BackendCapability] {
        &[BackendCapability::SensorPerturbation]
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
        &self.capabilities
    }

    fn uncertainty_capabilities(&self) -> &[BackendCapability] {
        &self.uncertainty
    }

    fn hover_estimator_mode(&self) -> HoverEstimatorMode {
        self.hover_estimator_mode
    }

    fn prepare_blocking(
        &mut self,
        _document: &pilotage_mission_core::MissionDocument,
        _context: &RunExecutionContext,
    ) -> Result<(), ScenarioRuntimeError> {
        self.prepare_count = self.prepare_count.wrapping_add(1);
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
    let mut reference_port = RecordingRuntime::new(identity.clone());
    let mut reference = CampaignMissionRuntime::start_blocking(
        document.clone(),
        start(&document),
        &identity,
        &mut reference_port,
        &context(),
    )
    .expect("reference runtime");
    let mut xplane_port = RecordingRuntime::new(identity.clone());
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
    let vehicle = RecordingRuntime::new(identity);
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
fn a_composed_runtime_admits_only_the_control_family_that_it_declares() {
    let identity =
        ArtifactIdentity::new("runtime", Digest::from_bytes([7; 32])).expect("runtime identity");
    let vehicle = RecordingRuntime::new(identity.clone())
        .with_capability(MissionCapability::OperatorVelocityControl);
    let mut runtime = XPlaneScenarioRuntime::new(RecordingSimulatorActions::default(), vehicle);
    let operator = mission_document_from_scenario(
        &stimulus_scenario(ControlFamily::OperatorVelocity),
        navigation(),
        0,
        1_000_000_000,
    )
    .expect("operator document");
    let direct = mission_document_from_scenario(
        &stimulus_scenario(ControlFamily::DirectAttitudeThrust),
        navigation(),
        0,
        1_000_000_000,
    )
    .expect("direct document");

    CampaignMissionRuntime::attest_capabilities(&operator, &runtime)
        .expect("the declared operator family is admitted");
    let refused = CampaignMissionRuntime::start_blocking(
        direct.clone(),
        start(&direct),
        &identity,
        &mut runtime,
        &context(),
    )
    .map(|_| ());

    assert!(matches!(
        refused,
        Err(ScenarioRuntimeError::MissingCapability {
            capability: MissionCapability::DirectAttitudeThrustControl,
            ..
        })
    ));
    let (simulator, vehicle) = runtime.into_inner();
    assert_eq!(vehicle.prepare_count, 0);
    assert!(simulator.actions.is_empty());
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

fn stimulus_scenario(family: ControlFamily) -> Scenario {
    let envelope = match family {
        ControlFamily::OperatorVelocity => StimulusEnvelope {
            id: "conformance.operator.roll".to_owned(),
            revision: 1,
            unit: PhysicalUnit::MetersPerSecond,
            reference: ReferenceRule::Zero,
            negative_endpoint: -3.0,
            neutral: 0.0,
            positive_endpoint: 3.0,
        },
        ControlFamily::DirectAttitudeThrust => StimulusEnvelope {
            id: "conformance.direct.roll".to_owned(),
            revision: 1,
            unit: PhysicalUnit::Radians,
            reference: ReferenceRule::EffectiveSetpointAtEntry,
            negative_endpoint: -0.2,
            neutral: 0.0,
            positive_endpoint: 0.2,
        },
    };
    Scenario {
        schema_version: SCENARIO_SCHEMA_VERSION,
        id: "family-admission".to_owned(),
        revision: 1,
        phases: vec![Phase {
            id: "stimulus".to_owned(),
            max_sim_time_ns: 2_000_000_000,
            required_capabilities: vec![BackendCapability::SimulatorTime, family.capability()],
            entry_conditions: vec![PhaseCondition::Always],
            action: PhaseAction::Stimulus {
                family,
                channel: ControlChannel::Roll,
                mapping: family.mapping(),
                envelope,
                waveform: Waveform::Step { value: 0.4 },
            },
            exit_conditions: vec![PhaseCondition::Always],
            abort_conditions: Vec::new(),
        }],
    }
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
        &MissionReference {
            revision_id: "projection-conformance".to_owned(),
            schema_version: flight_tune::MISSION_SCHEMA_VERSION,
            content_digest: Digest::from_bytes([3; 32]),
            max_samples: 2,
            sample_timeout_ns: 10_000_000,
        },
        0,
        4,
        0,
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

#[test]
fn the_composed_runtime_reads_the_hover_estimator_mode_from_the_vehicle() {
    let identity =
        ArtifactIdentity::new("vehicle", Digest::from_bytes([2; 32])).expect("vehicle identity");
    let online = XPlaneScenarioRuntime::new(
        RecordingSimulatorActions::default(),
        RecordingRuntime::new(identity.clone()),
    );
    let frozen = XPlaneScenarioRuntime::new(
        RecordingSimulatorActions::default(),
        RecordingRuntime::new(identity).with_uncertainty(
            BackendCapability::HoverTrimUncertainty,
            HoverEstimatorMode::Frozen,
        ),
    );

    assert_eq!(online.hover_estimator_mode(), HoverEstimatorMode::Online);
    assert_eq!(frozen.hover_estimator_mode(), HoverEstimatorMode::Frozen);
    assert_eq!(
        online.uncertainty_capabilities(),
        [BackendCapability::SensorPerturbation]
    );
    assert_eq!(
        frozen.uncertainty_capabilities(),
        [
            BackendCapability::HoverTrimUncertainty,
            BackendCapability::SensorPerturbation
        ]
    );
}
