#![allow(clippy::expect_used, clippy::panic)]

use super::*;
use crate::{
    ActuatorState, AdapterDisposition, ArtifactIdentity, BackendCapability, CausalStage,
    ClockDomain, ClockMapping, ClockMappingQuality, ClockReading, ConditionState, ControlAxes,
    ControlEventId, ControlStage, ControlValue, HealthState, KinematicState, LifecycleObservation,
    LifecycleState, MAX_MANIFEST_BYTES, MAX_RAW_AXES, MissingReason, Observed, Phase, PhaseAction,
    PhaseCondition, Quaternion, RUN_IDENTITY_SCHEMA_VERSION, RawInput, ReferenceFrame, RunIdentity,
    SCENARIO_SCHEMA_VERSION, SampleTime, ScenarioIdentity, SimulatorTruthEvidence, SourceStamp,
    StageProducerRole, StageStamp, TRIAL_MANIFEST_SCHEMA_VERSION, TRIAL_SAMPLE_SCHEMA_VERSION,
    Vector3,
};

fn digest(byte: u8) -> Digest {
    Digest::from_bytes([byte; 32])
}

fn artifact(id: &str, byte: u8) -> ArtifactIdentity {
    ArtifactIdentity {
        id: id.to_owned(),
        revision: "r1".to_owned(),
        digest: digest(byte),
    }
}

fn backend() -> BackendCapabilities {
    BackendCapabilities {
        schema_version: crate::BACKEND_CAPABILITIES_SCHEMA_VERSION,
        backend: artifact("xplane-backend", 1),
        capabilities: vec![
            BackendCapability::SimulatorTime,
            BackendCapability::LifecycleState,
            BackendCapability::ContactState,
            BackendCapability::KinematicTruth,
            BackendCapability::ConditionControl,
            BackendCapability::WindControl,
            BackendCapability::TurbulenceControl,
        ],
        hover_estimator_mode: crate::HoverEstimatorMode::Online,
    }
}

fn scenario() -> Scenario {
    Scenario {
        schema_version: SCENARIO_SCHEMA_VERSION,
        id: "feel-release".to_owned(),
        revision: 4,
        phases: vec![Phase {
            id: "observe-release".to_owned(),
            max_sim_time_ns: 2_000_000_000,
            required_capabilities: vec![
                BackendCapability::SimulatorTime,
                BackendCapability::LifecycleState,
                BackendCapability::ContactState,
            ],
            entry_conditions: vec![PhaseCondition::Lifecycle {
                state: LifecycleState::Armed,
            }],
            action: PhaseAction::Observe,
            exit_conditions: vec![PhaseCondition::Lifecycle {
                state: LifecycleState::Ready,
            }],
            abort_conditions: vec![PhaseCondition::Crashed { expected: true }],
        }],
    }
}

fn run_identity(backend: &BackendCapabilities, scenario: &Scenario) -> RunIdentity {
    RunIdentity {
        schema_version: RUN_IDENTITY_SCHEMA_VERSION,
        run_id: "run-0001".to_owned(),
        code_build: artifact("pilotage", 2),
        vehicle_adapter: artifact("aviate-adapter", 3),
        adapter_capabilities_digest: digest(4),
        backend_capabilities_digest: backend.canonical_digest().expect("backend digest"),
        device_profile: artifact("gamepad", 5),
        control_scheme: artifact("scheme", 6),
        control_feel_candidate: artifact("feel-a", 7),
        flight_controller_candidate: artifact("fc-a", 8),
        simulator_backend: backend.backend.clone(),
        simulator: artifact("x-plane", 9),
        vehicle_model: artifact("quadrotor", 10),
        condition_set: artifact("wind-5mps", 11),
        scenario: ScenarioIdentity {
            id: scenario.id.clone(),
            revision: scenario.revision,
            digest: scenario.canonical_digest().expect("scenario digest"),
        },
        seed: 42,
        repetition: 1,
        clock_mappings: [
            ClockDomain::Device,
            ClockDomain::Client,
            ClockDomain::Adapter,
            ClockDomain::FlightController,
            ClockDomain::Simulator,
        ]
        .map(|from| ClockMapping {
            from,
            to: ClockDomain::Recorder,
            source_epoch: 1,
            source_anchor_ns: 0,
            recorder_anchor_ns: recorder_anchor(from),
            rate_numerator: 1,
            rate_denominator: 1,
            valid_from_source_ns: 0,
            valid_until_source_ns: 10_000_000_000,
            uncertainty_ns: 0,
            quality: ClockMappingQuality::Exact,
        })
        .into(),
    }
}

