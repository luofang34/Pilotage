use crate::{AttemptRole, MissionReference, ScenarioSet, SearchStage, TuneError};

use super::TrainingSuiteAnchor;

/// The complete ordered run plan for one attempt.
///
/// A training attempt reads its missions and its repetition count from the
/// frozen suite that the role names. Every other attempt reads the complete
/// hidden partition, because a suite never narrows promotion or final
/// qualification.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct AttemptRunPlan {
    pub(crate) set: ScenarioSet,
    pub(crate) scenarios: Vec<MissionReference>,
    pub(crate) repetitions: u32,
    pub(crate) primary_run_count: usize,
    pub(crate) suite: Option<TrainingSuiteAnchor>,
}

impl AttemptRunPlan {
    /// Derives the run plan that one role takes from one frozen stage.
    ///
    /// # Errors
    ///
    /// Returns [`TuneError`] when the role names a suite the stage does not
    /// declare.
    pub(crate) fn new(stage: &SearchStage, role: AttemptRole) -> Result<Self, TuneError> {
        let set = role.scenario_set();
        let Some(index) = role.training_suite_index() else {
            let scenarios = match set {
                ScenarioSet::Training => stage.training_scenarios.clone(),
                ScenarioSet::Promotion => stage.promotion_scenarios.clone(),
                ScenarioSet::FinalQualification => stage.final_qualification_scenarios.clone(),
            };
            let primary_run_count = scenarios.len().saturating_mul(stage.repetitions as usize);
            return Ok(Self {
                set,
                scenarios,
                repetitions: stage.repetitions,
                primary_run_count,
                suite: None,
            });
        };
        let suite = stage.training_suite(index)?;
        Ok(Self {
            set,
            scenarios: suite.ordered_scenarios(),
            repetitions: suite.repetitions,
            primary_run_count: suite.primary_run_count(),
            suite: Some(suite.anchor(index)?),
        })
    }

    /// Returns how many runs this plan states.
    pub(crate) fn run_count(&self) -> usize {
        self.scenarios
            .len()
            .saturating_mul(self.repetitions as usize)
    }

    /// Returns the mission and repetition for one zero-based run index.
    ///
    /// # Errors
    ///
    /// Returns [`TuneError`] when the index is outside this plan.
    pub(crate) fn run_at(&self, index: usize) -> Result<(&MissionReference, u32), TuneError> {
        let repetitions = self.repetitions as usize;
        let scenario = self
            .scenarios
            .get(index / repetitions.max(1))
            .ok_or_else(|| super::invalid_stage("a run index exceeds the prepared run plan"))?;
        let repetition = u32::try_from(index % repetitions.max(1))
            .map_err(|_| super::invalid_stage("a repetition exceeds u32"))?;
        Ok((scenario, repetition))
    }
}
