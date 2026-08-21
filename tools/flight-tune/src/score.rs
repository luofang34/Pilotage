use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::{ArtifactIdentity, EvaluatorError, ScenarioRef, TelemetrySample, TuneError};

#[cfg(test)]
#[path = "score/tests.rs"]
mod tests;

/// One ordered hard gate result for one telemetry sample.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GateOutcome {
    /// The stable hard gate identity.
    pub id: String,
    /// Whether this gate passed.
    pub passed: bool,
    /// A stable result detail.
    pub detail: String,
}

impl GateOutcome {
    /// Creates a passing hard gate result.
    #[must_use]
    pub fn pass(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            passed: true,
            detail: "pass".to_owned(),
        }
    }

    /// Creates a failing hard gate result.
    #[must_use]
    pub fn fail(id: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            passed: false,
            detail: detail.into(),
        }
    }
}

/// A streaming evaluator for all hard gates in one stage.
pub trait GateEvaluator {
    /// Returns the exact gate implementation identity.
    fn identity(&self) -> &ArtifactIdentity;

    /// Starts evaluation for one scenario run.
    fn begin(&mut self, scenario: &ScenarioRef) -> Result<(), EvaluatorError>;

    /// Evaluates hard gates in order through the first failure.
    fn evaluate(&mut self, sample: &TelemetrySample) -> Result<Vec<GateOutcome>, EvaluatorError>;

    /// Completes gate evaluation for a normal scenario run.
    fn finish(&mut self) -> Result<(), EvaluatorError>;

    /// Cancels evaluator state after an incomplete run.
    fn cancel(&mut self) -> Result<(), EvaluatorError>;
}

/// Final continuous values from one completed scenario run.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MetricValues {
    /// Continuous loss, where a smaller value is better.
    pub loss: f64,
    /// Normalized control effort in the inclusive range from zero to one.
    pub control_effort: f64,
}

/// A streaming continuous metric evaluator.
pub trait MetricEvaluator {
    /// Returns the exact metric implementation identity.
    fn identity(&self) -> &ArtifactIdentity;

    /// Starts metric evaluation for one scenario run.
    fn begin(&mut self, scenario: &ScenarioRef) -> Result<(), EvaluatorError>;

    /// Adds one sample after all hard gates pass for that sample.
    fn observe(&mut self, sample: &TelemetrySample) -> Result<(), EvaluatorError>;

    /// Completes the metric for one normal scenario run.
    fn finish(&mut self) -> Result<MetricValues, EvaluatorError>;

    /// Cancels evaluator state after an incomplete run.
    fn cancel(&mut self) -> Result<(), EvaluatorError>;
}

/// The isolated scenario partition for one run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScenarioSet {
    /// A scenario that supplies adaptive search evidence.
    Training,
    /// A hidden scenario for the one promotion decision.
    Promotion,
    /// A hidden scenario for the final release decision.
    FinalQualification,
}

/// One saved passing scenario run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunRecord {
    /// The isolated scenario partition.
    pub scenario_set: ScenarioSet,
    /// The stable scenario identity.
    pub scenario_id: String,
    /// The repeated-run index.
    pub repetition: u32,
    /// The deterministic run seed.
    pub seed: u64,
    /// The continuous loss.
    pub loss: f64,
    /// The normalized control effort.
    pub control_effort: f64,
    /// The hard gate identities that passed for all samples.
    pub passed_hard_gates: Vec<String>,
}

/// The first hard gate failure for one candidate evaluation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HardGateFailure {
    /// The isolated scenario partition.
    pub scenario_set: ScenarioSet,
    /// The stable scenario identity.
    pub scenario_id: String,
    /// The repeated-run index.
    pub repetition: u32,
    /// The deterministic run seed.
    pub seed: u64,
    /// The sample sequence that failed.
    pub sample_sequence: u64,
    /// The elapsed simulator time at the failure.
    pub elapsed_ms: u64,
    /// The failed gate.
    pub gate: GateOutcome,
}

/// A two-sided 95 percent confidence interval for a mean.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfidenceInterval {
    /// The lower interval limit.
    pub lower: f64,
    /// The upper interval limit.
    pub upper: f64,
}

