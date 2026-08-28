use crate::journal::{JournalEvent, SessionIdentity};
use crate::{
    CandidateEvaluation, RunBindingReceipt, RunRecord, RunTerminalClass, RunTerminalCompletion,
    RunTerminalDisposition, RunTerminalIntent, RunTerminalPlan, RunTerminalQuarantine,
    RunTerminalReceipt, RunTerminalReport, RunTerminalSemanticOutcome, TuneError,
};

use super::{JournalState, PendingAttempt, PreparedRun, PreparedRunTerminalState, invalid};

#[cfg(test)]
#[path = "terminal/tests.rs"]
mod tests;

pub(super) fn apply_event(
    state: &mut JournalState,
    event: &JournalEvent,
    session: &SessionIdentity,
) -> Result<(), TuneError> {
    match event {
        JournalEvent::RunBound {
            trial_id,
            run_index,
            terminal_plan,
            binding,
        } => bind(
            state,
            *trial_id,
            *run_index,
            terminal_plan,
            binding,
            session,
        ),
        JournalEvent::RunTerminalIntentPrepared {
            trial_id,
            run_index,
            intent,
        } => prepare_intent(state, *trial_id, *run_index, intent),
        JournalEvent::RunTerminalReportRecorded {
            trial_id,
            run_index,
            report,
            base_class,
            expected_receipt,
        } => record_report(
            state,
            *trial_id,
            *run_index,
            report,
            *base_class,
            expected_receipt,
        ),
        JournalEvent::RunTerminalEvidenceFailureRecorded {
            trial_id,
            run_index,
            class,
        } => record_evidence_failure(state, *trial_id, *run_index, *class),
        JournalEvent::RunCommitted {
            trial_id,
            run_index,
            receipt,
        } => commit(state, *trial_id, *run_index, receipt, session),
        _ => Err(invalid("the event is not a run terminal event")),
    }
}

fn bind(
    state: &mut JournalState,
    trial_id: u64,
    run_index: u64,
    plan: &RunTerminalPlan,
    binding: &RunBindingReceipt,
    session: &SessionIdentity,
) -> Result<(), TuneError> {
    plan.validate()
        .map_err(|_| invalid("a terminal plan is not valid during replay"))?;
    binding
        .validate()
        .map_err(|_| invalid("a run binding is not valid during replay"))?;
    let run = current_run_mut(state, trial_id, run_index)?;
    if !matches!(run.terminal, PreparedRunTerminalState::Prepared)
        || binding.context() != &run.context
        || binding.run_intent_digest() != run.run_intent_digest
        || binding.terminal_plan_digest() != plan.plan_digest()
        || binding.adapter() != &session.runtimes.vehicle
    {
        return Err(invalid(
            "a run binding does not match the current run and vehicle",
        ));
    }
    run.terminal = PreparedRunTerminalState::Bound {
        plan: plan.clone(),
        binding: binding.clone(),
    };
    Ok(())
}

fn prepare_intent(
    state: &mut JournalState,
    trial_id: u64,
    run_index: u64,
    intent: &RunTerminalIntent,
) -> Result<(), TuneError> {
    intent
        .validate()
        .map_err(|_| invalid("a terminal intent is not valid during replay"))?;
    let run = current_run_mut(state, trial_id, run_index)?;
    if intent.context() != &run.context || intent.run_intent_digest() != run.run_intent_digest {
        return Err(invalid("a terminal intent does not match the current run"));
    }
    let PreparedRunTerminalState::Bound { plan, binding } = &run.terminal else {
        return Err(invalid("a terminal intent has no exact run binding"));
    };
    run.terminal = PreparedRunTerminalState::IntentPrepared {
        plan: plan.clone(),
        binding: binding.clone(),
        intent: intent.clone(),
    };
    Ok(())
}

fn record_report(
    state: &mut JournalState,
    trial_id: u64,
    run_index: u64,
    report: &RunTerminalReport,
    base_class: RunTerminalClass,
    expected_receipt: &RunTerminalReceipt,
) -> Result<(), TuneError> {
    report
        .validate()
        .map_err(|_| invalid("a terminal report is not valid during replay"))?;
    let run = current_run_mut(state, trial_id, run_index)?;
    let PreparedRunTerminalState::IntentPrepared {
        plan,
        binding,
        intent,
    } = &run.terminal
    else {
        return Err(invalid("a terminal report has no prepared intent"));
    };
    let expected_class = RunTerminalClass::classify(intent, report)
        .map_err(|_| invalid("a terminal report class cannot be recomputed"))?;
    expected_receipt
        .validate()
        .map_err(|_| invalid("an expected terminal receipt is not valid during replay"))?;
    if report.context() != &run.context
        || report.run_intent_digest() != run.run_intent_digest
        || report.plan() != plan
        || report.intent() != intent
        || base_class != expected_class
        || expected_receipt.binding() != binding
        || expected_receipt.context() != &run.context
        || expected_receipt.intent() != intent
        || expected_receipt.report() != report
        || expected_receipt.class() != base_class
    {
        return Err(invalid(
            "a terminal report authority changed its exact run chain",
        ));
    }
    run.terminal = PreparedRunTerminalState::ReportRecorded {
        binding: binding.clone(),
        report: report.clone(),
        base_class,
        expected_receipt: Box::new(expected_receipt.clone()),
    };
    Ok(())
}

