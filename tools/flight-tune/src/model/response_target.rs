use std::collections::{BTreeMap, BTreeSet};

use pilotage_trial::Digest;
use serde::{Deserialize, Serialize};
use sha2::{Digest as ShaDigest, Sha256};

use crate::{ControlChannel, ControlFamily, TuneError};

mod coverage;
mod document;
mod motion;
mod scope;

#[cfg(test)]
#[path = "response_target/tests.rs"]
mod tests;

/// Builders that give a test stage a complete scoped table.
///
/// A stage is only valid with one, so every fixture in the crate needs the
/// same construction. Sharing it keeps a fixture from stating a table that
/// agrees with itself but not with the stage it belongs to.
#[cfg(test)]
pub(crate) mod fixture {
    use std::collections::BTreeMap;

    use pilotage_trial::Digest;

    use crate::{ControlChannel, ControlFamily, MissionReference, PhysicalUnit};

    use super::{PhysicalTarget, ResponseTargetScope, ResponseTargetTable, TargetComparison};

    /// One operator-velocity table over each partition and its limits.
    #[allow(clippy::expect_used)]
    pub(crate) fn covering(
        partitions: &[(&[MissionReference], &BTreeMap<String, f64>)],
    ) -> ResponseTargetTable {
        let mut rows = Vec::new();
        for (scenarios, limits) in partitions {
            for scenario in *scenarios {
                let scope = ResponseTargetScope {
                    mission_revision_id: scenario.revision_id.clone(),
                    mission_content_digest: scenario.content_digest,
                    control_family: ControlFamily::OperatorVelocity,
                    control_channel: ControlChannel::Roll,
                    physical_target: PhysicalTarget {
                        unit: PhysicalUnit::MetersPerSecond,
                        value: 3.0,
                    },
                    envelope_digest: envelope_digest(scenario),
                    authority_band: None,
                };
                rows.extend(
                    scope.rows(
                        limits
                            .iter()
                            .map(|(name, limit)| (name.as_str(), TargetComparison::AtMost, *limit)),
                    ),
                );
            }
        }
        ResponseTargetTable::new(rows).expect("a fixture table is valid")
    }

    fn envelope_digest(scenario: &MissionReference) -> Digest {
        let mut bytes = *scenario.content_digest.as_bytes();
        bytes[0] ^= 0xa5;
        Digest::from_bytes(bytes)
    }
}

pub(super) use coverage::validate_for_stage;
pub(crate) use document::verify_document;
pub use motion::{ScenarioMotion, is_admissible};
pub use scope::{PhysicalTarget, ResponseTargetScope, TargetAuthorityBand};

/// The supported scoped response target table schema.
pub const RESPONSE_TARGET_TABLE_SCHEMA_VERSION: u16 = 1;

const RESPONSE_TARGET_TABLE_DOMAIN: &[u8] = b"pilotage.flight-tune.response-target-table.v1\0";

/// The most rows one table may state.
///
/// The bound is the scenario ceiling times the objective ceiling, so a stage
/// that fills both partitions can still state a complete table.
const MAX_RESPONSE_TARGETS: usize = 8192;
const MAX_NAME_BYTES: usize = 128;

/// The name a decision reports when a run gave up operator authority.
///
/// The value it names is not a policy objective and never has a table row: an
/// authority band is two-sided and absolute, while every table row is a
/// one-sided limit that promotion reads as a paired difference. The name
/// exists so a refusal states which measurement refused it.
pub const TARGET_AUTHORITY_OBJECTIVE: &str = "authority.resolved_target";

/// How a measured objective value is compared with its scoped limit.
///
/// A limit without a direction is not a bar. Most response measurements are
/// worse when larger, but a measurement of authority is worse when smaller,
/// and both belong in one table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetComparison {
    /// The measured value must be at or under the limit.
    AtMost,
    /// The measured value must be at or over the limit.
    AtLeast,
}

