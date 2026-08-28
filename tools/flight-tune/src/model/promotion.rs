use std::collections::{BTreeMap, BTreeSet, HashSet};

use serde::{Deserialize, Serialize};

use super::{MissionReference, SearchStage, derive_seed};
use crate::identity::digest_bytes;
use crate::{AttemptRole, Digest, PromotionDecision, RunExecutionContext, TuneError};

#[cfg(test)]
#[path = "promotion/tests.rs"]
mod tests;

/// The supported promotion policy schema.
pub const PROMOTION_POLICY_SCHEMA_VERSION: u16 = 1;

const PROMOTION_POLICY_DOMAIN: &[u8] = b"pilotage.flight-tune.promotion-policy.v1\0";
const MAX_PROMOTION_OBJECTIVES: usize = 64;

/// The versioned seed algorithm for paired promotion runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromotionSeedPolicy {
    /// Derive one shared seed from the scenario digest and repetition.
    PairedScenarioDigestV1,
}

/// The complete limits and seed policy for one promotion decision.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PromotionPolicy {
    /// The policy schema.
    pub schema_version: u16,
    /// The paired-run seed algorithm.
    pub seed_policy: PromotionSeedPolicy,
    /// The required reduction in paired mean loss.
    pub minimum_loss_improvement: f64,
    /// The required reduction as a fraction of baseline mean loss.
    pub minimum_relative_loss_improvement: f64,
    /// The largest permitted paired increase in mean control effort.
    pub maximum_control_effort_increase: f64,
    /// The largest permitted paired upper 95 percent increase by objective.
    pub objective_regression_upper_95: BTreeMap<String, f64>,
}

impl PromotionPolicy {
    /// Validates all promotion limits and policy identities.
    ///
    /// # Errors
    ///
    /// Returns [`TuneError`] when a version, name, or limit is not valid.
    pub fn validate(&self) -> Result<(), TuneError> {
        if self.schema_version != PROMOTION_POLICY_SCHEMA_VERSION
            || !valid_nonnegative(self.minimum_loss_improvement)
            || !self.minimum_relative_loss_improvement.is_finite()
            || !(0.0..=1.0).contains(&self.minimum_relative_loss_improvement)
            || !self.maximum_control_effort_increase.is_finite()
            || !(0.0..=1.0).contains(&self.maximum_control_effort_increase)
        {
            return Err(invalid_policy(
                "a scalar promotion policy value is not valid",
            ));
        }
        validate_objective_limits(&self.objective_regression_upper_95)
    }
}

/// Returns the domain-separated identity of one promotion policy.
///
/// # Errors
///
/// Returns [`TuneError`] when the policy or its encoding is not valid.
pub fn promotion_policy_digest(policy: &PromotionPolicy) -> Result<Digest, TuneError> {
    policy.validate()?;
    digest_policy_content(policy)
}

/// The immutable session inputs that identify both promotion attempts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PromotionRunPlan {
    /// The tuning session identity.
    pub tuning_session_digest: Digest,
    /// The baseline attempt identity.
    pub baseline_trial_id: u64,
    /// The frozen attempt identity.
    pub frozen_trial_id: u64,
    /// The initial candidate identity.
    pub initial_candidate_digest: Digest,
    /// The frozen candidate identity.
    pub frozen_candidate_digest: Digest,
    /// The session fixed seed.
    pub fixed_seed: u64,
}

impl PromotionRunPlan {
    /// Validates all immutable promotion attempt identities.
    ///
    /// # Errors
    ///
    /// Returns [`TuneError`] when an identity is zero or repeated.
    pub fn validate(&self) -> Result<(), TuneError> {
        if self.tuning_session_digest.is_zero()
            || self.initial_candidate_digest.is_zero()
            || self.frozen_candidate_digest.is_zero()
            || self.baseline_trial_id == self.frozen_trial_id
        {
            return Err(invalid_policy("the promotion run plan is not valid"));
        }
        Ok(())
    }
}

/// The exact role, candidate, scenario, repetition, and seed for one run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PromotionRunKey {
    /// The promotion attempt role.
    pub role: AttemptRole,
    /// The candidate identity.
    pub candidate_digest: Digest,
    /// The executed mission revision.
    pub mission_revision_id: String,
    /// The executed mission content identity.
    pub mission_content_digest: Digest,
    /// The zero-based repetition.
    pub repetition: u32,
    /// The deterministic run seed.
    pub seed: u64,
}

