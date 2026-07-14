//! Validation of the source references a lineage carries.
//!
//! The record-lineage bijection proves each output record has a lineage entry,
//! but a lineage entry is only useful if its source references are sound. This
//! module rejects a lineage record that lists no source, lists a duplicate, names
//! a source with no signed summary, or names a record index beyond the source's
//! recorded count. It also proves the reference-to-summary side of the source-set
//! bijection: every signed summary must be referenced by some lineage record, so
//! there is no extra source the output never draws from.

use std::collections::{BTreeMap, BTreeSet};

use crate::chain::BuildArtifact;
use crate::error::VerifyError;

/// Validates every lineage source reference and the reference↔summary bijection.
///
/// # Errors
///
/// [`VerifyError::EmptyLineageSources`], [`VerifyError::DuplicateLineageSource`],
/// [`VerifyError::UnknownLineageSource`], [`VerifyError::SourceRecordOutOfRange`],
/// or [`VerifyError::UnreferencedSourceSummary`].
pub(crate) fn check_lineage_sources(artifact: &BuildArtifact) -> Result<(), VerifyError> {
    let counts: BTreeMap<u32, u32> = artifact
        .provenance
        .sources
        .iter()
        .map(|summary| (summary.id.0, summary.record_count))
        .collect();
    let mut referenced: BTreeSet<u32> = BTreeSet::new();
    for record in &artifact.provenance.records {
        check_record_sources(record, &counts, &mut referenced)?;
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
    counts: &BTreeMap<u32, u32>,
    referenced: &mut BTreeSet<u32>,
) -> Result<(), VerifyError> {
    if record.sources.is_empty() {
        return Err(VerifyError::EmptyLineageSources {
            class: record.class,
            lat_index: record.lat_index,
            lon_index: record.lon_index,
        });
    }
    let mut seen: BTreeSet<(u32, u32)> = BTreeSet::new();
    for source in &record.sources {
        let id = source.source.0;
        if !seen.insert((id, source.record)) {
            return Err(VerifyError::DuplicateLineageSource {
                source_id: id,
                record: source.record,
            });
        }
        let count = counts
            .get(&id)
            .ok_or(VerifyError::UnknownLineageSource { source_id: id })?;
        if source.record >= *count {
            return Err(VerifyError::SourceRecordOutOfRange {
                source_id: id,
                record: source.record,
                count: *count,
            });
        }
        referenced.insert(id);
    }
    Ok(())
}
