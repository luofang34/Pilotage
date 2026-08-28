#![allow(clippy::expect_used, clippy::panic)]

use crate::{
    ArtifactIdentity, CodecError, Comparison, ControlChannel, Digest, ExecutionPolicy,
    ExecutionTarget, FlightAction, FlightPlanReference, MissionAction, MissionCapability,
    MissionCondition, MissionDocument, MissionPhase, NavigationDataIdentity, SignalCondition,
    SignalSelector, StartHeading, StartState, TrialAction, ValidationError, Waveform,
};

const CANONICAL_FIXTURE: &[u8] = concat!(
    "{\"identity\":{\"revision_id\":\"mission-1\",\"schema_version\":2,",
    "\"content_digest\":\"39948a4de2c0ffe060c45b243e82eb575d9cddb68204f097b36c5f3995c9b996\",",
    "\"navigation_data_identity\":{\"cycle\":\"2608\",\"snapshot_id\":\"nav-1\",",
    "\"snapshot_digest\":\"0101010101010101010101010101010101010101010101010101010101010101\"}},",
    "\"execution_policy\":{\"target\":\"real_vehicle\",\"retry_limit\":2,",
    "\"receipt_timeout_ns\":500000},\"phases\":[{\"id\":\"phase-1\",",
    "\"required_capabilities\":[\"simulator_time\",\"arm_disarm\"],\"entry_conditions\":[],",
    "\"action\":{\"domain\":\"flight\",\"action\":{\"kind\":\"arm\"}},",
    "\"cleanup_actions\":[],",
    "\"completion_conditions\":[],\"abort_conditions\":[],",
    "\"simulator_time_deadline_ns\":1000000}]}"
)
.as_bytes();

fn digest(byte: u8) -> Digest {
    Digest::from_bytes([byte; 32])
}

fn navigation_data(byte: u8) -> NavigationDataIdentity {
    NavigationDataIdentity {
        cycle: "2608".to_owned(),
        snapshot_id: format!("nav-{byte}"),
        snapshot_digest: digest(byte),
    }
}

fn phase(id: &str, action: MissionAction) -> MissionPhase {
    let mut required_capabilities = vec![MissionCapability::SimulatorTime];
    if let Some(capability) = action.required_capability()
        && capability != MissionCapability::SimulatorTime
    {
        required_capabilities.push(capability);
    }
    MissionPhase {
        id: id.to_owned(),
        required_capabilities,
        entry_conditions: Vec::new(),
        action,
        cleanup_actions: Vec::new(),
        completion_conditions: Vec::new(),
        abort_conditions: Vec::new(),
        simulator_time_deadline_ns: 1_000_000,
    }
}

fn document_for(
    action: MissionAction,
    target: ExecutionTarget,
) -> Result<MissionDocument, CodecError> {
    MissionDocument::new(
        "mission-1".to_owned(),
        navigation_data(1),
        ExecutionPolicy {
            target,
            retry_limit: 2,
            receipt_timeout_ns: 500_000,
        },
        vec![phase("phase-1", action)],
    )
}

fn valid_document() -> MissionDocument {
    document_for(
        MissionAction::Flight(FlightAction::Arm {}),
        ExecutionTarget::RealVehicle,
    )
    .expect("the test mission must be valid")
}

fn trial_artifact() -> ArtifactIdentity {
    ArtifactIdentity {
        id: "artifact-1".to_owned(),
        revision: "revision-1".to_owned(),
        digest: digest(2),
    }
}

fn trial_actions() -> Vec<TrialAction> {
    vec![
        TrialAction::Reset {},
        TrialAction::WaitReady {},
        TrialAction::ApplyConditions {
            condition_set: trial_artifact(),
        },
        TrialAction::ReachStartState {
            target: StartState {
                relative_position_ned_m: [0.0, 0.0, -10.0],
                heading: StartHeading::True { radians: 0.0 },
            },
        },
        TrialAction::Settle {},
        TrialAction::Stimulate {
            channel: ControlChannel::Pitch,
            waveform: Waveform::Step { value: 0.2 },
        },
        TrialAction::ReleaseControl {},
        TrialAction::Observe {},
        TrialAction::Stop {},
        TrialAction::Disarm {},
        TrialAction::CollectResults {},
    ]
}

