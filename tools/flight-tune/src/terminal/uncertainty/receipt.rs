//! The one document a run states about the uncertainty it executed.
//!
//! The receipt binds four things that must travel together: the run intent
//! that authorized the launch, the identities the launch passed and the
//! executor returned, what the condition declared, and what the verified
//! sample stream counted. A reader that holds only this document can derive
//! the counts again from the samples the stream identity names.

use serde::{Deserialize, Serialize};

use super::super::digest::domain_digest;
use super::super::invalid_terminal;
use super::launch::ExecutedLaunchIdentity;
use super::ledger::ExecutedUncertaintyLedger;
use super::{EXECUTED_UNCERTAINTY_SCHEMA_VERSION, ExecutedUncertaintyDeclaration};
use crate::{Digest, TuneError};

const RECEIPT_DOMAIN: &[u8] = b"pilotage.flight-tune.executed-uncertainty-receipt.v1\0";

/// Everything one run is answerable for under uncertainty.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutedUncertaintyReceipt {
    /// Receipt schema version.
    pub schema_version: u16,
    /// The identities the launch passed and the executor returned.
    pub launch: ExecutedLaunchIdentity,
    /// What the condition declared it would execute.
    pub declaration: ExecutedUncertaintyDeclaration,
    /// What the verified sample stream counted.
    pub ledger: ExecutedUncertaintyLedger,
    /// The identity of the exact ordered samples the counts came from.
    pub sample_stream_digest: Digest,
    /// The identity of this receipt.
    pub receipt_digest: Digest,
}

#[derive(Serialize)]
struct ReceiptDocument<'a> {
    schema_version: u16,
    launch: &'a ExecutedLaunchIdentity,
    declaration: &'a ExecutedUncertaintyDeclaration,
    ledger: &'a ExecutedUncertaintyLedger,
    sample_stream_digest: Digest,
}

impl ExecutedUncertaintyReceipt {
    /// Seals one verified run under its own identity.
    ///
    /// # Errors
    ///
    /// Returns [`TuneError`] when the launch and the declaration do not name
    /// one condition, when the ledger cannot answer for the declaration, or
    /// when the document cannot be encoded.
    pub fn new(
        launch: ExecutedLaunchIdentity,
        declaration: ExecutedUncertaintyDeclaration,
        ledger: ExecutedUncertaintyLedger,
        sample_stream_digest: Digest,
    ) -> Result<Self, TuneError> {
        let mut receipt = Self {
            schema_version: EXECUTED_UNCERTAINTY_SCHEMA_VERSION,
            launch,
            declaration,
            ledger,
            sample_stream_digest,
            receipt_digest: Digest::from_bytes([0; 32]),
        };
        receipt.validate_content()?;
        receipt.receipt_digest = receipt.recomputed_digest()?;
        Ok(receipt)
    }

    /// Rejects a receipt whose content is not the content it names.
    ///
    /// # Errors
    ///
    /// Returns [`TuneError`] when the schema differs, when a bound identity
    /// disagrees, or when the stated identity does not cover the content.
    pub fn validate(&self) -> Result<(), TuneError> {
        self.validate_content()?;
        if self.receipt_digest != self.recomputed_digest()? {
            return Err(invalid_terminal(
                "an executed uncertainty receipt does not cover its own content",
            ));
        }
        Ok(())
    }

    /// Returns the identity a run seal binds this receipt by.
    #[must_use]
    pub const fn digest(&self) -> Digest {
        self.receipt_digest
    }

    fn validate_content(&self) -> Result<(), TuneError> {
        if self.schema_version != EXECUTED_UNCERTAINTY_SCHEMA_VERSION {
            return Err(invalid_terminal(
                "the executed uncertainty receipt schema changed",
            ));
        }
        self.launch.validate()?;
        self.declaration.validate()?;
        self.ledger.validate()?;
        if self.sample_stream_digest.is_zero() {
            return Err(invalid_terminal("a receipt names no sample stream"));
        }
        self.validate_binding()
    }

    fn validate_binding(&self) -> Result<(), TuneError> {
        if self.launch.condition_digest != self.declaration.condition_digest
            || self.launch.artifact_digest != self.declaration.artifact_digest
            || self.launch.run_seed != self.declaration.run_seed
            || self.launch.required_capabilities != self.declaration.required_capabilities
        {
            return Err(invalid_terminal(
                "a receipt launch does not name the condition it declared",
            ));
        }
        if self.declaration.is_nominal() {
            return Err(invalid_terminal(
                "a nominal condition states no executed uncertainty",
            ));
        }
        let declared = self
            .declaration
            .sensor_lanes
            .iter()
            .map(|lane| lane.lane_tag)
            .collect::<Vec<_>>();
        let counted = self
            .ledger
            .sensor_lanes
            .iter()
            .map(|lane| lane.lane_tag)
            .collect::<Vec<_>>();
        if declared != counted {
            return Err(invalid_terminal(
                "a receipt ledger does not count the declared lanes",
            ));
        }
        Ok(())
    }

    fn recomputed_digest(&self) -> Result<Digest, TuneError> {
        domain_digest(
            RECEIPT_DOMAIN,
            &ReceiptDocument {
                schema_version: self.schema_version,
                launch: &self.launch,
                declaration: &self.declaration,
                ledger: &self.ledger,
                sample_stream_digest: self.sample_stream_digest,
            },
            "executed uncertainty receipt",
        )
    }
}
