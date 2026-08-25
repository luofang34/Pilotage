use serde::{Deserialize, Serialize};

use crate::{Digest, RunExecutionContext, TuneError};

use super::digest::domain_digest;
use super::{
    RunBindingReceipt, RunTerminalClass, RunTerminalDisposition, RunTerminalIntent,
    RunTerminalReport, invalid_terminal,
};

/// The supported terminal receipt schema.
pub const RUN_TERMINAL_RECEIPT_SCHEMA_VERSION: u16 = 1;

const COMPLETED_RECEIPT_DOMAIN: &[u8] = b"pilotage.flight-tune.run-terminal-receipt.completed.v1\0";
const QUARANTINE_RECEIPT_DOMAIN: &[u8] =
    b"pilotage.flight-tune.run-terminal-receipt.quarantine.v1\0";

/// One closed completed or quarantine terminal evidence receipt.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "receipt", rename_all = "snake_case", deny_unknown_fields)]
pub enum RunTerminalReceipt {
    /// The run can contribute its exact semantic result.
    Completed {
        /// The receipt schema.
        schema_version: u16,
        /// The immutable adapter binding.
        binding: RunBindingReceipt,
        /// The exact run context.
        context: RunExecutionContext,
        /// The canonical run intent identity.
        run_intent_digest: Digest,
        /// The durable semantic intent.
        intent: RunTerminalIntent,
        /// The complete terminal report.
        report: RunTerminalReport,
        /// The core-supplied completed class.
        class: RunTerminalClass,
        /// The exact causal evidence identity.
        causal_evidence_digest: Digest,
        /// The canonical receipt identity.
        receipt_digest: Digest,
    },
    /// The run cannot contribute evidence.
    Quarantine {
        /// The receipt schema.
        schema_version: u16,
        /// The immutable adapter binding.
        binding: RunBindingReceipt,
        /// The exact run context.
        context: RunExecutionContext,
        /// The canonical run intent identity.
        run_intent_digest: Digest,
        /// The durable semantic intent.
        intent: RunTerminalIntent,
        /// The complete terminal report.
        report: RunTerminalReport,
        /// The core-supplied quarantine class.
        class: RunTerminalClass,
        /// The exact causal evidence identity.
        causal_evidence_digest: Digest,
        /// The canonical receipt identity.
        receipt_digest: Digest,
    },
}

#[derive(Clone, Copy)]
struct ReceiptFields<'a> {
    schema_version: u16,
    binding: &'a RunBindingReceipt,
    context: &'a RunExecutionContext,
    run_intent_digest: Digest,
    intent: &'a RunTerminalIntent,
    report: &'a RunTerminalReport,
    class: RunTerminalClass,
    causal_evidence_digest: Digest,
    receipt_digest: Digest,
}

#[derive(Serialize)]
struct ReceiptDocument<'a> {
    schema_version: u16,
    binding: &'a RunBindingReceipt,
    context: &'a RunExecutionContext,
    run_intent_digest: Digest,
    intent: &'a RunTerminalIntent,
    report: &'a RunTerminalReport,
    class: RunTerminalClass,
    causal_evidence_digest: Digest,
}

impl RunTerminalReceipt {
    /// Creates the one closed receipt for the core-supplied terminal class.
    ///
    /// # Errors
    ///
    /// Returns [`TuneError`] when any binding or class is inconsistent.
    pub fn new(
        binding: &RunBindingReceipt,
        intent: &RunTerminalIntent,
        report: &RunTerminalReport,
        class: RunTerminalClass,
        causal_evidence_digest: Digest,
    ) -> Result<Self, TuneError> {
        let context = intent.context().clone();
        let mut receipt = match class.disposition() {
            RunTerminalDisposition::Completed { .. } => Self::Completed {
                schema_version: RUN_TERMINAL_RECEIPT_SCHEMA_VERSION,
                binding: binding.clone(),
                context,
                run_intent_digest: intent.run_intent_digest(),
                intent: intent.clone(),
                report: report.clone(),
                class,
                causal_evidence_digest,
                receipt_digest: Digest::from_bytes([0; 32]),
            },
            RunTerminalDisposition::Quarantine { .. } => Self::Quarantine {
                schema_version: RUN_TERMINAL_RECEIPT_SCHEMA_VERSION,
                binding: binding.clone(),
                context,
                run_intent_digest: intent.run_intent_digest(),
                intent: intent.clone(),
                report: report.clone(),
                class,
                causal_evidence_digest,
                receipt_digest: Digest::from_bytes([0; 32]),
            },
        };
        let receipt_digest = receipt.recompute_digest()?;
        receipt.set_receipt_digest(receipt_digest);
        Ok(receipt)
    }

    /// Validates the complete chain and canonical receipt identity.
    ///
    /// # Errors
    ///
    /// Returns [`TuneError`] when one binding, class, or digest differs.
    pub fn validate(&self) -> Result<(), TuneError> {
        self.validate_content()?;
        let fields = self.fields();
        if fields.receipt_digest.is_zero() || fields.receipt_digest != self.recompute_digest()? {
            return Err(invalid_terminal("the terminal receipt digest changed"));
        }
        Ok(())
    }