/// Repeated-run statistics for one scenario partition.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScoreAggregate {
    /// The number of passing runs.
    pub run_count: u32,
    /// The arithmetic mean loss.
    pub mean_loss: f64,
    /// The nearest-rank 95th percentile loss.
    pub p95_loss: f64,
    /// The sample variance of loss.
    pub loss_variance: f64,
    /// The Student t 95 percent confidence interval for mean loss.
    pub loss_confidence_95: ConfidenceInterval,
    /// The arithmetic mean normalized control effort.
    pub mean_control_effort: f64,
}

/// The saved result for one candidate in one isolated partition.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case", deny_unknown_fields)]
pub enum CandidateEvaluation {
    /// Every planned run passed all hard gates.
    Passed {
        /// Statistics for this isolated partition.
        aggregate: ScoreAggregate,
        /// Each passing run in deterministic order.
        runs: Vec<RunRecord>,
    },
    /// Evaluation stopped at the first hard gate failure.
    HardGateFailed {
        /// The first hard gate failure.
        failure: HardGateFailure,
        /// Passing runs completed before the failure.
        completed_runs: Vec<RunRecord>,
    },
    /// Recovery or an execution error quarantined the candidate.
    Quarantined {
        /// The stable quarantine reason.
        reason: String,
    },
}

impl CandidateEvaluation {
    /// Returns the aggregate when all hard gates passed.
    #[must_use]
    pub const fn aggregate(&self) -> Option<&ScoreAggregate> {
        match self {
            Self::Passed { aggregate, .. } => Some(aggregate),
            Self::HardGateFailed { .. } | Self::Quarantined { .. } => None,
        }
    }

    /// Returns the saved passing runs.
    #[must_use]
    pub fn runs(&self) -> &[RunRecord] {
        match self {
            Self::Passed { runs, .. } => runs,
            Self::HardGateFailed { completed_runs, .. } => completed_runs,
            Self::Quarantined { .. } => &[],
        }
    }

    pub(crate) fn validate(&self, set: ScenarioSet) -> Result<(), TuneError> {
        match self {
            Self::Passed { aggregate, runs } => {
                validate_runs(runs, set)?;
                if aggregate != &aggregate_runs(runs, set)? {
                    return Err(invalid_score("the saved aggregate does not match its runs"));
                }
            }
            Self::HardGateFailed {
                failure,
                completed_runs,
            } => {
                validate_runs(completed_runs, set)?;
                if failure.scenario_set != set || failure.gate.passed {
                    return Err(invalid_score("the saved hard gate failure is not valid"));
                }
            }
            Self::Quarantined { reason } if reason.trim().is_empty() => {
                return Err(invalid_score("a quarantine reason is empty"));
            }
            Self::Quarantined { .. } => {}
        }
        Ok(())
    }
}

pub(crate) fn validate_gate_outcomes(
    required: &[String],
    outcomes: &[GateOutcome],
) -> Result<Option<GateOutcome>, TuneError> {
    if outcomes.is_empty() || outcomes.len() > required.len() {
        return Err(invalid_score(
            "the evaluator returned an invalid gate count",
        ));
    }
    let mut seen = BTreeSet::new();
    for (index, (required_id, outcome)) in required.iter().zip(outcomes).enumerate() {
        if outcome.id != *required_id || !seen.insert(&outcome.id) {
            return Err(invalid_score("the evaluator changed hard gate order"));
        }
        if !outcome.passed {
            if index.wrapping_add(1) != outcomes.len() {
                return Err(invalid_score(
                    "the evaluator continued after a hard failure",
                ));
            }
            return Ok(Some(outcome.clone()));
        }
    }
    if outcomes.len() == required.len() {
        Ok(None)
    } else {
        Err(invalid_score("the evaluator omitted a passing hard gate"))
    }
}

pub(crate) fn validate_metric(values: MetricValues) -> Result<(), TuneError> {
    if !values.loss.is_finite() || values.loss < 0.0 {
        return Err(invalid_score("loss must be finite and nonnegative"));
    }
    if !values.control_effort.is_finite() || !(0.0..=1.0).contains(&values.control_effort) {
        return Err(invalid_score("control effort must be in zero to one"));
    }
    Ok(())
}

