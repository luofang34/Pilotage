#![allow(clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;

use crate::model::derive_seed;
use crate::{
    Digest, GateOutcome, HardGateFailure, MissionReference, ParameterBounds, PromotionPolicy,
    QualificationPolicy, ScenarioSet, SearchStage,
};

use super::validate_failure;
use crate::model::AttemptRunPlan;

#[test]
fn no_sample_failure_requires_the_exact_empty_stream_position() {
    let stage = stage();
    let plan = training_plan(&stage);
    let scenario = &stage.training_scenarios[0];
    let valid = failure(scenario, 0, 0);
    validate_failure(&valid, 0, ScenarioSet::Training, &plan, &stage, 91)
        .expect("validate exact empty stream");

    for forged in [failure(scenario, 1, 0), failure(scenario, 0, 1)] {
        assert!(validate_failure(&forged, 0, ScenarioSet::Training, &plan, &stage, 91,).is_err());
    }

    let mut declared_core_stage = stage.clone();
    declared_core_stage.required_hard_gates = vec!["core.no_samples".to_owned()];
    assert!(
        validate_failure(
            &failure(scenario, 0, 1),
            0,
            ScenarioSet::Training,
            &training_plan(&declared_core_stage),
            &declared_core_stage,
            91,
        )
        .is_err()
    );
}

fn training_plan(stage: &SearchStage) -> AttemptRunPlan {
    AttemptRunPlan::new(
        stage,
        crate::AttemptRole::TrainingBaseline { suite_index: 0 },
    )
    .expect("a training run plan")
}

fn failure(scenario: &MissionReference, sample_sequence: u64, elapsed_ms: u64) -> HardGateFailure {
    HardGateFailure {
        scenario_set: ScenarioSet::Training,
        mission_revision_id: scenario.revision_id.clone(),
        repetition: 0,
        seed: derive_seed(91, ScenarioSet::Training, scenario, 0),
        sample_sequence,
        elapsed_ms,
        gate: GateOutcome::fail(
            "core.no_samples",
            "the simulator completed without telemetry samples",
        ),
    }
}

fn stage() -> SearchStage {
    let scenario = MissionReference {
        revision_id: "empty-stream".to_owned(),
        schema_version: flight_tune::MISSION_SCHEMA_VERSION,
        content_digest: Digest::from_bytes([1; 32]),
        max_samples: 8,
        sample_timeout_ns: 100_000_000,
    };
    SearchStage {
        execution_retry: crate::ExecutionRetryPolicy::none(),
        id: "empty-stream-stage".to_owned(),
        allowlist: BTreeMap::from([(
            "gain".to_owned(),
            ParameterBounds {
                minimum: 0.0,
                maximum: 1.0,
            },
        )]),
        fixed_parameters: BTreeMap::new(),
        required_hard_gates: vec!["envelope".to_owned()],
        training_scenarios: vec![scenario.clone()],
        training_suites: vec![crate::TrainingSuite {
            schema_version: crate::TRAINING_SUITE_SCHEMA_VERSION,
            id: "plan-suite".to_owned(),
            primary_scenarios: vec![scenario.clone()],
            guard_scenarios: Vec::new(),
            guard_regression_limits: BTreeMap::new(),
            repetitions: 2,
        }],
        search_groups: vec![crate::SearchGroup {
            id: "plan-group".to_owned(),
            kind: crate::SearchGroupKind::Controller,
            parameters: std::collections::BTreeSet::from(["gain".to_owned()]),
            suite_id: "plan-suite".to_owned(),
        }],
        promotion_scenarios: vec![scenario.clone()],
        final_qualification_scenarios: vec![scenario],
        repetitions: 1,
        promotion: PromotionPolicy {
            schema_version: crate::PROMOTION_POLICY_SCHEMA_VERSION,
            seed_policy: crate::PromotionSeedPolicy::PairedScenarioDigestV1,
            minimum_loss_improvement: 0.0,
            minimum_relative_loss_improvement: 0.0,
            maximum_control_effort_increase: 0.0,
            objective_regression_upper_95: BTreeMap::from([("tracking".to_owned(), 0.0)]),
        },
        qualification: QualificationPolicy {
            maximum_loss_confidence_upper: 1.0,
            maximum_p95_loss: 1.0,
            maximum_mean_control_effort: 1.0,
            objective_maxima: BTreeMap::new(),
        },
    }
}
