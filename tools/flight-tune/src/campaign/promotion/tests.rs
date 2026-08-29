#![allow(clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;

use crate::model::{
    ExpectedPromotionPair, ExpectedPromotionRun, PROMOTION_POLICY_SCHEMA_VERSION, PromotionRunPlan,
    PromotionSeedPolicy, expected_promotion_pairs,
};
use crate::{
    ArtifactIdentity, Digest, MissionReference, ParameterBounds, PromotionPolicy,
    QualificationPolicy, RUN_TERMINAL_OPERATION_ORDER, RunBindingReceipt, RunRecord,
    RunTerminalClass, RunTerminalIntent, RunTerminalOperation, RunTerminalOperationOutcome,
    RunTerminalPlan, RunTerminalReceipt, RunTerminalRecoveryState, RunTerminalReport,
    RunTerminalScope, RunTerminalSemanticOutcome, ScenarioSet, SearchStage,
};

#[path = "tests/boundaries.rs"]
mod boundaries;
#[path = "tests/identity.rs"]
mod identity;
#[path = "tests/objectives.rs"]
mod objectives;

pub(super) struct PromotionEvidence {
    pub(super) baseline: Vec<RunTerminalReceipt>,
    pub(super) frozen: Vec<RunTerminalReceipt>,
}

#[derive(Clone, Copy)]
pub(super) struct MetricPoint {
    pub(super) loss: f64,
    pub(super) effort: f64,
    pub(super) tracking: f64,
    pub(super) settling: f64,
    pub(super) overshoot: f64,
}

impl MetricPoint {
    pub(super) const fn baseline() -> Self {
        Self {
            loss: 1.0,
            effort: 0.3,
            tracking: 0.2,
            settling: 0.2,
            overshoot: 0.2,
        }
    }

    pub(super) const fn passing() -> Self {
        Self {
            loss: 0.8,
            effort: 0.35,
            tracking: 0.21,
            settling: 0.21,
            overshoot: 0.21,
        }
    }

    pub(super) fn objectives(self) -> BTreeMap<String, f64> {
        BTreeMap::from([
            ("overshoot".to_owned(), self.overshoot),
            ("settling".to_owned(), self.settling),
            ("tracking".to_owned(), self.tracking),
        ])
    }
}

pub(super) fn policy() -> PromotionPolicy {
    PromotionPolicy {
        schema_version: PROMOTION_POLICY_SCHEMA_VERSION,
        seed_policy: PromotionSeedPolicy::PairedScenarioDigestV1,
        minimum_loss_improvement: 0.1,
        minimum_relative_loss_improvement: 0.05,
        maximum_control_effort_increase: 0.1,
        objective_regression_upper_95: BTreeMap::from([
            ("overshoot".to_owned(), 0.05),
            ("settling".to_owned(), 0.05),
            ("tracking".to_owned(), 0.05),
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
            id: "campaign-suite".to_owned(),
            primary_scenarios: vec![scenario("training-calm", 11)],
            guard_scenarios: Vec::new(),
            guard_regression_limits: BTreeMap::new(),
            repetitions: 3,
        }],
        search_groups: vec![crate::SearchGroup {
            id: "campaign-group".to_owned(),
            kind: crate::SearchGroupKind::Controller,
            parameters: std::collections::BTreeSet::from(["rate".to_owned()]),
            suite_id: "campaign-suite".to_owned(),
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
                ("overshoot".to_owned(), 1.0),
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

pub(super) fn evidence(
    stage: &SearchStage,
    baseline: MetricPoint,
    frozen: MetricPoint,
) -> PromotionEvidence {
    let pairs = expected_pairs(stage);
    PromotionEvidence {
        baseline: pairs
            .iter()
            .map(|pair| {
                receipt_with_gates(
                    &pair.baseline,
                    baseline,
                    baseline.objectives(),
                    stage.required_hard_gates.clone(),
                )
            })
            .collect(),
        frozen: pairs
            .iter()
            .map(|pair| {
                receipt_with_gates(
                    &pair.frozen,
                    frozen,
                    frozen.objectives(),
                    stage.required_hard_gates.clone(),
                )
            })
            .collect(),
    }
}

pub(super) fn expected_pairs(stage: &SearchStage) -> Vec<ExpectedPromotionPair> {
    expected_promotion_pairs(stage, plan()).expect("derive expected pairs")
}

pub(super) fn receipt(
    expected: &ExpectedPromotionRun,
    point: MetricPoint,
    objectives: BTreeMap<String, f64>,
) -> RunTerminalReceipt {
    receipt_with_gates(expected, point, objectives, vec!["crash".to_owned()])
}

fn receipt_with_gates(
    expected: &ExpectedPromotionRun,
    point: MetricPoint,
    objectives: BTreeMap<String, f64>,
    passed_hard_gates: Vec<String>,
) -> RunTerminalReceipt {
    let context = &expected.context;
    receipt_for_context(
        context,
        RunRecord {
            scenario_set: ScenarioSet::Promotion,
            mission_revision_id: context.mission_revision_id().to_owned(),
            repetition: context.repetition(),
            seed: context.seed(),
            loss: point.loss,
            control_effort: point.effort,
            objectives,
            passed_hard_gates,
        },
    )
}

pub(super) fn receipt_for_context(
    context: &crate::RunExecutionContext,
    run: RunRecord,
) -> RunTerminalReceipt {
    let intent = RunTerminalIntent::new(
        context,
        context.digest().expect("digest run context"),
        RunTerminalSemanticOutcome::ScenarioComplete {
            candidate_digest: context.candidate_digest(),
            mission_content_digest: context.mission_content_digest(),
            run,
        },
    )
    .expect("create terminal intent");
    let plan = RunTerminalPlan::new(RunTerminalScope::Active).expect("create terminal plan");
    let report = RunTerminalReport::new(
        &plan,
        &intent,
        RunTerminalRecoveryState::Live,
        successful_outcomes(),
    )
    .expect("create terminal report");
    let class = RunTerminalClass::classify(&intent, &report).expect("classify terminal report");
    let adapter = ArtifactIdentity::new("promotion-test-adapter", fixed_digest(90))
        .expect("create adapter identity");
    let binding = RunBindingReceipt::new(context, &plan, adapter).expect("create binding receipt");
    RunTerminalReceipt::new(&binding, &intent, &report, class, fixed_digest(91))
        .expect("create terminal receipt")
}

fn successful_outcomes() -> Vec<RunTerminalOperationOutcome> {
    RUN_TERMINAL_OPERATION_ORDER
        .into_iter()
        .map(|operation| {
            let proof =
                (operation == RunTerminalOperation::ChildTerminate).then_some(fixed_digest(92));
            RunTerminalOperationOutcome::succeeded(operation, proof)
                .expect("create terminal operation result")
        })
        .collect()
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

pub(super) fn next_up(value: f64) -> f64 {
    if value >= 0.0 {
        f64::from_bits(value.to_bits().wrapping_add(1))
    } else {
        f64::from_bits(value.to_bits().wrapping_sub(1))
    }
}
