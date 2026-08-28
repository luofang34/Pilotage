use super::super::calculate;
use super::{MetricPoint, evidence, expected_pairs, plan, receipt, stage};
use crate::score::aggregate_runs;
use crate::{CandidateEvaluation, RunRecord, RunTerminalSemanticOutcome, ScenarioSet};

#[test]
fn every_baseline_and_frozen_run_requires_the_exact_objective_set() {
    for frozen_side in [false, true] {
        for run_index in 0..3 {
            for extra in [false, true] {
                let stage = stage();
                let pairs = expected_pairs(&stage);
                let mut evidence =
                    evidence(&stage, MetricPoint::baseline(), MetricPoint::passing());
                let (expected, point) = if frozen_side {
                    (&pairs[run_index].frozen, MetricPoint::passing())
                } else {
                    (&pairs[run_index].baseline, MetricPoint::baseline())
                };
                let mut objectives = point.objectives();
                if extra {
                    objectives.insert("foreign".to_owned(), 0.0);
                } else {
                    objectives.remove("tracking");
                }
                let changed = receipt(expected, point, objectives);
                if frozen_side {
                    evidence.frozen[run_index] = changed;
                } else {
                    evidence.baseline[run_index] = changed;
                }
                assert!(
                    calculate(&stage, plan(), &evidence.baseline, &evidence.frozen).is_err(),
                    "side={frozen_side} run={run_index} extra={extra}"
                );
            }
        }
    }
}

#[test]
fn an_objective_in_other_pairs_cannot_supply_one_missing_value() {
    let stage = stage();
    let pairs = expected_pairs(&stage);
    let mut evidence = evidence(&stage, MetricPoint::baseline(), MetricPoint::passing());
    let mut objectives = MetricPoint::baseline().objectives();
    objectives.remove("settling");
    evidence.baseline[1] = receipt(&pairs[1].baseline, MetricPoint::baseline(), objectives);

    assert!(calculate(&stage, plan(), &evidence.baseline, &evidence.frozen).is_err());
}

#[test]
fn non_finite_run_and_aggregate_data_fail_before_decision() {
    for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let stage = stage();
        let evidence = evidence(&stage, MetricPoint::baseline(), MetricPoint::passing());
        let baseline_runs = records(&evidence.baseline);
        let baseline = evaluation(baseline_runs);

        let mut changed = baseline.clone();
        if let CandidateEvaluation::Passed { aggregate, .. } = &mut changed {
            aggregate.mean_loss = value;
        }
        assert!(
            changed.validate(ScenarioSet::Promotion).is_err(),
            "aggregate value {value}"
        );

        let mut changed = baseline;
        if let CandidateEvaluation::Passed { runs, .. } = &mut changed {
            runs[0].loss = value;
        }
        assert!(
            changed.validate(ScenarioSet::Promotion).is_err(),
            "run value {value}"
        );
    }
}

fn records(receipts: &[crate::RunTerminalReceipt]) -> Vec<RunRecord> {
    receipts
        .iter()
        .map(|receipt| match receipt.intent().outcome() {
            RunTerminalSemanticOutcome::ScenarioComplete { run, .. } => run.clone(),
            _ => panic!("test receipt is not complete"),
        })
        .collect()
}

fn evaluation(runs: Vec<RunRecord>) -> CandidateEvaluation {
    let aggregate = aggregate_runs(&runs, ScenarioSet::Promotion).expect("aggregate runs");
    CandidateEvaluation::Passed { aggregate, runs }
}