/// One expected promotion run and its canonical run identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExpectedPromotionRun {
    /// The simulator-neutral run key.
    pub key: PromotionRunKey,
    /// The complete run execution context.
    pub context: RunExecutionContext,
    /// The canonical run intent identity.
    pub run_intent_digest: Digest,
}

impl ExpectedPromotionRun {
    /// Validates the duplicated key and context binding.
    ///
    /// # Errors
    ///
    /// Returns [`TuneError`] when one run identity differs.
    pub fn validate(&self) -> Result<(), TuneError> {
        self.context.validate()?;
        if self.run_intent_digest != self.context.digest()?
            || self.key != key_from_context(&self.context)
            || !matches!(
                self.key.role,
                AttemptRole::PromotionBaseline | AttemptRole::PromotionFrozen
            )
        {
            return Err(invalid_policy("an expected promotion run identity changed"));
        }
        Ok(())
    }
}

/// One exact baseline and frozen run pair.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExpectedPromotionPair {
    /// The initial candidate run.
    pub baseline: ExpectedPromotionRun,
    /// The frozen candidate run.
    pub frozen: ExpectedPromotionRun,
}

impl ExpectedPromotionPair {
    /// Validates the paired run identities.
    ///
    /// # Errors
    ///
    /// Returns [`TuneError`] when the runs are not one exact pair.
    pub fn validate(&self) -> Result<(), TuneError> {
        self.baseline.validate()?;
        self.frozen.validate()?;
        let left = &self.baseline.key;
        let right = &self.frozen.key;
        if left.role != AttemptRole::PromotionBaseline
            || right.role != AttemptRole::PromotionFrozen
            || left.mission_revision_id != right.mission_revision_id
            || left.mission_content_digest != right.mission_content_digest
            || left.repetition != right.repetition
            || left.seed != right.seed
            || self.baseline.context.tuning_session_digest()
                != self.frozen.context.tuning_session_digest()
        {
            return Err(invalid_policy(
                "expected promotion runs do not form one pair",
            ));
        }
        Ok(())
    }
}

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
    /// Whether the paired upper limit met the policy.
    pub passed: bool,
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
    /// One explicit result for each declared named objective.
    pub objectives: BTreeMap<String, PromotionObjectiveResult>,
}

impl PromotionComparison {
    /// Validates all saved statistics and results against one policy.
    ///
    /// # Errors
    ///
    /// Returns [`TuneError`] when a value, key, threshold, or result differs.
    pub fn validate_for(&self, policy: &PromotionPolicy) -> Result<(), TuneError> {
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
        validate_objective_results(policy, &self.objectives)
    }

    /// Reports whether every relative promotion gate passed.
    #[must_use]
    pub fn all_passed(&self) -> bool {
        self.loss_passed
            && self.control_effort_passed
            && self.objectives.values().all(|result| result.passed)
    }
}

/// The promotion decision and its only permitted release candidate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PromotionSelection {
    /// The promotion result class.
    pub decision: PromotionDecision,
    /// The authorized candidate, if this result can enter final qualification.
    pub selected_candidate: Option<Digest>,
}

/// The complete result of one paired promotion calculation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PromotionCalculation {
    /// All paired statistics and named objective results.
    pub comparison: PromotionComparison,
    /// The decision and selected candidate.
    pub selection: PromotionSelection,
}

pub(crate) fn expected_promotion_pairs(
    stage: &SearchStage,
    plan: PromotionRunPlan,
) -> Result<Vec<ExpectedPromotionPair>, TuneError> {
    stage.validate()?;
    plan.validate()?;
    validate_promotion_scenarios(&stage.promotion_scenarios)?;
    let capacity = stage
        .promotion_scenarios
        .len()
        .checked_mul(stage.repetitions as usize)
        .ok_or_else(|| invalid_policy("the expected promotion run count overflowed"))?;
    let mut pairs = Vec::with_capacity(capacity);
    for scenario in &stage.promotion_scenarios {
        for repetition in 0..stage.repetitions {
            pairs.push(expected_pair(&stage.promotion, plan, scenario, repetition)?);
        }
    }
    Ok(pairs)
}

fn expected_pair(
    policy: &PromotionPolicy,
    plan: PromotionRunPlan,
    scenario: &MissionReference,
    repetition: u32,
) -> Result<ExpectedPromotionPair, TuneError> {
    let seed = match policy.seed_policy {
        PromotionSeedPolicy::PairedScenarioDigestV1 => derive_seed(
            plan.fixed_seed,
            crate::ScenarioSet::Promotion,
            scenario,
            repetition,
        ),
    };
    Ok(ExpectedPromotionPair {
        baseline: expected_run(
            plan,
            AttemptRole::PromotionBaseline,
            plan.baseline_trial_id,
            plan.initial_candidate_digest,
            scenario,
            repetition,
            seed,
        )?,
        frozen: expected_run(
            plan,
            AttemptRole::PromotionFrozen,
            plan.frozen_trial_id,
            plan.frozen_candidate_digest,
            scenario,
            repetition,
            seed,
        )?,
    })
}

