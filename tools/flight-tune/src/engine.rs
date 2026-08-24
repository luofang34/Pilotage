//! Isolated training, promotion, and final qualification.

mod evaluate;
mod promotion;
mod qualification;
mod reconcile;
mod session;

pub(crate) use qualification::final_outcome;

use std::path::Path;

use crate::journal::{AttemptRole, CampaignPhase};
use crate::{
    Candidate, CandidateEvaluation, Digest, FinalQualificationOutcome, GateEvaluator, Journal,
    MetricEvaluator, PromotionDecision, ProposalContext, ProposalStrategy, SearchStage,
    SessionChallenge, SimulatorBackend, SimulatorCapability, SimulatorVehicleAdapter,
    SimulatorVehicleFactory, TrainingView, TuneError, VehicleBinding,
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
    B: SimulatorBackend,
    V: SimulatorVehicleAdapter,
    G: GateEvaluator,
    M: MetricEvaluator,
    P: ProposalStrategy,
{
    /// Opens a matching campaign or creates a new campaign.
    ///
    /// The constructor binds the vehicle adapter to the validated simulator
    /// session. It also cleans and quarantines an incomplete prepared attempt.
    ///
    /// # Errors
    ///
    /// Returns [`TuneError`] when identity, binding, recovery, or storage fails.
    #[allow(clippy::too_many_arguments)]
    pub fn open_or_resume<F>(
        journal_root: impl AsRef<Path>,
        stage: SearchStage,
        fixed_seed: u64,
        initial_candidate: Candidate,
        mut backend: B,
        vehicle_factory: F,
        mut gates: G,
        mut metric: M,
        strategy: P,
    ) -> Result<Self, TuneError>
    where
        F: SimulatorVehicleFactory<Adapter = V>,
    {
        session::validate_component_identities(
            &backend,
            &vehicle_factory,
            &gates,
            &metric,
            &strategy,
        )?;
        let runtimes =
            session::runtime_identities(&backend, &vehicle_factory, &gates, &metric, &strategy);
        let vehicle_identity = vehicle_factory.vehicle_identity().clone();
        let mut journal = Journal::open_or_create(
            journal_root,
            &stage,
            fixed_seed,
            runtimes,
            &initial_candidate,
        )?;
        let session_digest = journal.session_digest()?;
        let challenge = SessionChallenge::new(session_digest);
        let receipt = backend
            .open_session_blocking(&challenge)
            .map_err(|source| TuneError::Adapter {
                adapter: backend.simulator_identity().id.clone(),
                operation: "open simulator session",
                source,
            })?;
        session::validate_simulator_receipt(&journal, receipt)?;
        let capability = SimulatorCapability::new(receipt);
        let vehicle = vehicle_factory
            .bind_blocking(&capability)
            .map_err(|source| TuneError::Adapter {
                adapter: vehicle_identity.id,
                operation: "bind simulator vehicle",
                source,
            })?;
        session::validate_vehicle_binding(&journal, &vehicle)?;
        evaluate::recover_pending_blocking(&mut journal, &mut backend, &mut gates, &mut metric)?;
        let mut tuner = Self {
            stage,
            backend,
            vehicle,
            capability,
            gates,
            metric,
            strategy,
            journal,
        };
        tuner.reconcile_settled_candidate_blocking()?;
        Ok(tuner)
    }

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
        let decision = promotion::decide(
            self.stage.promotion,
            self.journal.state().promotion_baseline.as_ref(),
            self.journal.state().promotion_frozen.as_ref(),
        )?;
        self.journal.close_promotion(decision.clone())?;
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
        if let Some(outcome) = self.journal.state().final_outcome.clone() {
            self.reconcile_settled_candidate_blocking()?;
            return Ok(outcome);
        }
        self.require_phase(CampaignPhase::PromotionClosed, "run final qualification")?;
        self.recover_pending_blocking()?;
        let candidate = self.selected_release_candidate();
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
        if self.journal.state().final_outcome != Some(FinalQualificationOutcome::Qualified) {
            return Err(invalid_state(
                "read qualified candidate",
                "the campaign does not have a qualified final result",
            ));
        }
        self.journal
            .read_candidate(self.selected_release_candidate())
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
        let Some(proposal) =
            self.strategy
                .propose(&context)
                .map_err(|error| TuneError::Proposal {
                    detail: error.to_string(),
                })?
        else {
            return Ok(false);
        };
        validate_proposal(&self.stage, &incumbent, &proposal, &history)?;
        let role = AttemptRole::TrainingChallenger {
            attempt_index: self.journal.training_attempt_count(),
        };
        self.run_new_attempt_blocking(role, &proposal.candidate)?;
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
        let digest = evaluate::candidate_digest(candidate)?;
        let plan =
            evaluate::plan_digest(&self.stage, role, digest, self.journal.session().fixed_seed)?;
        let (trial_id, stored_digest) = self.journal.prepare_attempt(role, candidate, plan)?;
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
            &mut self.backend,
            &mut self.vehicle,
            &self.capability,
            &mut self.gates,
            &mut self.metric,
        );
        self.finish_with_candidate_reconciliation_blocking("evaluate candidate", result)
    }

    fn recover_pending_blocking(&mut self) -> Result<(), TuneError> {
        let result = evaluate::recover_pending_blocking(
            &mut self.journal,
            &mut self.backend,
            &mut self.gates,
            &mut self.metric,
        );
        self.finish_with_candidate_reconciliation_blocking("recover pending attempt", result)
    }

    fn selected_release_candidate(&self) -> Digest {
        self.journal
            .state()
            .selected_release_candidate(self.journal.session().initial_candidate_digest)
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