pub(crate) fn aggregate_runs(
    runs: &[RunRecord],
    set: ScenarioSet,
) -> Result<ScoreAggregate, TuneError> {
    if runs.len() < 2 || runs.iter().any(|run| run.scenario_set != set) {
        return Err(invalid_score(
            "an aggregate needs two runs from one partition",
        ));
    }
    let loss_stats = OnlineStats::from_values(runs.iter().map(|run| run.loss))?;
    let effort_stats = OnlineStats::from_values(runs.iter().map(|run| run.control_effort))?;
    let count = runs.len();
    let variance = loss_stats.sample_variance()?;
    let critical = student_t_95(count - 1);
    let half_width = checked(critical * (variance / count as f64).sqrt())?;
    let mut losses = runs.iter().map(|run| run.loss).collect::<Vec<_>>();
    losses.sort_by(f64::total_cmp);
    let percentile_index = ((count * 95).div_ceil(100)).saturating_sub(1);
    Ok(ScoreAggregate {
        run_count: u32::try_from(count).map_err(|_| invalid_score("run count exceeds u32"))?,
        mean_loss: loss_stats.mean,
        p95_loss: losses[percentile_index],
        loss_variance: variance,
        loss_confidence_95: ConfidenceInterval {
            lower: checked(loss_stats.mean - half_width)?,
            upper: checked(loss_stats.mean + half_width)?,
        },
        mean_control_effort: effort_stats.mean,
    })
}

pub(crate) fn paired_stats(values: impl Iterator<Item = f64>) -> Result<PairedStats, TuneError> {
    let stats = OnlineStats::from_values(values)?;
    if stats.count < 2 {
        return Err(invalid_score("a paired comparison needs two run pairs"));
    }
    let variance = stats.sample_variance()?;
    let half_width =
        checked(student_t_95(stats.count - 1) * (variance / stats.count as f64).sqrt())?;
    Ok(PairedStats {
        mean: stats.mean,
        upper_95: checked(stats.mean + half_width)?,
    })
}

pub(crate) struct PairedStats {
    pub(crate) mean: f64,
    pub(crate) upper_95: f64,
}

struct OnlineStats {
    count: usize,
    mean: f64,
    m2: f64,
}

impl OnlineStats {
    fn from_values(values: impl Iterator<Item = f64>) -> Result<Self, TuneError> {
        let mut stats = Self {
            count: 0,
            mean: 0.0,
            m2: 0.0,
        };
        for value in values {
            if !value.is_finite() {
                return Err(invalid_score("a score input is not finite"));
            }
            stats.count = stats.count.wrapping_add(1);
            let delta = checked(value - stats.mean)?;
            stats.mean = checked(stats.mean + delta / stats.count as f64)?;
            let next_delta = checked(value - stats.mean)?;
            stats.m2 = checked(stats.m2 + checked(delta * next_delta)?)?;
        }
        Ok(stats)
    }

    fn sample_variance(&self) -> Result<f64, TuneError> {
        if self.count < 2 {
            return Err(invalid_score("sample variance needs two values"));
        }
        checked(self.m2 / (self.count - 1) as f64)
    }
}

fn validate_runs(runs: &[RunRecord], set: ScenarioSet) -> Result<(), TuneError> {
    for run in runs {
        validate_metric(MetricValues {
            loss: run.loss,
            control_effort: run.control_effort,
        })?;
        if run.scenario_set != set || run.passed_hard_gates.is_empty() {
            return Err(invalid_score(
                "a saved run has the wrong partition or gates",
            ));
        }
    }
    Ok(())
}

fn student_t_95(df: usize) -> f64 {
    const SMALL: [f64; 30] = [
        12.706, 4.303, 3.182, 2.776, 2.571, 2.447, 2.365, 2.306, 2.262, 2.228, 2.201, 2.179, 2.160,
        2.145, 2.131, 2.120, 2.110, 2.101, 2.093, 2.086, 2.080, 2.074, 2.069, 2.064, 2.060, 2.056,
        2.052, 2.048, 2.045, 2.042,
    ];
    match df {
        0 => f64::INFINITY,
        1..=30 => SMALL[df - 1],
        31..=40 => 2.042,
        41..=60 => 2.021,
        61..=80 => 2.000,
        81..=100 => 1.990,
        101..=120 => 1.984,
        _ => 1.980,
    }
}

fn checked(value: f64) -> Result<f64, TuneError> {
    if value.is_finite() {
        Ok(value)
    } else {
        Err(invalid_score(
            "score arithmetic produced a non-finite value",
        ))
    }
}

fn invalid_score(detail: impl Into<String>) -> TuneError {
    TuneError::InvalidScore {
        detail: detail.into(),
    }
}
