use std::path::Path;

use flight_tune::Journal;
use pilotage_tuning_feedback::{CampaignEvidence, EvidenceReceipt};

use crate::CampaignError;
use crate::error;

/// Publishes independently verified evidence from one stable journal head.
///
/// # Errors
///
/// Returns [`CampaignError`] when the journal is not stable, verification
/// fails, or durable storage fails.
pub fn publish_journal_evidence_blocking(
    journal: &Journal,
    root: impl AsRef<Path>,
) -> Result<EvidenceReceipt, CampaignError> {
    let snapshot = journal
        .verified_evidence_snapshot()
        .map_err(error::snapshot)?;
    let evidence = CampaignEvidence::new(snapshot).map_err(error::verification)?;
    evidence
        .store_content_addressed_blocking(root)
        .map_err(error::storage)
}

#[cfg(test)]
mod tests;
