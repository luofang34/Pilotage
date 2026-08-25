use serde::{Deserialize, Serialize};

use crate::{ArtifactIdentity, Digest, RunExecutionContext, TuneError};

use super::digest::domain_digest;
use super::{RunTerminalPlan, invalid_terminal};

/// The supported run binding receipt schema.
pub const RUN_BINDING_RECEIPT_SCHEMA_VERSION: u16 = 1;

const BINDING_DOMAIN: &[u8] = b"pilotage.flight-tune.run-binding-receipt.v1\0";

/// The immutable adapter binding for one prepared run and terminal plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunBindingReceipt {
    schema_version: u16,
    context: RunExecutionContext,
    run_intent_digest: Digest,
    terminal_plan_digest: Digest,
    adapter: ArtifactIdentity,
    receipt_digest: Digest,
}

#[derive(Serialize)]
struct BindingDocument<'a> {
    schema_version: u16,
    context: &'a RunExecutionContext,
    run_intent_digest: Digest,
    terminal_plan_digest: Digest,
    adapter: &'a ArtifactIdentity,
}

impl RunBindingReceipt {
    /// Creates a binding receipt for one exact run and plan.
    ///
    /// # Errors
    ///
    /// Returns [`TuneError`] when an identity is invalid or encoding fails.
    pub fn new(
        context: &RunExecutionContext,
        plan: &RunTerminalPlan,
        adapter: ArtifactIdentity,
    ) -> Result<Self, TuneError> {
        context.validate()?;
        plan.validate()?;
        adapter.validate()?;
        let mut receipt = Self {
            schema_version: RUN_BINDING_RECEIPT_SCHEMA_VERSION,
            context: context.clone(),
            run_intent_digest: context.digest()?,
            terminal_plan_digest: plan.plan_digest(),
            adapter,
            receipt_digest: Digest::from_bytes([0; 32]),
        };
        receipt.receipt_digest = receipt.recompute_digest()?;
        Ok(receipt)
    }

    /// Validates all binding identities and the canonical digest.
    ///
    /// # Errors
    ///
    /// Returns [`TuneError`] when the receipt is incomplete or changed.
    pub fn validate(&self) -> Result<(), TuneError> {
        self.validate_content()?;
        if self.receipt_digest.is_zero() || self.receipt_digest != self.recompute_digest()? {
            return Err(invalid_terminal("the run binding receipt digest changed"));
        }
        Ok(())
    }

    /// Recomputes the domain-separated receipt digest.
    ///
    /// # Errors
    ///
    /// Returns [`TuneError`] when the binding is invalid or encoding fails.
    pub fn recompute_digest(&self) -> Result<Digest, TuneError> {
        self.validate_content()?;
        domain_digest(
            BINDING_DOMAIN,
            &BindingDocument {
                schema_version: self.schema_version,
                context: &self.context,
                run_intent_digest: self.run_intent_digest,
                terminal_plan_digest: self.terminal_plan_digest,
                adapter: &self.adapter,
            },
            "run binding receipt",
        )
    }

    /// Returns the bound run context.
    #[must_use]
    pub const fn context(&self) -> &RunExecutionContext {
        &self.context
    }

    /// Returns the canonical run intent identity.
    #[must_use]
    pub const fn run_intent_digest(&self) -> Digest {
        self.run_intent_digest
    }

    /// Returns the terminal plan identity.
    #[must_use]
    pub const fn terminal_plan_digest(&self) -> Digest {
        self.terminal_plan_digest
    }

    /// Returns the adapter identity.
    #[must_use]
    pub const fn adapter(&self) -> &ArtifactIdentity {
        &self.adapter
    }

    /// Returns the complete binding identity.
    #[must_use]
    pub const fn receipt_digest(&self) -> Digest {
        self.receipt_digest
    }

    fn validate_content(&self) -> Result<(), TuneError> {
        self.context.validate()?;
        self.adapter.validate()?;
        if self.schema_version != RUN_BINDING_RECEIPT_SCHEMA_VERSION
            || self.run_intent_digest.is_zero()
            || self.run_intent_digest != self.context.digest()?
            || self.terminal_plan_digest.is_zero()
        {
            return Err(invalid_terminal("the run binding receipt is inconsistent"));
        }
        Ok(())
    }
}
