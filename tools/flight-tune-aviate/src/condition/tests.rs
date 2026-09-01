#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use std::path::Path;

use flight_tune::{
    BackendCapability, ConditionAdmission, ConditionSet, Digest, HoverEstimatorMode,
    UncertaintyDeclaration,
};

use super::*;

#[path = "tests/executor.rs"]
mod executor;

use executor::{Executor, Fault};

const RUN_INTENT: [u8; 32] = [0x21; 32];
const CONDITION: &str = concat!(
    r#"{"schema_version":4,"id":"executed-uncertainty-launch","revision":1,"seed":11,"#,
    r#""wind":{"steady":{"speed_mps":0.0,"direction_deg":0.0},"gusts":[],"#,
    r#""turbulence":{"kind":"none"}},"#,
    r#""timing":{"estimate_delay_ns":0,"update_jitter":{"kind":"none"}},"#,
    r#""sensor":{"kind":"bounded_noise","lanes":["#,
    r#"{"sensor":"accelerometer","axis":"x","peak_amplitude_mps2":2.0,"#,
    r#""update_interval_samples":2}]},"#,
    r#""actuator":{"authority_scale_basis_points":12000,"#,
    r#""command_loss":{"kind":"seeded_zero_order_hold","fraction_basis_points":1000,"#,
    r#""decision_interval_samples":10}},"#,
    r#""controller_initialization":{"hover_thrust_force":{"kind":"scale_baseline","#,
    r#""scale_basis_points":9000}},"#,
    r#""plant":{"payload_mass_delta_kg":0.0,"longitudinal_cg_offset_m":0.0,"#,
    r#""lateral_cg_offset_m":0.0,"hover_thrust_expectation":{"kind":"measured_weight_ratio"}}}"#,
);

#[test]
fn a_launch_states_every_condition_value_as_an_explicit_argument() {
    let directory = temporary_directory("launch-arguments");
    let launch = prepare(&directory).expect("prepared launch");
    let arguments = launch.arguments();

    let named = |flag: &str| {
        arguments
            .iter()
            .position(|value| value == flag)
            .and_then(|index| arguments.get(index.wrapping_add(1)))
            .cloned()
            .unwrap_or_else(|| panic!("{flag} is not stated"))
    };

    assert_eq!(
        named("--condition-artifact"),
        launch.artifact_path().to_string_lossy()
    );
    assert_eq!(
        named("--condition-artifact-sha256"),
        launch.identity().artifact_digest.to_string()
    );
    assert_eq!(
        named("--condition-digest"),
        launch.identity().condition_digest.to_string()
    );
    assert_eq!(named("--run-seed"), launch.identity().run_seed.to_string());
    assert_eq!(
        named("--required-perturbation-capabilities"),
        "actuator_authority,command_hold,hover_trim_uncertainty,sensor_perturbation"
    );
    assert!(named("--tuning-trace-endpoint").starts_with("127.0.0.1:"));
    assert_eq!(
        named("--run-manifest"),
        launch.manifest_path().to_string_lossy()
    );
    std::fs::remove_dir_all(&directory).ok();
}

#[test]
fn the_artifact_identity_is_not_the_identity_of_the_document_inside_it() {
    let directory = temporary_directory("artifact-identity");
    let launch = prepare(&directory).expect("prepared launch");
    let bytes = std::fs::read(launch.artifact_path()).expect("artifact bytes");
    let condition = condition();

    assert_ne!(
        launch.identity().artifact_digest,
        launch.identity().condition_digest
    );
    assert_eq!(
        bytes,
        [
            condition.to_canonical_json().expect("canonical"),
            b"\n".to_vec()
        ]
        .concat()
    );
    assert_eq!(
        ConditionSet::from_json(&bytes)
            .expect("artifact parses")
            .canonical_digest()
            .expect("identity"),
        launch.identity().condition_digest
    );
    std::fs::remove_dir_all(&directory).ok();
}

#[test]
fn a_backend_that_declares_no_uncertainty_refuses_before_the_artifact_exists() {
    let directory = temporary_directory("no-capability");
    let path = ConditionTracePath::bind_blocking().expect("trace path");
    let admission = ConditionAdmission::new(UncertaintyDeclaration::new(
        Vec::new(),
        HoverEstimatorMode::Online,
    ));

    let refused = ConditionLaunch::prepare_blocking(
        &condition(),
        &admission,
        Digest::from_bytes(RUN_INTENT),
        &directory,
        path.endpoint(),
    );

    assert!(matches!(
        refused,
        Err(AviateConditionError::Unsupported { .. })
    ));
    assert!(!directory.join("condition.json").exists());
    std::fs::remove_dir_all(&directory).ok();
}

