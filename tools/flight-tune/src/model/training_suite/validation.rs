use std::collections::{BTreeMap, BTreeSet};

use crate::{MissionReference, ParameterBounds, TuneError};

use super::{
    MAX_GUARD_LIMITS, MAX_SEARCH_GROUPS, MAX_TRAINING_SUITES, SearchGroup,
    TRAINING_SUITE_SCHEMA_VERSION, TrainingSuite, invalid_stage,
};

/// Validates the complete link from every search group to its frozen suite.
///
/// # Errors
///
/// Returns [`TuneError`] when a group, a suite, a parameter, or a training
/// mission does not have exactly one role.
pub(crate) fn validate_search_space(
    allowlist: &BTreeMap<String, ParameterBounds>,
    training_scenarios: &[MissionReference],
    suites: &[TrainingSuite],
    groups: &[SearchGroup],
) -> Result<(), TuneError> {
    if suites.is_empty() || suites.len() > MAX_TRAINING_SUITES {
        return Err(invalid_stage("the training suite count is not valid"));
    }
    if groups.is_empty() || groups.len() > MAX_SEARCH_GROUPS {
        return Err(invalid_stage("the search group count is not valid"));
    }
    let mut suite_ids = BTreeSet::new();
    for suite in suites {
        validate_suite(suite, training_scenarios)?;
        if !suite_ids.insert(suite.id.as_str()) {
            return Err(invalid_stage(format!("suite {} is repeated", suite.id)));
        }
    }
    let named = validate_groups(allowlist, groups, &suite_ids)?;
    for suite in suites {
        if !named.contains(suite.id.as_str()) {
            return Err(invalid_stage(format!(
                "suite {} has no search group",
                suite.id
            )));
        }
    }
    validate_scenario_coverage(training_scenarios, suites)
}

fn validate_groups<'a>(
    allowlist: &BTreeMap<String, ParameterBounds>,
    groups: &'a [SearchGroup],
    suite_ids: &BTreeSet<&str>,
) -> Result<BTreeSet<&'a str>, TuneError> {
    let mut group_ids = BTreeSet::new();
    let mut named_suites = BTreeSet::new();
    let mut owners = BTreeMap::new();
    for group in groups {
        super::super::validate_name(&group.id, "search group").map_err(as_stage_error)?;
        if !group_ids.insert(group.id.as_str()) {
            return Err(invalid_stage(format!("group {} is repeated", group.id)));
        }
        if !suite_ids.contains(group.suite_id.as_str()) {
            return Err(invalid_stage(format!(
                "group {} names suite {}, which the stage does not declare",
                group.id, group.suite_id
            )));
        }
        named_suites.insert(group.suite_id.as_str());
        validate_group_parameters(allowlist, group, &mut owners)?;
    }
    for name in allowlist.keys() {
        if !owners.contains_key(name.as_str()) {
            return Err(invalid_stage(format!(
                "parameter {name} has no search group"
            )));
        }
    }
    Ok(named_suites)
}

fn validate_group_parameters<'a>(
    allowlist: &BTreeMap<String, ParameterBounds>,
    group: &'a SearchGroup,
    owners: &mut BTreeMap<&'a str, &'a str>,
) -> Result<(), TuneError> {
    if group.parameters.is_empty() || group.parameters.len() > allowlist.len() {
        return Err(invalid_stage(format!(
            "group {} has an invalid parameter count",
            group.id
        )));
    }
    for name in &group.parameters {
        if !allowlist.contains_key(name) {
            return Err(invalid_stage(format!(
                "group {} claims parameter {name}, which the allowlist does not have",
                group.id
            )));
        }
        if let Some(other) = owners.insert(name.as_str(), group.id.as_str()) {
            return Err(invalid_stage(format!(
                "parameter {name} belongs to group {other} and group {}",
                group.id
            )));
        }
    }
    Ok(())
}

fn validate_suite(
    suite: &TrainingSuite,
    training_scenarios: &[MissionReference],
) -> Result<(), TuneError> {
    if suite.schema_version != TRAINING_SUITE_SCHEMA_VERSION {
        return Err(invalid_stage(format!(
            "training suite schema {} is not supported",
            suite.schema_version
        )));
    }
    super::super::validate_name(&suite.id, "training suite").map_err(as_stage_error)?;
    if suite.primary_scenarios.is_empty() {
        return Err(invalid_stage(format!(
            "suite {} has no primary mission",
            suite.id
        )));
    }
    let count = suite
        .primary_scenarios
        .len()
        .saturating_add(suite.guard_scenarios.len());
    if count > super::super::MAX_SCENARIOS_PER_SET {
        return Err(invalid_stage(format!(
            "suite {} has too many missions",
            suite.id
        )));
    }
    if !(2..=super::super::MAX_REPETITIONS).contains(&suite.repetitions) {
        return Err(invalid_stage(format!(
            "suite {} has an invalid repetition count",
            suite.id
        )));
    }
    validate_suite_scenarios(suite, training_scenarios)?;
    validate_guard_limits(suite)
}

fn validate_suite_scenarios(
    suite: &TrainingSuite,
    training_scenarios: &[MissionReference],
) -> Result<(), TuneError> {
    let mut used = BTreeSet::new();
    for scenario in suite.primary_scenarios.iter().chain(&suite.guard_scenarios) {
        scenario.validate()?;
        if !training_scenarios.contains(scenario) {
            return Err(invalid_stage(format!(
                "suite {} uses mission {}, which the training partition does not have",
                suite.id, scenario.revision_id
            )));
        }
        if !used.insert(scenario.revision_id.as_str()) {
            return Err(invalid_stage(format!(
                "suite {} repeats mission {}",
                suite.id, scenario.revision_id
            )));
        }
    }
    Ok(())
}

fn validate_guard_limits(suite: &TrainingSuite) -> Result<(), TuneError> {
    if suite.guard_scenarios.is_empty() != suite.guard_regression_limits.is_empty() {
        return Err(invalid_stage(format!(
            "suite {} states guard missions and guard limits separately",
            suite.id
        )));
    }
    if suite.guard_regression_limits.len() > MAX_GUARD_LIMITS {
        return Err(invalid_stage(format!(
            "suite {} has too many guard limits",
            suite.id
        )));
    }
    for (name, limit) in &suite.guard_regression_limits {
        super::super::validate_name(name, "guard objective").map_err(as_stage_error)?;
        if name.chars().any(char::is_whitespace) || !limit.is_finite() || *limit < 0.0 {
            return Err(invalid_stage(format!(
                "suite {} has an invalid guard limit for {name}",
                suite.id
            )));
        }
    }
    Ok(())
}

fn validate_scenario_coverage(
    training_scenarios: &[MissionReference],
    suites: &[TrainingSuite],
) -> Result<(), TuneError> {
    for scenario in training_scenarios {
        let used = suites.iter().any(|suite| {
            suite
                .primary_scenarios
                .iter()
                .chain(&suite.guard_scenarios)
                .any(|mission| mission == scenario)
        });
        if !used {
            return Err(invalid_stage(format!(
                "training mission {} belongs to no suite",
                scenario.revision_id
            )));
        }
    }
    Ok(())
}

fn as_stage_error(error: TuneError) -> TuneError {
    invalid_stage(error.to_string())
}
