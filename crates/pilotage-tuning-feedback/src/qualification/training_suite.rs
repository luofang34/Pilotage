//! The frozen suite rules the verifier derives instead of reading.
//!
//! A campaign states which search group a challenger changed. That statement
//! is not evidence: the group follows from the parameters that differ between
//! the incumbent and the challenger, and nothing else. Every rule here reads
//! the frozen stage and the exact candidates.

use std::collections::{BTreeMap, BTreeSet};

use flight_tune::{
    Candidate, Digest, MissionReference, RunRecord, SearchGroupBinding, SearchGroupKind,
    SearchStage, TrainingSuite,
};

use crate::{FeedbackError, digest, error::invalid};

const TRAINING_SUITE_DOMAIN: &[u8] = b"pilotage.flight-tune.training-suite.v1\0";
const TRAINING_SUITE_SCHEMA_VERSION: u16 = 1;
const MAX_TRAINING_SUITES: usize = 16;
const MAX_SEARCH_GROUPS: usize = 16;
const MAX_GUARD_LIMITS: usize = 32;
const MAX_SCENARIOS_PER_SET: usize = 64;

/// Returns the content identity of one frozen suite.
pub(super) fn suite_digest(suite: &TrainingSuite) -> Result<Digest, FeedbackError> {
    digest::domain("training suite", TRAINING_SUITE_DOMAIN, suite)
}

/// Returns the suite at one position in the frozen suite order.
pub(super) fn suite_at(stage: &SearchStage, index: u16) -> Result<&TrainingSuite, FeedbackError> {
    stage
        .training_suites
        .get(index as usize)
        .ok_or_else(|| invalid("an attempt names a suite the stage does not declare"))
}

/// Returns the complete ordered mission list of one suite run plan.
pub(super) fn ordered_scenarios(suite: &TrainingSuite) -> Vec<MissionReference> {
    let mut scenarios = suite.primary_scenarios.clone();
    scenarios.extend(suite.guard_scenarios.iter().cloned());
    scenarios
}

/// Returns how many runs of one suite attempt carry the primary loss.
pub(super) fn primary_runs(suite: &TrainingSuite) -> usize {
    suite
        .primary_scenarios
        .len()
        .saturating_mul(suite.repetitions as usize)
}

/// Derives the search group that one candidate difference selects.
pub(super) fn derived_group(
    stage: &SearchStage,
    incumbent: &Candidate,
    challenger: &Candidate,
) -> Result<SearchGroupBinding, FeedbackError> {
    let changed = changed_parameters(incumbent, challenger);
    let Some(first) = changed.first() else {
        return Err(invalid("a recorded transition changes no parameter"));
    };
    let mut owners = BTreeSet::new();
    for name in &changed {
        let owner = stage
            .search_groups
            .iter()
            .find(|group| group.parameters.contains(name))
            .ok_or_else(|| invalid("a changed parameter has no search group"))?;
        owners.insert(owner.id.as_str());
    }
    if owners.len() != 1 {
        return Err(invalid(
            "a recorded transition changes parameters from two search groups",
        ));
    }
    let group = stage
        .search_groups
        .iter()
        .find(|group| group.parameters.contains(first.as_str()))
        .ok_or_else(|| invalid("a changed parameter has no search group"))?;
    let (index, suite) = stage
        .training_suites
        .iter()
        .enumerate()
        .find(|(_, suite)| suite.id == group.suite_id)
        .ok_or_else(|| invalid("a search group has no declared suite"))?;
    Ok(SearchGroupBinding {
        group_id: group.id.clone(),
        suite_id: suite.id.clone(),
        suite_index: u16::try_from(index)
            .map_err(|_| invalid("a suite index exceeds the supported range"))?,
        suite_digest: suite_digest(suite)?,
    })
}

fn changed_parameters(incumbent: &Candidate, challenger: &Candidate) -> Vec<String> {
    challenger
        .parameters()
        .iter()
        .filter(|(name, value)| incumbent.parameters().get(*name) != Some(*value))
        .map(|(name, _)| name.clone())
        .collect()
}

