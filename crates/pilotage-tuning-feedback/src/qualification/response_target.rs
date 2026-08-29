//! The scoped response target table, recomputed rather than read.
//!
//! Every rule the tuning engine applies to a table is restated here over the
//! same bytes. Nothing in this module calls the engine's own validation: a
//! verifier that asked the engine whether the engine's table was consistent
//! would attest to agreement with itself, and the substitutions this table
//! exists to refuse are exactly the ones that leave a campaign
//! self-consistent.

use std::collections::{BTreeMap, BTreeSet};

use flight_tune::{
    ControlChannel, ControlFamily, Digest, MissionReference, PhysicalUnit, RunRecord,
    ScenarioMotion, ScopedResponseTarget, SearchStage, TARGET_AUTHORITY_OBJECTIVE,
    TargetComparison,
};

use crate::{FeedbackError, error::invalid};

/// The table schema this verifier reads.
const RESPONSE_TARGET_TABLE_SCHEMA_VERSION: u16 = 1;
/// The most rows one table may state.
const MAX_RESPONSE_TARGETS: usize = 8192;
const MAX_NAME_BYTES: usize = 128;

/// Objective-name prefixes that only a direct family may claim.
const ANGULAR_PREFIXES: [&str; 2] = ["angular.", "angular_release."];
const COLLECTIVE_PREFIX: &str = "collective.";
const RESPONSE_PREFIX: &str = "response.";

/// Recomputes every rule the scoped table has to satisfy for one stage.
///
/// # Errors
///
/// Returns [`FeedbackError`] when a row, order, scope, or coverage rule fails.
pub(super) fn verify(stage: &SearchStage) -> Result<(), FeedbackError> {
    let table = &stage.response_targets;
    if table.schema_version != RESPONSE_TARGET_TABLE_SCHEMA_VERSION {
        return Err(invalid("the response target table schema changed"));
    }
    if table.targets.is_empty() || table.targets.len() > MAX_RESPONSE_TARGETS {
        return Err(invalid("the response target table size is not valid"));
    }
    for target in &table.targets {
        verify_row(target)?;
    }
    verify_order(&table.targets)?;
    verify_scope_agreement(&table.targets)?;
    verify_coverage(stage)
}

fn verify_row(target: &ScopedResponseTarget) -> Result<(), FeedbackError> {
    verify_name(&target.mission_revision_id)?;
    verify_name(&target.objective)?;
    if target.objective == TARGET_AUTHORITY_OBJECTIVE {
        return Err(invalid("an authority band is not a response target row"));
    }
    if target.mission_content_digest.is_zero()
        || target.envelope_digest.is_zero()
        || !target.limit.is_finite()
        || target.limit < 0.0
    {
        return Err(invalid(format!(
            "the response target row for {} is not valid",
            target.objective
        )));
    }
    verify_physics(target)?;
    verify_objective_scope(target)
}

/// The motion and the unit both follow from the family and the channel.
///
/// The derivation is restated here as its own exhaustive match, so a row that
/// declares a motion or a unit its family does not produce is refused by this
/// crate's arithmetic rather than by the engine's.
fn verify_physics(target: &ScopedResponseTarget) -> Result<(), FeedbackError> {
    let motion = derive_motion(target.control_family, target.control_channel);
    if target.motion != motion {
        return Err(invalid(format!(
            "the response target row for {} states a substituted motion",
            target.mission_revision_id
        )));
    }
    if target.physical_target.unit != required_unit(target.control_family, target.control_channel) {
        return Err(invalid(format!(
            "the response target row for {} states a substituted unit",
            target.mission_revision_id
        )));
    }
    let value = target.physical_target.value;
    if !value.is_finite() || value == 0.0 {
        return Err(invalid("a physical target is finite and never zero"));
    }
    verify_authority_band(target, value)
}

fn verify_authority_band(target: &ScopedResponseTarget, value: f64) -> Result<(), FeedbackError> {
    let Some(band) = target.authority_band else {
        return Ok(());
    };
    if target.control_family != ControlFamily::OperatorVelocity
        || !band.minimum.is_finite()
        || !band.maximum.is_finite()
        || band.minimum <= 0.0
        || band.minimum >= band.maximum
        || band.maximum > value.abs()
    {
        return Err(invalid(format!(
            "the authority band of {} is not valid",
            target.mission_revision_id
        )));
    }
    Ok(())
}

fn verify_objective_scope(target: &ScopedResponseTarget) -> Result<(), FeedbackError> {
    if admissible(&target.objective, target.control_family, target.motion) {
        return Ok(());
    }
    Err(invalid(format!(
        "{} does not belong to this physical scope",
        target.objective
    )))
}

