use std::collections::BTreeMap;

use pilotage_trial::Digest;
use serde::{Deserialize, Serialize};

use super::{invalid_policy, required_improvement, valid_nonnegative};
use crate::{MissionReference, SearchStage, TargetAuthorityBand, TuneError};

/// Paired mean and upper 95 percent confidence statistics.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PromotionPairedStatistics {
    /// The paired frozen-minus-baseline mean.
    pub mean: f64,
    /// The upper 95 percent confidence limit for the paired mean.
    pub upper_95: f64,
}

impl PromotionPairedStatistics {
    pub(crate) fn validate(self) -> Result<(), TuneError> {
        if !self.mean.is_finite() || !self.upper_95.is_finite() {
            return Err(invalid_policy("promotion paired statistics are not finite"));
        }
        Ok(())
    }
}

/// One named objective non-regression result.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PromotionObjectiveResult {
    /// The complete paired statistics.
    pub statistics: PromotionPairedStatistics,
    /// The permitted paired upper 95 percent increase.
    pub maximum_upper_95: f64,
    /// Whether the paired upper limit met the scoped limit.
    pub passed: bool,
}

/// The paired objective results for one promotion scenario.
///
/// The results are grouped by scenario because the limits are. Pooling the
/// paired deltas of two scenarios and comparing the pool against one number
/// applies whichever limit the pool was written for to both, which is the
/// substitution a scoped table exists to refuse.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PromotionScenarioResults {
    /// The exact content identity of the measured scenario.
    pub mission_content_digest: Digest,
    /// The authority the scenario keeps over its resolved physical target.
    pub authority_band: Option<TargetAuthorityBand>,
    /// Whether every paired run resolved a target inside that band.
    pub authority_passed: bool,
    /// One explicit result for each declared named objective.
    pub objectives: BTreeMap<String, PromotionObjectiveResult>,
}

/// The complete paired promotion comparison.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PromotionComparison {
    /// The arithmetic baseline mean loss.
    pub baseline_mean_loss: f64,
    /// The larger absolute or relative required loss reduction.
    pub required_loss_improvement: f64,
    /// The complete paired loss statistics.
    pub loss: PromotionPairedStatistics,
    /// Whether the loss result met both improvement limits.
    pub loss_passed: bool,
    /// The complete paired control effort statistics.
    pub control_effort: PromotionPairedStatistics,
    /// Whether mean control effort met its limit.
    pub control_effort_passed: bool,
    /// One result group for each promotion scenario, by revision identity.
    pub scenarios: BTreeMap<String, PromotionScenarioResults>,
}

impl PromotionComparison {
    /// Validates all saved statistics and results against one stage.
    ///
    /// The stage carries both halves the check needs: the scalar policy and
    /// the scoped response target table that states each objective limit.
    ///
    /// # Errors
    ///
    /// Returns [`TuneError`] when a value, key, threshold, or result differs.
    pub fn validate_for(&self, stage: &SearchStage) -> Result<(), TuneError> {
        let policy = &stage.promotion;
        policy.validate()?;
        self.loss.validate()?;
        self.control_effort.validate()?;
        if !valid_nonnegative(self.baseline_mean_loss)
            || !valid_nonnegative(self.required_loss_improvement)
            || self.required_loss_improvement
                != required_improvement(policy, self.baseline_mean_loss)?
            || self.loss_passed != (self.loss.upper_95 <= -self.required_loss_improvement)
            || self.control_effort_passed
                != (self.control_effort.mean <= policy.maximum_control_effort_increase)
        {
            return Err(invalid_policy("the saved promotion comparison changed"));
        }
        validate_scenario_results(stage, &self.scenarios)
    }

    /// Reports whether every relative promotion gate passed.
    #[must_use]
    pub fn all_passed(&self) -> bool {
        self.loss_passed
            && self.control_effort_passed
            && self.scenarios.values().all(|scenario| {
                scenario.authority_passed
                    && scenario.objectives.values().all(|result| result.passed)
            })
    }
}

/// Every promotion scenario states one result group, with no scoped limit
/// left unread and no result that no limit answers.
fn validate_scenario_results(
    stage: &SearchStage,
    results: &BTreeMap<String, PromotionScenarioResults>,
) -> Result<(), TuneError> {
    if results
        .keys()
        .ne(stage.promotion_scenarios.iter().map(|s| &s.revision_id))
    {
        return Err(invalid_policy("the saved promotion scenario set changed"));
    }
    for scenario in &stage.promotion_scenarios {
        let saved = results
            .get(&scenario.revision_id)
            .ok_or_else(|| invalid_policy("a promotion scenario has no saved result"))?;
        if saved.mission_content_digest != scenario.content_digest
            || saved.authority_band != stage.response_targets.authority_band(&scenario.revision_id)
        {
            return Err(invalid_policy(format!(
                "the saved promotion result for {} names another scenario scope",
                scenario.revision_id
            )));
        }
        validate_objective_results(stage, scenario, &saved.objectives)?;
    }
    Ok(())
}

fn validate_objective_results(
    stage: &SearchStage,
    scenario: &MissionReference,
    results: &BTreeMap<String, PromotionObjectiveResult>,
) -> Result<(), TuneError> {
    if results.keys().ne(stage.promotion.objectives.iter()) {
        return Err(invalid_policy("the saved promotion objective set changed"));
    }
    for (name, result) in results {
        result.statistics.validate()?;
        let target = stage.response_targets.target(&scenario.revision_id, name)?;
        if result.maximum_upper_95 != target.limit
            || result.passed != target.holds(result.statistics.upper_95)
        {
            return Err(invalid_policy(format!(
                "the saved promotion objective result for {name} changed"
            )));
        }
    }
    Ok(())
}
