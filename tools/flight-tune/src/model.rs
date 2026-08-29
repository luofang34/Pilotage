use std::collections::{BTreeMap, BTreeSet, HashSet};

use pilotage_trial::Digest;
use serde::{Deserialize, Serialize};

use crate::flight_quality::MANDATORY_CRASH_GATE_ID;
use crate::{CandidateLineage, TuneError};

mod budget;
mod promotion;
mod reference;
pub(crate) mod response_target;
mod retry;
mod seed;
mod training_suite;

pub use budget::CampaignRunBound;
pub use promotion::{
    ExpectedPromotionPair, ExpectedPromotionRun, PROMOTION_POLICY_SCHEMA_VERSION,
    PromotionCalculation, PromotionComparison, PromotionObjectiveResult, PromotionPairedStatistics,
    PromotionPolicy, PromotionRunKey, PromotionRunPlan, PromotionScenarioResults,
    PromotionSeedPolicy, PromotionSelection, promotion_policy_digest,
};
pub(crate) use promotion::{expected_promotion_pairs, required_improvement};
pub use reference::MissionReference;
pub(crate) use response_target::verify_document;
pub use response_target::{
    PhysicalTarget, RESPONSE_TARGET_TABLE_SCHEMA_VERSION, ResponseTargetScope, ResponseTargetTable,
    ScenarioMotion, ScopedResponseTarget, TARGET_AUTHORITY_OBJECTIVE, TargetAuthorityBand,
    TargetComparison, is_admissible,
};
pub use retry::{EXECUTION_RETRY_POLICY_SCHEMA_VERSION, ExecutionRetryPolicy};
pub(crate) use seed::derive_seed;
#[cfg(test)]
pub(crate) use training_suite::tests::stage_for_budget;
pub(crate) use training_suite::{AttemptRunPlan, TrainingSuiteAnchor};
pub use training_suite::{
    SearchGroup, SearchGroupBinding, SearchGroupKind, TRAINING_SUITE_SCHEMA_VERSION, TrainingSuite,
};

const MAX_PARAMETERS: usize = 128;
const MAX_SCENARIOS_PER_SET: usize = 64;
const MAX_REPETITIONS: u32 = 32;
const MAX_GATE_COUNT: usize = 32;
const MAX_NAME_BYTES: usize = 128;

/// The inclusive range for one stage parameter.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ParameterBounds {
    /// The minimum permitted value.
    pub minimum: f64,
    /// The maximum permitted value.
    pub maximum: f64,
}

/// An immutable set of named numeric parameters and its source identity.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Candidate {
    lineage: CandidateLineage,
    parameters: BTreeMap<String, f64>,
}

impl Candidate {
    /// Creates a candidate.
    ///
    /// # Errors
    ///
    /// Returns [`TuneError`] when the lineage or parameter map is not valid.
    pub fn new(
        lineage: CandidateLineage,
        parameters: BTreeMap<String, f64>,
    ) -> Result<Self, TuneError> {
        let candidate = Self {
            lineage,
            parameters,
        };
        candidate.validate()?;
        Ok(candidate)
    }

    /// Returns the candidate source identity.
    #[must_use]
    pub const fn lineage(&self) -> &CandidateLineage {
        &self.lineage
    }

    /// Returns the complete parameter map.
    #[must_use]
    pub const fn parameters(&self) -> &BTreeMap<String, f64> {
        &self.parameters
    }

    /// Creates a candidate with one replaced parameter value.
    ///
    /// # Errors
    ///
    /// Returns [`TuneError`] when the parameter does not exist or the value is
    /// not finite.
    pub fn with_parameter(&self, name: &str, value: f64) -> Result<Self, TuneError> {
        if !value.is_finite() {
            return Err(invalid_candidate(format!("parameter {name} is not finite")));
        }
        if !self.parameters.contains_key(name) {
            return Err(invalid_candidate(format!(
                "parameter {name} does not exist"
            )));
        }
        let mut parameters = self.parameters.clone();
        parameters.insert(name.to_owned(), value);
        Self::new(self.lineage.clone(), parameters)
    }