/// Whether one objective belongs to one physical scope.
///
/// A scenario measures the objectives its own family and motion produce, so a
/// matrix that mixes families states different objectives for different
/// scenarios. The predicate is restated here rather than called, because the
/// bijection it feeds is what a verifier is for.
fn admissible(objective: &str, family: ControlFamily, motion: ScenarioMotion) -> bool {
    let direct = matches!(family, ControlFamily::DirectAttitudeThrust);
    let angular = matches!(
        motion,
        ScenarioMotion::Roll | ScenarioMotion::Pitch | ScenarioMotion::Yaw
    );
    if ANGULAR_PREFIXES
        .iter()
        .any(|prefix| objective.starts_with(prefix))
    {
        return direct && angular;
    }
    if objective.starts_with(COLLECTIVE_PREFIX) {
        return direct && motion == ScenarioMotion::Collective;
    }
    if objective.starts_with(RESPONSE_PREFIX) {
        return matches!(family, ControlFamily::OperatorVelocity);
    }
    true
}

fn verify_order(targets: &[ScopedResponseTarget]) -> Result<(), FeedbackError> {
    for pair in targets.windows(2) {
        let left = (&pair[0].mission_revision_id, &pair[0].objective);
        let right = (&pair[1].mission_revision_id, &pair[1].objective);
        if left >= right {
            return Err(invalid("response target rows are repeated or out of order"));
        }
    }
    Ok(())
}

fn verify_scope_agreement(targets: &[ScopedResponseTarget]) -> Result<(), FeedbackError> {
    for pair in targets.windows(2) {
        if pair[0].mission_revision_id == pair[1].mission_revision_id
            && !same_scope(&pair[0], &pair[1])
        {
            return Err(invalid(format!(
                "two response target rows for {} state different scopes",
                pair[0].mission_revision_id
            )));
        }
    }
    Ok(())
}

fn same_scope(left: &ScopedResponseTarget, right: &ScopedResponseTarget) -> bool {
    left.mission_content_digest == right.mission_content_digest
        && left.control_family == right.control_family
        && left.control_channel == right.control_channel
        && left.motion == right.motion
        && left.physical_target == right.physical_target
        && left.envelope_digest == right.envelope_digest
        && left.authority_band == right.authority_band
}

/// The table covers exactly the decisions the stage takes: no missing row and
/// no extra row.
fn verify_coverage(stage: &SearchStage) -> Result<(), FeedbackError> {
    verify_every_scenario_is_scoped(stage)?;
    let expected = expected_rows(stage);
    let stated = stage
        .response_targets
        .targets
        .iter()
        .map(|target| {
            (
                target.mission_revision_id.as_str(),
                target.objective.as_str(),
            )
        })
        .collect::<BTreeSet<_>>();
    if let Some((mission, objective)) = expected.difference(&stated).next() {
        return Err(invalid(format!(
            "the response target table states no {objective} limit for {mission}"
        )));
    }
    if let Some((mission, objective)) = stated.difference(&expected).next() {
        return Err(invalid(format!(
            "the response target table states an unread {objective} limit for {mission}"
        )));
    }
    verify_scenario_identities(stage)
}

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
            let Some(scope) = scope_of(stage, &scenario.revision_id) else {
                continue;
            };
            for objective in objectives
                .iter()
                .filter(|name| admissible(name, scope.control_family, scope.motion))
            {
                rows.insert((scenario.revision_id.as_str(), objective.as_str()));
            }
        }
    }
    rows
}

/// The scope one scenario states, if it states one.
fn scope_of<'a>(
    stage: &'a SearchStage,
    mission_revision_id: &str,
) -> Option<&'a ScopedResponseTarget> {
    stage
        .response_targets
        .targets
        .iter()
        .find(|target| target.mission_revision_id == mission_revision_id)
}

/// Every hidden scenario states at least one scoped limit.
fn verify_every_scenario_is_scoped(stage: &SearchStage) -> Result<(), FeedbackError> {
    for scenario in stage
        .promotion_scenarios
        .iter()
        .chain(&stage.final_qualification_scenarios)
    {
        if scope_of(stage, &scenario.revision_id).is_none() {
            return Err(invalid(format!(
                "the response target table states no limit for {}",
                scenario.revision_id
            )));
        }
    }
    Ok(())
}

fn verify_scenario_identities(stage: &SearchStage) -> Result<(), FeedbackError> {
    let scheduled = stage
        .promotion_scenarios
        .iter()
        .chain(&stage.final_qualification_scenarios)
        .map(|scenario: &MissionReference| (scenario.revision_id.as_str(), scenario.content_digest))
        .collect::<BTreeMap<&str, Digest>>();
    for target in &stage.response_targets.targets {
        let expected = scheduled
            .get(target.mission_revision_id.as_str())
            .copied()
            .ok_or_else(|| {
                invalid(format!(
                    "the response target table names {}, which no decision reads",
                    target.mission_revision_id
                ))
            })?;
        if target.mission_content_digest != expected {
            return Err(invalid(format!(
                "the response target row for {} names other scenario content",
                target.mission_revision_id
            )));
        }
    }
    Ok(())
}

