#![allow(clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;

use crate::model::derive_seed;
use crate::{
    Digest, GateOutcome, HardGateFailure, ParameterBounds, PromotionPolicy, QualificationPolicy,
    ScenarioRef, ScenarioSet, SearchStage,
};

use super::validate_failure;

#[test]
fn no_sample_failure_requires_the_exact_empty_stream_position() {
    let stage = stage();
    let scenario = &stage.training_scenarios[0];
    let valid = failure(scenario, 0, 0);
    validate_failure(
        &valid,
        0,
        ScenarioSet::Training,
        &stage.training_scenarios,
        &stage,
        91,
    )
    .expect("validate exact empty stream");

    for forged in [failure(scenario, 1, 0), failure(scenario, 0, 1)] {
        assert!(
            validate_failure(
                &forged,
                0,
                ScenarioSet::Training,
                &stage.training_scenarios,
                &stage,
                91,
            )
            .is_err()
        );
    }

    let mut declared_core_stage = stage.clone();
    declared_core_stage.required_hard_gates = vec!["core.no_samples".to_owned()];
    assert!(
        validate_failure(
            &failure(scenario, 0, 1),
            0,
            ScenarioSet::Training,
            &declared_core_stage.training_scenarios,
            &declared_core_stage,
            91,
        )
        .is_err()
    );
}

fn failure(scenario: &ScenarioRef, sample_sequence: u64, elapsed_ms: u64) -> HardGateFailure {
    HardGateFailure {
        scenario_set: ScenarioSet::Training,
        scenario_id: scenario.id.clone(),
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
    let scenario = ScenarioRef {
        id: "empty-stream".to_owned(),
        digest: Digest::from_bytes([1; 32]),
        max_samples: 8,
        sample_timeout_ms: 100,
    };
    SearchStage {
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
        promotion_scenarios: vec![scenario.clone()],
        final_qualification_scenarios: vec![scenario],
        repetitions: 1,
        promotion: PromotionPolicy {
            minimum_loss_improvement: 0.0,
            minimum_relative_loss_improvement: 0.0,
            maximum_control_effort_increase: 0.0,
        },
        qualification: QualificationPolicy {
            maximum_loss_confidence_upper: 1.0,
            maximum_p95_loss: 1.0,
            maximum_mean_control_effort: 1.0,
            objective_maxima: BTreeMap::new(),
        },
    }
}