fn record_evidence_failure(
    state: &mut JournalState,
    trial_id: u64,
    run_index: u64,
    class: RunTerminalClass,
) -> Result<(), TuneError> {
    let run = current_run_mut(state, trial_id, run_index)?;
    let PreparedRunTerminalState::ReportRecorded {
        binding,
        report,
        base_class,
        expected_receipt,
    } = &run.terminal
    else {
        return Err(invalid(
            "an evidence failure has no uncommitted terminal report",
        ));
    };
    let expected = RunTerminalClass::evidence_failure(report.intent(), report)
        .map_err(|_| invalid("an evidence failure needs a completed base class"))?;
    if !base_class.is_completed() || class != expected {
        return Err(invalid("an evidence failure class is not reproducible"));
    }
    run.terminal = PreparedRunTerminalState::EvidenceFailureRecorded {
        binding: binding.clone(),
        report: report.clone(),
        base_class: *base_class,
        expected_receipt: expected_receipt.clone(),
        class,
    };
    Ok(())
}

fn commit(
    state: &mut JournalState,
    trial_id: u64,
    run_index: u64,
    receipt: &RunTerminalReceipt,
    session: &SessionIdentity,
) -> Result<(), TuneError> {
    receipt
        .validate()
        .map_err(|_| invalid("a terminal receipt is not valid during replay"))?;
    let run = current_run_mut(state, trial_id, run_index)?;
    let expected = expected_commit_receipt(&run.terminal)?;
    if receipt != &expected || receipt.binding().adapter() != &session.runtimes.vehicle {
        return Err(invalid(
            "a terminal receipt changed its exact journal chain",
        ));
    }
    run.terminal = PreparedRunTerminalState::Committed {
        receipt: Box::new(receipt.clone()),
    };
    Ok(())
}

fn expected_commit_receipt(
    state: &PreparedRunTerminalState,
) -> Result<RunTerminalReceipt, TuneError> {
    match state {
        PreparedRunTerminalState::ReportRecorded {
            expected_receipt, ..
        } => Ok(expected_receipt.as_ref().clone()),
        PreparedRunTerminalState::EvidenceFailureRecorded {
            binding,
            report,
            base_class,
            expected_receipt,
            class,
        } if base_class.is_completed() => RunTerminalReceipt::new(
            binding,
            report.intent(),
            report,
            *class,
            expected_receipt.causal_evidence_digest(),
        )
        .map_err(|_| invalid("an evidence failure receipt cannot be recomputed")),
        _ => Err(invalid(
            "a terminal receipt has no exact uncommitted report",
        )),
    }
}

fn current_run_mut(
    state: &mut JournalState,
    trial_id: u64,
    run_index: u64,
) -> Result<&mut PreparedRun, TuneError> {
    let pending = state
        .pending
        .as_mut()
        .filter(|pending| pending.trial_id == trial_id && pending.outcome.is_none())
        .ok_or_else(|| invalid("a terminal event has no active attempt"))?;
    pending
        .prepared_runs
        .last_mut()
        .filter(|run| run.run_index == run_index)
        .ok_or_else(|| invalid("a terminal event does not refer to the current run"))
}

pub(super) fn validate_completed_attempt(
    pending: &PendingAttempt,
    evaluation: &CandidateEvaluation,
) -> Result<(), TuneError> {
    let receipts = committed_receipts(pending)?;
    match evaluation {
        CandidateEvaluation::Passed { runs, .. } => validate_passed(&receipts, runs),
        CandidateEvaluation::HardGateFailed {
            failure,
            completed_runs,
        } => validate_hard_gate(&receipts, completed_runs, failure),
        CandidateEvaluation::Quarantined { .. } => Err(invalid(
            "AttemptCompleted cannot contain a quarantined evaluation",
        )),
    }
}

fn validate_passed(receipts: &[&RunTerminalReceipt], runs: &[RunRecord]) -> Result<(), TuneError> {
    if receipts.len() != runs.len()
        || !receipts
            .iter()
            .zip(runs)
            .all(|(receipt, run)| completed_run(receipt).is_some_and(|saved| saved == run))
    {
        return Err(invalid(
            "a passing attempt does not match its committed scenario runs",
        ));
    }
    Ok(())
}

