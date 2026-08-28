#![allow(clippy::expect_used)]

use std::cell::Cell;
use std::collections::BTreeMap;
use std::rc::Rc;

use pilotage_mission_core::{
    Digest as MissionDigest, EngineStart, EngineState, ExecutionTarget, MissionAction,
    MissionCapability, MissionDirective, NavigationDataIdentity, ReceiptResult, TrialAction,
    WallDeadline,
};
use pilotage_trial::{
    BackendCapability, Comparison, ControlChannel, Phase, PhaseAction, PhaseCondition,
    SCENARIO_SCHEMA_VERSION, Scenario, Waveform,
};

use super::*;
use crate::{
    ArtifactIdentity, AttemptRole, Digest, MissionReference, RunExecutionContext, ScenarioSet,
};

#[path = "../../build_support/scenario_runtime_identity.rs"]
#[allow(dead_code)]
mod source_identity;

#[derive(Default)]
struct ReferenceRuntime {
    identity: Option<ArtifactIdentity>,
    prepared: bool,
    started: bool,
    directives: Vec<MissionDirective>,
    mutations: Option<Rc<Cell<u32>>>,
    fail_start: bool,
    stop_count: u32,
    cleanup_count: u32,
}

impl ReferenceRuntime {
    fn new(identity: ArtifactIdentity) -> Self {
        Self {
            identity: Some(identity),
            ..Self::default()
        }
    }

    fn tracked(identity: ArtifactIdentity, mutations: Rc<Cell<u32>>) -> Self {
        Self {
            identity: Some(identity),
            mutations: Some(mutations),
            ..Self::default()
        }
    }
}

impl ScenarioRuntime for ReferenceRuntime {
    fn identity(&self) -> &ArtifactIdentity {
        self.identity.as_ref().expect("runtime identity")
    }

    fn capabilities(&self) -> &[MissionCapability] {
        &[MissionCapability::SimulatorTime]
    }

    fn prepare_blocking(
        &mut self,
        _document: &pilotage_mission_core::MissionDocument,
        _context: &RunExecutionContext,
    ) -> Result<(), ScenarioRuntimeError> {
        if let Some(mutations) = &self.mutations {
            mutations.set(mutations.get().wrapping_add(1));
        }
        self.prepared = true;
        Ok(())
    }

    fn start_blocking(&mut self) -> Result<(), ScenarioRuntimeError> {
        assert!(self.prepared);
        if self.fail_start {
            return Err(ScenarioRuntimeError::action_port(
                "start",
                "the test start failed",
            ));
        }
        self.started = true;
        Ok(())
    }

    fn observe_blocking(
        &mut self,
        frame: &ScenarioFrame,
        directive: Option<&MissionDirective>,
    ) -> Result<ScenarioObservationReceipt, ScenarioRuntimeError> {
        assert!(self.started);
        if let Some(directive) = directive {
            self.directives.push(directive.clone());
        }
        Ok(ScenarioObservationReceipt {
            source_sequence: frame.source_sequence,
            action_result: directive.map(|_| ReceiptResult::Succeeded {}),
        })
    }

    fn stop_blocking(
        &mut self,
        _context: &mut ScenarioStopContext,
    ) -> Result<(), ScenarioRuntimeError> {
        self.stop_count = self.stop_count.wrapping_add(1);
        Ok(())
    }

    fn cleanup_blocking(&mut self) -> Result<(), ScenarioRuntimeError> {
        self.cleanup_count = self.cleanup_count.wrapping_add(1);
        Ok(())
    }
}

#[test]
fn campaign_host_uses_the_shared_mission_engine() {
    let scenario = one_phase_scenario();
    let document = mission_document_from_scenario(&scenario, navigation(), 0, 1_000_000)
        .expect("project scenario");
    let identity =
        ArtifactIdentity::new("runtime", Digest::from_bytes([9; 32])).expect("runtime identity");
    let start = start(&document);
    let mut action_port = ReferenceRuntime::new(identity.clone());
    let mut runtime = CampaignMissionRuntime::start_blocking(
        document,
        start,
        &identity,
        &mut action_port,
        &context(),
    )
    .expect("start campaign runtime");

    let first = runtime
        .advance_blocking(&mut action_port, &frame(0), 1)
        .expect("enter phase");
    assert_eq!(first.directives.len(), 1);
    assert!(matches!(first.state, EngineState::Terminal { .. }));
    assert!(matches!(
        runtime.state(),
        Some(EngineState::Terminal { .. })
    ));
}

