use std::collections::{BTreeMap, HashSet};

use crate::model::{
    ExpectedPromotionPair, ExpectedPromotionRun, PromotionCalculation, PromotionComparison,
    PromotionObjectiveResult, PromotionPairedStatistics, PromotionRunPlan, PromotionSelection,
    expected_promotion_pairs, required_improvement,
};
use crate::score::{PairedStats, paired_stats};
use crate::{
    Digest, PromotionDecision, PromotionPolicy, RunRecord, RunTerminalCompletion,
    RunTerminalDisposition, RunTerminalReceipt, RunTerminalSemanticOutcome, SearchStage, TuneError,
};

#[cfg(test)]
#[path = "promotion/tests.rs"]
mod tests;

pub(crate) fn calculate(
    stage: &SearchStage,
    plan: PromotionRunPlan,
    baseline_receipts: &[RunTerminalReceipt],
    frozen_receipts: &[RunTerminalReceipt],
) -> Result<PromotionCalculation, TuneError> {
    let expected = expected_promotion_pairs(stage, plan)?;
    validate_receipt_counts(&expected, baseline_receipts, frozen_receipts)?;
    let baseline = authenticated_runs(&expected, baseline_receipts, |pair| &pair.baseline, stage)?;
    let frozen = authenticated_runs(&expected, frozen_receipts, |pair| &pair.frozen, stage)?;
    let comparison = compare_runs(&stage.promotion, &baseline, &frozen)?;
    let selection = select(
        &comparison,
        plan.initial_candidate_digest,
        plan.frozen_candidate_digest,
    );
    Ok(PromotionCalculation {
        comparison,
        selection,
    })
}

fn validate_receipt_counts(
    expected: &[ExpectedPromotionPair],
    baseline: &[RunTerminalReceipt],
    frozen: &[RunTerminalReceipt],
) -> Result<(), TuneError> {
    if expected.len() < 2 || baseline.len() != expected.len() || frozen.len() != expected.len() {
        return Err(pair_error(
            "promotion receipts do not match the expected run count",
        ));
    }
    let mut identities = HashSet::new();
    for pair in expected {
        pair.validate()?;
        if !identities.insert(pair.baseline.run_intent_digest)
            || !identities.insert(pair.frozen.run_intent_digest)
        {
            return Err(pair_error("an expected promotion run is repeated"));
        }
    }
    Ok(())
}

fn authenticated_runs<'a>(
    expected: &[ExpectedPromotionPair],
    receipts: &'a [RunTerminalReceipt],
    side: impl Fn(&ExpectedPromotionPair) -> &ExpectedPromotionRun,
    stage: &SearchStage,
) -> Result<Vec<&'a RunRecord>, TuneError> {
    let mut runs = Vec::with_capacity(receipts.len());
    let mut receipt_digests = HashSet::new();
    for (pair, receipt) in expected.iter().zip(receipts) {
        receipt.validate()?;
        let expected_run = side(pair);
        if receipt.context() != &expected_run.context
            || receipt.intent().run_intent_digest() != expected_run.run_intent_digest
            || !receipt_digests.insert(receipt.receipt_digest())
        {
            return Err(pair_error(
                "a promotion receipt identity changed or repeated",
            ));
        }
        let run = completed_run(receipt)?;
        validate_run_result(run, stage)?;
        runs.push(run);
    }
    Ok(runs)
}

fn validate_run_result(run: &RunRecord, stage: &SearchStage) -> Result<(), TuneError> {
    if run.passed_hard_gates != stage.required_hard_gates {
        return Err(pair_error(
            "one promotion run changed the exact ordered hard-gate set",
        ));
    }
    validate_objective_set(run, &stage.promotion)
}

fn completed_run(receipt: &RunTerminalReceipt) -> Result<&RunRecord, TuneError> {
    match (receipt.class().disposition(), receipt.intent().outcome()) {
        (
            RunTerminalDisposition::Completed {
                completion: RunTerminalCompletion::ScenarioComplete,
            },
            RunTerminalSemanticOutcome::ScenarioComplete { run, .. },
        ) => Ok(run),
        _ => Err(pair_error(
            "promotion comparison requires completed scenario receipts",
        )),
    }
}