    pub(crate) fn validate(&self) -> Result<(), TuneError> {
        self.lineage.validate()?;
        if self.parameters.is_empty() || self.parameters.len() > MAX_PARAMETERS {
            return Err(invalid_candidate(format!(
                "a candidate needs 1 to {MAX_PARAMETERS} parameters"
            )));
        }
        for (name, value) in &self.parameters {
            validate_name(name, "parameter")?;
            if !value.is_finite() {
                return Err(invalid_candidate(format!("parameter {name} is not finite")));
            }
        }
        Ok(())
    }
}

/// Absolute release limits for the untouched final partition.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QualificationPolicy {
    /// Largest permitted upper 95 percent confidence limit for mean loss.
    pub maximum_loss_confidence_upper: f64,
    /// Largest permitted 95th percentile loss.
    pub maximum_p95_loss: f64,
    /// Largest permitted mean normalized control effort.
    pub maximum_mean_control_effort: f64,
    /// The objectives that every final qualification run has to state.
    ///
    /// The policy declares the names only. Each absolute maximum is one row of
    /// the stage's scoped response target table, so a limit written for a
    /// direct attitude scenario can never decide a velocity one.
    pub objectives: BTreeSet<String>,
}

/// One bounded search and qualification stage.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SearchStage {
    /// The stable stage identity.
    pub id: String,
    /// The only parameters that a proposal can change.
    pub allowlist: BTreeMap<String, ParameterBounds>,
    /// Parameters that must have these exact values.
    pub fixed_parameters: BTreeMap<String, f64>,
    /// Hard gates in their evaluation priority order.
    pub required_hard_gates: Vec<String>,
    /// Missions that supply adaptive search evidence.
    pub training_scenarios: Vec<MissionReference>,
    /// The frozen training suites in their declared order.
    pub training_suites: Vec<TrainingSuite>,
    /// The search parameter groups in their declared order.
    pub search_groups: Vec<SearchGroup>,
    /// Hidden missions for the one promotion decision.
    pub promotion_scenarios: Vec<MissionReference>,
    /// Hidden missions for the final release decision.
    pub final_qualification_scenarios: Vec<MissionReference>,
    /// The run count for each mission.
    pub repetitions: u32,
    /// The limits for the one promotion decision.
    pub promotion: PromotionPolicy,
    /// Absolute limits for the final release decision.
    pub qualification: QualificationPolicy,
    /// The exact scoped limit for every decision this stage takes.
    pub response_targets: ResponseTargetTable,
    /// How many replacement executions one quarantined attempt may receive.
    pub execution_retry: ExecutionRetryPolicy,
}

impl SearchStage {
    /// Validates the complete stage contract.
    ///
    /// # Errors
    ///
    /// Returns [`TuneError`] when a bound, identity, partition, or policy is
    /// not valid.
    pub fn validate(&self) -> Result<(), TuneError> {
        validate_name(&self.id, "stage").map_err(as_stage_error)?;
        self.validate_counts()?;
        self.validate_parameters()?;
        self.validate_scenarios()?;
        training_suite::validate_search_space(
            &self.allowlist,
            &self.training_scenarios,
            &self.training_suites,
            &self.search_groups,
        )?;
        self.validate_promotion()?;
        self.execution_retry.validate()?;
        self.validate_qualification()?;
        response_target::validate_for_stage(&self.response_targets, self)
    }

    /// Returns the identity of the complete scoped response target table.
    ///
    /// # Errors
    ///
    /// Returns [`TuneError`] when the table cannot encode.
    pub fn response_target_digest(&self) -> Result<Digest, TuneError> {
        self.response_targets.digest()
    }

    /// Returns the frozen suite at one position in the declared suite order.
    ///
    /// # Errors
    ///
    /// Returns [`TuneError`] when the stage does not declare that suite.
    pub fn training_suite(&self, index: u16) -> Result<&TrainingSuite, TuneError> {
        self.training_suites
            .get(index as usize)
            .ok_or_else(|| invalid_stage(format!("the stage does not declare suite {index}")))
    }

