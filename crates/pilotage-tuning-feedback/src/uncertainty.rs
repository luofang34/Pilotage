//! Independent recomputation of the executed uncertainty relation.
//!
//! Nothing here reads a count, an offset, a schedule, or a decision out of
//! the receipt the run states. Every value is derived again from the sample
//! stream and the declaration, and the stated receipt is then required to be
//! exactly that.
//!
//! A run that says it flew under uncertainty is answerable for two things:
//! that the identities it launched with are the identities it flew under,
//! and that every sample states the decision the declaration required. A
//! receipt whose counts no sample produced is refused here, which is what a
//! reader who never watched the run needs before trusting one.

use flight_tune::{
    Digest, ExecutedLaunchIdentity, ExecutedSample, ExecutedUncertaintyDeclaration,
    ExecutedUncertaintyLedger, ExecutedUncertaintyReceipt,
};
use serde::Serialize;
use sha2::{Digest as ShaDigest, Sha256};

mod actuator;
mod counts;
mod derivation;
mod relation;

use crate::{FeedbackError, digest, error::invalid};

/// The receipt schema this verifier reproduces.
const RECEIPT_SCHEMA_VERSION: u16 = 1;

/// The domain the core binds one executed uncertainty receipt under.
const RECEIPT_DOMAIN: &[u8] = b"pilotage.flight-tune.executed-uncertainty-receipt.v1\0";

/// The domain the core chains one sample stream under.
const SAMPLE_STREAM_DOMAIN: &[u8] = b"pilotage.flight-tune.executed-uncertainty-sample.v1\0";

/// The basis-point value that requests no scaling.
const NOMINAL_BASIS_POINTS: u16 = 10_000;

/// One run whose executed uncertainty was derived again and agreed.
pub struct VerifiedExecutedUncertainty {
    receipt_digest: Digest,
    run_intent_digest: Digest,
    sample_count: u64,
}

impl VerifiedExecutedUncertainty {
    /// Returns the identity a run seal binds this receipt by.
    #[must_use]
    pub const fn receipt_digest(&self) -> Digest {
        self.receipt_digest
    }

    /// Returns the run intent this uncertainty was executed for.
    #[must_use]
    pub const fn run_intent_digest(&self) -> Digest {
        self.run_intent_digest
    }

    /// Returns how many samples the derived relation accepted.
    #[must_use]
    pub const fn sample_count(&self) -> u64 {
        self.sample_count
    }
}

#[derive(Serialize)]
struct ReceiptDocument<'a> {
    schema_version: u16,
    launch: &'a ExecutedLaunchIdentity,
    declaration: &'a ExecutedUncertaintyDeclaration,
    ledger: &'a ExecutedUncertaintyLedger,
    sample_stream_digest: Digest,
}

/// Derives one run's executed uncertainty again and requires agreement.
///
/// The samples are the only source. The stated ledger, the stated sample
/// stream identity, and the stated receipt identity are each required to be
/// the derived one, so a receipt cannot answer for itself.
///
/// # Errors
///
/// Returns [`FeedbackError`] when the receipt schema changed, when the
/// launch identities do not name the declared condition, when the sample
/// stream is not the one the receipt names, when any sample does not state
/// the decision the declaration required, or when the counts differ.
pub fn verify_executed_uncertainty(
    receipt: &ExecutedUncertaintyReceipt,
    samples: &[ExecutedSample],
) -> Result<VerifiedExecutedUncertainty, FeedbackError> {
    if receipt.schema_version != RECEIPT_SCHEMA_VERSION
        || receipt.declaration.schema_version != RECEIPT_SCHEMA_VERSION
        || receipt.launch.schema_version != RECEIPT_SCHEMA_VERSION
        || receipt.ledger.schema_version != RECEIPT_SCHEMA_VERSION
    {
        return Err(invalid("the executed uncertainty schema changed"));
    }
    require_bound(receipt)?;
    relation::require_digest(
        stream_digest(samples)?,
        receipt.sample_stream_digest,
        "the receipt does not name the samples it travels with",
    )?;
    let derived = relation::walk(&receipt.declaration, samples, RECEIPT_SCHEMA_VERSION)?;
    if derived != receipt.ledger {
        return Err(invalid(
            "the executed uncertainty counts do not follow from the samples",
        ));
    }
    relation::require_digest(
        receipt_digest(receipt)?,
        receipt.receipt_digest,
        "an executed uncertainty receipt does not cover its own content",
    )?;
    Ok(VerifiedExecutedUncertainty {
        receipt_digest: receipt.receipt_digest,
        run_intent_digest: receipt.launch.run_intent_digest,
        sample_count: derived.sample_count,
    })
}