fn recorder_anchor(domain: ClockDomain) -> u64 {
    match domain {
        ClockDomain::Device => 3,
        ClockDomain::Client => 2,
        ClockDomain::Recorder => 0,
        ClockDomain::Adapter | ClockDomain::FlightController => 1,
        ClockDomain::Simulator => 50,
    }
}

fn manifest() -> TrialManifest {
    let backend = backend();
    let scenario = scenario();
    TrialManifest {
        schema_version: TRIAL_MANIFEST_SCHEMA_VERSION,
        run: run_identity(&backend, &scenario),
        backend,
        scenario,
    }
}

fn vector(value: f64) -> Vector3 {
    Vector3 {
        x: value,
        y: value + 1.0,
        z: value + 2.0,
    }
}

fn kinematic_state() -> KinematicState {
    KinematicState {
        position_m: Observed::present(vector(1.0)),
        velocity_mps: Observed::present(vector(2.0)),
        acceleration_mps2: Observed::present(vector(3.0)),
        attitude: Observed::present(Quaternion {
            w: 1.0,
            x: 0.0,
            y: 0.0,
            z: 0.0,
        }),
        body_rates_rad_s: Observed::present(vector(0.1)),
    }
}

fn simulator_truth() -> SimulatorTruthEvidence {
    SimulatorTruthEvidence {
        position_m: Observed::present(vector(1.0)),
        velocity_mps: Observed::present(vector(2.0)),
        acceleration_mps2: Observed::present(vector(3.0)),
        attitude: Observed::present(Quaternion {
            w: 1.0,
            x: 0.0,
            y: 0.0,
            z: 0.0,
        }),
        body_rates_rad_s: Observed::present(vector(0.1)),
    }
}

fn sample_time(recorder_ns: u64, simulator_ns: u64) -> SampleTime {
    SampleTime {
        recorder_monotonic_ns: recorder_ns,
        device: clock_reading(recorder_ns.saturating_sub(3)),
        client: clock_reading(recorder_ns.saturating_sub(2)),
        adapter: clock_reading(recorder_ns.saturating_sub(1)),
        flight_controller: clock_reading(recorder_ns.saturating_sub(1)),
        simulator: clock_reading(simulator_ns),
        clock_discontinuities: Vec::new(),
    }
}

fn clock_reading(time_ns: u64) -> Observed<ClockReading> {
    Observed::present(ClockReading { epoch: 1, time_ns })
}

fn stage<T>(
    producer: StageProducerRole,
    clock: ClockDomain,
    sequence: u64,
    source_ns: u64,
    recorder_ns: u64,
    predecessor: Option<ControlEventId>,
    value: T,
) -> CausalStage<T> {
    CausalStage::present(
        StageStamp {
            source: SourceStamp {
                producer,
                clock,
                epoch: 1,
                sequence,
                time_ns: Observed::present(source_ns),
            },
            predecessor,
            recorder_receive_ns: recorder_ns,
            recorder_apply_ns: recorder_ns,
        },
        value,
    )
}

fn event(stage: ControlStage, clock: ClockDomain, sequence: u64) -> ControlEventId {
    ControlEventId {
        stage,
        clock,
        epoch: 1,
        sequence,
    }
}

fn raw_input_stage(sequence: u64, recorder_ns: u64) -> CausalStage<RawInput> {
    stage(
        StageProducerRole::InputCapture,
        ClockDomain::Device,
        sequence,
        recorder_ns.saturating_sub(13),
        recorder_ns.saturating_sub(10),
        None,
        RawInput {
            axes: vec![0.2, -0.1],
            buttons: vec![false, true],
        },
    )
}

