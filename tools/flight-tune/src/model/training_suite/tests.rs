#![allow(clippy::expect_used, clippy::panic)]

use std::collections::{BTreeMap, BTreeSet};

use pilotage_mission_core::MISSION_SCHEMA_VERSION;
use pilotage_trial::Digest;

use crate::{
    Candidate, CandidateLineage, ExecutionRetryPolicy, MissionReference, ParameterBounds,
    PromotionPolicy, PromotionSeedPolicy, QualificationPolicy, SearchGroup, SearchGroupKind,
    SearchStage, TRAINING_SUITE_SCHEMA_VERSION, TrainingSuite,
};

fn mission(id: &str, seed: u8) -> MissionReference {
    MissionReference {
        revision_id: id.to_owned(),
        schema_version: MISSION_SCHEMA_VERSION,
        content_digest: Digest::from_bytes([seed; 32]),
        max_samples: 64,
        sample_timeout_ns: 20_000_000,
    }
}

fn suite(id: &str, primary: Vec<MissionReference>, guard: Vec<MissionReference>) -> TrainingSuite {
    let guard_regression_limits = if guard.is_empty() {
        BTreeMap::new()
    } else {
        BTreeMap::from([("test.response".to_owned(), 0.1)])
    };
    TrainingSuite {
        schema_version: TRAINING_SUITE_SCHEMA_VERSION,
        id: id.to_owned(),
        primary_scenarios: primary,
        guard_scenarios: guard,
        guard_regression_limits,
        repetitions: 2,
    }
}

fn group(id: &str, kind: SearchGroupKind, parameters: &[&str], suite_id: &str) -> SearchGroup {
    SearchGroup {
        id: id.to_owned(),
        kind,
        parameters: parameters.iter().map(|name| (*name).to_owned()).collect(),
        suite_id: suite_id.to_owned(),
    }
}

pub(crate) fn stage_for_budget() -> SearchStage {
    stage()
}

fn stage() -> SearchStage {
    let direct = mission("direct", 1);
    let operator = mission("operator", 2);
    SearchStage {
        id: "two-group".to_owned(),
        allowlist: BTreeMap::from([
            ("rate".to_owned(), bounds()),
            ("shape".to_owned(), bounds()),
        ]),
        fixed_parameters: BTreeMap::new(),
        required_hard_gates: vec![
            crate::MANDATORY_CRASH_GATE_ID.to_owned(),
            "envelope".to_owned(),
        ],
        training_scenarios: vec![direct.clone(), operator.clone()],
        training_suites: vec![
            suite("direct-response", vec![direct.clone()], Vec::new()),
            suite("operator-feel", vec![operator], vec![direct]),
        ],
        search_groups: vec![
            group(
                "dynamics",
                SearchGroupKind::Controller,
                &["rate"],
                "direct-response",
            ),
            group(
                "shape",
                SearchGroupKind::OperatorFeel,
                &["shape"],
                "operator-feel",
            ),
        ],
        promotion_scenarios: vec![mission("promotion", 3)],
        final_qualification_scenarios: vec![mission("final", 4)],
        repetitions: 2,
        promotion: PromotionPolicy {
            schema_version: crate::PROMOTION_POLICY_SCHEMA_VERSION,
            seed_policy: PromotionSeedPolicy::PairedScenarioDigestV1,
            minimum_loss_improvement: 0.0,
            minimum_relative_loss_improvement: 0.1,
            maximum_control_effort_increase: 1.0,
            objectives: BTreeSet::from(["test.response".to_owned()]),
        },
        qualification: QualificationPolicy {
            maximum_loss_confidence_upper: 1.0,
            maximum_p95_loss: 1.0,
            maximum_mean_control_effort: 1.0,
            objectives: BTreeSet::from(["test.response".to_owned()]),
        },
        execution_retry: ExecutionRetryPolicy::none(),
        response_targets: crate::model::response_target::fixture::covering(&[
            (&[mission("promotion", 3)], &limits()),
            (&[mission("final", 4)], &limits()),
        ]),
    }
}

fn limits() -> BTreeMap<String, f64> {
    BTreeMap::from([("test.response".to_owned(), 1.0)])
}

const fn bounds() -> ParameterBounds {
    ParameterBounds {
        minimum: 0.0,
        maximum: 2.0,
    }
}

fn candidate(rate: f64, shape: f64) -> Candidate {
    Candidate::new(
        CandidateLineage {
            schema: "suite-test-v1".to_owned(),
            base_preset_digest: Digest::from_bytes([9; 32]),
            plant_digest: Digest::from_bytes([10; 32]),
        },
        BTreeMap::from([("rate".to_owned(), rate), ("shape".to_owned(), shape)]),
    )
    .expect("candidate")
}

#[test]
fn a_two_group_stage_is_valid() {
    stage().validate().expect("the two group stage is valid");
}

#[test]
fn a_parameter_in_two_groups_is_refused() {
    let mut stage = stage();
    stage.search_groups[1].parameters.insert("rate".to_owned());

    assert!(stage.validate().is_err());
}

#[test]
fn a_parameter_in_no_group_is_refused() {
    let mut stage = stage();
    stage.allowlist.insert("extra".to_owned(), bounds());

    assert!(stage.validate().is_err());
}

#[test]
fn a_group_that_names_an_absent_suite_is_refused() {
    let mut stage = stage();
    stage.search_groups[0].suite_id = "absent".to_owned();

    assert!(stage.validate().is_err());
}

#[test]
fn a_suite_no_group_names_is_refused() {
    let mut stage = stage();
    stage
        .training_suites
        .push(suite("spare", vec![mission("direct", 1)], Vec::new()));

    assert!(stage.validate().is_err());
}