fn validate_objective_set(run: &RunRecord, policy: &PromotionPolicy) -> Result<(), TuneError> {
    if run
        .objectives
        .keys()
        .ne(policy.objective_regression_upper_95.keys())
    {
        return Err(pair_error(
            "one promotion run changed the exact objective key set",
        ));
    }
    Ok(())
}

fn compare_runs(
    policy: &PromotionPolicy,
    baseline: &[&RunRecord],
    frozen: &[&RunRecord],
) -> Result<PromotionComparison, TuneError> {
    policy.validate()?;
    if baseline.len() != frozen.len() || baseline.len() < 2 {
        return Err(pair_error("promotion run keys do not form exact pairs"));
    }
    for run in baseline.iter().chain(frozen) {
        validate_objective_set(run, policy)?;
    }
    let baseline_mean_loss = finite_mean(baseline.iter().map(|run| run.loss))?;
    let loss = paired(
        baseline
            .iter()
            .zip(frozen)
            .map(|(left, right)| right.loss - left.loss),
    )?;
    let control_effort = paired(
        baseline
            .iter()
            .zip(frozen)
            .map(|(left, right)| right.control_effort - left.control_effort),
    )?;
    let required_loss_improvement = required_improvement(policy, baseline_mean_loss)?;
    let objectives = objective_results(policy, baseline, frozen)?;
    Ok(PromotionComparison {
        baseline_mean_loss,
        required_loss_improvement,
        loss,
        loss_passed: loss.upper_95 <= -required_loss_improvement,
        control_effort,
        control_effort_passed: control_effort.mean <= policy.maximum_control_effort_increase,
        objectives,
    })
}

fn objective_results(
    policy: &PromotionPolicy,
    baseline: &[&RunRecord],
    frozen: &[&RunRecord],
) -> Result<BTreeMap<String, PromotionObjectiveResult>, TuneError> {
    let mut results = BTreeMap::new();
    for (name, maximum) in &policy.objective_regression_upper_95 {
        let mut deltas = Vec::with_capacity(baseline.len());
        for (left, right) in baseline.iter().zip(frozen) {
            let left = objective(left, name)?;
            let right = objective(right, name)?;
            deltas.push(right - left);
        }
        let statistics = paired(deltas.into_iter())?;
        results.insert(
            name.clone(),
            PromotionObjectiveResult {
                statistics,
                maximum_upper_95: *maximum,
                passed: statistics.upper_95 <= *maximum,
            },
        );
    }
    Ok(results)
}

fn objective(run: &RunRecord, name: &str) -> Result<f64, TuneError> {
    run.objectives
        .get(name)
        .copied()
        .ok_or_else(|| pair_error(format!("promotion run has no objective {name}")))
}

fn paired(values: impl Iterator<Item = f64>) -> Result<PromotionPairedStatistics, TuneError> {
    let PairedStats { mean, upper_95 } = paired_stats(values)?;
    Ok(PromotionPairedStatistics { mean, upper_95 })
}

fn finite_mean(values: impl Iterator<Item = f64>) -> Result<f64, TuneError> {
    let mut count = 0_usize;
    let mut mean = 0.0;
    for value in values {
        if !value.is_finite() {
            return Err(pair_error("a promotion mean input is not finite"));
        }
        count = count.wrapping_add(1);
        mean += (value - mean) / count as f64;
        if !mean.is_finite() {
            return Err(pair_error("promotion mean arithmetic is not finite"));
        }
    }
    if count < 2 {
        return Err(pair_error("a promotion mean needs two runs"));
    }
    Ok(mean)
}

fn select(
    comparison: &PromotionComparison,
    initial_candidate: Digest,
    frozen_candidate: Digest,
) -> PromotionSelection {
    let decision = decision(comparison);
    let selected_candidate = if comparison.all_passed() {
        frozen_candidate
    } else {
        initial_candidate
    };
    PromotionSelection {
        decision,
        selected_candidate: Some(selected_candidate),
    }
}

fn decision(comparison: &PromotionComparison) -> PromotionDecision {
    if comparison.all_passed() {
        PromotionDecision::Promoted {}
    } else {
        PromotionDecision::RejectedNoImprovement {}
    }
}

fn pair_error(detail: impl Into<String>) -> TuneError {
    TuneError::InvalidScore {
        detail: detail.into(),
    }
}