/// The exact scoped row one decision reads.
///
/// # Errors
///
/// Returns [`FeedbackError`] when the table states no row for that pair.
pub(super) fn row<'a>(
    stage: &'a SearchStage,
    mission_revision_id: &str,
    objective: &str,
) -> Result<&'a ScopedResponseTarget, FeedbackError> {
    stage
        .response_targets
        .targets
        .iter()
        .find(|target| {
            target.mission_revision_id == mission_revision_id && target.objective == objective
        })
        .ok_or_else(|| {
            invalid(format!(
                "the response target table states no {objective} limit for {mission_revision_id}"
            ))
        })
}

/// Reports whether one measured value meets one scoped row.
#[must_use]
pub(super) fn holds(target: &ScopedResponseTarget, value: f64) -> bool {
    match target.comparison {
        TargetComparison::AtMost => value <= target.limit,
        TargetComparison::AtLeast => value >= target.limit,
    }
}

/// Reports whether one run kept the authority its scenario states.
#[must_use]
pub(super) fn authority_holds(stage: &SearchStage, run: &RunRecord) -> bool {
    let Some(band) = band_for(stage, &run.mission_revision_id) else {
        return true;
    };
    run.objectives
        .get(TARGET_AUTHORITY_OBJECTIVE)
        .copied()
        .is_some_and(|resolved| {
            resolved.is_finite() && (band.minimum..=band.maximum).contains(&resolved.abs())
        })
}

/// The authority band one scenario keeps, if it keeps one.
#[must_use]
pub(super) fn band_for(
    stage: &SearchStage,
    mission_revision_id: &str,
) -> Option<flight_tune::TargetAuthorityBand> {
    stage
        .response_targets
        .targets
        .iter()
        .find(|target| target.mission_revision_id == mission_revision_id)
        .and_then(|target| target.authority_band)
}

/// The exact objective names every run of one scenario has to state.
#[must_use]
pub(super) fn expected_objective_names(
    stage: &SearchStage,
    mission_revision_id: &str,
    declared: &BTreeSet<String>,
) -> BTreeSet<String> {
    let Some(scope) = scope_of(stage, mission_revision_id) else {
        return declared.clone();
    };
    let mut names = declared
        .iter()
        .filter(|name| admissible(name, scope.control_family, scope.motion))
        .cloned()
        .collect::<BTreeSet<String>>();
    if scope.authority_band.is_some() {
        names.insert(TARGET_AUTHORITY_OBJECTIVE.to_owned());
    }
    names
}

/// The one motion that a family and channel produce.
const fn derive_motion(family: ControlFamily, channel: ControlChannel) -> ScenarioMotion {
    match (family, channel) {
        (
            ControlFamily::OperatorVelocity,
            ControlChannel::Roll | ControlChannel::Pitch | ControlChannel::Vertical,
        ) => ScenarioMotion::Linear,
        (ControlFamily::OperatorVelocity, ControlChannel::Yaw) => ScenarioMotion::Yaw,
        (ControlFamily::DirectAttitudeThrust, ControlChannel::Roll) => ScenarioMotion::Roll,
        (ControlFamily::DirectAttitudeThrust, ControlChannel::Pitch) => ScenarioMotion::Pitch,
        (ControlFamily::DirectAttitudeThrust, ControlChannel::Yaw) => ScenarioMotion::Yaw,
        (ControlFamily::DirectAttitudeThrust, ControlChannel::Vertical) => {
            ScenarioMotion::Collective
        }
    }
}

/// The one unit that a family and channel measure in.
const fn required_unit(family: ControlFamily, channel: ControlChannel) -> PhysicalUnit {
    match (family, channel) {
        (
            ControlFamily::OperatorVelocity,
            ControlChannel::Roll | ControlChannel::Pitch | ControlChannel::Vertical,
        ) => PhysicalUnit::MetersPerSecond,
        (ControlFamily::OperatorVelocity, ControlChannel::Yaw) => PhysicalUnit::RadiansPerSecond,
        (
            ControlFamily::DirectAttitudeThrust,
            ControlChannel::Roll | ControlChannel::Pitch | ControlChannel::Yaw,
        ) => PhysicalUnit::Radians,
        (ControlFamily::DirectAttitudeThrust, ControlChannel::Vertical) => {
            PhysicalUnit::NormalizedCollectiveForce
        }
    }
}

fn verify_name(value: &str) -> Result<(), FeedbackError> {
    if value.trim().is_empty()
        || value.len() > MAX_NAME_BYTES
        || value.chars().any(char::is_whitespace)
    {
        return Err(invalid("a response target name is not valid"));
    }
    Ok(())
}
