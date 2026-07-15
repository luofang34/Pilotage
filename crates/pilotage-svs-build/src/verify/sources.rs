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

use std::collections::{BTreeMap, BTreeSet};

use crate::chain::BuildArtifact;
use crate::error::VerifyError;
use crate::source::{SourceDataset, SourceRecordRef, source_record_refs};

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

/// Rejects a dataset that declares the same source id twice: which metadata
/// (datum, license, version) governs that source would be ambiguous, and the
/// digest check would silently bind to whichever entry came first.
///
/// # Errors
///
/// [`VerifyError::DuplicateSourceMeta`] if a source id appears twice.
pub(crate) fn reject_duplicate_meta(
    source: &crate::source::SourceDataset,
) -> Result<(), VerifyError> {
    let mut seen: BTreeSet<u32> = BTreeSet::new();
    for meta in &source.meta {
        if !seen.insert(meta.id.0) {
            return Err(VerifyError::DuplicateSourceMeta {
                source_id: meta.id.0,
            });
        }
    }
    Ok(())
}

/// Validates the summary sequence and every lineage source reference.
///
/// A summary counts as referenced when output lineage draws from it **or**
/// when a recorded disposition names one of its records — a source whose
/// every record was rejected, voided, or clipped is still traceably
/// consumed. A summary referenced by neither is a phantom source and fails.
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
    let mut lineage_refs: BTreeSet<crate::source::SourceRecordRef> = BTreeSet::new();
    for record in &artifact.provenance.records {
        check_record_sources(record, &summaries, &mut referenced)?;
        lineage_refs.extend(record.sources.iter().copied());
    }
    check_dispositions(artifact, &summaries, &lineage_refs, &mut referenced)?;
    for summary in &artifact.provenance.sources {
        if !referenced.contains(&summary.id.0) {
            return Err(VerifyError::UnreferencedSourceSummary {
                source_id: summary.id.0,
            });
        }
    }
    Ok(())
}

/// Validates every recorded disposition before it may vouch for a summary: it
/// must reference a source with a signed summary, no record may carry two
/// dispositions, and a no-contribution fate must not contradict output
/// lineage that draws from the same record.
fn check_dispositions(
    artifact: &BuildArtifact,
    summaries: &BTreeSet<u32>,
    lineage_refs: &BTreeSet<crate::source::SourceRecordRef>,
    referenced: &mut BTreeSet<u32>,
) -> Result<(), VerifyError> {
    let mut seen: BTreeSet<crate::source::SourceRecordRef> = BTreeSet::new();
    for disposition in &artifact.provenance.dispositions {
        let source_id = disposition.source.source.0;
        if !summaries.contains(&source_id) {
            return Err(VerifyError::DispositionInvalid {
                source_id,
                reason: "references a source with no signed summary",
            });
        }
        if !seen.insert(disposition.source) {
            return Err(VerifyError::DispositionInvalid {
                source_id,
                reason: "the record carries more than one disposition",
            });
        }
        if matches!(
            disposition.disposition,
            crate::provenance::Disposition::NoContribution { .. }
        ) && lineage_refs.contains(&disposition.source)
        {
            return Err(VerifyError::DispositionInvalid {
                source_id,
                reason: "claims no contribution but output lineage draws from the record",
            });
        }
        referenced.insert(source_id);
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

/// Verifies the recorded source digests against the source input over the exact
/// source set: every signed summary matches a dataset source's version and
/// content digest, and every dataset source has a signed summary. Neither side
/// may carry a source the other lacks.
///
/// # Errors
///
/// [`VerifyError::SourceDigestMismatch`] for a summary that names no dataset
/// source or whose digest/version disagrees, or [`VerifyError::SourceSummaryMissing`]
/// for a dataset source with no summary.
pub fn verify_source_digests(
    artifact: &BuildArtifact,
    source: &SourceDataset,
) -> Result<(), VerifyError> {
    reject_duplicate_summaries(artifact)?;
    reject_duplicate_meta(source)?;
    let summary_ids: BTreeSet<u32> = artifact
        .provenance
        .sources
        .iter()
        .map(|summary| summary.id.0)
        .collect();
    for summary in &artifact.provenance.sources {
        let meta = source
            .meta_for(summary.id)
            .ok_or(VerifyError::SourceDigestMismatch {
                source_id: summary.id.0,
            })?;
        let digest = crate::source::source_content_digest(source, meta);
        if summary.version != meta.version || summary.content_digest != digest {
            return Err(VerifyError::SourceDigestMismatch {
                source_id: summary.id.0,
            });
        }
    }
    for meta in &source.meta {
        if !summary_ids.contains(&meta.id.0) {
            return Err(VerifyError::SourceSummaryMissing {
                source_id: meta.id.0,
            });
        }
    }
    check_source_resolution(artifact, source)
}

/// Proves every lineage source reference resolves to exactly one dataset source
/// record and that no two distinct dataset records share a reference.
fn check_source_resolution(
    artifact: &BuildArtifact,
    source: &SourceDataset,
) -> Result<(), VerifyError> {
    let mut counts: BTreeMap<SourceRecordRef, u32> = BTreeMap::new();
    for source_ref in source_record_refs(source) {
        counts
            .entry(source_ref)
            .and_modify(|count| *count = count.wrapping_add(1))
            .or_insert(1);
    }
    for (source_ref, count) in &counts {
        if *count > 1 {
            return Err(VerifyError::AmbiguousSourceRecord {
                source_id: source_ref.source.0,
            });
        }
    }
    for record in &artifact.provenance.records {
        for source_ref in &record.sources {
            match counts.get(source_ref) {
                None => {
                    return Err(VerifyError::UnresolvedSourceRef {
                        source_id: source_ref.source.0,
                    });
                }
                Some(1) => {}
                Some(_) => {
                    return Err(VerifyError::AmbiguousSourceRecord {
                        source_id: source_ref.source.0,
                    });
                }
            }
        }
    }
    // A disposition is a claim about a real input record: one that resolves to
    // no dataset record is fabricated provenance and is refused.
    for disposition in &artifact.provenance.dispositions {
        if counts.get(&disposition.source) != Some(&1) {
            return Err(VerifyError::DispositionInvalid {
                source_id: disposition.source.source.0,
                reason: "resolves to no unique dataset source record",
            });
        }
    }
    Ok(())
}