/// Requires the launch, the declaration, and the ledger to name one run.
fn require_bound(receipt: &ExecutedUncertaintyReceipt) -> Result<(), FeedbackError> {
    let launch = &receipt.launch;
    let declaration = &receipt.declaration;
    if launch.run_intent_digest.is_zero()
        || launch.artifact_digest.is_zero()
        || launch.condition_digest.is_zero()
        || launch.artifact_digest == launch.condition_digest
    {
        return Err(invalid(
            "a launch identity is absent or names one value twice",
        ));
    }
    if launch.condition_digest != declaration.condition_digest
        || launch.artifact_digest != declaration.artifact_digest
        || launch.run_seed != declaration.run_seed
        || launch.required_capabilities != declaration.required_capabilities
    {
        return Err(invalid(
            "a receipt launch does not name the condition it declared",
        ));
    }
    require_declared(declaration)?;
    let declared = declaration
        .sensor_lanes
        .iter()
        .map(|lane| lane.lane_tag)
        .collect::<Vec<_>>();
    let counted = receipt
        .ledger
        .sensor_lanes
        .iter()
        .map(|lane| lane.lane_tag)
        .collect::<Vec<_>>();
    if declared != counted {
        return Err(invalid(
            "a receipt ledger does not count the declared lanes",
        ));
    }
    Ok(())
}

/// Requires the declared factors to be the ones the capabilities name.
///
/// A declaration that asks for a capability no factor needs, or that hides a
/// factor behind a capability it never asked for, would let a run claim
/// coverage of an uncertainty it did not execute.
fn require_declared(declaration: &ExecutedUncertaintyDeclaration) -> Result<(), FeedbackError> {
    let mut expected: Vec<&'static str> = Vec::new();
    if declaration.authority_scale_basis_points != NOMINAL_BASIS_POINTS {
        expected.push("actuator_authority");
    }
    if declaration.command_hold.is_some() {
        expected.push("command_hold");
    }
    if declaration.hover_scale_basis_points != NOMINAL_BASIS_POINTS {
        expected.push("hover_trim_uncertainty");
    }
    if !declaration.sensor_lanes.is_empty() {
        expected.push("sensor_perturbation");
    }
    if expected.is_empty() {
        return Err(invalid(
            "a nominal condition states no executed uncertainty",
        ));
    }
    let stated = declaration
        .required_capabilities
        .iter()
        .map(|capability| capability.as_str())
        .collect::<Vec<_>>();
    if stated != expected {
        return Err(invalid(
            "the required capabilities do not follow from the declared factors",
        ));
    }
    let mut previous: Option<u8> = None;
    for lane in &declaration.sensor_lanes {
        if previous.is_some_and(|prior| prior >= lane.lane_tag)
            || usize::from(lane.lane_tag) >= derivation::SENSOR_LANE_COUNT
        {
            return Err(invalid("the declared sensor lanes are not in lane order"));
        }
        previous = Some(lane.lane_tag);
    }
    Ok(())
}

/// Chains the identity of the exact ordered samples.
fn stream_digest(samples: &[ExecutedSample]) -> Result<Digest, FeedbackError> {
    let mut chained = {
        let mut hasher = Sha256::new();
        hasher.update(SAMPLE_STREAM_DOMAIN);
        Digest::from_bytes(hasher.finalize().into())
    };
    for sample in samples {
        let bytes = digest::encode("executed uncertainty sample", sample)?;
        let mut hasher = Sha256::new();
        hasher.update(SAMPLE_STREAM_DOMAIN);
        hasher.update(chained.as_bytes());
        hasher.update((bytes.len() as u64).to_le_bytes());
        hasher.update(&bytes);
        chained = Digest::from_bytes(hasher.finalize().into());
    }
    Ok(chained)
}

/// Rebuilds the identity the receipt content produces.
fn receipt_digest(receipt: &ExecutedUncertaintyReceipt) -> Result<Digest, FeedbackError> {
    digest::domain(
        "executed uncertainty receipt",
        RECEIPT_DOMAIN,
        &ReceiptDocument {
            schema_version: receipt.schema_version,
            launch: &receipt.launch,
            declaration: &receipt.declaration,
            ledger: &receipt.ledger,
            sample_stream_digest: receipt.sample_stream_digest,
        },
    )
}

#[cfg(test)]
mod tests;