impl TargetComparison {
    /// Reports whether one measured value meets one limit.
    #[must_use]
    pub fn holds(self, value: f64, limit: f64) -> bool {
        match self {
            Self::AtMost => value <= limit,
            Self::AtLeast => value >= limit,
        }
    }

    /// Gets the stable comparison name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AtMost => "at_most",
            Self::AtLeast => "at_least",
        }
    }
}

/// One objective limit and the exact physical scope it applies to.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScopedResponseTarget {
    /// The scenario this limit applies to.
    pub mission_revision_id: String,
    /// The exact content identity of that scenario.
    pub mission_content_digest: Digest,
    /// The physical control family the scenario commands.
    pub control_family: ControlFamily,
    /// The control channel the scenario commands.
    pub control_channel: ControlChannel,
    /// The motion that family and channel produce.
    pub motion: ScenarioMotion,
    /// The physical quantity the scenario asks for.
    pub physical_target: PhysicalTarget,
    /// The identity of the stimulus envelope the scenario carries.
    pub envelope_digest: Digest,
    /// The authority the operator keeps over the resolved physical target.
    pub authority_band: Option<TargetAuthorityBand>,
    /// The measured objective this row limits.
    pub objective: String,
    /// The comparison operation.
    pub comparison: TargetComparison,
    /// The limit in the objective's own units.
    pub limit: f64,
}

impl ScopedResponseTarget {
    /// Reports whether one measured objective value meets this row.
    #[must_use]
    pub fn holds(&self, value: f64) -> bool {
        self.comparison.holds(value, self.limit)
    }

    /// The fields that every row for one scenario has to agree on.
    fn scope(&self) -> ScenarioScope<'_> {
        ScenarioScope {
            mission_content_digest: self.mission_content_digest,
            control_family: self.control_family,
            control_channel: self.control_channel,
            motion: self.motion,
            physical_target: self.physical_target,
            envelope_digest: self.envelope_digest,
            authority_band: self.authority_band,
            mission_revision_id: &self.mission_revision_id,
        }
    }

    fn validate(&self) -> Result<(), TuneError> {
        validate_name(&self.mission_revision_id, "mission revision")?;
        validate_name(&self.objective, "objective")?;
        if self.objective == TARGET_AUTHORITY_OBJECTIVE {
            return Err(invalid_table(format!(
                "{TARGET_AUTHORITY_OBJECTIVE} is an authority band, not a table row"
            )));
        }
        if self.mission_content_digest.is_zero() || self.envelope_digest.is_zero() {
            return Err(invalid_table(format!(
                "the scope of {} states a zero identity",
                self.objective
            )));
        }
        if !self.limit.is_finite() || self.limit < 0.0 {
            return Err(invalid_table(format!(
                "the limit for {} is not finite and nonnegative",
                self.objective
            )));
        }
        self.validate_physics()?;
        motion::validate_objective_scope(&self.objective, self.control_family, self.motion)
    }

    fn validate_physics(&self) -> Result<(), TuneError> {
        if self.motion != ScenarioMotion::derive(self.control_family, self.control_channel) {
            return Err(invalid_table(format!(
                "the stated motion {} is not the one {} produces",
                self.motion.as_str(),
                self.control_family.as_str()
            )));
        }
        motion::validate_unit(
            self.control_family,
            self.control_channel,
            self.physical_target.unit,
        )?;
        let target = self.physical_target.value;
        if !target.is_finite() || target == 0.0 {
            return Err(invalid_table(
                "a physical target is finite and never exactly zero",
            ));
        }
        self.validate_authority_band(target)
    }

    fn validate_authority_band(&self, target: f64) -> Result<(), TuneError> {
        let Some(band) = self.authority_band else {
            return Ok(());
        };
        // A direct stimulus resolves its physical command from the envelope
        // alone, so no candidate can move the target and there is no authority
        // to keep. Admitting a band there would state a check that can never
        // fire.
        if self.control_family != ControlFamily::OperatorVelocity {
            return Err(invalid_table(
                "only an operator scope keeps an authority band",
            ));
        }
        // The envelope BOUNDS an operator output rather than fixing it: the
        // candidate curve shapes the normalized input to at most the
        // requested target. A band above that target could never be met, and
        // one that reached zero would be no band at all.
        if !band.minimum.is_finite()
            || !band.maximum.is_finite()
            || band.minimum <= 0.0
            || band.minimum >= band.maximum
            || band.maximum > target.abs()
        {
            return Err(invalid_table(format!(
                "the authority band of {} is not inside its own physical target",
                self.mission_revision_id
            )));
        }
        Ok(())
    }
}