fn recalculate(document: &mut MissionDocument) {
    document.identity.content_digest = document
        .calculate_content_digest()
        .expect("the test document must be encodable");
}

#[test]
fn canonical_fixture_round_trips_byte_identical() {
    let expected = valid_document()
        .to_canonical_json()
        .expect("the fixture document must encode");
    assert_eq!(CANONICAL_FIXTURE, expected);
    let decoded =
        MissionDocument::from_json(CANONICAL_FIXTURE).expect("the canonical fixture must decode");
    assert_eq!(
        decoded.to_canonical_json().expect("re-encode"),
        CANONICAL_FIXTURE
    );
}

#[test]
fn identity_and_policy_field_groups_change_digest() {
    let document = valid_document();
    let original = document.calculate_content_digest().expect("digest");
    let mut revision = document.clone();
    revision.identity.revision_id.push_str("-changed");
    assert_ne!(
        revision.calculate_content_digest().expect("digest"),
        original
    );
    let mut schema = document.clone();
    schema.identity.schema_version = 3;
    assert_ne!(schema.calculate_content_digest().expect("digest"), original);
    let mut navdata = document.clone();
    navdata.identity.navigation_data_identity.cycle = "2609".to_owned();
    assert_ne!(
        navdata.calculate_content_digest().expect("digest"),
        original
    );
    let mut policy = document;
    policy.execution_policy.target = ExecutionTarget::Simulator;
    assert_ne!(policy.calculate_content_digest().expect("digest"), original);
    let mut timeout = valid_document();
    timeout.execution_policy.receipt_timeout_ns =
        timeout.execution_policy.receipt_timeout_ns.wrapping_add(1);
    assert_ne!(
        timeout.calculate_content_digest().expect("digest"),
        original
    );
}

#[test]
fn retry_limit_change_changes_content_digest() {
    let mut document = valid_document();
    let original = document.calculate_content_digest().expect("digest");
    document.execution_policy.retry_limit = document.execution_policy.retry_limit.wrapping_add(1);
    assert_ne!(
        document.calculate_content_digest().expect("digest"),
        original
    );
}

#[test]
fn each_phase_field_group_changes_digest() {
    let document = valid_document();
    let original = document.calculate_content_digest().expect("digest");
    let mut changed = document.clone();
    changed.phases[0].id.push_str("-changed");
    assert_ne!(
        changed.calculate_content_digest().expect("digest"),
        original
    );
    changed = document.clone();
    changed.phases[0]
        .cleanup_actions
        .push(MissionAction::Flight(FlightAction::Disarm {}));
    assert_ne!(
        changed.calculate_content_digest().expect("digest"),
        original
    );
    changed = document.clone();
    changed.phases[0]
        .required_capabilities
        .push(MissionCapability::DeterministicSeed);
    assert_ne!(
        changed.calculate_content_digest().expect("digest"),
        original
    );
    changed = document.clone();
    changed.phases[0]
        .entry_conditions
        .push(MissionCondition::Always {});
    assert_ne!(
        changed.calculate_content_digest().expect("digest"),
        original
    );
    changed = document.clone();
    changed.phases[0].action = MissionAction::Flight(FlightAction::Disarm {});
    assert_ne!(
        changed.calculate_content_digest().expect("digest"),
        original
    );
    changed = document.clone();
    changed.phases[0]
        .completion_conditions
        .push(MissionCondition::Always {});
    assert_ne!(
        changed.calculate_content_digest().expect("digest"),
        original
    );
    changed = document.clone();
    changed.phases[0]
        .abort_conditions
        .push(MissionCondition::Always {});
    assert_ne!(
        changed.calculate_content_digest().expect("digest"),
        original
    );
    changed = document;
    changed.phases[0].simulator_time_deadline_ns += 1;
    assert_ne!(
        changed.calculate_content_digest().expect("digest"),
        original
    );
}

