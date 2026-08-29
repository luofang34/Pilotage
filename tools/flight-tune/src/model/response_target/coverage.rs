use std::collections::{BTreeMap, BTreeSet};

use pilotage_trial::Digest;

use crate::{MissionReference, SearchStage, TuneError};

use super::{ResponseTargetTable, invalid_table, is_admissible};

/// Requires that one table covers exactly the decisions one stage will take.
///
/// The check is a bijection, not a subset test in either direction. Every
/// hidden scenario needs a limit for every objective its policy declares, and
/// the table may state nothing else: a missing row would leave a decision with
/// no bar, and an extra row would be a limit that never applies and can be
/// changed without any decision changing.
///
/// # Errors
///
/// Returns [`TuneError`] when a row is missing, extra, or names a scenario
/// identity the stage does not carry.
pub(crate) fn validate_for_stage(
    table: &ResponseTargetTable,
    stage: &SearchStage,
) -> Result<(), TuneError> {
    table.validate()?;
    let expected = expected_rows(stage);
    let stated = stated_rows(table);
    verify_every_scenario_is_scoped(stage)?;
    if let Some((mission, objective)) = first_difference(&expected, &stated) {
        return Err(invalid_table(format!(
            "the response target table states no {objective} limit for {mission}"
        )));
    }
    if let Some((mission, objective)) = first_difference(&stated, &expected) {
        return Err(invalid_table(format!(
            "the response target table states a {objective} limit for {mission}, which no decision reads"
        )));
    }
    validate_scenario_identities(table, stage)
}

/// Every scenario and objective pair one stage decides on.
///
/// The promotion partition is scored against the promotion objective names and
/// the final partition against the qualification names, so a stage whose two
/// policies declare different objectives still gets one exact table.
fn expected_rows(stage: &SearchStage) -> BTreeSet<(&str, &str)> {
    let mut rows = BTreeSet::new();
    for (scenarios, objectives) in [
        (&stage.promotion_scenarios, &stage.promotion.objectives),
        (
            &stage.final_qualification_scenarios,
            &stage.qualification.objectives,
        ),
    ] {
        for scenario in scenarios {
            let Some(scope) = stage
                .response_targets
                .targets
                .iter()
                .find(|target| target.mission_revision_id == scenario.revision_id)
            else {
                // A scenario with no row at all is refused by the identity
                // check below, which names it rather than reporting one
                // missing objective at a time.
                continue;
            };
            for objective in objectives
                .iter()
                .filter(|name| is_admissible(name, scope.control_family, scope.motion))
            {
                rows.insert((scenario.revision_id.as_str(), objective.as_str()));
            }
        }
    }
    rows
}

/// Every hidden scenario has at least one scoped row.
///
/// A scenario with none states no bar at all, and the objective bijection
/// would report it as complete because a scope that answers nothing owes
/// nothing.
fn verify_every_scenario_is_scoped(stage: &SearchStage) -> Result<(), TuneError> {
    for scenario in stage
        .promotion_scenarios
        .iter()
        .chain(&stage.final_qualification_scenarios)
    {
        if !stage
            .response_targets
            .targets
            .iter()
            .any(|target| target.mission_revision_id == scenario.revision_id)
        {
            return Err(invalid_table(format!(
                "the response target table states no limit for {}",
                scenario.revision_id
            )));
        }
    }
    Ok(())
}

fn stated_rows(table: &ResponseTargetTable) -> BTreeSet<(&str, &str)> {
    table
        .targets
        .iter()
        .map(|target| {
            (
                target.mission_revision_id.as_str(),
                target.objective.as_str(),
            )
        })
        .collect()
}

fn first_difference<'a>(
    left: &BTreeSet<(&'a str, &'a str)>,
    right: &BTreeSet<(&'a str, &'a str)>,
) -> Option<(&'a str, &'a str)> {
    left.difference(right).next().copied()
}

/// Every row names the exact scenario content the stage schedules.
///
/// A row could otherwise carry a digest of its own and be checked only against
/// itself, so a scenario substitution would move the executed mission and
/// leave the table naming a different one.
fn validate_scenario_identities(
    table: &ResponseTargetTable,
    stage: &SearchStage,
) -> Result<(), TuneError> {
    let scheduled = scheduled_digests(stage);
    for target in &table.targets {
        let expected = scheduled
            .get(target.mission_revision_id.as_str())
            .copied()
            .ok_or_else(|| {
                invalid_table(format!(
                    "the response target table names {}, which the stage does not decide on",
                    target.mission_revision_id
                ))
            })?;
        if target.mission_content_digest != expected {
            return Err(invalid_table(format!(
                "the response target row for {} names other scenario content",
                target.mission_revision_id
            )));
        }
    }
    Ok(())
}

fn scheduled_digests(stage: &SearchStage) -> BTreeMap<&str, Digest> {
    stage
        .promotion_scenarios
        .iter()
        .chain(&stage.final_qualification_scenarios)
        .map(|scenario: &MissionReference| (scenario.revision_id.as_str(), scenario.content_digest))
        .collect()
}