    /// Derives the search group and training suite for one exact proposal.
    ///
    /// The derivation reads only the parameters that differ between the two
    /// candidates. A proposal strategy cannot name its own suite, so a
    /// controller change cannot take an operator-feel suite.
    ///
    /// # Errors
    ///
    /// Returns [`TuneError`] when the proposal changes no parameter or changes
    /// parameters from two groups.
    pub fn derive_search_group(
        &self,
        incumbent: &Candidate,
        challenger: &Candidate,
    ) -> Result<SearchGroupBinding, TuneError> {
        let changed = training_suite::changed_parameters(incumbent, challenger);
        let Some(first) = changed.first() else {
            return Err(invalid_candidate("a proposal changes no parameter"));
        };
        let mut owners = BTreeSet::new();
        for name in &changed {
            let owner = self.owning_group(name)?;
            owners.insert(owner.id.as_str());
        }
        if owners.len() != 1 {
            return Err(invalid_candidate(
                "a proposal changes parameters from two search groups",
            ));
        }
        self.binding_for(self.owning_group(first)?)
    }

    fn owning_group(&self, parameter: &str) -> Result<&SearchGroup, TuneError> {
        self.search_groups
            .iter()
            .find(|group| group.parameters.contains(parameter))
            .ok_or_else(|| invalid_candidate(format!("parameter {parameter} has no search group")))
    }

    fn binding_for(&self, group: &SearchGroup) -> Result<SearchGroupBinding, TuneError> {
        let (index, suite) = self
            .training_suites
            .iter()
            .enumerate()
            .find(|(_, suite)| suite.id == group.suite_id)
            .ok_or_else(|| invalid_stage(format!("group {} has no suite", group.id)))?;
        let suite_index =
            u16::try_from(index).map_err(|_| invalid_stage("a suite index exceeds u16"))?;
        Ok(SearchGroupBinding {
            group_id: group.id.clone(),
            suite_id: suite.id.clone(),
            suite_index,
            suite_digest: suite.digest()?,
        })
    }

    /// Validates one candidate against its current training incumbent.
    ///
    /// # Errors
    ///
    /// Returns [`TuneError`] when a candidate changes a protected value.
    pub fn validate_challenger(
        &self,
        incumbent: &Candidate,
        challenger: &Candidate,
    ) -> Result<(), TuneError> {
        incumbent.validate()?;
        challenger.validate()?;
        if incumbent.lineage != challenger.lineage {
            return Err(invalid_candidate("a challenger changed candidate lineage"));
        }
        if incumbent.parameters.keys().ne(challenger.parameters.keys()) {
            return Err(invalid_candidate(
                "a challenger must keep the incumbent parameter set",
            ));
        }
        self.validate_fixed_values(challenger)?;
        self.validate_changed_values(incumbent, challenger)
    }

    fn validate_counts(&self) -> Result<(), TuneError> {
        if self.allowlist.is_empty() || self.allowlist.len() > MAX_PARAMETERS {
            return Err(invalid_stage("the stage allowlist size is not valid"));
        }
        if !(2..=MAX_REPETITIONS).contains(&self.repetitions) {
            return Err(invalid_stage(format!(
                "repetitions must be in 2 to {MAX_REPETITIONS}"
            )));
        }
        if self.required_hard_gates.is_empty() || self.required_hard_gates.len() > MAX_GATE_COUNT {
            return Err(invalid_stage("the hard gate count is not valid"));
        }
        // The crash gate is the floor of every campaign and the first gate
        // evaluated. A campaign that scored a run after the vehicle hit
        // something states a measurement of the collision, and a gate that
        // ran after a bound gate would let that measurement be reported as a
        // limit failure instead of as the crash it was.
        if self.required_hard_gates.first().map(String::as_str) != Some(MANDATORY_CRASH_GATE_ID) {
            return Err(invalid_stage(format!(
                "{MANDATORY_CRASH_GATE_ID} must be the first required hard gate"
            )));
        }
        let mut gates = BTreeSet::new();
        for gate in &self.required_hard_gates {
            validate_name(gate, "hard gate").map_err(as_stage_error)?;
            if !gates.insert(gate) {
                return Err(invalid_stage("a hard gate id is repeated"));
            }
        }
        Ok(())
    }