#[test]
fn each_flight_plan_reference_field_group_changes_digest() {
    let plan = FlightPlanReference {
        plan_id: "plan-1".to_owned(),
        plan_content_digest: digest(3),
        navigation_data_identity: navigation_data(1),
    };
    let document = document_for(
        MissionAction::Flight(FlightAction::FollowPlan { plan }),
        ExecutionTarget::RealVehicle,
    )
    .expect("the plan mission must be valid");
    let original = document.calculate_content_digest().expect("digest");
    assert_plan_change_changes_digest(&document, original, |plan| {
        plan.plan_id.push_str("-changed");
    });
    assert_plan_change_changes_digest(&document, original, |plan| {
        plan.plan_content_digest = digest(4);
    });
    assert_plan_change_changes_digest(&document, original, |plan| {
        plan.navigation_data_identity.cycle = "2609".to_owned();
    });
}

fn assert_plan_change_changes_digest(
    document: &MissionDocument,
    original: Digest,
    change: impl FnOnce(&mut FlightPlanReference),
) {
    let mut changed = document.clone();
    let MissionAction::Flight(FlightAction::FollowPlan { plan }) = &mut changed.phases[0].action
    else {
        panic!("the test action must follow a plan");
    };
    change(plan);
    assert_ne!(
        changed.calculate_content_digest().expect("digest"),
        original
    );
}

#[test]
fn declared_content_digest_is_not_part_of_digest_input() {
    let document = valid_document();
    let original = document.calculate_content_digest().expect("digest");
    let mut changed = document;
    changed.identity.content_digest = digest(9);
    assert_eq!(
        changed.calculate_content_digest().expect("digest"),
        original
    );
}

#[test]
fn validation_rejects_empty_and_repeated_phases() {
    let mut document = valid_document();
    document.phases.clear();
    assert!(matches!(
        document.validate(),
        Err(ValidationError::EmptyList { .. })
    ));
    let mut document = valid_document();
    document.phases.push(document.phases[0].clone());
    assert!(matches!(
        document.validate(),
        Err(ValidationError::RepeatedPhaseId { .. })
    ));
}

#[test]
fn validation_rejects_missing_deadline_and_capability() {
    let mut document = valid_document();
    document.phases[0].simulator_time_deadline_ns = 0;
    assert!(matches!(
        document.validate(),
        Err(ValidationError::MissingDeadline { .. })
    ));
    let mut document = valid_document();
    document.phases[0]
        .required_capabilities
        .retain(|value| *value != MissionCapability::ArmDisarm);
    assert!(matches!(
        document.validate(),
        Err(ValidationError::UndeclaredCapability {
            capability: MissionCapability::ArmDisarm,
            ..
        })
    ));
    let mut document = valid_document();
    document.execution_policy.receipt_timeout_ns = 0;
    assert!(matches!(
        document.validate(),
        Err(ValidationError::ZeroDuration { .. })
    ));
}

#[test]
fn real_vehicle_admission_rejects_each_trial_action() {
    for action in trial_actions() {
        let document = document_for(MissionAction::Trial(action), ExecutionTarget::Simulator)
            .expect("the simulator trial action must be valid");
        let error = document
            .validate_for_target(ExecutionTarget::RealVehicle)
            .expect_err("a real vehicle must reject every trial action");
        assert!(matches!(error, ValidationError::SimulatorOnlyAction { .. }));
    }
}

#[test]
fn real_vehicle_admission_rejects_a_trial_cleanup_action() {
    let mut document = valid_document();
    document.phases[0]
        .required_capabilities
        .push(MissionCapability::Reset);
    document.phases[0]
        .cleanup_actions
        .push(MissionAction::Trial(TrialAction::Reset {}));
    assert!(matches!(
        document.validate_for_target(ExecutionTarget::RealVehicle),
        Err(ValidationError::SimulatorOnlyAction { .. })
    ));
}

