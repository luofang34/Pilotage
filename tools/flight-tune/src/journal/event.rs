use serde::{Deserialize, Serialize};

use crate::identity::digest_bytes;
use crate::{
    AuthenticatedEvaluationProof, CandidateEvaluation, CandidateTransitionReceipt,
    CandidateTransitionReference, Digest, MissionReference, PromotionClosure, RunBindingReceipt,
    RunExecutionContext, RunTerminalClass, RunTerminalIntent, RunTerminalPlan, RunTerminalReceipt,
    RunTerminalReport, ScenarioSet, SearchStage, TuneError,
};

#[derive(Serialize)]
struct RunPlan<'a> {
    role: AttemptRole,
    candidate: Digest,
    scenario_set: ScenarioSet,
    scenarios: &'a [MissionReference],
    repetitions: u32,
    fixed_seed: u64,
}

/// The role of one durably prepared candidate evaluation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "role", rename_all = "snake_case", deny_unknown_fields)]
pub enum AttemptRole {
    /// The initial candidate on adaptive training scenarios.
    TrainingBaseline,
    /// One adaptive training challenger.
    TrainingChallenger {
        /// The zero-based challenger index.
        attempt_index: u64,
    },
    /// The initial candidate on hidden promotion scenarios.
    PromotionBaseline,
    /// The frozen candidate on hidden promotion scenarios.
    PromotionFrozen,
    /// The selected candidate on hidden final qualification scenarios.
    FinalQualification,
}

impl AttemptRole {
    pub(crate) const fn scenario_set(self) -> ScenarioSet {
        match self {
            Self::TrainingBaseline | Self::TrainingChallenger { .. } => ScenarioSet::Training,
            Self::PromotionBaseline | Self::PromotionFrozen => ScenarioSet::Promotion,
            Self::FinalQualification => ScenarioSet::FinalQualification,
        }
    }

    pub(crate) fn plan_digest(
        self,
        stage: &SearchStage,
        candidate: Digest,
        fixed_seed: u64,
    ) -> Result<Digest, TuneError> {
        let scenario_set = self.scenario_set();
        let scenarios = match scenario_set {
            ScenarioSet::Training => &stage.training_scenarios,
            ScenarioSet::Promotion => &stage.promotion_scenarios,
            ScenarioSet::FinalQualification => &stage.final_qualification_scenarios,
        };
        let plan = RunPlan {
            role: self,
            candidate,
            scenario_set,
            scenarios,
            repetitions: stage.repetitions,
            fixed_seed,
        };
        let bytes = serde_json::to_vec(&plan).map_err(|source| TuneError::Encode {
            document: "run plan",
            source,
        })?;
        Ok(digest_bytes(&bytes))
    }
}

/// The saved status of one lifecycle cleanup operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum OperationStatus {
    /// The operation did not apply to this lifecycle state.
    NotRequired,
    /// The operation completed.
    Succeeded,
    /// The operation failed and saved its diagnostic.
    Failed {
        /// The stable failure detail.
        detail: String,
    },
}

impl OperationStatus {
    pub(crate) const fn succeeded(&self) -> bool {
        matches!(self, Self::Succeeded)
    }
}

/// The result of the one hidden promotion comparison.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "decision", rename_all = "snake_case", deny_unknown_fields)]
pub enum PromotionDecision {
    /// The frozen candidate became the release candidate.
    Promoted {},
    /// A hidden run failed a hard gate.
    RejectedHardGate {
        /// The first failed hard gate identity.
        gate_id: String,
    },
    /// The paired result did not meet promotion limits.
    RejectedNoImprovement {},
    /// Recovery or an execution failure made the decision incomplete.
    Indeterminate {
        /// The stable reason.
        reason: String,
    },
}

impl PromotionDecision {
    /// Reports whether the frozen candidate passed promotion.
    #[must_use]
    pub const fn is_promoted(&self) -> bool {
        matches!(self, Self::Promoted { .. })
    }
}

/// The final sealed release result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case", deny_unknown_fields)]
pub enum FinalQualificationOutcome {
    /// The selected candidate passed final qualification.
    Qualified,
    /// The selected candidate failed a hard gate.
    FailedHardGate {
        /// The first failed hard gate identity.
        gate_id: String,
    },
    /// The selected candidate exceeded a declared final objective limit.
    FailedObjective {
        /// Stable name of the first failed objective.
        metric: String,
    },
    /// Recovery or an execution failure made qualification incomplete.
    Indeterminate {
        /// The stable reason.
        reason: String,
    },
}

/// The externally visible campaign phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CampaignPhase {
    /// Adaptive training is open.
    Searching,
    /// One candidate is frozen and training is closed.
    Frozen,
    /// The one promotion decision is closed.
    PromotionClosed,
    /// Final qualification is complete and the journal is sealed.
    Sealed,
}