fn validate_hard_gate(
    receipts: &[&RunTerminalReceipt],
    completed_runs: &[RunRecord],
    failure: &crate::HardGateFailure,
) -> Result<(), TuneError> {
    let Some((last, prefix)) = receipts.split_last() else {
        return Err(invalid("a hard gate attempt has no committed abort run"));
    };
    let prefix_matches = prefix.len() == completed_runs.len()
        && prefix
            .iter()
            .zip(completed_runs)
            .all(|(receipt, run)| completed_run(receipt).is_some_and(|saved| saved == run));
    if !prefix_matches || completed_hard_gate(last) != Some(failure) {
        return Err(invalid(
            "a hard gate attempt does not match its committed run prefix",
        ));
    }
    Ok(())
}

pub(super) fn validate_quarantined_attempt(
    pending: &PendingAttempt,
    reason: &str,
) -> Result<(), TuneError> {
    let receipts = committed_receipts(pending)?;
    let Some((last, prefix)) = receipts.split_last() else {
        return Err(invalid("a quarantined attempt has no committed run"));
    };
    if prefix
        .iter()
        .any(|receipt| completed_run(receipt).is_none())
        || !matches!(
            last.class().disposition(),
            RunTerminalDisposition::Quarantine { .. }
        )
        || reason != quarantine_reason(last)?
    {
        return Err(invalid(
            "a quarantined attempt does not have one final quarantine receipt",
        ));
    }
    Ok(())
}

pub(crate) fn quarantine_reason(receipt: &RunTerminalReceipt) -> Result<String, TuneError> {
    let RunTerminalDisposition::Quarantine { quarantine } = receipt.class().disposition() else {
        return Err(invalid(
            "a completed receipt cannot supply a quarantine reason",
        ));
    };
    let class = match quarantine {
        RunTerminalQuarantine::TerminalFailure => "terminal_failure",
        RunTerminalQuarantine::ExecutionFailure => "execution_failure",
        RunTerminalQuarantine::Recovery => "recovery",
        RunTerminalQuarantine::EvidenceFailure => "evidence_failure",
    };
    Ok(format!(
        "terminal receipt {} has quarantine class {class}",
        receipt.receipt_digest()
    ))
}

fn committed_receipts(pending: &PendingAttempt) -> Result<Vec<&RunTerminalReceipt>, TuneError> {
    pending
        .prepared_runs
        .iter()
        .enumerate()
        .map(|(index, run)| {
            let identity_matches = u64::try_from(index) == Ok(run.run_index)
                && run
                    .context
                    .digest()
                    .is_ok_and(|digest| digest == run.run_intent_digest);
            let PreparedRunTerminalState::Committed { receipt } = &run.terminal else {
                return Err(invalid("an attempt contains an uncommitted run"));
            };
            if !identity_matches {
                return Err(invalid("a committed run identity changed"));
            }
            Ok(receipt.as_ref())
        })
        .collect()
}

pub(super) fn owned_committed_receipts(
    pending: &PendingAttempt,
) -> Result<Vec<RunTerminalReceipt>, TuneError> {
    committed_receipts(pending).map(|receipts| receipts.into_iter().cloned().collect())
}

fn completed_run(receipt: &RunTerminalReceipt) -> Option<&RunRecord> {
    if !matches!(
        receipt.class().disposition(),
        RunTerminalDisposition::Completed {
            completion: RunTerminalCompletion::ScenarioComplete
        }
    ) {
        return None;
    }
    match receipt.intent().outcome() {
        RunTerminalSemanticOutcome::ScenarioComplete { run, .. } => Some(run),
        _ => None,
    }
}

fn completed_hard_gate(receipt: &RunTerminalReceipt) -> Option<&crate::HardGateFailure> {
    if !matches!(
        receipt.class().disposition(),
        RunTerminalDisposition::Completed {
            completion: RunTerminalCompletion::HardGateAbort
        }
    ) {
        return None;
    }
    match receipt.intent().outcome() {
        RunTerminalSemanticOutcome::HardGateAbort { failure, .. } => Some(failure),
        _ => None,
    }
}

impl PendingAttempt {
    pub(crate) fn terminal_quarantine_reason(&self) -> Result<String, TuneError> {
        let run = self
            .prepared_runs
            .last()
            .ok_or_else(|| invalid("a quarantined attempt has no prepared run"))?;
        let PreparedRunTerminalState::Committed { receipt } = &run.terminal else {
            return Err(invalid(
                "a quarantined attempt has no committed terminal receipt",
            ));
        };
        crate::journal::terminal_quarantine_reason(receipt)
    }
}