#[allow(clippy::too_many_arguments)]
fn expected_run(
    plan: PromotionRunPlan,
    role: AttemptRole,
    trial_id: u64,
    candidate_digest: Digest,
    scenario: &MissionReference,
    repetition: u32,
    seed: u64,
) -> Result<ExpectedPromotionRun, TuneError> {
    let context = RunExecutionContext::new(
        plan.tuning_session_digest,
        trial_id,
        role,
        candidate_digest,
        None,
        crate::ScenarioSet::Promotion,
        scenario,
        repetition,
        seed,
    )?;
    Ok(ExpectedPromotionRun {
        key: key_from_context(&context),
        run_intent_digest: context.digest()?,
        context,
    })
}

fn key_from_context(context: &RunExecutionContext) -> PromotionRunKey {
    PromotionRunKey {
        role: context.role(),
        candidate_digest: context.candidate_digest(),
        mission_revision_id: context.mission_revision_id().to_owned(),
        mission_content_digest: context.mission_content_digest(),
        repetition: context.repetition(),
        seed: context.seed(),
    }
}

fn validate_promotion_scenarios(scenarios: &[MissionReference]) -> Result<(), TuneError> {
    let mut ids = BTreeSet::new();
    let mut digests = HashSet::new();
    if scenarios.is_empty()
        || scenarios.iter().any(|scenario| {
            !ids.insert(scenario.revision_id.as_str()) || !digests.insert(scenario.content_digest)
        })
    {
        return Err(invalid_policy("promotion scenarios are empty or repeated"));
    }
    Ok(())
}

fn validate_objective_limits(limits: &BTreeMap<String, f64>) -> Result<(), TuneError> {
    if limits.is_empty()
        || limits.len() > MAX_PROMOTION_OBJECTIVES
        || limits.iter().any(|(name, limit)| {
            name.is_empty()
                || name.len() > 128
                || name.chars().any(char::is_whitespace)
                || !valid_nonnegative(*limit)
        })
    {
        return Err(invalid_policy("promotion objective limits are not valid"));
    }
    Ok(())
}

fn validate_objective_results(
    policy: &PromotionPolicy,
    results: &BTreeMap<String, PromotionObjectiveResult>,
) -> Result<(), TuneError> {
    if results
        .keys()
        .ne(policy.objective_regression_upper_95.keys())
    {
        return Err(invalid_policy("the saved promotion objective set changed"));
    }
    for (name, result) in results {
        result.statistics.validate()?;
        let maximum = policy
            .objective_regression_upper_95
            .get(name)
            .copied()
            .ok_or_else(|| invalid_policy("a promotion objective result has no policy limit"))?;
        if result.maximum_upper_95 != maximum
            || result.passed != (result.statistics.upper_95 <= maximum)
        {
            return Err(invalid_policy(format!(
                "the saved promotion objective result for {name} changed"
            )));
        }
    }
    Ok(())
}

pub(crate) fn required_improvement(
    policy: &PromotionPolicy,
    baseline_mean_loss: f64,
) -> Result<f64, TuneError> {
    let relative = baseline_mean_loss * policy.minimum_relative_loss_improvement;
    let required = policy.minimum_loss_improvement.max(relative);
    if required.is_finite() {
        Ok(required)
    } else {
        Err(invalid_policy(
            "promotion threshold arithmetic is not finite",
        ))
    }
}

fn digest_policy_content(policy: &PromotionPolicy) -> Result<Digest, TuneError> {
    let document = serde_json::to_vec(policy).map_err(|source| TuneError::Encode {
        document: "promotion policy",
        source,
    })?;
    let mut bytes =
        Vec::with_capacity(PROMOTION_POLICY_DOMAIN.len().saturating_add(document.len()));
    bytes.extend_from_slice(PROMOTION_POLICY_DOMAIN);
    bytes.extend_from_slice(&document);
    Ok(digest_bytes(&bytes))
}

const fn valid_nonnegative(value: f64) -> bool {
    value.is_finite() && value >= 0.0
}

fn invalid_policy(detail: impl Into<String>) -> TuneError {
    TuneError::InvalidStage {
        detail: detail.into(),
    }
}