    /// Recomputes the domain-separated receipt identity.
    ///
    /// # Errors
    ///
    /// Returns [`TuneError`] when the receipt is invalid or encoding fails.
    pub fn recompute_digest(&self) -> Result<Digest, TuneError> {
        self.validate_content()?;
        let fields = self.fields();
        domain_digest(
            self.digest_domain(),
            &ReceiptDocument {
                schema_version: fields.schema_version,
                binding: fields.binding,
                context: fields.context,
                run_intent_digest: fields.run_intent_digest,
                intent: fields.intent,
                report: fields.report,
                class: fields.class,
                causal_evidence_digest: fields.causal_evidence_digest,
            },
            "run terminal receipt",
        )
    }

    /// Reports whether this is the one completed receipt class.
    #[must_use]
    pub const fn is_completed(&self) -> bool {
        matches!(self, Self::Completed { .. })
    }

    /// Returns the adapter binding.
    #[must_use]
    pub fn binding(&self) -> &RunBindingReceipt {
        self.fields().binding
    }

    /// Returns the exact run context.
    #[must_use]
    pub fn context(&self) -> &RunExecutionContext {
        self.fields().context
    }

    /// Returns the semantic terminal intent.
    #[must_use]
    pub fn intent(&self) -> &RunTerminalIntent {
        self.fields().intent
    }

    /// Returns the complete terminal report.
    #[must_use]
    pub fn report(&self) -> &RunTerminalReport {
        self.fields().report
    }

    /// Returns the actual terminal class.
    #[must_use]
    pub fn class(&self) -> RunTerminalClass {
        self.fields().class
    }

    /// Returns the causal evidence identity.
    #[must_use]
    pub fn causal_evidence_digest(&self) -> Digest {
        self.fields().causal_evidence_digest
    }

    /// Returns the complete receipt identity.
    #[must_use]
    pub fn receipt_digest(&self) -> Digest {
        self.fields().receipt_digest
    }

    fn validate_content(&self) -> Result<(), TuneError> {
        let fields = self.fields();
        fields.binding.validate()?;
        fields.context.validate()?;
        fields.intent.validate()?;
        fields.report.validate()?;
        fields.class.validate_for(fields.intent, fields.report)?;
        if !self.bindings_match(fields)? || !self.variant_matches_class(fields.class) {
            return Err(invalid_terminal(
                "the terminal receipt chain is inconsistent",
            ));
        }
        Ok(())
    }

    fn bindings_match(&self, fields: ReceiptFields<'_>) -> Result<bool, TuneError> {
        Ok(fields.schema_version == RUN_TERMINAL_RECEIPT_SCHEMA_VERSION
            && !fields.causal_evidence_digest.is_zero()
            && fields.context == fields.binding.context()
            && fields.context == fields.intent.context()
            && fields.context == fields.report.context()
            && fields.run_intent_digest == fields.context.digest()?
            && fields.run_intent_digest == fields.binding.run_intent_digest()
            && fields.run_intent_digest == fields.intent.run_intent_digest()
            && fields.run_intent_digest == fields.report.run_intent_digest()
            && fields.binding.terminal_plan_digest() == fields.report.plan().plan_digest()
            && fields.intent == fields.report.intent())
    }

    const fn variant_matches_class(&self, class: RunTerminalClass) -> bool {
        matches!(
            (self, class.disposition()),
            (
                Self::Completed { .. },
                RunTerminalDisposition::Completed { .. }
            ) | (
                Self::Quarantine { .. },
                RunTerminalDisposition::Quarantine { .. }
            )
        )
    }

    const fn digest_domain(&self) -> &'static [u8] {
        match self {
            Self::Completed { .. } => COMPLETED_RECEIPT_DOMAIN,
            Self::Quarantine { .. } => QUARANTINE_RECEIPT_DOMAIN,
        }
    }

    fn set_receipt_digest(&mut self, digest: Digest) {
        match self {
            Self::Completed { receipt_digest, .. } | Self::Quarantine { receipt_digest, .. } => {
                *receipt_digest = digest
            }
        }
    }

    const fn fields(&self) -> ReceiptFields<'_> {
        match self {
            Self::Completed {
                schema_version,
                binding,
                context,
                run_intent_digest,
                intent,
                report,
                class,
                causal_evidence_digest,
                receipt_digest,
            }
            | Self::Quarantine {
                schema_version,
                binding,
                context,
                run_intent_digest,
                intent,
                report,
                class,
                causal_evidence_digest,
                receipt_digest,
            } => ReceiptFields {
                schema_version: *schema_version,
                binding,
                context,
                run_intent_digest: *run_intent_digest,
                intent,
                report,
                class: *class,
                causal_evidence_digest: *causal_evidence_digest,
                receipt_digest: *receipt_digest,
            },
        }
    }
}
