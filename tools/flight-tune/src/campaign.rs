//! Isolated training, promotion, and final qualification.

mod evaluate;
mod open;
mod promotion;
mod qualification;
mod reconcile;
mod session;
mod transition;

#[cfg(test)]
mod tests;

#[cfg(test)]
pub(crate) use open::validate_open_components;
pub(crate) use promotion::calculate as calculate_promotion;
pub(crate) use qualification::final_outcome;

use crate::journal::{AttemptRole, CampaignPhase};
use crate::{
    CampaignBackend, Candidate, CandidateEvaluation, CandidateTransitionReference, Digest,
    FinalQualificationOutcome, GateEvaluator, Journal, MetricEvaluator, PromotionDecision,
    ProposalContext, ProposalStrategy, RunTerminalAdapter, SearchStage, SimulatorCapability,
    SimulatorVehicleAdapter, TrainingView, TuneError, VehicleBinding,
};

/// Why one bounded training search call stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReason {
    /// The call evaluated its requested challenger count.
    AttemptLimit,
    /// The proposal strategy reported that search was complete.
    StrategyExhausted,
}

/// The result of one bounded adaptive training call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TuningSummary {
    /// The training challenger count evaluated during this call.
    pub evaluated_challengers: u64,
    /// The current training incumbent digest.
    pub training_incumbent_digest: Digest,
    /// Why this call stopped.
    pub stop_reason: StopReason,
}

/// A deterministic simulator-only tuning campaign.
pub struct Tuner<B, V, G, M, P> {
    stage: SearchStage,
    backend: B,
    vehicle: VehicleBinding<V>,
    capability: SimulatorCapability,
    gates: G,
    metric: M,
    strategy: P,
    journal: Journal,
}

