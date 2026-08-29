#![allow(clippy::expect_used)]

use std::collections::BTreeMap;

use serde_json::{Value, json};

use super::{
    PROMOTION_POLICY_DOMAIN, PROMOTION_POLICY_SCHEMA_VERSION, PromotionPolicy, PromotionRunPlan,
    PromotionSeedPolicy, digest_policy_content, expected_promotion_pairs, promotion_policy_digest,
};
use crate::identity::digest_bytes;
use crate::{
    AttemptRole, Digest, MissionReference, ParameterBounds, QualificationPolicy, SearchStage,
};

#[test]
fn policy_digest_is_domain_separated_and_covers_every_field() {
    let policy = policy();
    let digest = promotion_policy_digest(&policy).expect("digest policy");
    let plain = digest_bytes(&serde_json::to_vec(&policy).expect("encode policy"));
    assert_ne!(digest, plain);

    let document = serde_json::to_value(&policy).expect("encode policy value");
    let changes = [
        ("schema_version", json!(2)),
        ("seed_policy", json!("paired_scenario_digest_v2")),
        ("minimum_loss_improvement", json!(0.11)),
        ("minimum_relative_loss_improvement", json!(0.21)),
        ("maximum_control_effort_increase", json!(0.31)),
        (
            "objective_regression_upper_95",
            json!({"settling": 0.41, "tracking": 0.2}),
        ),
    ];
    for (field, changed) in changes {
        let mut changed_document = document.clone();
        changed_document[field] = changed;
        assert_ne!(
            digest,
            raw_policy_digest(&changed_document),
            "field {field}"
        );
    }
}

#[test]
fn policy_rejects_versions_non_finite_values_and_bad_objectives() {
    let mut changed = policy();
    changed.schema_version = PROMOTION_POLICY_SCHEMA_VERSION.wrapping_add(1);
    assert!(changed.validate().is_err());

    for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let mut changed = policy();
        changed.minimum_loss_improvement = value;
        assert!(changed.validate().is_err());
        let mut changed = policy();
        changed.minimum_relative_loss_improvement = value;
        assert!(changed.validate().is_err());
        let mut changed = policy();
        changed.maximum_control_effort_increase = value;
        assert!(changed.validate().is_err());
        let mut changed = policy();
        changed
            .objective_regression_upper_95
            .insert("tracking".to_owned(), value);
        assert!(changed.validate().is_err());
    }

    let mut changed = policy();
    changed.objective_regression_upper_95.clear();
    assert!(changed.validate().is_err());
    let mut changed = policy();
    changed
        .objective_regression_upper_95
        .insert("bad name".to_owned(), 0.1);
    assert!(changed.validate().is_err());
}

#[test]
fn expected_pairs_bind_role_candidate_scenario_repetition_seed_and_intent() {
    let stage = stage();
    let plan = plan();
    let pairs = expected_promotion_pairs(&stage, plan).expect("derive expected pairs");
    assert_eq!(pairs.len(), 3);

    for (repetition, pair) in pairs.iter().enumerate() {
        pair.validate().expect("validate expected pair");
        assert_eq!(pair.baseline.key.role, AttemptRole::PromotionBaseline);
        assert_eq!(pair.frozen.key.role, AttemptRole::PromotionFrozen);
        assert_eq!(
            pair.baseline.key.candidate_digest,
            plan.initial_candidate_digest
        );
        assert_eq!(
            pair.frozen.key.candidate_digest,
            plan.frozen_candidate_digest
        );
        assert_eq!(pair.baseline.key.mission_revision_id, "promotion-calm");
        assert_eq!(pair.baseline.key.mission_content_digest, fixed_digest(12));
        assert_eq!(pair.baseline.key.repetition, repetition as u32);
        assert_eq!(pair.baseline.key.seed, pair.frozen.key.seed);
        assert_eq!(
            pair.baseline.run_intent_digest,
            pair.baseline.context.digest().expect("digest context")
        );
    }
}

#[test]
fn paired_scenario_digest_v1_has_fixed_seed_vectors() {
    let pairs = expected_promotion_pairs(&stage(), plan()).expect("derive expected pairs");
    let seeds = pairs
        .iter()
        .map(|pair| pair.baseline.key.seed)
        .collect::<Vec<_>>();

    assert_eq!(
        seeds,
        vec![
            14_595_588_602_921_820_888,
            12_462_641_087_678_968_576,
            16_933_578_650_998_301_027,
        ]
    );
}