#[test]
fn an_empty_suite_is_refused() {
    let mut stage = stage();
    stage.training_suites[0].primary_scenarios.clear();

    assert!(stage.validate().is_err());
}

#[test]
fn a_repeated_mission_inside_one_suite_is_refused() {
    let mut stage = stage();
    let repeated = stage.training_suites[0].primary_scenarios[0].clone();
    stage.training_suites[0].guard_scenarios.push(repeated);
    stage.training_suites[0]
        .guard_regression_limits
        .insert("test.response".to_owned(), 0.1);

    assert!(stage.validate().is_err());
}

#[test]
fn a_suite_mission_outside_the_training_partition_is_refused() {
    let mut stage = stage();
    stage.training_suites[0].primary_scenarios = vec![mission("promotion", 3)];

    assert!(stage.validate().is_err());
}

#[test]
fn a_suite_mission_with_a_changed_digest_is_refused() {
    let mut stage = stage();
    stage.training_suites[0].primary_scenarios = vec![mission("direct", 99)];

    assert!(stage.validate().is_err());
}

#[test]
fn a_training_mission_no_suite_uses_is_refused() {
    let mut stage = stage();
    stage.training_scenarios.push(mission("spare", 5));

    assert!(stage.validate().is_err());
}

#[test]
fn a_guard_mission_without_a_guard_limit_is_refused() {
    let mut stage = stage();
    stage.training_suites[1].guard_regression_limits.clear();

    assert!(stage.validate().is_err());
}

#[test]
fn an_unsupported_suite_schema_is_refused() {
    let mut stage = stage();
    stage.training_suites[0].schema_version = TRAINING_SUITE_SCHEMA_VERSION.wrapping_add(1);

    assert!(stage.validate().is_err());
}

#[test]
fn an_operator_feel_group_with_an_unguarded_suite_is_refused() {
    let mut stage = stage();
    stage.training_suites[1].guard_scenarios.clear();
    stage.training_suites[1].guard_regression_limits.clear();
    // The operator mission now has to carry the whole training partition, so
    // the direct mission needs a suite of its own to stay in use.
    stage.training_suites[1]
        .primary_scenarios
        .push(mission("direct", 1));

    assert!(stage.validate().is_err());
}

#[test]
fn a_controller_change_derives_the_controller_suite() {
    let stage = stage();
    let binding = stage
        .derive_search_group(&candidate(1.0, 1.0), &candidate(1.5, 1.0))
        .expect("a controller change derives one group");

    assert_eq!(binding.group_id, "dynamics");
    assert_eq!(binding.suite_id, "direct-response");
    assert_eq!(binding.suite_index, 0);
}

#[test]
fn a_feel_change_derives_the_operator_feel_suite() {
    let stage = stage();
    let binding = stage
        .derive_search_group(&candidate(1.0, 1.0), &candidate(1.0, 1.5))
        .expect("a feel change derives one group");

    assert_eq!(binding.group_id, "shape");
    assert_eq!(binding.suite_id, "operator-feel");
    assert_eq!(binding.suite_index, 1);
}

#[test]
fn a_proposal_that_changes_two_groups_is_refused() {
    let stage = stage();

    assert!(
        stage
            .derive_search_group(&candidate(1.0, 1.0), &candidate(1.5, 1.5))
            .is_err()
    );
}

#[test]
fn a_proposal_that_changes_nothing_is_refused() {
    let stage = stage();

    assert!(
        stage
            .derive_search_group(&candidate(1.0, 1.0), &candidate(1.0, 1.0))
            .is_err()
    );
}

#[test]
fn a_suite_digest_follows_its_declaration() {
    let stage = stage();
    let first = stage.training_suites[0]
        .digest()
        .expect("the first suite digest");
    let second = stage.training_suites[1]
        .digest()
        .expect("the second suite digest");
    let mut changed = stage.training_suites[0].clone();
    changed.repetitions = 3;

    assert_ne!(first, second);
    assert_ne!(first, changed.digest().expect("the changed suite digest"));
}

#[test]
fn a_suite_run_plan_puts_primary_missions_first() {
    let stage = stage();
    let feel = &stage.training_suites[1];
    let ordered = feel.ordered_scenarios();

    assert_eq!(ordered.len(), 2);
    assert_eq!(ordered[0].revision_id, "operator");
    assert_eq!(ordered[1].revision_id, "direct");
    assert_eq!(feel.primary_run_count(), 2);
    assert_eq!(feel.run_count(), 4);
}

#[test]
fn a_search_group_set_is_ordered_and_unique() {
    let group = group(
        "dynamics",
        SearchGroupKind::Controller,
        &["rate", "rate"],
        "direct-response",
    );

    assert_eq!(group.parameters, BTreeSet::from(["rate".to_owned()]));
}

/// The crash gate is the floor of every campaign and the first gate.
///
/// A stage that dropped it, renamed it, or let another gate run first would
/// let a run that hit something be scored, and every measurement of that run
/// describes the collision rather than the command law.
#[test]
fn the_crash_gate_cannot_be_removed_renamed_or_moved() {
    let valid = stage();
    valid.validate().expect("the fixture stage is valid");

    let mut removed = stage();
    removed.required_hard_gates = vec!["envelope".to_owned()];
    assert!(
        removed.validate().is_err(),
        "an omitted crash gate is refused"
    );

    let mut renamed = stage();
    renamed.required_hard_gates = vec!["crash".to_owned(), "envelope".to_owned()];
    assert!(
        renamed.validate().is_err(),
        "a renamed crash gate is refused"
    );

    let mut reordered = stage();
    reordered.required_hard_gates = vec![
        "envelope".to_owned(),
        crate::MANDATORY_CRASH_GATE_ID.to_owned(),
    ];
    assert!(
        reordered.validate().is_err(),
        "a crash gate behind another gate is refused"
    );
}