/// Derives whether one challenger replaces the incumbent on one suite.
///
/// The primary missions decide the improvement and the guard missions decide
/// the regression. A recorded decision this derivation does not reproduce is
/// refused, so a primary gain cannot pay for a guard loss.
pub(super) fn training_better(
    suite: &TrainingSuite,
    baseline: Option<&[RunRecord]>,
    challenger: Option<&[RunRecord]>,
) -> bool {
    let (Some(incumbent), Some(proposed)) = (baseline, challenger) else {
        return false;
    };
    let primary = primary_runs(suite);
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

/// Checks the complete link from every search group to its frozen suite.
pub(super) fn verify_search_space(stage: &SearchStage) -> Result<(), FeedbackError> {
    if stage.training_suites.is_empty()
        || stage.training_suites.len() > MAX_TRAINING_SUITES
        || stage.search_groups.is_empty()
        || stage.search_groups.len() > MAX_SEARCH_GROUPS
    {
        return Err(invalid(
            "the training suite or search group count is not valid",
        ));
    }
    let mut suite_ids = BTreeSet::new();
    for suite in &stage.training_suites {
        verify_suite(suite, &stage.training_scenarios)?;
        if !suite_ids.insert(suite.id.as_str()) {
            return Err(invalid("a training suite identity is repeated"));
        }
    }
    let named = verify_groups(stage, &suite_ids)?;
    verify_group_families(stage)?;
    for suite in &stage.training_suites {
        if !named.contains(suite.id.as_str()) {
            return Err(invalid("a training suite has no search group"));
        }
    }
    for scenario in &stage.training_scenarios {
        if !stage.training_suites.iter().any(|suite| {
            suite
                .primary_scenarios
                .iter()
                .chain(&suite.guard_scenarios)
                .any(|mission| mission == scenario)
        }) {
            return Err(invalid("a training mission belongs to no suite"));
        }
    }
    Ok(())
}

/// Requires each operator-feel suite to guard a direct response.
///
/// A suite that only scores the operator response would accept a command shape
/// that improves the operator trial and degrades the response a controller
/// group was tuned for.
fn verify_group_families(stage: &SearchStage) -> Result<(), FeedbackError> {
    for group in &stage.search_groups {
        if group.kind != SearchGroupKind::OperatorFeel {
            continue;
        }
        let guarded = stage
            .training_suites
            .iter()
            .find(|suite| suite.id == group.suite_id)
            .is_some_and(|suite| !suite.guard_scenarios.is_empty());
        if !guarded {
            return Err(invalid("an operator-feel group takes an unguarded suite"));
        }
    }
    Ok(())
}

fn verify_groups<'a>(
    stage: &'a SearchStage,
    suite_ids: &BTreeSet<&str>,
) -> Result<BTreeSet<&'a str>, FeedbackError> {
    let mut group_ids = BTreeSet::new();
    let mut named = BTreeSet::new();
    let mut owners = BTreeMap::new();
    for group in &stage.search_groups {
        if group.id.trim().is_empty()
            || group.id.len() > 128
            || !group_ids.insert(group.id.as_str())
            || !suite_ids.contains(group.suite_id.as_str())
            || group.parameters.is_empty()
            || group.parameters.len() > stage.allowlist.len()
        {
            return Err(invalid("a search group declaration is not valid"));
        }
        named.insert(group.suite_id.as_str());
        for name in &group.parameters {
            if !stage.allowlist.contains_key(name)
                || owners.insert(name.as_str(), group.id.as_str()).is_some()
            {
                return Err(invalid("a search group parameter has more than one role"));
            }
        }
    }
    if stage
        .allowlist
        .keys()
        .any(|name| !owners.contains_key(name.as_str()))
    {
        return Err(invalid("an allowlisted parameter has no search group"));
    }
    Ok(named)
}

fn verify_suite(
    suite: &TrainingSuite,
    training_scenarios: &[MissionReference],
) -> Result<(), FeedbackError> {
    let count = suite
        .primary_scenarios
        .len()
        .saturating_add(suite.guard_scenarios.len());
    if suite.schema_version != TRAINING_SUITE_SCHEMA_VERSION
        || suite.id.trim().is_empty()
        || suite.id.len() > 128
        || suite.primary_scenarios.is_empty()
        || count > MAX_SCENARIOS_PER_SET
        || !(2..=32).contains(&suite.repetitions)
        || suite.guard_scenarios.is_empty() != suite.guard_regression_limits.is_empty()
        || suite.guard_regression_limits.len() > MAX_GUARD_LIMITS
    {
        return Err(invalid("a training suite declaration is not valid"));
    }
    let mut used = BTreeSet::new();
    for scenario in suite.primary_scenarios.iter().chain(&suite.guard_scenarios) {
        if !training_scenarios.contains(scenario) || !used.insert(scenario.revision_id.as_str()) {
            return Err(invalid(
                "a training suite mission is repeated or is not a training mission",
            ));
        }
    }
    for (name, limit) in &suite.guard_regression_limits {
        if name.trim().is_empty()
            || name.len() > 128
            || name.chars().any(char::is_whitespace)
            || !limit.is_finite()
            || *limit < 0.0
        {
            return Err(invalid("a training suite guard limit is not valid"));
        }
    }
    Ok(())
}