#[test]
fn identity_mismatch_precedes_action_port_mutation() {
    let scenario = one_phase_scenario();
    let document = mission_document_from_scenario(&scenario, navigation(), 0, 1_000_000)
        .expect("project scenario");
    let actual =
        ArtifactIdentity::new("runtime", Digest::from_bytes([9; 32])).expect("runtime identity");
    let expected =
        ArtifactIdentity::new("runtime", Digest::from_bytes([8; 32])).expect("runtime identity");
    let mutations = Rc::new(Cell::new(0));
    let mut action_port = ReferenceRuntime::tracked(actual, mutations.clone());
    let result = CampaignMissionRuntime::start_blocking(
        document.clone(),
        start(&document),
        &expected,
        &mut action_port,
        &context(),
    );

    assert!(matches!(
        result,
        Err(ScenarioRuntimeError::IdentityMismatch)
    ));
    assert_eq!(mutations.get(), 0);
}

#[test]
fn unsupported_capability_precedes_action_port_mutation() {
    let mut scenario = one_phase_scenario();
    scenario.phases[0].action = PhaseAction::Stimulus {
        channel: ControlChannel::Roll,
        waveform: Waveform::Pulse {
            value: 0.25,
            duration_ns: 1,
        },
    };
    let document = mission_document_from_scenario(&scenario, navigation(), 0, 1_000_000)
        .expect("project scenario");
    let identity =
        ArtifactIdentity::new("runtime", Digest::from_bytes([9; 32])).expect("runtime identity");
    let mutations = Rc::new(Cell::new(0));
    let mut action_port = ReferenceRuntime::tracked(identity.clone(), mutations.clone());

    let result = CampaignMissionRuntime::start_blocking(
        document.clone(),
        start(&document),
        &identity,
        &mut action_port,
        &context(),
    );

    assert!(matches!(
        result,
        Err(ScenarioRuntimeError::MissingCapability {
            capability: MissionCapability::SimulatorControl,
            ..
        })
    ));
    assert_eq!(mutations.get(), 0);
}

#[test]
fn uncertain_start_failure_stops_and_cleans_the_action_port() {
    let scenario = one_phase_scenario();
    let document = mission_document_from_scenario(&scenario, navigation(), 0, 1_000_000)
        .expect("project scenario");
    let identity =
        ArtifactIdentity::new("runtime", Digest::from_bytes([9; 32])).expect("runtime identity");
    let mut action_port = ReferenceRuntime::new(identity.clone());
    action_port.fail_start = true;

    let result = CampaignMissionRuntime::start_blocking(
        document.clone(),
        start(&document),
        &identity,
        &mut action_port,
        &context(),
    );

    assert!(matches!(
        result,
        Err(ScenarioRuntimeError::ActionPort { .. })
    ));
    assert_eq!(action_port.stop_count, 1);
    assert_eq!(action_port.cleanup_count, 1);
}

#[test]
fn phase_clock_starts_with_the_first_simulator_frame() {
    let mut scenario = one_phase_scenario();
    scenario.phases[0].max_sim_time_ns = 2;
    let document = mission_document_from_scenario(&scenario, navigation(), 0, 1_000_000)
        .expect("project scenario");
    let identity =
        ArtifactIdentity::new("runtime", Digest::from_bytes([9; 32])).expect("runtime identity");
    let mut action_port = ReferenceRuntime::new(identity.clone());
    let mut runtime = CampaignMissionRuntime::start_blocking(
        document.clone(),
        start(&document),
        &identity,
        &mut action_port,
        &context(),
    )
    .expect("start campaign runtime");
    let mut first = frame(0);
    first.simulator_time_ns = 1_000_000_000;

    let output = runtime
        .advance_blocking(&mut action_port, &first, 1)
        .expect("advance first frame");

    assert!(matches!(output.state, EngineState::Terminal { .. }));
}

