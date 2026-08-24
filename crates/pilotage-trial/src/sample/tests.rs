use std::any::TypeId;

use super::{
    ActuatorState, AdapterDisposition, CausalStage, ClockReading, ConditionState, ControlAxes,
    ControlEventId, ControlStage, ControlValue, HealthState, KinematicState, LifecycleObservation,
    LifecycleState, Observed, Quaternion, RawInput, SampleTime, SimulatorTruthEvidence,
    SourceStamp, StageProducerRole, StageStamp, TrialSample, TrialStreamValidator, Vector3,
};
use crate::{
    ArtifactIdentity, ClockDomain, ClockMapping, ClockMappingQuality, CodecError, Digest,
    MAX_SAMPLE_BYTES, RUN_IDENTITY_SCHEMA_VERSION, RunIdentity, ScenarioIdentity,
    TRIAL_SAMPLE_SCHEMA_VERSION, ValidationError,
};

mod clock;
mod stream;

fn digest(value: u8) -> Digest {
    Digest::from_bytes([value; 32])
}

fn artifact(id: &str, value: u8) -> ArtifactIdentity {
    ArtifactIdentity {
        id: id.to_owned(),
        revision: "1".to_owned(),
        digest: digest(value),
    }
}

fn mapping(from: ClockDomain) -> ClockMapping {
    ClockMapping {
        from,
        to: ClockDomain::Recorder,
        source_epoch: 7,
        source_anchor_ns: 10,
        recorder_anchor_ns: 20,
        rate_numerator: 1,
        rate_denominator: 1,
        valid_from_source_ns: 0,
        valid_until_source_ns: 1_000,
        uncertainty_ns: 1,
        quality: ClockMappingQuality::Estimated,
    }
}

fn run_identity() -> RunIdentity {
    RunIdentity {
        schema_version: RUN_IDENTITY_SCHEMA_VERSION,
        run_id: "run-1".to_owned(),
        code_build: artifact("code", 1),
        vehicle_adapter: artifact("adapter", 2),
        adapter_capabilities_digest: digest(3),
        backend_capabilities_digest: digest(4),
        device_profile: artifact("device", 5),
        control_scheme: artifact("scheme", 6),
        control_feel_candidate: artifact("feel", 7),
        flight_controller_candidate: artifact("controller", 8),
        simulator_backend: artifact("backend", 9),
        simulator: artifact("simulator", 10),
        vehicle_model: artifact("model", 11),
        condition_set: artifact("conditions", 12),
        scenario: ScenarioIdentity {
            id: "scenario".to_owned(),
            revision: 1,
            digest: digest(13),
        },
        seed: 14,
        repetition: 0,
        clock_mappings: [
            ClockDomain::Device,
            ClockDomain::Client,
            ClockDomain::Adapter,
            ClockDomain::FlightController,
            ClockDomain::Simulator,
        ]
        .map(mapping)
        .into(),
    }
}

fn stage<T>(
    producer: StageProducerRole,
    clock: ClockDomain,
    sequence: u64,
    predecessor: Option<ControlEventId>,
    value: T,
) -> CausalStage<T> {
    let offset = sequence.saturating_sub(1).saturating_mul(2);
    let source_time_ns = 10_u64.saturating_add(offset);
    let recorder_event_ns = 21_u64.saturating_add(offset);
    CausalStage::present(
        StageStamp {
            source: SourceStamp {
                producer,
                clock,
                epoch: 7,
                sequence,
                time_ns: Observed::present(source_time_ns),
            },
            predecessor,
            recorder_receive_ns: recorder_event_ns,
            recorder_apply_ns: recorder_event_ns,
        },
        value,
    )
}

fn event(stage: ControlStage, clock: ClockDomain, sequence: u64) -> ControlEventId {
    ControlEventId {
        stage,
        clock,
        epoch: 7,
        sequence,
    }
}

fn raw_input_stage() -> CausalStage<RawInput> {
    stage(
        StageProducerRole::InputCapture,
        ClockDomain::Device,
        1,
        None,
        RawInput {
            axes: vec![0.1],
            buttons: vec![false],
        },
    )
}

