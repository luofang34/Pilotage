use std::collections::{BTreeMap, HashSet};

use crate::model::{
    ExpectedPromotionPair, ExpectedPromotionRun, PromotionCalculation, PromotionComparison,
    PromotionObjectiveResult, PromotionPairedStatistics, PromotionRunPlan,
    PromotionScenarioResults, PromotionSelection, expected_promotion_pairs, required_improvement,
};
use crate::score::{PairedStats, paired_stats};
use crate::{
    Digest, PromotionDecision, RunRecord, RunTerminalCompletion, RunTerminalDisposition,
    RunTerminalReceipt, RunTerminalSemanticOutcome, SearchStage, TuneError,
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
    let comparison = compare_runs(stage, &expected, &baseline, &frozen)?;
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
    validate_objective_set(run, stage)
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

fn validate_objective_set(run: &RunRecord, stage: &SearchStage) -> Result<(), TuneError> {
    let expected = stage
        .response_targets
        .expected_objective_names(&run.mission_revision_id, &stage.promotion.objectives);
    if run.objectives.keys().ne(expected.iter()) {
        return Err(pair_error(
            "one promotion run changed the exact objective key set",
        ));
    }
    Ok(())
}

fn compare_runs(
    stage: &SearchStage,
    expected: &[ExpectedPromotionPair],
    baseline: &[&RunRecord],
    frozen: &[&RunRecord],
) -> Result<PromotionComparison, TuneError> {
    let policy = &stage.promotion;
    policy.validate()?;
    if baseline.len() != frozen.len() || baseline.len() < 2 || expected.len() != baseline.len() {
        return Err(pair_error("promotion run keys do not form exact pairs"));
    }
    for run in baseline.iter().chain(frozen) {
        validate_objective_set(run, stage)?;
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
    let scenarios = scenario_results(stage, expected, baseline, frozen)?;
    Ok(PromotionComparison {
        baseline_mean_loss,
        required_loss_improvement,
        loss,
        loss_passed: loss.upper_95 <= -required_loss_improvement,
        control_effort,
        control_effort_passed: control_effort.mean <= policy.maximum_control_effort_increase,
        scenarios,
    })
}

/// One result group for each promotion scenario, over that scenario's own
/// paired runs and against that scenario's own scoped limits.
fn scenario_results(
    stage: &SearchStage,
    expected: &[ExpectedPromotionPair],
    baseline: &[&RunRecord],
    frozen: &[&RunRecord],
) -> Result<BTreeMap<String, PromotionScenarioResults>, TuneError> {
    let mut results = BTreeMap::new();
    for scenario in &stage.promotion_scenarios {
        let pairs = scenario_pairs(expected, baseline, frozen, &scenario.revision_id);
        if pairs.len() < 2 {
            return Err(pair_error(format!(
                "promotion scenario {} has fewer than two paired runs",
                scenario.revision_id
            )));
        }
        results.insert(
            scenario.revision_id.clone(),
            PromotionScenarioResults {
                mission_content_digest: scenario.content_digest,
                authority_band: stage.response_targets.authority_band(&scenario.revision_id),
                authority_passed: authority_holds(stage, &scenario.revision_id, &pairs),
                objectives: objective_results(stage, &scenario.revision_id, &pairs)?,
            },
        );
    }
    Ok(results)
}

/// The baseline and frozen runs that one scenario produced, in run order.
fn scenario_pairs<'a>(
    expected: &[ExpectedPromotionPair],
    baseline: &[&'a RunRecord],
    frozen: &[&'a RunRecord],
    mission_revision_id: &str,
) -> Vec<(&'a RunRecord, &'a RunRecord)> {
    expected
        .iter()
        .zip(baseline.iter().zip(frozen))
        .filter(|(pair, _)| pair.baseline.key.mission_revision_id == mission_revision_id)
        .map(|(_, (left, right))| (*left, *right))
        .collect()
}

/// Whether both sides of every pair kept the operator authority.
///
/// The challenger is the candidate under decision, but the incumbent is the
/// evidence it is measured against: a paired improvement over a baseline that
/// had already given the authority away is not an improvement anyone asked
/// for.
fn authority_holds(
    stage: &SearchStage,
    mission_revision_id: &str,
    pairs: &[(&RunRecord, &RunRecord)],
) -> bool {
    pairs.iter().all(|(left, right)| {
        [left, right].into_iter().all(|run| {
            stage.response_targets.authority_holds(
                mission_revision_id,
                run.objectives
                    .get(crate::TARGET_AUTHORITY_OBJECTIVE)
                    .copied(),
            )
        })
    })
}

fn objective_results(
    stage: &SearchStage,
    mission_revision_id: &str,
    pairs: &[(&RunRecord, &RunRecord)],
) -> Result<BTreeMap<String, PromotionObjectiveResult>, TuneError> {
    let mut results = BTreeMap::new();
    for name in &stage.promotion.objectives {
        let target = stage.response_targets.target(mission_revision_id, name)?;
        let mut deltas = Vec::with_capacity(pairs.len());
        for (left, right) in pairs {
            deltas.push(objective(right, name)? - objective(left, name)?);
        }
        let statistics = paired(deltas.into_iter())?;
        results.insert(
            name.clone(),
            PromotionObjectiveResult {
                statistics,
                maximum_upper_95: target.limit,
                passed: target.holds(statistics.upper_95),
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
