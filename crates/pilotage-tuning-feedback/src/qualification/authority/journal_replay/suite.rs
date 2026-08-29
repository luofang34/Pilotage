//! The suite link the independent replay derives instead of reading.

use flight_tune::{AttemptRole, CandidateEvaluation, Digest, RunRecord, SearchGroupBinding};

use crate::{FeedbackError, error::invalid};

use super::super::super::training_suite;
use super::{PendingOutcome, Phase, ReplayState, authorized_final_candidate};

/// One comparable incumbent evaluation on one exact training suite.
pub(super) struct SuiteBaseline {
    pub(super) candidate: Digest,
    pub(super) suite_index: u16,
    pub(super) runs: Option<Vec<RunRecord>>,
}

impl ReplayState<'_> {
    pub(super) fn role_allowed(
        &self,
        role: AttemptRole,
        candidate: Digest,
        transition: bool,
    ) -> bool {
        match role {
            AttemptRole::TrainingBaseline { suite_index } => {
                self.phase == Phase::Searching
                    && candidate == self.training_incumbent
                    && self.suite_baseline(candidate, suite_index).is_none()
                    && !transition
            }
            AttemptRole::TrainingChallenger {
                attempt_index,
                suite_index,
            } => {
                self.phase == Phase::Searching
                    && self
                        .suite_baseline(self.training_incumbent, suite_index)
                        .is_some_and(Option::is_some)
                    && attempt_index == self.training_attempt_count
                    && transition
            }
            AttemptRole::PromotionBaseline => {
                self.phase == Phase::Frozen
                    && self.promotion_baseline_passed.is_none()
                    && candidate == self.session.initial_candidate_digest
                    && !transition
            }
            AttemptRole::PromotionFrozen => {
                self.phase == Phase::Frozen
                    && self.promotion_baseline_passed == Some(true)
                    && !self.promotion_frozen_done
                    && self.frozen_candidate == Some(candidate)
                    && !transition
            }
            AttemptRole::FinalQualification => {
                self.phase == Phase::PromotionClosed
                    && !self.final_done
                    && self
                        .promotion_closure
                        .as_ref()
                        .and_then(authorized_final_candidate)
                        == Some(candidate)
                    && !transition
            }
        }
    }

    /// Returns the runs of the exact incumbent baseline for one suite.
    ///
    /// The outer option states whether a baseline exists at all. The inner
    /// option states whether that baseline passed every hard gate.
    pub(super) fn suite_baseline(
        &self,
        candidate: Digest,
        suite_index: u16,
    ) -> Option<&Option<Vec<RunRecord>>> {
        self.suite_baselines
            .iter()
            .find(|baseline| baseline.candidate == candidate && baseline.suite_index == suite_index)
            .map(|baseline| &baseline.runs)
    }

    pub(super) fn record_suite_baseline(
        &mut self,
        candidate: Digest,
        suite_index: u16,
        outcome: &PendingOutcome,
    ) {
        self.suite_baselines
            .retain(|held| held.candidate != candidate || held.suite_index != suite_index);
        self.suite_baselines.push(SuiteBaseline {
            candidate,
            suite_index,
            runs: outcome.runs.clone(),
        });
    }

    /// Derives the search group of one recorded transition from its candidates.
    ///
    /// A campaign states its group in the chain. The statement is checked
    /// against the difference between the two exact candidates, so a
    /// controller change cannot take an operator-feel suite.
    pub(super) fn verify_derived_group(
        &self,
        candidate: Digest,
        source: Digest,
        group: &SearchGroupBinding,
    ) -> Result<(), FeedbackError> {
        let incumbent = self.candidates.get(source)?;
        let challenger = self.candidates.get(candidate)?;
        if source != self.training_incumbent {
            return Err(invalid(
                "a recorded transition does not start from the training incumbent",
            ));
        }
        if &training_suite::derived_group(self.stage, incumbent, challenger)? != group {
            return Err(invalid(
                "a recorded transition suite does not match its candidate difference",
            ));
        }
        Ok(())
    }
}

pub(super) fn passing_runs(evaluation: &CandidateEvaluation) -> Option<Vec<RunRecord>> {
    match evaluation {
        CandidateEvaluation::Passed { runs, .. } => Some(runs.clone()),
        CandidateEvaluation::HardGateFailed { .. } | CandidateEvaluation::Quarantined { .. } => {
            None
        }
    }
}