#[test]
fn a_complete_run_over_the_trace_path_seals_what_it_executed() {
    let receipt = flown("complete-run", 21, Fault::None).expect("sealed receipt");

    receipt.validate().expect("valid receipt");
    assert_eq!(receipt.ledger.sample_count, 21);
    assert_eq!(receipt.ledger.actuator.applied_hold, 2);
    assert_eq!(receipt.ledger.sensor_lanes[0].eligible, 21);
    assert_eq!(receipt.ledger.sensor_lanes[0].held, 10);
}

#[test]
fn an_executor_that_loaded_another_condition_cannot_arm() {
    assert!(matches!(
        flown("changed-condition", 21, Fault::ChangedConditionDigest),
        Err(AviateConditionError::Identity { .. })
    ));
}

#[test]
fn an_executor_that_drew_from_another_seed_cannot_arm() {
    assert!(matches!(
        flown("changed-seed", 21, Fault::ChangedRunSeed),
        Err(AviateConditionError::Identity { .. })
    ));
}

#[test]
fn an_executor_that_supplies_fewer_capabilities_cannot_arm() {
    assert!(matches!(
        flown("missing-capability", 21, Fault::MissingCapability),
        Err(AviateConditionError::Identity { .. })
    ));
}

#[test]
fn an_executor_with_an_active_hover_estimator_cannot_arm() {
    assert!(matches!(
        flown("active-estimator", 21, Fault::ActiveHoverEstimator),
        Err(AviateConditionError::Identity { .. })
    ));
}

#[test]
fn a_trace_sequence_gap_ends_the_run() {
    assert!(matches!(
        flown("sequence-gap", 21, Fault::SequenceGap),
        Err(AviateConditionError::Relation { .. })
    ));
}

#[test]
fn a_sample_that_states_a_value_it_did_not_derive_ends_the_run() {
    assert!(matches!(
        flown("changed-value", 21, Fault::ChangedSensorValue),
        Err(AviateConditionError::Relation { .. })
    ));
}

#[test]
fn a_hover_force_that_moves_inside_one_run_ends_the_run() {
    assert!(matches!(
        flown("moved-hover", 21, Fault::MovedHoverForce),
        Err(AviateConditionError::Relation { .. })
    ));
}

#[test]
fn an_executor_that_stops_inside_a_decision_interval_is_not_sealed() {
    assert!(matches!(
        flown("short-run", 21, Fault::ShortRun),
        Err(AviateConditionError::Relation { .. })
    ));
}

/// Runs one complete executor against one launch over a real loopback path.
fn flown(
    name: &str,
    samples: u64,
    fault: Fault,
) -> Result<flight_tune::ExecutedUncertaintyReceipt, AviateConditionError> {
    let directory = temporary_directory(name);
    let path = ConditionTracePath::bind_blocking().expect("trace path");
    let endpoint = path.endpoint();
    let launch = ConditionLaunch::prepare_blocking(
        &condition(),
        &admission(),
        Digest::from_bytes(RUN_INTENT),
        &directory,
        endpoint,
    )
    .expect("prepared launch");
    let executor = Executor::new(
        launch.declaration(),
        launch.artifact_path().to_path_buf(),
        samples,
        fault,
    );
    let child = std::thread::spawn(move || executor.run_blocking(endpoint));
    let sealed = path.verify_blocking(&launch);
    // The executor stops when a sample goes unanswered, which is the
    // failure policy rather than a fault in the test.
    child.join().expect("the executor thread finished").ok();
    std::fs::remove_dir_all(&directory).ok();
    sealed
}

fn prepare(directory: &Path) -> Result<ConditionLaunch, AviateConditionError> {
    let path = ConditionTracePath::bind_blocking().expect("trace path");
    ConditionLaunch::prepare_blocking(
        &condition(),
        &admission(),
        Digest::from_bytes(RUN_INTENT),
        directory,
        path.endpoint(),
    )
}

fn admission() -> ConditionAdmission {
    ConditionAdmission::new(UncertaintyDeclaration::new(
        vec![
            BackendCapability::SensorPerturbation,
            BackendCapability::ActuatorAuthority,
            BackendCapability::CommandHold,
            BackendCapability::HoverTrimUncertainty,
        ],
        HoverEstimatorMode::Disabled,
    ))
}

fn condition() -> ConditionSet {
    ConditionSet::from_json(CONDITION.as_bytes()).expect("condition")
}

fn temporary_directory(name: &str) -> std::path::PathBuf {
    let directory = std::env::temp_dir().join(format!(
        "pilotage-executed-uncertainty-{name}-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&directory).expect("test directory");
    directory
}