fn vector() -> Vector3 {
    Vector3 {
        x: 0.0,
        y: 0.0,
        z: 0.0,
    }
}

fn kinematic_state() -> KinematicState {
    KinematicState {
        position_m: Observed::present(vector()),
        velocity_mps: Observed::present(vector()),
        acceleration_mps2: Observed::present(vector()),
        attitude: Observed::present(Quaternion {
            w: 1.0,
            x: 0.0,
            y: 0.0,
            z: 0.0,
        }),
        body_rates_rad_s: Observed::present(vector()),
    }
}

fn simulator_truth() -> SimulatorTruthEvidence {
    SimulatorTruthEvidence {
        position_m: Observed::present(vector()),
        velocity_mps: Observed::present(vector()),
        acceleration_mps2: Observed::present(vector()),
        attitude: Observed::present(Quaternion {
            w: 1.0,
            x: 0.0,
            y: 0.0,
            z: 0.0,
        }),
        body_rates_rad_s: Observed::present(vector()),
    }
}

fn sample(run: &RunIdentity) -> TrialSample {
    let axes = ControlAxes {
        roll: 0.1,
        pitch: 0.2,
        vertical: 0.3,
        yaw: 0.4,
    };
    TrialSample {
        schema_version: TRIAL_SAMPLE_SCHEMA_VERSION,
        run_digest: run.canonical_digest().expect("run identity digest"),
        sequence: 1,
        dropped_before: 0,
        phase_index: 0,
        time: sample_time(),
        raw_input: raw_input_stage(),
        normalized_control: stage(
            StageProducerRole::ControlClient,
            ClockDomain::Client,
            2,
            Some(event(ControlStage::RawInput, ClockDomain::Device, 1)),
            axes.clone(),
        ),
        typed_intent: stage(
            StageProducerRole::ControlClient,
            ClockDomain::Client,
            3,
            Some(event(
                ControlStage::NormalizedControl,
                ClockDomain::Client,
                2,
            )),
            ControlValue::Axes { axes: axes.clone() },
        ),
        adapter_demand: stage(
            StageProducerRole::ControlClient,
            ClockDomain::Client,
            4,
            Some(event(ControlStage::TypedIntent, ClockDomain::Client, 3)),
            ControlValue::Axes { axes: axes.clone() },
        ),
        transmitted_setpoint: stage(
            StageProducerRole::VehicleAdapter,
            ClockDomain::Adapter,
            5,
            Some(event(ControlStage::AdapterDemand, ClockDomain::Client, 4)),
            ControlValue::Axes { axes },
        ),
        flight_controller_estimate: stage(
            StageProducerRole::FlightController,
            ClockDomain::FlightController,
            6,
            None,
            kinematic_state(),
        ),
        simulator_truth: stage(
            StageProducerRole::SimulatorBackend,
            ClockDomain::Simulator,
            7,
            None,
            simulator_truth(),
        ),
        actuator: Observed::present(ActuatorState {
            values: vec![0.2],
            saturated: false,
        }),
        adapter_disposition: Observed::present(AdapterDisposition::Accepted),
        lifecycle: Observed::present(LifecycleObservation {
            state: LifecycleState::Armed,
            ground_contact: false,
            crashed: false,
        }),
        condition_state: Observed::present(condition_state(0.1)),
        link_state: Observed::present(health()),
        estimator_state: Observed::present(health()),
    }
}

fn sample_time() -> SampleTime {
    SampleTime {
        recorder_monotonic_ns: 40,
        device: clock_reading(30),
        client: clock_reading(30),
        adapter: clock_reading(30),
        flight_controller: clock_reading(30),
        simulator: clock_reading(30),
        clock_discontinuities: Vec::new(),
    }
}

fn clock_reading(time_ns: u64) -> Observed<ClockReading> {
    Observed::present(ClockReading { epoch: 7, time_ns })
}

fn condition_state(turbulence_rms_mps: f64) -> ConditionState {
    ConditionState {
        wind_velocity_ned_mps: Observed::present(vector()),
        turbulence_rms_mps: Observed::present(turbulence_rms_mps),
        values: Vec::new(),
    }
}

