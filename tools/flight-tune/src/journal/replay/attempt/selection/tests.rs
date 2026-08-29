#![allow(clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;

use pilotage_mission_core::MISSION_SCHEMA_VERSION;
use pilotage_trial::Digest;

use crate::{
    CandidateEvaluation, MissionReference, RunRecord, ScenarioSet, TRAINING_SUITE_SCHEMA_VERSION,
    TrainingSuite,
};

use super::training_better;

fn mission(id: &str, seed: u8) -> MissionReference {
    MissionReference {
        revision_id: id.to_owned(),
        schema_version: MISSION_SCHEMA_VERSION,
        content_digest: Digest::from_bytes([seed; 32]),
        max_samples: 64,
        sample_timeout_ns: 20_000_000,
    }
}

fn guarded_suite() -> TrainingSuite {
    TrainingSuite {
        schema_version: TRAINING_SUITE_SCHEMA_VERSION,
        id: "operator-feel".to_owned(),
        primary_scenarios: vec![mission("operator", 1)],
        guard_scenarios: vec![mission("direct", 2)],
        guard_regression_limits: BTreeMap::from([("response.overshoot".to_owned(), 0.05)]),
        repetitions: 2,
    }
}

fn run(mission: &str, loss: f64, guard_value: f64) -> RunRecord {
    RunRecord {
        scenario_set: ScenarioSet::Training,
        mission_revision_id: mission.to_owned(),
        repetition: 0,
        seed: 7,
        loss,
        control_effort: 0.2,
        objectives: BTreeMap::from([("response.overshoot".to_owned(), guard_value)]),
        passed_hard_gates: vec!["envelope".to_owned()],
    }
}

/// Two primary runs then two guard runs, in the suite's own run order.
fn evaluation(primary_loss: f64, guard_value: f64) -> CandidateEvaluation {
    let runs = vec![
        run("operator", primary_loss, 0.0),
        run("operator", primary_loss, 0.0),
        run("direct", 9.0, guard_value),
        run("direct", 9.0, guard_value),
    ];
    CandidateEvaluation::Passed {
        aggregate: crate::score::aggregate_runs(&runs, ScenarioSet::Training)
            .expect("an aggregate"),
        runs,
    }
}

#[test]
fn a_primary_improvement_inside_the_guard_limit_is_selected() {
    let suite = guarded_suite();
    let baseline = evaluation(1.0, 0.10);

    assert!(training_better(
        &suite,
        Some(&baseline),
        &evaluation(0.5, 0.14)
    ));
}

#[test]
fn a_primary_improvement_cannot_pass_after_a_guard_regression() {
    let suite = guarded_suite();
    let baseline = evaluation(1.0, 0.10);

    assert!(!training_better(
        &suite,
        Some(&baseline),
        &evaluation(0.1, 0.20)
    ));
}

#[test]
fn a_guard_improvement_cannot_pay_for_a_primary_regression() {
    let suite = guarded_suite();
    let baseline = evaluation(1.0, 0.10);

    assert!(!training_better(
        &suite,
        Some(&baseline),
        &evaluation(1.5, 0.01)
    ));
}

#[test]
fn the_guard_runs_never_enter_the_primary_loss() {
    let suite = guarded_suite();
    let baseline = evaluation(1.0, 0.10);
    let mut challenger = evaluation(0.9, 0.10);
    if let CandidateEvaluation::Passed { runs, .. } = &mut challenger {
        // A challenger that is far worse on the guard mission stays selected
        // while the guard objective holds, because the guard loss is not part
        // of the primary comparison.
        runs[2].loss = 90.0;
        runs[3].loss = 90.0;
    }

    assert!(training_better(&suite, Some(&baseline), &challenger));
}

#[test]
fn a_missing_guard_objective_refuses_the_challenger() {
    let suite = guarded_suite();
    let baseline = evaluation(1.0, 0.10);
    let mut challenger = evaluation(0.5, 0.10);
    if let CandidateEvaluation::Passed { runs, .. } = &mut challenger {
        runs[2].objectives.clear();
    }

    assert!(!training_better(&suite, Some(&baseline), &challenger));
}

#[test]
fn an_absent_baseline_refuses_the_challenger() {
    let suite = guarded_suite();

    assert!(!training_better(&suite, None, &evaluation(0.1, 0.0)));
}

#[test]
fn a_quarantined_baseline_refuses_the_challenger() {
    let suite = guarded_suite();
    let baseline = CandidateEvaluation::Quarantined {
        reason: "the run did not complete".to_owned(),
    };

    assert!(!training_better(
        &suite,
        Some(&baseline),
        &evaluation(0.1, 0.0)
    ));
}

#[test]
fn a_baseline_from_another_suite_length_refuses_the_challenger() {
    let suite = guarded_suite();
    let mut baseline = evaluation(1.0, 0.10);
    if let CandidateEvaluation::Passed { runs, .. } = &mut baseline {
        runs.truncate(2);
    }

    assert!(!training_better(
        &suite,
        Some(&baseline),
        &evaluation(0.1, 0.0)
    ));
}