    fn validate_parameters(&self) -> Result<(), TuneError> {
        for (name, bounds) in &self.allowlist {
            validate_name(name, "parameter").map_err(as_stage_error)?;
            if !bounds.minimum.is_finite()
                || !bounds.maximum.is_finite()
                || bounds.minimum >= bounds.maximum
            {
                return Err(invalid_stage(format!("bounds for {name} are not valid")));
            }
            if self.fixed_parameters.contains_key(name) {
                return Err(invalid_stage(format!("parameter {name} has two roles")));
            }
        }
        for (name, value) in &self.fixed_parameters {
            validate_name(name, "fixed parameter").map_err(as_stage_error)?;
            if !value.is_finite() {
                return Err(invalid_stage(format!(
                    "fixed parameter {name} is not finite"
                )));
            }
        }
        Ok(())
    }

    fn validate_scenarios(&self) -> Result<(), TuneError> {
        let sets = [
            &self.training_scenarios,
            &self.promotion_scenarios,
            &self.final_qualification_scenarios,
        ];
        let mut ids = BTreeSet::new();
        let mut digests = HashSet::new();
        for scenarios in sets {
            if scenarios.is_empty() || scenarios.len() > MAX_SCENARIOS_PER_SET {
                return Err(invalid_stage("a scenario set size is not valid"));
            }
            for scenario in scenarios {
                validate_scenario(scenario, &mut ids, &mut digests)?;
            }
        }
        Ok(())
    }

    fn validate_promotion(&self) -> Result<(), TuneError> {
        self.promotion.validate().map_err(as_stage_error)
    }

    fn validate_qualification(&self) -> Result<(), TuneError> {
        let policy = &self.qualification;
        if !policy.maximum_loss_confidence_upper.is_finite()
            || policy.maximum_loss_confidence_upper < 0.0
            || !policy.maximum_p95_loss.is_finite()
            || policy.maximum_p95_loss < 0.0
            || !policy.maximum_mean_control_effort.is_finite()
            || !(0.0..=1.0).contains(&policy.maximum_mean_control_effort)
            || policy.objectives.is_empty()
            || policy
                .objectives
                .iter()
                .any(|name| validate_name(name, "qualification objective").is_err())
        {
            return Err(invalid_stage("the qualification policy is not valid"));
        }
        Ok(())
    }

    fn validate_fixed_values(&self, candidate: &Candidate) -> Result<(), TuneError> {
        for (name, expected) in &self.fixed_parameters {
            if candidate.parameters.get(name) != Some(expected) {
                return Err(invalid_candidate(format!(
                    "fixed parameter {name} does not match the stage"
                )));
            }
        }
        Ok(())
    }

    fn validate_changed_values(
        &self,
        incumbent: &Candidate,
        challenger: &Candidate,
    ) -> Result<(), TuneError> {
        for name in self.allowlist.keys() {
            if !challenger.parameters.contains_key(name) {
                return Err(invalid_candidate(format!("candidate has no {name}")));
            }
        }
        for (name, value) in &challenger.parameters {
            if incumbent.parameters.get(name) != Some(value) && !self.allowlist.contains_key(name) {
                return Err(invalid_candidate(format!("parameter {name} is protected")));
            }
            if let Some(bounds) = self.allowlist.get(name)
                && !(bounds.minimum..=bounds.maximum).contains(value)
            {
                return Err(invalid_candidate(format!(
                    "parameter {name} is out of bounds"
                )));
            }
        }
        Ok(())
    }
}

fn validate_scenario(
    scenario: &MissionReference,
    ids: &mut BTreeSet<String>,
    digests: &mut HashSet<Digest>,
) -> Result<(), TuneError> {
    scenario.validate()?;
    if !ids.insert(scenario.revision_id.clone()) || !digests.insert(scenario.content_digest) {
        return Err(invalid_stage(format!(
            "mission {} is repeated across stage partitions",
            scenario.revision_id
        )));
    }
    Ok(())
}

fn validate_name(value: &str, kind: &str) -> Result<(), TuneError> {
    if value.trim().is_empty() || value.len() > MAX_NAME_BYTES {
        return Err(invalid_candidate(format!(
            "{kind} names need 1 to {MAX_NAME_BYTES} bytes"
        )));
    }
    Ok(())
}

fn as_stage_error(error: TuneError) -> TuneError {
    invalid_stage(error.to_string())
}

fn invalid_candidate(detail: impl Into<String>) -> TuneError {
    TuneError::InvalidCandidate {
        detail: detail.into(),
    }
}

fn invalid_stage(detail: impl Into<String>) -> TuneError {
    TuneError::InvalidStage {
        detail: detail.into(),
    }
}