fn client_axes_stage(
    sequence: u64,
    recorder_ns: u64,
    axes: ControlAxes,
) -> CausalStage<ControlAxes> {
    stage(
        StageProducerRole::ControlClient,
        ClockDomain::Client,
        sequence,
        recorder_ns.saturating_sub(10),
        recorder_ns.saturating_sub(8),
        Some(event(ControlStage::RawInput, ClockDomain::Device, sequence)),
        axes,
    )
}

fn client_control_stage(
    sequence: u64,
    recorder_ns: u64,
    predecessor_stage: ControlStage,
    axes: ControlAxes,
) -> CausalStage<ControlValue> {
    let (source_delta, event_delta) = match predecessor_stage {
        ControlStage::NormalizedControl => (8, 6),
        ControlStage::TypedIntent => (6, 4),
        _ => (2, 2),
    };
    stage(
        StageProducerRole::ControlClient,
        ClockDomain::Client,
        sequence,
        recorder_ns.saturating_sub(source_delta),
        recorder_ns.saturating_sub(event_delta),
        Some(event(predecessor_stage, ClockDomain::Client, sequence)),
        ControlValue::Axes { axes },
    )
}

fn transmitted_stage(sequence: u64, recorder_ns: u64) -> CausalStage<ControlValue> {
    stage(
        StageProducerRole::VehicleAdapter,
        ClockDomain::Adapter,
        sequence,
        recorder_ns.saturating_sub(3),
        recorder_ns.saturating_sub(2),
        Some(event(
            ControlStage::AdapterDemand,
            ClockDomain::Client,
            sequence,
        )),
        ControlValue::Velocity {
            frame: ReferenceFrame::LocalNed,
            linear_mps: vector(0.2),
            yaw_rate_rad_s: 0.0,
        },
    )
}

fn estimate_stage(sequence: u64, recorder_ns: u64) -> CausalStage<KinematicState> {
    stage(
        StageProducerRole::FlightController,
        ClockDomain::FlightController,
        sequence,
        recorder_ns.saturating_sub(1),
        recorder_ns,
        None,
        kinematic_state(),
    )
}

fn truth_stage(sequence: u64, recorder_ns: u64) -> CausalStage<SimulatorTruthEvidence> {
    stage(
        StageProducerRole::SimulatorBackend,
        ClockDomain::Simulator,
        sequence,
        recorder_ns.saturating_sub(50),
        recorder_ns,
        None,
        simulator_truth(),
    )
}

fn sample(manifest: &TrialManifest, sequence: u64, recorder_ns: u64) -> TrialSample {
    let axes = ControlAxes {
        roll: 0.2,
        pitch: -0.1,
        vertical: 0.3,
        yaw: 0.0,
    };
    TrialSample {
        schema_version: TRIAL_SAMPLE_SCHEMA_VERSION,
        run_digest: manifest
            .run
            .canonical_digest()
            .expect("run identity digest"),
        sequence,
        dropped_before: 0,
        phase_index: 0,
        time: sample_time(recorder_ns, recorder_ns - 50),
        raw_input: raw_input_stage(sequence, recorder_ns),
        normalized_control: client_axes_stage(sequence, recorder_ns, axes.clone()),
        typed_intent: client_control_stage(
            sequence,
            recorder_ns,
            ControlStage::NormalizedControl,
            axes.clone(),
        ),
        adapter_demand: client_control_stage(
            sequence,
            recorder_ns,
            ControlStage::TypedIntent,
            axes,
        ),
        transmitted_setpoint: transmitted_stage(sequence, recorder_ns),
        flight_controller_estimate: estimate_stage(sequence, recorder_ns),
        simulator_truth: truth_stage(sequence, recorder_ns),
        actuator: Observed::present(ActuatorState {
            values: vec![0.2, 0.3, 0.2, 0.3],
            saturated: false,
        }),
        adapter_disposition: Observed::present(AdapterDisposition::Accepted),
        lifecycle: Observed::present(LifecycleObservation {
            state: LifecycleState::Armed,
            ground_contact: false,
            crashed: false,
        }),
        condition_state: Observed::present(ConditionState {
            wind_velocity_ned_mps: Observed::present(vector(5.0)),
            turbulence_rms_mps: Observed::present(0.4),
            values: Vec::new(),
        }),
        link_state: Observed::present(HealthState {
            valid: true,
            detail: None,
        }),
        estimator_state: Observed::present(HealthState {
            valid: true,
            detail: None,
        }),
    }
}