#[derive(PartialEq)]
struct ScenarioScope<'a> {
    mission_content_digest: Digest,
    control_family: ControlFamily,
    control_channel: ControlChannel,
    motion: ScenarioMotion,
    physical_target: PhysicalTarget,
    envelope_digest: Digest,
    authority_band: Option<TargetAuthorityBand>,
    mission_revision_id: &'a str,
}

/// The versioned table of exact scoped response limits.
///
/// One row states one objective limit for one scenario. There is no global
/// maximum behind it: a decision that finds no row refuses rather than falling
/// back, because a fallback would apply an operator velocity limit to a direct
/// attitude result whenever a row went missing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResponseTargetTable {
    /// The table schema.
    pub schema_version: u16,
    /// The rows in ascending scenario and objective order.
    pub targets: Vec<ScopedResponseTarget>,
}

impl ResponseTargetTable {
    /// Returns the row for one scenario and objective.
    ///
    /// # Errors
    ///
    /// Returns [`TuneError`] when the table states no row for that pair.
    pub fn target(
        &self,
        mission_revision_id: &str,
        objective: &str,
    ) -> Result<&ScopedResponseTarget, TuneError> {
        self.targets
            .iter()
            .find(|target| {
                target.mission_revision_id == mission_revision_id && target.objective == objective
            })
            .ok_or_else(|| {
                invalid_table(format!(
                    "the response target table states no {objective} limit for {mission_revision_id}"
                ))
            })
    }

    /// Returns every row for one scenario, by objective name.
    #[must_use]
    pub fn scenario_targets(
        &self,
        mission_revision_id: &str,
    ) -> BTreeMap<&str, &ScopedResponseTarget> {
        self.targets
            .iter()
            .filter(|target| target.mission_revision_id == mission_revision_id)
            .map(|target| (target.objective.as_str(), target))
            .collect()
    }

    /// Returns the authority band one scenario keeps, if it keeps one.
    #[must_use]
    pub fn authority_band(&self, mission_revision_id: &str) -> Option<TargetAuthorityBand> {
        self.targets
            .iter()
            .find(|target| target.mission_revision_id == mission_revision_id)
            .and_then(|target| target.authority_band)
    }

    /// Reports whether one run kept the authority its scenario states.
    ///
    /// A scenario with no band keeps whatever authority its envelope resolves
    /// to, so the answer is yes. A scenario with a band needs the resolved
    /// target the run measured: a run that states none has not shown that it
    /// kept anything, so the answer is no.
    #[must_use]
    pub fn authority_holds(&self, mission_revision_id: &str, resolved_target: Option<f64>) -> bool {
        let Some(band) = self.authority_band(mission_revision_id) else {
            return true;
        };
        resolved_target.is_some_and(|resolved| band.contains(resolved.abs()))
    }

