//! The record-lineage bijection check.
//!
//! Proves a one-to-one correspondence between the records decoded from the
//! package and the per-record lineage entries in the provenance: no duplicate
//! identity on either side, no decoded record without a lineage entry, and no
//! lineage entry without a decoded record. A failure in either direction means
//! an output record is untraceable or the lineage claims a record the package
//! does not contain.

use std::collections::BTreeSet;

use crate::chain::BuildArtifact;
use crate::error::VerifyError;

use super::RecordIdentity;

/// Checks the record↔lineage bijection over `package`, the identities decoded
/// from the package, against the provenance's lineage records.
///
/// # Errors
///
/// [`VerifyError::DuplicateRecord`], [`VerifyError::LineageMissingForRecord`], or
/// [`VerifyError::LineageOrphan`].
pub(crate) fn check_bijection(
    artifact: &BuildArtifact,
    package: &[RecordIdentity],
) -> Result<(), VerifyError> {
    let mut pkg: Vec<RecordIdentity> = package.to_vec();
    pkg.sort();
    if let Some(dup) = first_duplicate(&pkg) {
        return Err(VerifyError::DuplicateRecord { class: dup.0 });
    }
    let mut lineage: Vec<RecordIdentity> = artifact
        .provenance
        .records
        .iter()
        .map(|r| (r.class, r.lat_index, r.lon_index, r.key))
        .collect();
    lineage.sort();
    if let Some(dup) = first_duplicate(&lineage) {
        return Err(VerifyError::DuplicateRecord { class: dup.0 });
    }
    let lineage_set: BTreeSet<RecordIdentity> = lineage.iter().copied().collect();
    let package_set: BTreeSet<RecordIdentity> = pkg.iter().copied().collect();
    if let Some(missing) = pkg.iter().find(|id| !lineage_set.contains(id)) {
        return Err(VerifyError::LineageMissingForRecord {
            class: missing.0,
            lat_index: missing.1,
            lon_index: missing.2,
        });
    }
    if let Some(orphan) = lineage.iter().find(|id| !package_set.contains(id)) {
        return Err(VerifyError::LineageOrphan {
            class: orphan.0,
            lat_index: orphan.1,
            lon_index: orphan.2,
        });
    }
    Ok(())
}

/// The first identity that appears twice in a sorted slice, if any.
fn first_duplicate(sorted: &[RecordIdentity]) -> Option<RecordIdentity> {
    sorted.windows(2).find(|w| w[0] == w[1]).map(|w| w[0])
}
