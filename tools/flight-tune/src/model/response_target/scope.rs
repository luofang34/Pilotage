use pilotage_trial::Digest;
use serde::{Deserialize, Serialize};

use crate::{ControlChannel, ControlFamily, PhysicalUnit};

use super::{ScenarioMotion, ScopedResponseTarget, TargetComparison};

/// The physical quantity that one scenario asks the vehicle to produce.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PhysicalTarget {
    /// The unit that the family and channel measure in.
    pub unit: PhysicalUnit,
    /// The requested physical value in that unit.
    pub value: f64,
}

/// The band that a candidate-resolved physical target must stay inside.
///
/// An operator stimulus states a normalized input, and the candidate feel
/// profile decides what physical target that input asks for. A candidate can
/// therefore improve every normalized response metric by asking for less: a
/// larger expo lowers the target, so the vehicle reaches it sooner and
/// overshoots it less, and nothing in a normalized measurement can tell that
/// apart from a better command law. The band is the authority the operator
/// keeps, stated in physical units, and it is checked against the target the
/// candidate actually resolved.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TargetAuthorityBand {
    /// The smallest physical target the operator input may resolve to.
    pub minimum: f64,
    /// The largest physical target the operator input may resolve to.
    pub maximum: f64,
}

impl TargetAuthorityBand {
    /// Reports whether one resolved physical target is inside the band.
    #[must_use]
    pub fn contains(self, resolved: f64) -> bool {
        resolved.is_finite() && (self.minimum..=self.maximum).contains(&resolved)
    }
}

/// The physical scope every limit for one scenario shares.
///
/// A producer states the scope once and its objective limits beside it, so two
/// rows for one scenario cannot disagree about what the scenario measures by
/// construction rather than only by validation.
#[derive(Debug, Clone, PartialEq)]
pub struct ResponseTargetScope {
    /// The scenario these limits apply to.
    pub mission_revision_id: String,
    /// The exact content identity of that scenario.
    pub mission_content_digest: Digest,
    /// The physical control family the scenario commands.
    pub control_family: ControlFamily,
    /// The control channel the scenario commands.
    pub control_channel: ControlChannel,
    /// The physical quantity the scenario asks for.
    pub physical_target: PhysicalTarget,
    /// The identity of the stimulus envelope the scenario carries.
    pub envelope_digest: Digest,
    /// The authority the operator keeps over the resolved physical target.
    pub authority_band: Option<TargetAuthorityBand>,
}

impl ResponseTargetScope {
    /// Builds one row for each objective limit this scope states.
    #[must_use]
    pub fn rows<'a, I>(&self, limits: I) -> Vec<ScopedResponseTarget>
    where
        I: IntoIterator<Item = (&'a str, TargetComparison, f64)>,
    {
        limits
            .into_iter()
            .map(|(objective, comparison, limit)| ScopedResponseTarget {
                mission_revision_id: self.mission_revision_id.clone(),
                mission_content_digest: self.mission_content_digest,
                control_family: self.control_family,
                control_channel: self.control_channel,
                motion: ScenarioMotion::derive(self.control_family, self.control_channel),
                physical_target: self.physical_target,
                envelope_digest: self.envelope_digest,
                authority_band: self.authority_band,
                objective: objective.to_owned(),
                comparison,
                limit,
            })
            .collect()
    }
}
