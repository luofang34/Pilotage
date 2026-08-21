use std::collections::{BTreeMap, BTreeSet, HashSet};

use pilotage_trial::Digest;
use serde::{Deserialize, Serialize};

use crate::{CandidateLineage, ScenarioSet, TuneError};

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

/// A content-identified scenario for one simulator adapter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScenarioRef {
    /// The stable scenario name.
    pub id: String,
    /// The digest of the scenario artifact bytes.
    pub digest: Digest,
    /// The largest permitted sample count for one run.
    pub max_samples: u32,
    /// The timeout for each requested sample.
    pub sample_timeout_ms: u32,
}

/// The limits for the one promotion decision.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PromotionPolicy {
    /// The required reduction in paired mean loss.
    pub minimum_loss_improvement: f64,
    /// The required reduction as a fraction of baseline mean loss.
    pub minimum_relative_loss_improvement: f64,
    /// The largest permitted paired increase in mean control effort.
    pub maximum_control_effort_increase: f64,
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
    /// Maximum value for each required named objective.
    pub objective_maxima: BTreeMap<String, f64>,
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
    /// Scenarios that supply adaptive search evidence.
    pub training_scenarios: Vec<ScenarioRef>,
    /// Hidden scenarios for the one promotion decision.
    pub promotion_scenarios: Vec<ScenarioRef>,
    /// Hidden scenarios for the final release decision.
    pub final_qualification_scenarios: Vec<ScenarioRef>,
    /// The run count for each scenario.
    pub repetitions: u32,
    /// The limits for the one promotion decision.
    pub promotion: PromotionPolicy,
    /// Absolute limits for the final release decision.
    pub qualification: QualificationPolicy,
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
        self.validate_promotion()?;
        self.validate_qualification()
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
        let policy = self.promotion;
        if !policy.minimum_loss_improvement.is_finite()
            || policy.minimum_loss_improvement < 0.0
            || !policy.minimum_relative_loss_improvement.is_finite()
            || !(0.0..=1.0).contains(&policy.minimum_relative_loss_improvement)
            || !policy.maximum_control_effort_increase.is_finite()
            || policy.maximum_control_effort_increase < 0.0
        {
            return Err(invalid_stage("the promotion policy is not valid"));
        }
        Ok(())
    }

    fn validate_qualification(&self) -> Result<(), TuneError> {
        let policy = &self.qualification;
        if !policy.maximum_loss_confidence_upper.is_finite()
            || policy.maximum_loss_confidence_upper < 0.0
            || !policy.maximum_p95_loss.is_finite()
            || policy.maximum_p95_loss < 0.0
            || !policy.maximum_mean_control_effort.is_finite()
            || !(0.0..=1.0).contains(&policy.maximum_mean_control_effort)
            || policy.objective_maxima.iter().any(|(name, maximum)| {
                validate_name(name, "qualification objective").is_err()
                    || !maximum.is_finite()
                    || *maximum < 0.0
            })
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
    scenario: &ScenarioRef,
    ids: &mut BTreeSet<String>,
    digests: &mut HashSet<Digest>,
) -> Result<(), TuneError> {
    validate_name(&scenario.id, "scenario").map_err(as_stage_error)?;
    if !ids.insert(scenario.id.clone()) || !digests.insert(scenario.digest) {
        return Err(invalid_stage(format!(
            "scenario {} is repeated across stage partitions",
            scenario.id
        )));
    }
    if scenario.digest.is_zero()
        || scenario.max_samples == 0
        || scenario.sample_timeout_ms == 0
        || scenario.sample_timeout_ms > 60_000
    {
        return Err(invalid_stage(format!(
            "scenario {} limits or digest are not valid",
            scenario.id
        )));
    }
    Ok(())
}

pub(crate) fn derive_seed(
    fixed_seed: u64,
    set: ScenarioSet,
    scenario: &ScenarioRef,
    repetition: u32,
) -> u64 {
    let partition = match set {
        ScenarioSet::Training => 0x243f_6a88_85a3_08d3,
        ScenarioSet::Promotion => 0x1319_8a2e_0370_7344,
        ScenarioSet::FinalQualification => 0xa409_3822_299f_31d0,
    };
    let bytes = scenario.digest.as_bytes();
    let key = digest_word(bytes, 0)
        ^ digest_word(bytes, 8).rotate_left(13)
        ^ digest_word(bytes, 16).rotate_left(29)
        ^ digest_word(bytes, 24).rotate_left(47);
    split_mix(fixed_seed ^ partition ^ key ^ u64::from(repetition))
}

fn digest_word(bytes: &[u8; 32], start: usize) -> u64 {
    u64::from_le_bytes([
        bytes[start],
        bytes[start + 1],
        bytes[start + 2],
        bytes[start + 3],
        bytes[start + 4],
        bytes[start + 5],
        bytes[start + 6],
        bytes[start + 7],
    ])
}

fn split_mix(mut value: u64) -> u64 {
    value = value.wrapping_add(0x9e37_79b9_7f4a_7c15);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
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