#[test]
fn production_identity_changes_for_engine_input_and_ignores_test_input() {
    let base = [
        (
            "crates/pilotage-mission-core/src/engine.rs",
            b"phase".as_slice(),
        ),
        (
            "crates/pilotage-mission-core/src/engine/condition.rs",
            b"condition".as_slice(),
        ),
        (
            "crates/pilotage-mission-core/src/engine/runtime.rs",
            b"timing".as_slice(),
        ),
        (
            "crates/pilotage-mission-core/src/trial.rs",
            b"waveform".as_slice(),
        ),
        (
            "tools/flight-tune/src/scenario_runtime/tests.rs",
            b"first".as_slice(),
        ),
    ];
    let changed_test = [
        (
            "crates/pilotage-mission-core/src/engine.rs",
            b"phase".as_slice(),
        ),
        (
            "crates/pilotage-mission-core/src/engine/condition.rs",
            b"condition".as_slice(),
        ),
        (
            "crates/pilotage-mission-core/src/engine/runtime.rs",
            b"timing".as_slice(),
        ),
        (
            "crates/pilotage-mission-core/src/trial.rs",
            b"waveform".as_slice(),
        ),
        (
            "tools/flight-tune/src/scenario_runtime/tests.rs",
            b"second".as_slice(),
        ),
    ];
    let base_digest = source_identity::digest_named_for_test(&base).expect("base identity");

    assert_eq!(
        base_digest,
        source_identity::digest_named_for_test(&changed_test).expect("test identity")
    );
    for index in 0..4 {
        let mut changed = base;
        changed[index].1 = b"changed";
        assert_ne!(
            base_digest,
            source_identity::digest_named_for_test(&changed).expect("production identity")
        );
    }
}

#[test]
fn projection_adds_the_neutral_simulator_control_capability() {
    let scenario = Scenario {
        schema_version: SCENARIO_SCHEMA_VERSION,
        id: "stimulus-scenario".to_owned(),
        revision: 1,
        phases: vec![Phase {
            id: "stimulus".to_owned(),
            max_sim_time_ns: 2_000_000,
            required_capabilities: vec![BackendCapability::SimulatorTime],
            entry_conditions: vec![PhaseCondition::Always],
            action: PhaseAction::Stimulus {
                channel: ControlChannel::Roll,
                waveform: Waveform::Pulse {
                    value: 0.25,
                    duration_ns: 1_000_000,
                },
            },
            exit_conditions: vec![PhaseCondition::SimulatorTime {
                comparison: Comparison::GreaterOrEqual,
                value_ns: 1_000_000,
            }],
            abort_conditions: Vec::new(),
        }],
    };

    let document = mission_document_from_scenario(&scenario, navigation(), 0, 1_000_000)
        .expect("project stimulus");

    assert!(
        document.phases[0]
            .required_capabilities
            .contains(&MissionCapability::SimulatorControl)
    );
    assert!(matches!(
        document.phases[0].action,
        MissionAction::Trial(TrialAction::Stimulate { .. })
    ));
}

fn one_phase_scenario() -> Scenario {
    Scenario {
        schema_version: SCENARIO_SCHEMA_VERSION,
        id: "reference-scenario".to_owned(),
        revision: 1,
        phases: vec![Phase {
            id: "observe".to_owned(),
            max_sim_time_ns: 1_000_000,
            required_capabilities: vec![BackendCapability::SimulatorTime],
            entry_conditions: vec![PhaseCondition::Always],
            action: PhaseAction::Observe,
            exit_conditions: vec![PhaseCondition::Always],
            abort_conditions: Vec::new(),
        }],
    }
}

fn navigation() -> NavigationDataIdentity {
    NavigationDataIdentity {
        cycle: "none".to_owned(),
        snapshot_id: "calibration".to_owned(),
        snapshot_digest: MissionDigest::from_bytes([4; 32]),
    }
}

fn start(document: &pilotage_mission_core::MissionDocument) -> EngineStart {
    EngineStart {
        host_target: ExecutionTarget::Simulator,
        simulator_time_ns: 0,
        wall_time_ns: 0,
        wall_deadline: WallDeadline {
            mission_content_digest: document.identity.content_digest,
            expires_at_ns: 10_000_000,
        },
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
            revision_id: "reference-scenario".to_owned(),
            schema_version: flight_tune::MISSION_SCHEMA_VERSION,
            content_digest: Digest::from_bytes([3; 32]),
            max_samples: 2,
            sample_timeout_ns: 10_000_000,
        },
        0,
        4,
    )
    .expect("run context")
}

fn frame(sequence: u64) -> ScenarioFrame {
    ScenarioFrame {
        source_sequence: sequence,
        simulator_time_ns: sequence,
        trial_time_ns: sequence,
        lifecycle: None,
        ground_contact: Some(false),
        crashed: Some(false),
        link_valid: Some(true),
        estimator_valid: Some(true),
        truth: KinematicTruth {
            position_ned_m: [0.0; 3],
            velocity_ned_mps: [0.0; 3],
            acceleration_ned_mps2: [0.0; 3],
            attitude_wxyz: [1.0, 0.0, 0.0, 0.0],
            body_rates_rps: [0.0; 3],
        },
        applied_conditions: BTreeMap::new(),
        canonical_signals: Vec::new(),
    }
}