#[test]
fn canonical_digest_does_not_depend_on_json_format() {
    let manifest = manifest();
    let compact = manifest.to_canonical_json().expect("compact manifest");
    let pretty = serde_json::to_vec_pretty(&manifest).expect("pretty manifest");
    let compact_value = TrialManifest::from_json(&compact).expect("compact parse");
    let pretty_value = TrialManifest::from_json(&pretty).expect("pretty parse");

    assert_eq!(
        compact_value.canonical_digest().expect("compact digest"),
        pretty_value.canonical_digest().expect("pretty digest")
    );
}

#[test]
fn unsupported_backend_capability_fails_before_a_run() {
    let mut manifest = manifest();
    manifest
        .backend
        .capabilities
        .retain(|item| *item != BackendCapability::LifecycleState);

    assert!(matches!(
        manifest.validate(),
        Err(CodecError::Validation(
            ValidationError::UnsupportedCapability { .. }
        ))
    ));
}

#[test]
fn manifest_size_limit_applies_before_json_decode() {
    let bytes = vec![b' '; MAX_MANIFEST_BYTES + 1];

    assert!(matches!(
        TrialManifest::from_json(&bytes),
        Err(CodecError::DocumentTooLarge { .. })
    ));
}

#[test]
fn sample_round_trip_preserves_a_missing_signal_reason() {
    let manifest = manifest();
    let mut sample = sample(&manifest, 0, 1_000);
    sample.raw_input.observation = Observed::missing(
        MissingReason::RecorderLag,
        Some("writer queue full".to_owned()),
    );

    let bytes = sample
        .to_canonical_json_for_run(&manifest.run)
        .expect("sample JSON");
    let decoded = TrialSample::from_json_for_run(&bytes, &manifest.run).expect("sample parse");

    assert_eq!(decoded, sample);
}

#[test]
fn a_sequence_gap_must_match_the_declared_loss() {
    let manifest = manifest();
    let previous = sample(&manifest, 7, 1_000);
    let current = sample(&manifest, 9, 1_100);

    assert!(matches!(
        manifest.validate_sample(Some(&previous), &current),
        Err(CodecError::Validation(ValidationError::SequenceGap {
            expected: 8,
            actual: 9
        }))
    ));
}

#[test]
fn a_discontinuity_must_change_the_source_epoch() {
    let manifest = manifest();
    let previous = sample(&manifest, 0, 1_000);
    let mut current = sample(&manifest, 1, 1_100);

    current
        .time
        .clock_discontinuities
        .push(ClockDomain::Simulator);
    assert!(matches!(
        manifest.validate_sample(Some(&previous), &current),
        Err(CodecError::Validation(
            ValidationError::InvalidClockObservation { .. }
        ))
    ));
}

#[test]
fn raw_input_axis_count_has_a_hard_limit() {
    let manifest = manifest();
    let mut sample = sample(&manifest, 0, 1_000);
    sample.raw_input.observation = Observed::present(RawInput {
        axes: vec![0.0; MAX_RAW_AXES + 1],
        buttons: Vec::new(),
    });

    assert!(matches!(
        sample.validate_local(),
        Err(ValidationError::TooManyItems { .. })
    ));
}

#[test]
fn a_semantic_backend_change_changes_its_digest() {
    let first = backend();
    let mut second = first.clone();
    second.capabilities.push(BackendCapability::Reset);

    assert_ne!(
        first.canonical_digest().expect("first digest"),
        second.canonical_digest().expect("second digest")
    );
}