#[test]
fn phase_order_is_preserved_by_canonical_serialization() {
    let document = MissionDocument::new(
        "mission-ordered".to_owned(),
        navigation_data(1),
        ExecutionPolicy {
            target: ExecutionTarget::RealVehicle,
            retry_limit: 2,
            receipt_timeout_ns: 500_000,
        },
        vec![
            phase("first", MissionAction::Flight(FlightAction::Arm {})),
            phase("second", MissionAction::Flight(FlightAction::Disarm {})),
        ],
    )
    .expect("the ordered mission must be valid");
    let bytes = document.to_canonical_json().expect("encode");
    let decoded = MissionDocument::from_json(&bytes).expect("decode");
    assert_eq!(decoded.phases[0].id, "first");
    assert_eq!(decoded.phases[1].id, "second");
}

#[test]
fn signal_condition_requires_declared_truth_capability() {
    let mut document = valid_document();
    document.phases[0]
        .completion_conditions
        .push(MissionCondition::Signal(SignalCondition::Value {
            selector: SignalSelector::TruthPosition {
                component: crate::VectorComponent::Z,
            },
            comparison: Comparison::GreaterThan,
            value: 0.0,
        }));
    assert!(matches!(
        document.validate(),
        Err(ValidationError::UndeclaredCapability {
            capability: MissionCapability::KinematicTruth,
            ..
        })
    ));
}

#[test]
fn flight_plan_never_serializes_waypoint_data() {
    let plan = FlightPlanReference {
        plan_id: "plan-1".to_owned(),
        plan_content_digest: digest(3),
        navigation_data_identity: navigation_data(1),
    };
    let document = document_for(
        MissionAction::Flight(FlightAction::FollowPlan { plan }),
        ExecutionTarget::RealVehicle,
    )
    .expect("the plan mission must be valid");
    let bytes = document.to_canonical_json().expect("encode");
    let text = std::str::from_utf8(&bytes).expect("JSON must be UTF-8");
    assert!(!text.contains("waypoint"));
}

#[test]
fn decoder_rejects_unknown_fields_and_digest_changes() {
    let document = valid_document();
    let mut value = serde_json::to_value(&document).expect("encode value");
    value["unknown"] = serde_json::Value::Bool(true);
    let bytes = serde_json::to_vec(&value).expect("encode changed value");
    assert!(matches!(
        MissionDocument::from_json(&bytes),
        Err(CodecError::Decode { .. })
    ));
    let mut changed = document;
    changed.phases[0].simulator_time_deadline_ns += 1;
    assert!(matches!(
        changed.to_canonical_json(),
        Err(CodecError::Validation(
            ValidationError::ContentDigestMismatch { .. }
        ))
    ));
}

#[test]
fn decoder_rejects_unknown_nested_fields() {
    let document = valid_document();
    assert_unknown_field_rejected(&document, |value| {
        value["phases"][0]["unknown"] = serde_json::Value::Bool(true);
    });
    assert_unknown_field_rejected(&document, |value| {
        value["phases"][0]["action"]["action"]["unknown"] = serde_json::Value::Bool(true);
    });
}

fn assert_unknown_field_rejected(
    document: &MissionDocument,
    change: impl FnOnce(&mut serde_json::Value),
) {
    let mut value = serde_json::to_value(document).expect("encode value");
    change(&mut value);
    let bytes = serde_json::to_vec(&value).expect("encode changed value");
    let result = MissionDocument::from_json(&bytes);
    assert!(
        matches!(&result, Err(CodecError::Decode { .. })),
        "unexpected result: {result:?}"
    );
}

#[test]
fn recalculated_digest_validates_changed_content() {
    let mut document = valid_document();
    document.phases[0].simulator_time_deadline_ns += 1;
    recalculate(&mut document);
    document
        .to_canonical_json()
        .expect("recalculated content must validate");
}
