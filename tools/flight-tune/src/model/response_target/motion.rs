use serde::{Deserialize, Serialize};

use crate::{ControlChannel, ControlFamily, PhysicalUnit, TuneError};

use super::invalid_table;

/// The physical motion that one scenario measures.
///
/// A control family and a control channel together fix the motion, so the
/// motion is derived and never declared on its own. A table states it anyway
/// because a reader of one row should not have to run the derivation to know
/// what the row measures, and validation then refuses a row whose stated
/// motion is not the one its family and channel produce.
///
/// The motion does not fix the measured signal by itself. An operator yaw
/// stimulus and a direct yaw stimulus are both [`ScenarioMotion::Yaw`], and
/// the family decides that the first is measured from yaw rate and the second
/// from attitude. The unit the row carries is what separates them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScenarioMotion {
    /// A roll attitude response.
    Roll,
    /// A pitch attitude response.
    Pitch,
    /// A yaw response, as an attitude or as a rate.
    Yaw,
    /// A linear translation response.
    Linear,
    /// A collective force response.
    Collective,
}

impl ScenarioMotion {
    /// Derives the one motion that a family and channel produce.
    ///
    /// The match is exhaustive over both enums, so a combination this table
    /// does not name cannot exist.
    #[must_use]
    pub const fn derive(family: ControlFamily, channel: ControlChannel) -> Self {
        match (family, channel) {
            (
                ControlFamily::OperatorVelocity,
                ControlChannel::Roll | ControlChannel::Pitch | ControlChannel::Vertical,
            ) => Self::Linear,
            (ControlFamily::OperatorVelocity, ControlChannel::Yaw) => Self::Yaw,
            (ControlFamily::DirectAttitudeThrust, ControlChannel::Roll) => Self::Roll,
            (ControlFamily::DirectAttitudeThrust, ControlChannel::Pitch) => Self::Pitch,
            (ControlFamily::DirectAttitudeThrust, ControlChannel::Yaw) => Self::Yaw,
            (ControlFamily::DirectAttitudeThrust, ControlChannel::Vertical) => Self::Collective,
        }
    }

    /// Gets the stable motion name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Roll => "roll",
            Self::Pitch => "pitch",
            Self::Yaw => "yaw",
            Self::Linear => "linear",
            Self::Collective => "collective",
        }
    }
}

/// The objective-name prefix reserved for direct angular step measurements.
const ANGULAR_PREFIX: &str = "angular.";
/// The objective-name prefix reserved for direct angular release measurements.
const ANGULAR_RELEASE_PREFIX: &str = "angular_release.";
/// The objective-name prefix reserved for collective force measurements.
const COLLECTIVE_PREFIX: &str = "collective.";
/// The objective-name prefix reserved for operator step measurements.
const RESPONSE_PREFIX: &str = "response.";

/// Requires that one objective name belongs to the scope that states it.
///
/// The rule is a prefix rule and not a list of names, so a vehicle that
/// measures something this repository does not know about still passes. What
/// it cannot do is answer an attitude limit with a velocity run: the direct
/// families own their prefixes, and a scope of another family that names one
/// is refused before any campaign starts.
///
/// # Errors
///
/// Returns [`TuneError`] when the objective belongs to a family or motion the
/// scope does not have.
pub(super) fn validate_objective_scope(
    objective: &str,
    family: ControlFamily,
    motion: ScenarioMotion,
) -> Result<(), TuneError> {
    if is_admissible(objective, family, motion) {
        return Ok(());
    }
    Err(invalid_table(format!(
        "objective {objective} does not belong to a {} {} scope",
        family.as_str(),
        motion.as_str()
    )))
}

/// Reports whether one objective belongs to one physical scope.
///
/// A scenario measures the objectives its own family and motion produce. A
/// campaign whose matrix mixes families therefore states different objectives
/// for different scenarios, and the coverage bijection reads this predicate
/// rather than requiring every declared name of every scenario.
#[must_use]
pub fn is_admissible(objective: &str, family: ControlFamily, motion: ScenarioMotion) -> bool {
    let direct = matches!(family, ControlFamily::DirectAttitudeThrust);
    let angular_motion = matches!(
        motion,
        ScenarioMotion::Roll | ScenarioMotion::Pitch | ScenarioMotion::Yaw
    );
    if objective.starts_with(ANGULAR_PREFIX) || objective.starts_with(ANGULAR_RELEASE_PREFIX) {
        return direct && angular_motion;
    }
    if objective.starts_with(COLLECTIVE_PREFIX) {
        return direct && motion == ScenarioMotion::Collective;
    }
    // The reserved prefixes run both ways. A velocity run cannot answer an
    // attitude limit because it produces no angular name, and an attitude run
    // cannot answer a velocity limit because the operator family owns this
    // one. Everything outside the reserved prefixes belongs to any scope, so a
    // vehicle measuring something this repository does not know about still
    // states a valid bar.
    if objective.starts_with(RESPONSE_PREFIX) {
        return matches!(family, ControlFamily::OperatorVelocity);
    }
    true
}

/// Requires that a stated physical unit is the one the family and channel use.
///
/// # Errors
///
/// Returns [`TuneError`] when the unit is not the one the combination permits.
pub(super) fn validate_unit(
    family: ControlFamily,
    channel: ControlChannel,
    unit: PhysicalUnit,
) -> Result<(), TuneError> {
    let (required, _) = family.required_physics(channel);
    if unit == required {
        return Ok(());
    }
    Err(invalid_table(format!(
        "a {} scope measures {}, not {}",
        family.as_str(),
        required.as_str(),
        unit.as_str()
    )))
}
