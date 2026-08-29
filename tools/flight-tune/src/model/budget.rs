use serde::{Deserialize, Serialize};

use crate::{MissionReference, SearchStage, TuneError};

#[cfg(test)]
#[path = "budget/tests.rs"]
mod tests;

/// The largest run count and wall-clock time one prepared campaign can take.
///
/// The bound counts every challenger run and every suite-baseline run that a
/// challenger can require. A campaign whose bound does not fit the declared
/// budget must change its stage before it starts, not during execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CampaignRunBound {
    /// The largest simulator run count for the complete campaign.
    pub maximum_runs: u64,
    /// The largest wall-clock time for the complete campaign.
    pub maximum_duration_ns: u64,
}

impl SearchStage {
    /// Returns the run and duration bound for one bounded campaign.
    ///
    /// Each challenger can need one fresh incumbent baseline on its suite, so
    /// the bound counts two suite attempts for each challenger. The hidden
    /// partitions add one promotion pair and one final attempt. Every attempt
    /// can use its complete replacement allowance.
    ///
    /// # Errors
    ///
    /// Returns [`TuneError`] when the stage is not valid or the bound
    /// overflows.
    pub fn run_bound(&self, attempt_limit: u64) -> Result<CampaignRunBound, TuneError> {
        self.validate()?;
        let (suite_runs, suite_duration) = self.widest_training_attempt()?;
        let (promotion_runs, promotion_duration) =
            partition_attempt(&self.promotion_scenarios, self.repetitions)?;
        let (final_runs, final_duration) =
            partition_attempt(&self.final_qualification_scenarios, self.repetitions)?;
        let training_attempts = attempt_limit
            .checked_mul(2)
            .and_then(|paired| paired.checked_add(1))
            .ok_or_else(|| overflow("the training attempt count"))?;
        let executions = u64::from(self.execution_retry.execution_retry_limit)
            .checked_add(1)
            .ok_or_else(|| overflow("the execution allowance"))?;
        let runs = total(
            scale(training_attempts, suite_runs)?,
            scale(2, promotion_runs)?,
            final_runs,
        )?;
        let duration = total(
            scale(training_attempts, suite_duration)?,
            scale(2, promotion_duration)?,
            final_duration,
        )?;
        Ok(CampaignRunBound {
            maximum_runs: scale(executions, runs)?,
            maximum_duration_ns: scale(executions, duration)?,
        })
    }

    fn widest_training_attempt(&self) -> Result<(u64, u64), TuneError> {
        let mut runs = 0;
        let mut duration = 0;
        for suite in &self.training_suites {
            let count = u64::try_from(suite.run_count())
                .map_err(|_| overflow("a training suite run count"))?;
            runs = runs.max(count);
            duration = duration.max(suite.run_duration_ns());
        }
        Ok((runs, duration))
    }
}

fn partition_attempt(
    scenarios: &[MissionReference],
    repetitions: u32,
) -> Result<(u64, u64), TuneError> {
    let count = u64::try_from(scenarios.len())
        .map_err(|_| overflow("a partition mission count"))?
        .checked_mul(u64::from(repetitions))
        .ok_or_else(|| overflow("a partition run count"))?;
    let longest = scenarios
        .iter()
        .map(MissionReference::run_duration_ns)
        .max()
        .unwrap_or(1);
    Ok((count, count.saturating_mul(longest)))
}

fn scale(factor: u64, value: u64) -> Result<u64, TuneError> {
    factor
        .checked_mul(value)
        .ok_or_else(|| overflow("a campaign bound"))
}

fn total(training: u64, promotion: u64, qualification: u64) -> Result<u64, TuneError> {
    training
        .checked_add(promotion)
        .and_then(|partial| partial.checked_add(qualification))
        .ok_or_else(|| overflow("a campaign bound"))
}

fn overflow(detail: &str) -> TuneError {
    TuneError::InvalidStage {
        detail: format!("{detail} exceeds the supported range"),
    }
}
