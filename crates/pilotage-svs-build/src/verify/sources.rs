//! Validation of the source references a lineage carries, and of the signed
//! source-summary sequence.
//!
//! The signed summaries are an ordered sequence, so a source listed twice is a
//! duplicate and is rejected rather than silently collapsed. Each lineage record
//! must list at least one source, with no duplicate reference, and every
//! reference must name a source that has a signed summary. Finally every signed
//! summary must be referenced by some lineage record, so there is no extra source
//! the output never draws from. (That each reference resolves to exactly one
//! dataset source record, and that no two dataset records share a reference, is
//! proven against the dataset in [`super::verify_source_digests`].)

use std::collections::BTreeSet;

use crate::chain::BuildArtifact;
use crate::error::VerifyError;

/// Rejects a duplicate source id in the signed summary sequence.
///
/// # Errors
///
/// [`VerifyError::DuplicateSourceSummary`] if a source id appears twice.
pub(crate) fn reject_duplicate_summaries(artifact: &BuildArtifact) -> Result<(), VerifyError> {
    let mut seen: BTreeSet<u32> = BTreeSet::new();
    for summary in &artifact.provenance.sources {
        if !seen.insert(summary.id.0) {
            return Err(VerifyError::DuplicateSourceSummary {
                source_id: summary.id.0,
            });
        }
    }
    Ok(())
}

/// Validates the summary sequence and every lineage source reference.
///
/// # Errors
///
/// [`VerifyError::DuplicateSourceSummary`], [`VerifyError::EmptyLineageSources`],
/// [`VerifyError::DuplicateLineageSource`], [`VerifyError::UnknownLineageSource`],
/// or [`VerifyError::UnreferencedSourceSummary`].
pub(crate) fn check_lineage_sources(artifact: &BuildArtifact) -> Result<(), VerifyError> {
    reject_duplicate_summaries(artifact)?;
    let summaries: BTreeSet<u32> = artifact
        .provenance
        .sources
        .iter()
        .map(|summary| summary.id.0)
        .collect();
    let mut referenced: BTreeSet<u32> = BTreeSet::new();
    for record in &artifact.provenance.records {
        check_record_sources(record, &summaries, &mut referenced)?;
    }
    for summary in &artifact.provenance.sources {
        if !referenced.contains(&summary.id.0) {
            return Err(VerifyError::UnreferencedSourceSummary {
                source_id: summary.id.0,
            });
        }
    }
    Ok(())
}

/// Validates one lineage record's source references.
fn check_record_sources(
    record: &crate::provenance::RecordLineage,
    summaries: &BTreeSet<u32>,
    referenced: &mut BTreeSet<u32>,
) -> Result<(), VerifyError> {
    if record.sources.is_empty() {
        return Err(VerifyError::EmptyLineageSources {
            class: record.class,
            lat_index: record.lat_index,
            lon_index: record.lon_index,
        });
    }
    let mut seen: BTreeSet<crate::source::SourceRecordRef> = BTreeSet::new();
    for source in &record.sources {
        if !seen.insert(*source) {
            return Err(VerifyError::DuplicateLineageSource {
                source_id: source.source.0,
            });
        }
        if !summaries.contains(&source.source.0) {
            return Err(VerifyError::UnknownLineageSource {
                source_id: source.source.0,
            });
        }
        referenced.insert(source.source.0);
    }
    Ok(())
}
