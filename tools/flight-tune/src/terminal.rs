//! Simulator-neutral run terminal evidence contracts.

mod binding;
mod class;
mod diagnostic;
mod digest;
mod intent;
mod plan;
mod receipt;
mod report;

pub use binding::{RUN_BINDING_RECEIPT_SCHEMA_VERSION, RunBindingReceipt};
pub use class::{
    RUN_TERMINAL_CLASS_SCHEMA_VERSION, RunTerminalClass, RunTerminalCompletion,
    RunTerminalDisposition, RunTerminalQuarantine, run_terminal_policy_digest,
};
pub use diagnostic::{MAX_TERMINAL_DIAGNOSTIC_PROJECTION_BYTES, RunTerminalDiagnostic};
pub use intent::{
    RUN_TERMINAL_INTENT_SCHEMA_VERSION, RunTerminalIntent, RunTerminalSemanticOutcome,
};
pub use plan::{
    RUN_TERMINAL_OPERATION_ORDER, RUN_TERMINAL_PLAN_SCHEMA_VERSION, RunTerminalOperation,
    RunTerminalPlan, RunTerminalRequirement, RunTerminalScope,
};
pub use receipt::{RUN_TERMINAL_RECEIPT_SCHEMA_VERSION, RunTerminalReceipt};
pub use report::{
    RUN_TERMINAL_REPORT_SCHEMA_VERSION, RunTerminalBindingStatus, RunTerminalOperationOutcome,
    RunTerminalOperationStatus, RunTerminalRecoveryState, RunTerminalReport,
};

use crate::TuneError;

pub(super) fn invalid_terminal(detail: impl Into<String>) -> TuneError {
    TuneError::ReceiptMismatch {
        operation: "validate run terminal contract",
        detail: detail.into(),
    }
}

#[cfg(test)]
mod tests;
