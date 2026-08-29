use std::collections::BTreeMap;

use crate::{CandidateEvaluation, RunRecord, TrainingSuite};

#[cfg(test)]
#[path = "selection/tests.rs"]
mod tests;

/// Decides whether one challenger replaces the incumbent on one suite.
///
/// The primary missions decide the improvement and the guard missions decide
/// the regression, so a primary gain cannot pay for a guard loss. Both sides
/// run the same missions in the same order with the same seeds, so the
/// comparison reads paired runs and never runs from two different plans.
pub(crate) fn training_better(
    suite: &TrainingSuite,
    baseline: Option<&CandidateEvaluation>,
    challenger: &CandidateEvaluation,
) -> bool {
    let Some(CandidateEvaluation::Passed {
        runs: incumbent, ..
    }) = baseline
    else {
        return false;
    };
    let CandidateEvaluation::Passed { runs: proposed, .. } = challenger else {
        return false;
    };
    let primary = suite.primary_run_count();
    if primary == 0 || incumbent.len() != proposed.len() || incumbent.len() < primary {
        return false;
    }
    let (Some(before), Some(after)) = (
        mean_loss(incumbent.get(..primary)),
        mean_loss(proposed.get(..primary)),
    ) else {
        return false;
    };
    after < before && guards_hold(suite, incumbent.get(primary..), proposed.get(primary..))
}

fn guards_hold(
    suite: &TrainingSuite,
    incumbent: Option<&[RunRecord]>,
    proposed: Option<&[RunRecord]>,
) -> bool {
    if suite.guard_regression_limits.is_empty() {
        return true;
    }
    let (Some(incumbent), Some(proposed)) = (incumbent, proposed) else {
        return false;
    };
    if incumbent.is_empty() || incumbent.len() != proposed.len() {
        return false;
    }
    let before = guard_means(incumbent);
    let after = guard_means(proposed);
    suite.guard_regression_limits.iter().all(|(name, limit)| {
        match (before.get(name), after.get(name)) {
            (Some(baseline), Some(challenger)) => *challenger <= baseline + limit,
            _ => false,
        }
    })
}

/// Returns the mean of each named objective across the guard runs.
///
/// An objective that a run does not state leaves the comparison. A guard that
/// the evidence cannot show is a guard that does not hold.
fn guard_means(runs: &[RunRecord]) -> BTreeMap<String, f64> {
    let mut totals: BTreeMap<String, (f64, usize)> = BTreeMap::new();
    for run in runs {
        for (name, value) in &run.objectives {
            let entry = totals.entry(name.clone()).or_insert((0.0, 0));
            *entry = (entry.0 + value, entry.1.wrapping_add(1));
        }
    }
    totals
        .into_iter()
        .filter(|(_, (sum, count))| *count == runs.len() && sum.is_finite())
        .map(|(name, (sum, count))| (name, sum / count as f64))
        .collect()
}

fn mean_loss(runs: Option<&[RunRecord]>) -> Option<f64> {
    let runs = runs?;
    if runs.is_empty() {
        return None;
    }
    let sum: f64 = runs.iter().map(|run| run.loss).sum();
    let mean = sum / runs.len() as f64;
    mean.is_finite().then_some(mean)
}