/// One immutable tuning journal event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case", deny_unknown_fields)]
pub enum JournalEvent {
    /// The campaign started with an immutable release candidate.
    Started {
        /// The initial candidate digest.
        candidate: Digest,
    },
    /// One vehicle validator authorized an exact training transition.
    CandidateTransitionAuthorized {
        /// The zero-based challenger index.
        attempt_index: u64,
        /// The stable proposal reason.
        reason: String,
        /// The immutable target candidate digest.
        candidate: Digest,
        /// The complete vehicle authorization receipt.
        receipt: CandidateTransitionReceipt,
    },
    /// One complete evaluation was durable before simulator mutation.
    AttemptPrepared {
        /// The monotonic trial identity.
        trial_id: u64,
        /// The evaluation role.
        role: AttemptRole,
        /// The immutable candidate digest.
        candidate: Digest,
        /// The digest of the complete ordered run plan.
        plan_digest: Digest,
        /// The exact training transition authorization, if this is a challenger.
        transition: Option<CandidateTransitionReference>,
    },
    /// One exact run identity became durable before external mutation.
    RunPrepared {
        /// The campaign trial identity.
        trial_id: u64,
        /// The zero-based run index in the attempt plan.
        run_index: u64,
        /// The complete simulator-neutral run identity.
        context: RunExecutionContext,
        /// The canonical digest of the run identity.
        run_intent_digest: Digest,
    },
    /// One exact terminal plan and adapter binding became durable.
    RunBound {
        /// The campaign trial identity.
        trial_id: u64,
        /// The zero-based run index in the attempt plan.
        run_index: u64,
        /// The immutable terminal operation plan.
        terminal_plan: RunTerminalPlan,
        /// The exact run and adapter binding.
        binding: RunBindingReceipt,
    },
    /// One semantic run result became durable before terminal operations.
    RunTerminalIntentPrepared {
        /// The campaign trial identity.
        trial_id: u64,
        /// The zero-based run index in the attempt plan.
        run_index: u64,
        /// The exact semantic terminal intent.
        intent: RunTerminalIntent,
    },
    /// One complete terminal operation report became durable.
    RunTerminalReportRecorded {
        /// The campaign trial identity.
        trial_id: u64,
        /// The zero-based run index in the attempt plan.
        run_index: u64,
        /// The complete ordered terminal report.
        report: Box<RunTerminalReport>,
        /// The class that the core calculated from the report.
        base_class: RunTerminalClass,
        /// The exact receipt that evidence publication must produce.
        expected_receipt: Box<RunTerminalReceipt>,
    },
    /// A definite completed-receipt absence became durable.
    RunTerminalEvidenceFailureRecorded {
        /// The campaign trial identity.
        trial_id: u64,
        /// The zero-based run index in the attempt plan.
        run_index: u64,
        /// The evidence-failure class for the saved report.
        class: RunTerminalClass,
    },
    /// One exact terminal receipt closed the run.
    RunCommitted {
        /// The campaign trial identity.
        trial_id: u64,
        /// The zero-based run index in the attempt plan.
        run_index: u64,
        /// The exact completed or quarantine receipt.
        receipt: Box<RunTerminalReceipt>,
    },
    /// One evaluation produced a score or hard gate result.
    AttemptCompleted {
        /// The monotonic trial identity.
        trial_id: u64,
        /// The complete partition result.
        evaluation: CandidateEvaluation,
        /// The authenticated hidden evaluation proof, when this role requires one.
        proof: Option<Box<AuthenticatedEvaluationProof>>,
        /// The training incumbent decision, if this is a training role.
        selected_as_training_incumbent: Option<bool>,
    },
    /// An incomplete or failed attempt cannot run again in this session.
    AttemptQuarantined {
        /// The monotonic trial identity.
        trial_id: u64,
        /// The stable quarantine reason.
        reason: String,
        /// The authenticated hidden evaluation proof, when this role requires one.
        proof: Option<Box<AuthenticatedEvaluationProof>>,
    },
    /// The runner saved one independent cleanup result.
    CleanupRecorded {
        /// The monotonic trial identity.
        trial_id: u64,
        /// The cleanup operation result.
        cleanup: OperationStatus,
    },
    /// Adaptive training closed with one immutable candidate.
    Frozen {
        /// The initial release candidate.
        baseline: Digest,
        /// The only candidate permitted in hidden promotion.
        candidate: Digest,
    },
    /// The one hidden promotion decision closed.
    PromotionClosed {
        /// The replay-computed promotion closure.
        closure: PromotionClosure,
    },
    /// Final qualification completed and sealed the journal.
    Sealed {
        /// The candidate that received final qualification.
        candidate: Digest,
        /// The final release result.
        outcome: FinalQualificationOutcome,
        /// The exact promotion closure identity.
        promotion_closure_digest: Digest,
        /// The final evaluation identity.
        final_evaluation_digest: Digest,
        /// The final authenticated proof identity.
        final_proof_digest: Digest,
    },
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{JournalEvent, OperationStatus};

    #[test]
    fn cleanup_event_has_no_terminal_stop_field() -> Result<(), serde_json::Error> {
        let event = JournalEvent::CleanupRecorded {
            trial_id: 7,
            cleanup: OperationStatus::Succeeded,
        };
        let document = serde_json::to_value(event)?;

        assert_eq!(document.get("event"), Some(&json!("cleanup_recorded")));
        assert!(document.get("stop").is_none());
        Ok(())
    }

    #[test]
    fn cleanup_event_rejects_the_schema_three_stop_field() {
        let document = json!({
            "event": "cleanup_recorded",
            "trial_id": 7,
            "stop": { "status": "succeeded" },
            "cleanup": { "status": "succeeded" }
        });

        assert!(serde_json::from_value::<JournalEvent>(document).is_err());
    }

    #[test]
    fn legacy_caller_supplied_promotion_decision_is_rejected() {
        let document = json!({
            "event": "promotion_closed",
            "decision": { "decision": "promoted" }
        });

        assert!(serde_json::from_value::<JournalEvent>(document).is_err());
    }
}
