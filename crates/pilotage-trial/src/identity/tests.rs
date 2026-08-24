use super::{
    ArtifactIdentity, ClockDomain, ClockMapping, ClockMappingQuality, RunIdentity, ScenarioIdentity,
};
use crate::{
    CodecError, Digest, MAX_RUN_IDENTITY_BYTES, RUN_IDENTITY_SCHEMA_VERSION, ValidationError,
};

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
        source_anchor_ns: 100,
        recorder_anchor_ns: 200,
        rate_numerator: 1,
        rate_denominator: 1,
        valid_from_source_ns: 0,
        valid_until_source_ns: 1_000,
        uncertainty_ns: 0,
        quality: ClockMappingQuality::Exact,
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

#[test]
fn public_run_digest_is_stable_after_json_normalization() {
    let run = run_identity();
    let compact = run.to_canonical_json().expect("canonical run identity");
    let pretty = serde_json::to_vec_pretty(&run).expect("pretty run identity");
    let decoded = RunIdentity::from_json(&pretty).expect("validated run identity");

    assert_eq!(decoded, run);
    assert_eq!(
        decoded.to_canonical_json().expect("canonical decoded"),
        compact
    );
    assert_eq!(
        decoded.canonical_digest().expect("decoded digest"),
        run.canonical_digest().expect("source digest")
    );
    assert_eq!(
        run.canonical_digest().expect("golden digest").to_string(),
        "fe8f15ae1f4213e7c80f16189b1462f806f828d225e4b8a73eed5426472e5388"
    );
}

#[test]
fn run_identity_rejects_an_unknown_schema_version() {
    let mut run = run_identity();
    run.schema_version = RUN_IDENTITY_SCHEMA_VERSION.wrapping_add(1);

    assert!(matches!(
        run.validate(),
        Err(ValidationError::UnsupportedSchemaVersion {
            document: "run identity",
            ..
        })
    ));
}

#[test]
fn unused_clock_domains_do_not_require_fake_mappings() {
    let mut run = run_identity();
    run.clock_mappings.clear();

    assert_eq!(run.validate(), Ok(()));
}

#[test]
fn clock_mapping_rejects_an_interval_before_the_recorder_epoch() {
    let mut run = run_identity();
    run.clock_mappings[0].source_anchor_ns = 100;
    run.clock_mappings[0].recorder_anchor_ns = 0;

    assert_eq!(
        run.validate(),
        Err(ValidationError::InvalidClockMapping {
            index: 0,
            reason: "the mapping interval exceeds the recorder clock",
        })
    );
}

#[test]
fn clock_mapping_rejects_uncertainty_underflow() {
    let mut run = run_identity();
    run.clock_mappings[0].source_anchor_ns = 0;
    run.clock_mappings[0].recorder_anchor_ns = 0;
    run.clock_mappings[0].quality = ClockMappingQuality::Estimated;
    run.clock_mappings[0].uncertainty_ns = 1;

    assert!(matches!(
        run.validate(),
        Err(ValidationError::InvalidClockMapping { index: 0, .. })
    ));
}

#[test]
fn fractional_rate_returns_both_adjacent_recorder_nanoseconds() {
    let mut mapping = mapping(ClockDomain::Device);
    mapping.source_anchor_ns = 0;
    mapping.recorder_anchor_ns = 100;
    mapping.rate_numerator = 3;
    mapping.rate_denominator = 2;

    let interval = mapping
        .mapped_recorder_interval(1)
        .expect("mapped recorder interval");
    assert_eq!(interval.earliest_ns, 101);
    assert_eq!(interval.latest_ns, 102);
}

#[test]
fn public_mapping_api_rejects_an_invalid_target() {
    let mut mapping = mapping(ClockDomain::Device);
    mapping.to = ClockDomain::Client;

    assert_eq!(mapping.mapped_recorder_interval(100), None);
}

#[test]
fn clock_mapping_must_target_the_recorder() {
    let mut run = run_identity();
    run.clock_mappings[0].to = ClockDomain::Client;

    assert_eq!(
        run.validate(),
        Err(ValidationError::InvalidClockMapping {
            index: 0,
            reason: "each source must map directly to the recorder",
        })
    );
}

#[test]
fn clock_mapping_epoch_is_unique_for_each_source() {
    let mut run = run_identity();
    run.clock_mappings.push(mapping(ClockDomain::Device));

    assert_eq!(
        run.validate(),
        Err(ValidationError::InvalidClockMapping {
            index: 5,
            reason: "the source clock epoch occurs more than once",
        })
    );
}

#[test]
fn clock_mapping_anchor_must_be_in_its_validity_interval() {
    let mut run = run_identity();
    run.clock_mappings[0].source_anchor_ns = 1_001;

    assert_eq!(
        run.validate(),
        Err(ValidationError::InvalidClockMapping {
            index: 0,
            reason: "the source anchor is outside the validity interval",
        })
    );
}

#[test]
fn run_identity_decode_rejects_a_zero_clock_rate() {
    let mut run = run_identity();
    run.clock_mappings[0].rate_denominator = 0;
    let bytes = serde_json::to_vec(&run).expect("run identity JSON");

    assert!(matches!(
        RunIdentity::from_json(&bytes),
        Err(CodecError::Validation(
            ValidationError::InvalidClockMapping { index: 0, .. }
        ))
    ));
}

#[test]
fn run_identity_decode_rejects_an_unknown_field() {
    let run = run_identity();
    let mut value = serde_json::to_value(run).expect("run identity value");
    value
        .as_object_mut()
        .expect("run identity object")
        .insert("unknown".to_owned(), serde_json::Value::Bool(true));
    let bytes = serde_json::to_vec(&value).expect("run identity JSON");

    assert!(matches!(
        RunIdentity::from_json(&bytes),
        Err(CodecError::Decode {
            document: "run identity",
            ..
        })
    ));
}

#[test]
fn run_identity_size_limit_applies_before_decode() {
    let bytes = vec![b' '; MAX_RUN_IDENTITY_BYTES + 1];

    assert!(matches!(
        RunIdentity::from_json(&bytes),
        Err(CodecError::DocumentTooLarge {
            document: "run identity",
            ..
        })
    ));
}

#[test]
fn clock_mapping_rejects_an_interval_after_the_recorder_limit() {
    let mut run = run_identity();
    run.clock_mappings[0].source_anchor_ns = 0;
    run.clock_mappings[0].recorder_anchor_ns = u64::MAX;
    run.clock_mappings[0].valid_from_source_ns = 0;
    run.clock_mappings[0].valid_until_source_ns = 1;

    assert_eq!(
        run.validate(),
        Err(ValidationError::InvalidClockMapping {
            index: 0,
            reason: "the mapping interval exceeds the recorder clock",
        })
    );
}

#[test]
fn fractional_rate_before_the_anchor_contains_both_adjacent_times() {
    let mut value = mapping(ClockDomain::Device);
    value.source_anchor_ns = 2;
    value.recorder_anchor_ns = 100;
    value.rate_numerator = 3;
    value.rate_denominator = 2;
    value.valid_from_source_ns = 1;

    let interval = value
        .mapped_recorder_interval(1)
        .expect("mapped recorder interval");
    assert_eq!(interval.earliest_ns, 98);
    assert_eq!(interval.latest_ns, 99);
}
