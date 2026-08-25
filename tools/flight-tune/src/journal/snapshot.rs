use crate::{
    CandidateTransitionReference, Digest, RunBindingReceipt, RunExecutionContext, RunTerminalClass,
    RunTerminalIntent, RunTerminalPlan, RunTerminalReceipt, RunTerminalReport,
};

use super::AttemptRole;
use super::replay::{PendingAttempt, PendingOutcome, PreparedRun, PreparedRunTerminalState};

#[derive(Debug, Clone)]
pub(crate) struct PendingAttemptSnapshot {
    pub(crate) trial_id: u64,
    pub(crate) role: AttemptRole,
    pub(crate) candidate: Digest,
    pub(crate) transition: Option<CandidateTransitionReference>,
    pub(crate) prepared_runs: Vec<PreparedRunSnapshot>,
    pub(crate) outcome: Option<AttemptOutcomeSnapshot>,
}

impl PendingAttemptSnapshot {
    pub(crate) fn current_run(&self) -> Option<&PreparedRunSnapshot> {
        self.prepared_runs.last()
    }

    pub(crate) fn committed_prefix(&self) -> Vec<&RunTerminalReceipt> {
        self.prepared_runs
            .iter()
            .map(|run| &run.terminal)
            .map_while(RunTerminalSnapshot::committed_receipt)
            .collect()
    }
}

#[derive(Debug, Clone)]
pub(crate) struct AttemptOutcomeSnapshot;

#[derive(Debug, Clone)]
pub(crate) struct PreparedRunSnapshot {
    pub(crate) run_index: u64,
    pub(crate) context: RunExecutionContext,
    pub(crate) run_intent_digest: Digest,
    pub(crate) terminal: RunTerminalSnapshot,
}

#[derive(Debug, Clone)]
pub(crate) enum RunTerminalSnapshot {
    Prepared,
    Bound {
        plan: RunTerminalPlan,
        binding: RunBindingReceipt,
    },
    IntentPrepared {
        plan: RunTerminalPlan,
        binding: RunBindingReceipt,
        intent: RunTerminalIntent,
    },
    ReportRecorded {
        binding: RunBindingReceipt,
        report: RunTerminalReport,
        base_class: RunTerminalClass,
        expected_receipt: Box<RunTerminalReceipt>,
    },
    EvidenceFailureRecorded {
        binding: RunBindingReceipt,
        report: RunTerminalReport,
        expected_receipt: Box<RunTerminalReceipt>,
        class: RunTerminalClass,
    },
    Committed {
        receipt: Box<RunTerminalReceipt>,
    },
}

impl RunTerminalSnapshot {
    fn committed_receipt(&self) -> Option<&RunTerminalReceipt> {
        match self {
            Self::Committed { receipt } => Some(receipt),
            Self::Prepared
            | Self::Bound { .. }
            | Self::IntentPrepared { .. }
            | Self::ReportRecorded { .. }
            | Self::EvidenceFailureRecorded { .. } => None,
        }
    }
}

impl From<&PendingAttempt> for PendingAttemptSnapshot {
    fn from(pending: &PendingAttempt) -> Self {
        Self {
            trial_id: pending.trial_id,
            role: pending.role,
            candidate: pending.candidate,
            transition: pending.transition,
            prepared_runs: pending.prepared_runs.iter().map(Into::into).collect(),
            outcome: pending.outcome.as_ref().map(Into::into),
        }
    }
}

impl From<&PendingOutcome> for AttemptOutcomeSnapshot {
    fn from(_outcome: &PendingOutcome) -> Self {
        Self
    }
}

impl From<&PreparedRun> for PreparedRunSnapshot {
    fn from(run: &PreparedRun) -> Self {
        Self {
            run_index: run.run_index,
            context: run.context.clone(),
            run_intent_digest: run.run_intent_digest,
            terminal: (&run.terminal).into(),
        }
    }
}

impl From<&PreparedRunTerminalState> for RunTerminalSnapshot {
    fn from(state: &PreparedRunTerminalState) -> Self {
        match state {
            PreparedRunTerminalState::Prepared => Self::Prepared,
            PreparedRunTerminalState::Bound { plan, binding } => Self::Bound {
                plan: plan.clone(),
                binding: binding.clone(),
            },
            PreparedRunTerminalState::IntentPrepared {
                plan,
                binding,
                intent,
            } => Self::IntentPrepared {
                plan: plan.clone(),
                binding: binding.clone(),
                intent: intent.clone(),
            },
            PreparedRunTerminalState::ReportRecorded {
                binding,
                report,
                base_class,
                expected_receipt,
            } => Self::ReportRecorded {
                binding: binding.clone(),
                report: report.clone(),
                base_class: *base_class,
                expected_receipt: expected_receipt.clone(),
            },
            PreparedRunTerminalState::EvidenceFailureRecorded {
                binding,
                report,
                expected_receipt,
                class,
                ..
            } => Self::EvidenceFailureRecorded {
                binding: binding.clone(),
                report: report.clone(),
                expected_receipt: expected_receipt.clone(),
                class: *class,
            },
            PreparedRunTerminalState::Committed { receipt } => Self::Committed {
                receipt: receipt.clone(),
            },
        }
    }
}