    /// Returns the objective names every run of one scenario has to state.
    ///
    /// A scenario measures the objectives its own family and motion produce,
    /// so a declared name a scope cannot answer is not one its runs owe. A
    /// banded scenario reports one further value the policy does not declare
    /// at all, so the exact key set a run must carry is derived here rather
    /// than read from the policy alone.
    #[must_use]
    pub fn expected_objective_names<'a>(
        &self,
        mission_revision_id: &str,
        declared: impl IntoIterator<Item = &'a String>,
    ) -> BTreeSet<String> {
        let Some(scope) = self
            .targets
            .iter()
            .find(|target| target.mission_revision_id == mission_revision_id)
        else {
            return declared.into_iter().cloned().collect();
        };
        let mut names = declared
            .into_iter()
            .filter(|name| is_admissible(name, scope.control_family, scope.motion))
            .cloned()
            .collect::<BTreeSet<String>>();
        if scope.authority_band.is_some() {
            names.insert(TARGET_AUTHORITY_OBJECTIVE.to_owned());
        }
        names
    }

    /// Returns the content identity of the complete table.
    ///
    /// # Errors
    ///
    /// Returns [`TuneError`] when the table cannot encode.
    pub fn digest(&self) -> Result<Digest, TuneError> {
        let bytes = serde_json::to_vec(self).map_err(|source| TuneError::Encode {
            document: "response target table",
            source,
        })?;
        let mut hasher = Sha256::new();
        hasher.update(RESPONSE_TARGET_TABLE_DOMAIN);
        hasher.update(bytes);
        Ok(Digest::from_bytes(hasher.finalize().into()))
    }

    /// Builds one canonically ordered table from its rows.
    ///
    /// The constructor sorts, so a producer cannot state an out-of-order
    /// table by accident. A decoded table is checked instead of sorted: a
    /// reordered document is a changed document, and accepting it silently
    /// would give one table two identities.
    ///
    /// # Errors
    ///
    /// Returns [`TuneError`] when the rows are not a valid table.
    pub fn new(mut targets: Vec<ScopedResponseTarget>) -> Result<Self, TuneError> {
        targets.sort_by(|left, right| {
            (&left.mission_revision_id, &left.objective)
                .cmp(&(&right.mission_revision_id, &right.objective))
        });
        let table = Self {
            schema_version: RESPONSE_TARGET_TABLE_SCHEMA_VERSION,
            targets,
        };
        table.validate()?;
        Ok(table)
    }

    /// Validates every row and the agreement between rows of one scenario.
    ///
    /// # Errors
    ///
    /// Returns [`TuneError`] when a version, name, identity, unit, limit, or
    /// row order is not valid, or when two rows of one scenario disagree.
    pub fn validate(&self) -> Result<(), TuneError> {
        if self.schema_version != RESPONSE_TARGET_TABLE_SCHEMA_VERSION {
            return Err(invalid_table("the response target table schema changed"));
        }
        if self.targets.is_empty() || self.targets.len() > MAX_RESPONSE_TARGETS {
            return Err(invalid_table("the response target table size is not valid"));
        }
        for target in &self.targets {
            target.validate()?;
        }
        self.validate_order()?;
        self.validate_scope_agreement()
    }

    /// The rows rise strictly by scenario and then by objective.
    ///
    /// A canonical order makes a reordered table invalid as well as differently
    /// identified, so a reorder cannot be presented as an equivalent table that
    /// happens to have another digest.
    fn validate_order(&self) -> Result<(), TuneError> {
        for pair in self.targets.windows(2) {
            let left = (&pair[0].mission_revision_id, &pair[0].objective);
            let right = (&pair[1].mission_revision_id, &pair[1].objective);
            if left >= right {
                return Err(invalid_table(
                    "response target rows are repeated or out of order",
                ));
            }
        }
        Ok(())
    }

    fn validate_scope_agreement(&self) -> Result<(), TuneError> {
        for pair in self.targets.windows(2) {
            if pair[0].mission_revision_id == pair[1].mission_revision_id
                && pair[0].scope() != pair[1].scope()
            {
                return Err(invalid_table(format!(
                    "two rows for {} state different physical scopes",
                    pair[0].mission_revision_id
                )));
            }
        }
        Ok(())
    }
}

fn validate_name(value: &str, kind: &str) -> Result<(), TuneError> {
    if value.trim().is_empty()
        || value.len() > MAX_NAME_BYTES
        || value.chars().any(char::is_whitespace)
    {
        return Err(invalid_table(format!(
            "{kind} names need 1 to {MAX_NAME_BYTES} bytes and no whitespace"
        )));
    }
    Ok(())
}

fn invalid_table(detail: impl Into<String>) -> TuneError {
    TuneError::InvalidStage {
        detail: detail.into(),
    }
}
