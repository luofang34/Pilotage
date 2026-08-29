use std::collections::{BTreeMap, BTreeSet};

use pilotage_trial::Digest;
use serde::{Deserialize, Serialize};
use sha2::{Digest as ShaDigest, Sha256};

use crate::{Candidate, MissionReference, TuneError};

mod plan;
mod validation;

#[cfg(test)]
#[path = "training_suite/tests.rs"]
pub(crate) mod tests;

pub(crate) use plan::AttemptRunPlan;
pub(crate) use validation::validate_search_space;

/// The supported training suite schema.
pub const TRAINING_SUITE_SCHEMA_VERSION: u16 = 1;

const TRAINING_SUITE_DOMAIN: &[u8] = b"pilotage.flight-tune.training-suite.v1\0";

pub(super) const MAX_TRAINING_SUITES: usize = 16;
pub(super) const MAX_SEARCH_GROUPS: usize = 16;
pub(super) const MAX_GUARD_LIMITS: usize = 32;

/// The response family that one search parameter group changes.
///
/// The family states which evidence answers a change. A controller group
/// changes the closed-loop response, so direct stimulus data answers it. An
/// operator-feel group changes the command shape between the operator and the
/// controller, so operator-velocity data answers it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchGroupKind {
    /// The group changes the direct controller response.
    Controller,
    /// The group changes the operator command shape.
    OperatorFeel,
}

/// One frozen training suite.
///
/// The suite states the complete training evidence for one search parameter
/// group. The primary missions carry the loss that decides a challenger. The
/// guard missions carry the responses that a change must not degrade, so a
/// primary improvement cannot hide a guard regression.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrainingSuite {
    /// The suite schema.
    pub schema_version: u16,
    /// The stable suite identity.
    pub id: String,
    /// The ordered missions that carry the primary loss.
    pub primary_scenarios: Vec<MissionReference>,
    /// The ordered missions that only guard against a regression.
    pub guard_scenarios: Vec<MissionReference>,
    /// The largest permitted guard increase for each named objective.
    pub guard_regression_limits: BTreeMap<String, f64>,
    /// The run count for each mission in this suite.
    pub repetitions: u32,
}

impl TrainingSuite {
    /// Returns the complete ordered mission list for one suite run plan.
    ///
    /// The primary missions run first, then the guard missions. Both sides of
    /// a comparison use this exact order.
    #[must_use]
    pub fn ordered_scenarios(&self) -> Vec<MissionReference> {
        let mut scenarios = self.primary_scenarios.clone();
        scenarios.extend(self.guard_scenarios.iter().cloned());
        scenarios
    }

    /// Returns how many runs of one attempt carry the primary loss.
    #[must_use]
    pub fn primary_run_count(&self) -> usize {
        self.primary_scenarios
            .len()
            .saturating_mul(self.repetitions as usize)
    }

    /// Returns how many runs one attempt on this suite executes.
    #[must_use]
    pub fn run_count(&self) -> usize {
        self.primary_scenarios
            .len()
            .saturating_add(self.guard_scenarios.len())
            .saturating_mul(self.repetitions as usize)
    }

    /// Returns the wall-clock budget for one complete attempt on this suite.
    #[must_use]
    pub fn run_duration_ns(&self) -> u64 {
        let longest = self
            .primary_scenarios
            .iter()
            .chain(&self.guard_scenarios)
            .map(MissionReference::run_duration_ns)
            .max()
            .unwrap_or(1);
        longest.saturating_mul(self.run_count() as u64)
    }

    /// Returns the content identity of this frozen suite.
    ///
    /// # Errors
    ///
    /// Returns [`TuneError`] when the declaration cannot encode.
    pub fn digest(&self) -> Result<Digest, TuneError> {
        let bytes = serde_json::to_vec(self).map_err(|source| TuneError::Encode {
            document: "training suite",
            source,
        })?;
        let mut hasher = Sha256::new();
        hasher.update(TRAINING_SUITE_DOMAIN);
        hasher.update(bytes);
        Ok(Digest::from_bytes(hasher.finalize().into()))
    }

    pub(super) fn anchor(&self, index: u16) -> Result<TrainingSuiteAnchor, TuneError> {
        Ok(TrainingSuiteAnchor {
            index,
            id: self.id.clone(),
            digest: self.digest()?,
        })
    }
}

/// One search parameter group and the suite that answers a change to it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SearchGroup {
    /// The stable group identity.
    pub id: String,
    /// The response family this group changes.
    pub kind: SearchGroupKind,
    /// The allowlisted parameters that belong only to this group.
    pub parameters: BTreeSet<String>,
    /// The identity of the one suite that answers a change to this group.
    pub suite_id: String,
}

/// The derived link from one candidate difference to its frozen suite.
///
/// The engine derives this binding from the parameters that differ between the
/// incumbent and the challenger. A proposal cannot state it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SearchGroupBinding {
    /// The derived search group identity.
    pub group_id: String,
    /// The derived training suite identity.
    pub suite_id: String,
    /// The position of the derived suite in the frozen suite order.
    pub suite_index: u16,
    /// The content identity of the derived suite.
    pub suite_digest: Digest,
}

/// The identity of the suite that one training run plan uses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct TrainingSuiteAnchor {
    pub(crate) index: u16,
    pub(crate) id: String,
    pub(crate) digest: Digest,
}

/// Returns the allowlisted parameter names that differ between two candidates.
///
/// Both candidates carry the same parameter names, so a difference is a
/// changed value and never a changed key set.
pub(super) fn changed_parameters(incumbent: &Candidate, challenger: &Candidate) -> Vec<String> {
    challenger
        .parameters()
        .iter()
        .filter(|(name, value)| incumbent.parameters().get(*name) != Some(*value))
        .map(|(name, _)| name.clone())
        .collect()
}

pub(super) fn invalid_stage(detail: impl Into<String>) -> TuneError {
    TuneError::InvalidStage {
        detail: detail.into(),
    }
}
