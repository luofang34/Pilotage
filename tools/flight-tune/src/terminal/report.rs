use serde::{Deserialize, Serialize};

use crate::{Digest, RunExecutionContext, TuneError};

use super::diagnostic::RunTerminalDiagnostic;
use super::digest::domain_digest;
use super::{
    RUN_TERMINAL_OPERATION_ORDER, RunTerminalIntent, RunTerminalOperation, RunTerminalPlan,
    RunTerminalRequirement, RunTerminalScope, RunTerminalSemanticOutcome, invalid_terminal,
};

/// The supported terminal report schema.
pub const RUN_TERMINAL_REPORT_SCHEMA_VERSION: u16 = 1;

const REPORT_DOMAIN: &[u8] = b"pilotage.flight-tune.run-terminal-report.v1\0";

/// How the terminal sequence started.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunTerminalRecoveryState {
    /// The live run entered its terminal sequence.
    Live,
    /// Recovery resumed an interrupted run.
    Resumed,
}

/// The saved result of one terminal operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum RunTerminalOperationStatus {
    /// The immutable terminal plan did not require this operation.
    NotRequired,
    /// The operation succeeded.
    Succeeded {
        /// The exact external durable proof, when one exists.
        durable_receipt_digest: Option<Digest>,
    },
    /// The operation failed.
    Failed {
        /// The bounded identity of the full failure diagnostic.
        diagnostic: RunTerminalDiagnostic,
    },
}

/// One operation result in the fixed terminal order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunTerminalOperationOutcome {
    operation: RunTerminalOperation,
    status: RunTerminalOperationStatus,
}

impl RunTerminalOperationOutcome {
    /// Creates a result for an operation that the plan did not require.
    #[must_use]
    pub const fn not_required(operation: RunTerminalOperation) -> Self {
        Self {
            operation,
            status: RunTerminalOperationStatus::NotRequired,
        }
    }

    /// Creates a successful operation result.
    ///
    /// # Errors
    ///
    /// Returns [`TuneError`] when a supplied durable proof digest is zero.
    pub fn succeeded(
        operation: RunTerminalOperation,
        durable_receipt_digest: Option<Digest>,
    ) -> Result<Self, TuneError> {
        if durable_receipt_digest.is_some_and(Digest::is_zero) {
            return Err(invalid_terminal(
                "a terminal operation durable receipt digest is zero",
            ));
        }
        Ok(Self {
            operation,
            status: RunTerminalOperationStatus::Succeeded {
                durable_receipt_digest,
            },
        })
    }

    /// Creates a failed operation result.
    ///
    /// # Errors
    ///
    /// Returns [`TuneError`] when the diagnostic is invalid.
    pub fn failed(
        operation: RunTerminalOperation,
        diagnostic: RunTerminalDiagnostic,
    ) -> Result<Self, TuneError> {
        diagnostic.validate()?;
        Ok(Self {
            operation,
            status: RunTerminalOperationStatus::Failed { diagnostic },
        })
    }

    /// Returns the operation.
    #[must_use]
    pub const fn operation(&self) -> RunTerminalOperation {
        self.operation
    }

    /// Returns the operation status.
    #[must_use]
    pub const fn status(&self) -> &RunTerminalOperationStatus {
        &self.status
    }
}

/// The complete immutable terminal result for one run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunTerminalReport {
    schema_version: u16,
    context: RunExecutionContext,
    run_intent_digest: Digest,
    plan: RunTerminalPlan,
    intent: RunTerminalIntent,
    scope: RunTerminalScope,
    recovery_state: RunTerminalRecoveryState,
    operations: Vec<RunTerminalOperationOutcome>,
    report_digest: Digest,
}

#[derive(Serialize)]
struct ReportDocument<'a> {
    schema_version: u16,
    context: &'a RunExecutionContext,
    run_intent_digest: Digest,
    plan: &'a RunTerminalPlan,
    intent: &'a RunTerminalIntent,
    scope: RunTerminalScope,
    recovery_state: RunTerminalRecoveryState,
    operations: &'a [RunTerminalOperationOutcome],
}

impl RunTerminalReport {
    /// Creates a complete report from all six operation results.
    ///
    /// # Errors
    ///
    /// Returns [`TuneError`] when a result is missing, repeated, or inconsistent.
    pub fn new(
        plan: &RunTerminalPlan,
        intent: &RunTerminalIntent,
        recovery_state: RunTerminalRecoveryState,
        operations: Vec<RunTerminalOperationOutcome>,
    ) -> Result<Self, TuneError> {
        let mut report = Self {
            schema_version: RUN_TERMINAL_REPORT_SCHEMA_VERSION,
            context: intent.context().clone(),
            run_intent_digest: intent.run_intent_digest(),
            plan: plan.clone(),
            intent: intent.clone(),
            scope: plan.scope(),
            recovery_state,
            operations,
            report_digest: Digest::from_bytes([0; 32]),
        };
        report.report_digest = report.recompute_digest()?;
        Ok(report)
    }

    /// Validates every binding, operation result, and canonical digest.
    ///
    /// # Errors
    ///
    /// Returns [`TuneError`] when the report is incomplete or changed.
    pub fn validate(&self) -> Result<(), TuneError> {
        self.validate_content()?;
        if self.report_digest.is_zero() || self.report_digest != self.recompute_digest()? {
            return Err(invalid_terminal("the terminal report digest changed"));
        }
        Ok(())
    }