impl<B, V, G, M, P> Tuner<B, V, G, M, P>
where
    B: CampaignBackend,
    V: SimulatorVehicleAdapter + RunTerminalAdapter,
    G: GateEvaluator,
    M: MetricEvaluator,
    P: ProposalStrategy,
{
    /// Evaluates at most `attempt_limit` adaptive training challengers.
    ///
    /// This method does not run promotion or final qualification scenarios.
    ///
    /// # Errors
    ///
    /// Returns [`TuneError`] when the campaign is closed or one operation fails.
    pub fn run_training_attempts_blocking(
        &mut self,
        attempt_limit: u64,
    ) -> Result<TuningSummary, TuneError> {
        self.journal.ensure_usable()?;
        self.require_phase(CampaignPhase::Searching, "run training")?;
        self.recover_pending_blocking()?;
        self.ensure_training_baseline_blocking()?;
        let starting_attempt = self.journal.training_attempt_count();
        for _ in 0..attempt_limit {
            if !self.run_one_training_challenger_blocking()? {
                return self.training_summary(starting_attempt, StopReason::StrategyExhausted);
            }
        }
        self.training_summary(starting_attempt, StopReason::AttemptLimit)
    }

    /// Freezes the only candidate that hidden promotion can inspect.
    ///
    /// # Errors
    ///
    /// Returns [`TuneError`] when training is incomplete or already closed.
    pub fn freeze_candidate(&mut self) -> Result<Digest, TuneError> {
        self.journal.ensure_usable()?;
        self.require_phase(CampaignPhase::Searching, "freeze training candidate")?;
        self.recover_pending_blocking()?;
        self.ensure_safe_baseline()?;
        let candidate = self.journal.freeze()?;
        self.reconcile_settled_candidate_blocking()?;
        Ok(candidate)
    }

    /// Runs the one hidden paired promotion comparison.
    ///
    /// A repeated call returns the saved decision and does not run a scenario.
    ///
    /// # Errors
    ///
    /// Returns [`TuneError`] when training is not frozen or execution fails.
    pub fn run_promotion_once_blocking(&mut self) -> Result<PromotionDecision, TuneError> {
        self.journal.ensure_usable()?;
        if let Some(decision) = self.journal.state().promotion_decision.clone() {
            self.reconcile_settled_candidate_blocking()?;
            return Ok(decision);
        }
        self.require_phase(CampaignPhase::Frozen, "run promotion")?;
        self.recover_pending_blocking()?;
        if self.journal.state().promotion_baseline.is_none() {
            let digest = self.journal.session().initial_candidate_digest;
            self.run_partition_attempt_blocking(AttemptRole::PromotionBaseline, digest)?;
        }
        if should_run_frozen(self.journal.state().promotion_baseline.as_ref())
            && self.journal.state().promotion_frozen.is_none()
        {
            let digest = self
                .journal
                .state()
                .frozen_candidate
                .ok_or_else(|| invalid_state("run promotion", "no frozen candidate"))?;
            self.run_partition_attempt_blocking(AttemptRole::PromotionFrozen, digest)?;
        }
        let decision = self.journal.close_promotion()?;
        self.reconcile_settled_candidate_blocking()?;
        Ok(decision)
    }

    /// Runs final qualification once and seals the campaign journal.
    ///
    /// A repeated call returns the saved result and does not run a scenario.
    ///
    /// # Errors
    ///
    /// Returns [`TuneError`] when promotion is open or execution fails.
    pub fn run_final_qualification_once_blocking(
        &mut self,
    ) -> Result<FinalQualificationOutcome, TuneError> {
        self.journal.ensure_usable()?;
        if let Some(outcome) = self.journal.state().final_outcome.clone() {
            self.reconcile_settled_candidate_blocking()?;
            return Ok(outcome);
        }
        self.require_phase(CampaignPhase::PromotionClosed, "run final qualification")?;
        let candidate = self.authorized_final_candidate()?;
        self.recover_pending_blocking()?;
        if self.journal.state().final_evaluation.is_none() {
            self.run_partition_attempt_blocking(AttemptRole::FinalQualification, candidate)?;
        }
        let outcome = final_outcome(&self.stage, self.journal.state().final_evaluation.as_ref());
        self.journal.seal(candidate, outcome.clone())?;
        self.reconcile_settled_candidate_blocking()?;
        Ok(outcome)
    }

    /// Returns the qualified release candidate.
    ///
    /// # Errors
    ///
    /// Returns [`TuneError`] when final qualification did not pass.
    pub fn qualified_candidate(&self) -> Result<Candidate, TuneError> {
        self.journal.ensure_usable()?;
        if self.journal.state().final_outcome != Some(FinalQualificationOutcome::Qualified) {
            return Err(invalid_state(
                "read qualified candidate",
                "the campaign does not have a qualified final result",
            ));
        }
        self.journal
            .read_candidate(self.authorized_final_candidate()?)
    }

    /// Returns the audit journal.
    #[must_use]
    pub const fn journal(&self) -> &Journal {
        &self.journal
    }

    fn ensure_training_baseline_blocking(&mut self) -> Result<(), TuneError> {
        if self.journal.state().training_baseline.is_none() {
            let candidate = self
                .journal
                .read_candidate(self.journal.session().initial_candidate_digest)?;
            self.run_new_attempt_blocking(AttemptRole::TrainingBaseline, &candidate)?;
        }
        self.ensure_safe_baseline()
    }

    fn ensure_safe_baseline(&self) -> Result<(), TuneError> {
        match self.journal.state().training_baseline.as_ref() {
            Some(CandidateEvaluation::Passed { .. }) => Ok(()),
            Some(CandidateEvaluation::HardGateFailed { failure, .. }) => {
                Err(TuneError::UnsafeBaseline {
                    detail: format!("hard gate {} failed", failure.gate.id),
                })
            }
            Some(CandidateEvaluation::Quarantined { reason }) => Err(TuneError::UnsafeBaseline {
                detail: reason.clone(),
            }),
            None => Err(TuneError::UnsafeBaseline {
                detail: "training baseline did not complete".to_owned(),
            }),
        }
    }

    fn run_one_training_challenger_blocking(&mut self) -> Result<bool, TuneError> {
        if let Some(authorized) = self.journal.authorized_training_transition().cloned() {
            let candidate = self.journal.read_candidate(authorized.candidate)?;
            let role = AttemptRole::TrainingChallenger {
                attempt_index: authorized.attempt_index,
            };
            self.run_attempt_blocking(role, &candidate, Some(authorized.reference))?;
            return Ok(true);
        }
        let incumbent = self.journal.training_incumbent()?;
        let history = self.journal.training_history();
        let context = ProposalContext {
            training: TrainingView {
                fixed_seed: self.journal.session().fixed_seed,
                attempt_index: self.journal.training_attempt_count(),
                stage_id: &self.stage.id,
                allowlist: &self.stage.allowlist,
                scenarios: &self.stage.training_scenarios,
                repetitions: self.stage.repetitions,
                incumbent: &incumbent,
                history: &history,
            },
        };
        self.journal.ensure_usable()?;
        let proposal = match self.strategy.propose(&context) {
            Ok(proposal) => {
                self.journal.ensure_usable()?;
                proposal
            }
            Err(error) => {
                self.journal.ensure_usable().ok();
                return Err(TuneError::Proposal {
                    detail: error.to_string(),
                });
            }
        };
        let Some(proposal) = proposal else {
            return Ok(false);
        };
        validate_proposal(&self.stage, &incumbent, &proposal, &history)?;
        let attempt_index = self.journal.training_attempt_count();
        let role = AttemptRole::TrainingChallenger { attempt_index };
        let transition = transition::authorize_new(
            &mut self.journal,
            &self.stage,
            &self.vehicle,
            &incumbent,
            &proposal.candidate,
            attempt_index,
            &proposal.reason,
        )?;
        self.run_attempt_blocking(role, &proposal.candidate, Some(transition))?;
        Ok(true)
    }

    fn run_partition_attempt_blocking(
        &mut self,
        role: AttemptRole,
        digest: Digest,
    ) -> Result<(), TuneError> {
        let candidate = self.journal.read_candidate(digest)?;
        self.run_new_attempt_blocking(role, &candidate)
    }

    fn run_new_attempt_blocking(
        &mut self,
        role: AttemptRole,
        candidate: &Candidate,
    ) -> Result<(), TuneError> {
        self.run_attempt_blocking(role, candidate, None)
    }

    /// Runs one attempt and every replacement the declared limit authorizes.
    ///
    /// A quarantine that received a replacement is not a campaign failure.
    /// The declared limit already answered it, and the execution the campaign
    /// records for that condition is the replacement.
    fn run_attempt_blocking(
        &mut self,
        role: AttemptRole,
        candidate: &Candidate,
        transition: Option<CandidateTransitionReference>,
    ) -> Result<(), TuneError> {
        let result = self.run_one_attempt_blocking(role, candidate, transition);
        self.settle_authorized_retries_blocking(result)
    }

    fn settle_authorized_retries_blocking(
        &mut self,
        first: Result<(), TuneError>,
    ) -> Result<(), TuneError> {
        let mut result = first;
        let mut remaining = self.stage.execution_retry.execution_retry_limit;
        while let Some(retry) = self.journal.authorized_retry() {
            if remaining == 0 {
                return Err(invalid_state(
                    "run an authorized replacement",
                    "the execution retry limit was exceeded",
                ));
            }
            remaining = remaining.wrapping_sub(1);
            let candidate = self.journal.read_candidate(retry.candidate)?;
            result = self.run_one_attempt_blocking(retry.role, &candidate, retry.transition);
        }
        result
    }

    fn run_one_attempt_blocking(
        &mut self,
        role: AttemptRole,
        candidate: &Candidate,
        transition: Option<CandidateTransitionReference>,
    ) -> Result<(), TuneError> {
        let digest = evaluate::candidate_digest(candidate)?;
        let plan =
            evaluate::plan_digest(&self.stage, role, digest, self.journal.session().fixed_seed)?;
        let (trial_id, stored_digest) = self
            .journal
            .prepare_attempt(role, candidate, plan, transition)?;
        if stored_digest != digest {
            return Err(TuneError::DigestMismatch { expected: digest });
        }
        let result = evaluate::run_prepared_blocking(
            &mut self.journal,
            &self.stage,
            trial_id,
            role,
            candidate,
            digest,
            transition,
            &mut self.backend,
            &mut self.vehicle,
            &self.capability,
            &mut self.gates,
            &mut self.metric,
        );
        self.finish_with_candidate_reconciliation_blocking("evaluate candidate", result)
    }

    fn recover_pending_blocking(&mut self) -> Result<(), TuneError> {
        self.journal.ensure_usable()?;
        if self.journal.state().pending.is_none() && self.journal.authorized_retry().is_none() {
            return Ok(());
        }
        let result = if self.journal.state().pending.is_some() {
            let recovered = evaluate::recover_pending_blocking(
                &mut self.journal,
                &self.stage,
                &mut self.backend,
                &mut self.vehicle,
                &self.capability,
                &mut self.gates,
                &mut self.metric,
            );
            self.finish_with_candidate_reconciliation_blocking("recover pending attempt", recovered)
        } else {
            Ok(())
        };
        self.settle_authorized_retries_blocking(result)
    }

    fn authorized_final_candidate(&self) -> Result<Digest, TuneError> {
        self.journal
            .state()
            .authorized_final_candidate(self.journal.session().initial_candidate_digest)
    }

    fn require_phase(
        &self,
        expected: CampaignPhase,
        operation: &'static str,
    ) -> Result<(), TuneError> {
        if self.journal.phase() == expected {
            Ok(())
        } else {
            Err(invalid_state(
                operation,
                "the campaign phase does not match",
            ))
        }
    }

    fn training_summary(
        &self,
        starting_attempt: u64,
        stop_reason: StopReason,
    ) -> Result<TuningSummary, TuneError> {
        Ok(TuningSummary {
            evaluated_challengers: self
                .journal
                .training_attempt_count()
                .wrapping_sub(starting_attempt),
            training_incumbent_digest: evaluate::candidate_digest(
                &self.journal.training_incumbent()?,
            )?,
            stop_reason,
        })
    }
}

