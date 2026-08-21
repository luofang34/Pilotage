use serde::{Deserialize, Serialize};

use crate::identity::digest_bytes;
use crate::{CandidateEvaluation, Digest, ScenarioRef, ScenarioSet, SearchStage, TuneError};

#[derive(Serialize)]
struct RunPlan<'a> {
    role: AttemptRole,
    candidate: Digest,
    scenario_set: ScenarioSet,
    scenarios: &'a [ScenarioRef],
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
    Promoted {
        /// The paired mean challenger-minus-baseline loss.
        mean_loss_delta: f64,
        /// The upper 95 percent confidence limit for paired loss.
        loss_delta_upper_95: f64,
        /// The paired mean challenger-minus-baseline effort.
        mean_effort_delta: f64,
    },
    /// A hidden run failed a hard gate.
    RejectedHardGate {
        /// The first failed hard gate identity.
        gate_id: String,
    },
    /// The paired result did not meet promotion limits.
    RejectedNoImprovement {
        /// The upper 95 percent confidence limit for paired loss.
        loss_delta_upper_95: f64,
        /// The paired mean challenger-minus-baseline effort.
        mean_effort_delta: f64,
    },
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
    },
    /// One evaluation produced a score or hard gate result.
    AttemptCompleted {
        /// The monotonic trial identity.
        trial_id: u64,
        /// The complete partition result.
        evaluation: CandidateEvaluation,
        /// The training incumbent decision, if this is a training role.
        selected_as_training_incumbent: Option<bool>,
    },
    /// An incomplete or failed attempt cannot run again in this session.
    AttemptQuarantined {
        /// The monotonic trial identity.
        trial_id: u64,
        /// The stable quarantine reason.
        reason: String,
    },
    /// The runner saved stop and cleanup results.
    CleanupRecorded {
        /// The monotonic trial identity.
        trial_id: u64,
        /// The stop operation result.
        stop: OperationStatus,
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
        /// The complete promotion result.
        decision: PromotionDecision,
    },
    /// Final qualification completed and sealed the journal.
    Sealed {
        /// The candidate that received final qualification.
        candidate: Digest,
        /// The final release result.
        outcome: FinalQualificationOutcome,
    },
}