    /// Recomputes the domain-separated report digest.
    ///
    /// # Errors
    ///
    /// Returns [`TuneError`] when the report is invalid or encoding fails.
    pub fn recompute_digest(&self) -> Result<Digest, TuneError> {
        self.validate_content()?;
        domain_digest(
            REPORT_DOMAIN,
            &self.digest_document(),
            "run terminal report",
        )
    }

    /// Returns the exact run context.
    #[must_use]
    pub const fn context(&self) -> &RunExecutionContext {
        &self.context
    }

    /// Returns the canonical run intent identity.
    #[must_use]
    pub const fn run_intent_digest(&self) -> Digest {
        self.run_intent_digest
    }

    /// Returns the immutable terminal plan.
    #[must_use]
    pub const fn plan(&self) -> &RunTerminalPlan {
        &self.plan
    }

    /// Returns the semantic terminal intent.
    #[must_use]
    pub const fn intent(&self) -> &RunTerminalIntent {
        &self.intent
    }

    /// Returns the terminal scope.
    #[must_use]
    pub const fn scope(&self) -> RunTerminalScope {
        self.scope
    }

    /// Returns the recovery state.
    #[must_use]
    pub const fn recovery_state(&self) -> RunTerminalRecoveryState {
        self.recovery_state
    }

    /// Returns the exact ordered operation results.
    #[must_use]
    pub fn operations(&self) -> &[RunTerminalOperationOutcome] {
        &self.operations
    }

    /// Returns the canonical report identity.
    #[must_use]
    pub const fn report_digest(&self) -> Digest {
        self.report_digest
    }

    /// Reports whether every required operation succeeded.
    #[must_use]
    pub fn all_required_succeeded(&self) -> bool {
        self.plan
            .requirements()
            .iter()
            .zip(&self.operations)
            .all(|(requirement, outcome)| operation_satisfies_success(*requirement, outcome))
    }

    fn validate_content(&self) -> Result<(), TuneError> {
        self.context.validate()?;
        self.plan.validate()?;
        self.intent.validate()?;
        if self.schema_version != RUN_TERMINAL_REPORT_SCHEMA_VERSION
            || self.context != *self.intent.context()
            || self.run_intent_digest != self.context.digest()?
            || self.run_intent_digest != self.intent.run_intent_digest()
            || self.scope != self.plan.scope()
        {
            return Err(invalid_terminal("the terminal report binding changed"));
        }
        validate_semantic_scope(self)?;
        validate_operations(&self.plan, &self.operations)
    }

    fn digest_document(&self) -> ReportDocument<'_> {
        ReportDocument {
            schema_version: self.schema_version,
            context: &self.context,
            run_intent_digest: self.run_intent_digest,
            plan: &self.plan,
            intent: &self.intent,
            scope: self.scope,
            recovery_state: self.recovery_state,
            operations: &self.operations,
        }
    }
}

fn validate_semantic_scope(report: &RunTerminalReport) -> Result<(), TuneError> {
    let outcome = report.intent.outcome();
    if outcome.permits_completion() && report.scope != RunTerminalScope::Active {
        return Err(invalid_terminal(
            "a completed semantic result needs an active run scope",
        ));
    }
    match (outcome, report.recovery_state) {
        (RunTerminalSemanticOutcome::Recovery, RunTerminalRecoveryState::Live) => Err(
            invalid_terminal("a recovery intent cannot use the live terminal state"),
        ),
        _ => Ok(()),
    }
}

fn validate_operations(
    plan: &RunTerminalPlan,
    operations: &[RunTerminalOperationOutcome],
) -> Result<(), TuneError> {
    if operations.len() != RUN_TERMINAL_OPERATION_ORDER.len() {
        return Err(invalid_terminal(
            "the terminal report operation set is incomplete",
        ));
    }
    for ((requirement, outcome), expected) in plan
        .requirements()
        .iter()
        .copied()
        .zip(operations)
        .zip(RUN_TERMINAL_OPERATION_ORDER)
    {
        if outcome.operation != expected || requirement.operation() != expected {
            return Err(invalid_terminal(
                "a terminal report operation is missing, repeated, or out of order",
            ));
        }
        validate_operation_status(requirement, outcome)?;
    }
    Ok(())
}

fn validate_operation_status(
    requirement: RunTerminalRequirement,
    outcome: &RunTerminalOperationOutcome,
) -> Result<(), TuneError> {
    match (requirement.is_required(), &outcome.status) {
        (false, RunTerminalOperationStatus::NotRequired) => Ok(()),
        (
            true,
            RunTerminalOperationStatus::Succeeded {
                durable_receipt_digest,
            },
        ) => validate_success_proof(outcome.operation, *durable_receipt_digest),
        (true, RunTerminalOperationStatus::Failed { diagnostic }) => diagnostic.validate(),
        _ => Err(invalid_terminal(
            "a terminal operation result does not match its plan requirement",
        )),
    }
}

fn validate_success_proof(
    operation: RunTerminalOperation,
    durable_receipt_digest: Option<Digest>,
) -> Result<(), TuneError> {
    if durable_receipt_digest.is_some_and(Digest::is_zero)
        || (operation == RunTerminalOperation::ChildTerminate && durable_receipt_digest.is_none())
    {
        return Err(invalid_terminal(
            "a successful terminal operation has no required durable proof",
        ));
    }
    Ok(())
}

fn operation_satisfies_success(
    requirement: RunTerminalRequirement,
    outcome: &RunTerminalOperationOutcome,
) -> bool {
    matches!(
        (requirement.is_required(), &outcome.status),
        (false, RunTerminalOperationStatus::NotRequired)
            | (true, RunTerminalOperationStatus::Succeeded { .. })
    )
}