fn validate_proposal(
    stage: &SearchStage,
    incumbent: &Candidate,
    proposal: &crate::Proposal,
    history: &[crate::TrainingObservation],
) -> Result<(), TuneError> {
    if proposal.reason.trim().is_empty() || proposal.reason.len() > 4_096 {
        return Err(TuneError::InvalidCandidate {
            detail: "a proposal reason is empty or too long".to_owned(),
        });
    }
    if proposal.candidate == *incumbent {
        return Err(TuneError::InvalidCandidate {
            detail: "a challenger must differ from the training incumbent".to_owned(),
        });
    }
    stage.validate_challenger(incumbent, &proposal.candidate)?;
    let digest = evaluate::candidate_digest(&proposal.candidate)?;
    if history.iter().any(|prior| prior.candidate_digest == digest) {
        return Err(TuneError::InvalidCandidate {
            detail: "a strategy cannot repeat a prior training candidate".to_owned(),
        });
    }
    Ok(())
}

fn should_run_frozen(baseline: Option<&CandidateEvaluation>) -> bool {
    matches!(baseline, Some(CandidateEvaluation::Passed { .. }))
}

fn invalid_state(operation: &'static str, detail: impl Into<String>) -> TuneError {
    TuneError::InvalidState {
        operation,
        detail: detail.into(),
    }
}
