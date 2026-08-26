use crate::{
    RunBindingReceipt, RunTerminalClass, RunTerminalIntent, RunTerminalPlan, RunTerminalReceipt,
    RunTerminalReport, TuneError,
};

use super::{Journal, JournalEvent};

impl Journal {
    pub(crate) fn bind_run_terminal(
        &mut self,
        trial_id: u64,
        run_index: u64,
        terminal_plan: RunTerminalPlan,
        binding: RunBindingReceipt,
    ) -> Result<(), TuneError> {
        self.ensure_usable()?;
        self.append(JournalEvent::RunBound {
            trial_id,
            run_index,
            terminal_plan,
            binding,
        })
    }

    pub(crate) fn prepare_run_terminal_intent(
        &mut self,
        trial_id: u64,
        run_index: u64,
        intent: RunTerminalIntent,
    ) -> Result<(), TuneError> {
        self.ensure_usable()?;
        self.append(JournalEvent::RunTerminalIntentPrepared {
            trial_id,
            run_index,
            intent,
        })
    }

    pub(crate) fn record_run_terminal_report(
        &mut self,
        trial_id: u64,
        run_index: u64,
        report: RunTerminalReport,
        base_class: RunTerminalClass,
        expected_receipt: RunTerminalReceipt,
    ) -> Result<(), TuneError> {
        self.ensure_usable()?;
        self.append(JournalEvent::RunTerminalReportRecorded {
            trial_id,
            run_index,
            report: Box::new(report),
            base_class,
            expected_receipt: Box::new(expected_receipt),
        })
    }

    pub(crate) fn record_run_terminal_evidence_failure(
        &mut self,
        trial_id: u64,
        run_index: u64,
        class: RunTerminalClass,
    ) -> Result<(), TuneError> {
        self.ensure_usable()?;
        self.append(JournalEvent::RunTerminalEvidenceFailureRecorded {
            trial_id,
            run_index,
            class,
        })
    }

    pub(crate) fn commit_run(
        &mut self,
        trial_id: u64,
        run_index: u64,
        receipt: RunTerminalReceipt,
    ) -> Result<(), TuneError> {
        self.ensure_usable()?;
        self.append(JournalEvent::RunCommitted {
            trial_id,
            run_index,
            receipt: Box::new(receipt),
        })
    }
}