fn health() -> HealthState {
    HealthState {
        valid: true,
        detail: None,
    }
}

#[test]
fn sample_verifies_the_public_run_identity_digest() {
    let run = run_identity();
    let sample = sample(&run);
    let bytes = sample
        .to_canonical_json_for_run(&run)
        .expect("canonical sample");

    assert_eq!(
        TrialSample::from_json_for_run(&bytes, &run).expect("sample for run"),
        sample
    );
}

#[test]
fn sample_rejects_a_changed_run_identity() {
    let run = run_identity();
    let sample = sample(&run);
    let mut changed_run = run;
    changed_run.seed = changed_run.seed.wrapping_add(1);

    assert!(matches!(
        sample.validate_for_run(&changed_run),
        Err(CodecError::Validation(ValidationError::IdentityMismatch { field }))
            if field == "sample.run_digest"
    ));
}

#[test]
fn adjacent_samples_reject_mixed_run_identities() {
    let run = run_identity();
    let previous = sample(&run);
    let mut changed_run = run.clone();
    changed_run.seed = changed_run.seed.wrapping_add(1);
    let mut current = sample(&changed_run);
    current.sequence = 2;

    assert!(matches!(
        current.validate_after(&previous, &run),
        Err(CodecError::Validation(ValidationError::MixedRun))
    ));
}

#[test]
fn adjacent_samples_reject_an_undeclared_sequence_gap() {
    let run = run_identity();
    let mut previous = sample(&run);
    previous.sequence = 7;
    let mut current = sample(&run);
    current.sequence = 9;

    assert!(matches!(
        current.validate_after(&previous, &run),
        Err(CodecError::Validation(ValidationError::SequenceGap {
            expected: 8,
            actual: 9,
        }))
    ));
}

#[test]
fn adjacent_samples_accept_an_exact_declared_loss() {
    let run = run_identity();
    let mut previous = sample(&run);
    previous.sequence = 7;
    let mut current = sample(&run);
    current.sequence = 9;
    current.dropped_before = 1;

    assert!(current.validate_after(&previous, &run).is_ok());
}

#[test]
fn simulator_truth_has_a_distinct_evidence_type() {
    assert_ne!(
        TypeId::of::<KinematicState>(),
        TypeId::of::<SimulatorTruthEvidence>()
    );
}

#[test]
fn negative_turbulence_rms_is_rejected() {
    assert!(matches!(
        condition_state(-0.1).validate("condition"),
        Err(ValidationError::OutOfRange { field, .. })
            if field == "condition.turbulence_rms_mps"
    ));
}

#[test]
fn non_unit_quaternion_is_rejected() {
    let quaternion = Quaternion {
        w: 0.0,
        x: 0.0,
        y: 0.0,
        z: 0.0,
    };

    assert!(matches!(
        quaternion.validate("attitude"),
        Err(ValidationError::InvalidQuaternionNorm { field, .. }) if field == "attitude"
    ));
}

#[test]
fn quaternion_accepts_a_small_norm_error() {
    let quaternion = Quaternion {
        w: 1.000_5,
        x: 0.0,
        y: 0.0,
        z: 0.0,
    };

    assert_eq!(quaternion.validate("attitude"), Ok(()));
}

#[test]
fn sample_decode_rejects_an_unknown_field() {
    let run = run_identity();
    let mut value = serde_json::to_value(sample(&run)).expect("sample value");
    value
        .as_object_mut()
        .expect("sample object")
        .insert("unknown".to_owned(), serde_json::Value::Bool(true));
    let bytes = serde_json::to_vec(&value).expect("sample JSON");

    assert!(matches!(
        TrialSample::from_json_for_run(&bytes, &run),
        Err(CodecError::Decode {
            document: "trial sample",
            ..
        })
    ));
}

#[test]
fn sample_size_limit_applies_before_decode() {
    let run = run_identity();
    let bytes = vec![b' '; MAX_SAMPLE_BYTES + 1];

    assert!(matches!(
        TrialSample::from_json_for_run(&bytes, &run),
        Err(CodecError::DocumentTooLarge {
            document: "trial sample",
            ..
        })
    ));
}