#[test]
fn expected_run_validation_rejects_a_changed_key_or_context_digest() {
    let stage = stage();
    let mut pair = expected_promotion_pairs(&stage, plan())
        .expect("derive expected pairs")
        .remove(0);
    pair.baseline.key.seed = pair.baseline.key.seed.wrapping_add(1);
    assert!(pair.validate().is_err());

    let mut pair = expected_promotion_pairs(&stage, plan())
        .expect("derive expected pairs")
        .remove(0);
    pair.frozen.run_intent_digest = fixed_digest(99);
    assert!(pair.validate().is_err());
}

#[test]
fn promotion_run_plan_rejects_missing_or_repeated_identities() {
    let mut changed = plan();
    changed.tuning_session_digest = Digest::from_bytes([0; 32]);
    assert!(changed.validate().is_err());
    let mut changed = plan();
    changed.frozen_trial_id = changed.baseline_trial_id;
    assert!(changed.validate().is_err());
    let mut changed = plan();
    changed.initial_candidate_digest = Digest::from_bytes([0; 32]);
    assert!(changed.validate().is_err());
}

pub(super) fn policy() -> PromotionPolicy {
    PromotionPolicy {
        schema_version: PROMOTION_POLICY_SCHEMA_VERSION,
        seed_policy: PromotionSeedPolicy::PairedScenarioDigestV1,
        minimum_loss_improvement: 0.1,
        minimum_relative_loss_improvement: 0.2,
        maximum_control_effort_increase: 0.3,
        objective_regression_upper_95: BTreeMap::from([
            ("settling".to_owned(), 0.4),
            ("tracking".to_owned(), 0.2),
        ]),
    }
}

pub(super) fn stage() -> SearchStage {
    SearchStage {
        execution_retry: crate::ExecutionRetryPolicy::none(),
        id: "stage-one".to_owned(),
        allowlist: BTreeMap::from([(
            "rate".to_owned(),
            ParameterBounds {
                minimum: 0.0,
                maximum: 1.0,
            },
        )]),
        fixed_parameters: BTreeMap::new(),
        required_hard_gates: vec!["crash".to_owned()],
        training_scenarios: vec![scenario("training-calm", 11)],
        training_suites: vec![crate::TrainingSuite {
            schema_version: crate::TRAINING_SUITE_SCHEMA_VERSION,
            id: "promotion-suite".to_owned(),
            primary_scenarios: vec![scenario("training-calm", 11)],
            guard_scenarios: Vec::new(),
            guard_regression_limits: BTreeMap::new(),
            repetitions: 3,
        }],
        search_groups: vec![crate::SearchGroup {
            id: "promotion-group".to_owned(),
            kind: crate::SearchGroupKind::Controller,
            parameters: std::collections::BTreeSet::from(["rate".to_owned()]),
            suite_id: "promotion-suite".to_owned(),
        }],
        promotion_scenarios: vec![scenario("promotion-calm", 12)],
        final_qualification_scenarios: vec![scenario("final-calm", 13)],
        repetitions: 3,
        promotion: policy(),
        qualification: QualificationPolicy {
            maximum_loss_confidence_upper: 1.0,
            maximum_p95_loss: 1.0,
            maximum_mean_control_effort: 1.0,
            objective_maxima: BTreeMap::from([
                ("settling".to_owned(), 1.0),
                ("tracking".to_owned(), 1.0),
            ]),
        },
    }
}

pub(super) fn plan() -> PromotionRunPlan {
    PromotionRunPlan {
        baseline_retry_index: 0,
        frozen_retry_index: 0,
        tuning_session_digest: fixed_digest(20),
        baseline_trial_id: 40,
        frozen_trial_id: 41,
        initial_candidate_digest: fixed_digest(21),
        frozen_candidate_digest: fixed_digest(22),
        fixed_seed: 23,
    }
}

fn scenario(id: &str, digest: u8) -> MissionReference {
    MissionReference {
        revision_id: id.to_owned(),
        schema_version: flight_tune::MISSION_SCHEMA_VERSION,
        content_digest: fixed_digest(digest),
        max_samples: 100,
        sample_timeout_ns: 20_000_000,
    }
}

pub(super) fn fixed_digest(value: u8) -> Digest {
    Digest::from_bytes([value; 32])
}

fn raw_policy_digest(document: &Value) -> Digest {
    let encoded = serde_json::to_vec(document).expect("encode changed policy");
    let mut bytes = Vec::with_capacity(PROMOTION_POLICY_DOMAIN.len().saturating_add(encoded.len()));
    bytes.extend_from_slice(PROMOTION_POLICY_DOMAIN);
    bytes.extend_from_slice(&encoded);
    digest_bytes(&bytes)
}

#[test]
fn validated_policy_uses_the_same_canonical_document_digest() {
    let policy = policy();
    assert_eq!(
        promotion_policy_digest(&policy).expect("digest policy"),
        digest_policy_content(&policy).expect("digest policy content")
    );
}
