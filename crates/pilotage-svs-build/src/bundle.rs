//! The provenance + report bundle and its canonical bytes.
//!
//! The detailed provenance and reports are not fields of the SVS-02 manifest, so
//! on their own they would be unsigned sidecars a tamperer could rewrite while
//! the package signature still verified. To close that gap the bundle is signed:
//! [`canonical_bundle_bytes`] fixes exactly what is signed — a domain tag, then
//! the length-prefixed deterministic JSON of the provenance and of the reports.
//! Because the provenance carries the package content hash, a valid bundle
//! signature over these bytes binds the provenance and reports to the exact
//! package, and any mutation of either changes the bytes and breaks the
//! signature.

use crate::provenance::BuildProvenance;
use crate::report::BuildReports;

/// Domain-separating magic for the signed bundle.
const BUNDLE_MAGIC: &[u8; 8] = b"SVSBBNDL";

/// The canonical bytes the bundle signature is computed over: magic, then the
/// length-prefixed deterministic JSON of the provenance and the reports. The JSON
/// is deterministic (sorted vectors, no maps), so these bytes reproduce exactly.
///
/// # Errors
///
/// [`serde_json::Error`] if serialization fails (it does not for these types).
pub fn canonical_bundle_bytes(
    provenance: &BuildProvenance,
    reports: &BuildReports,
) -> Result<Vec<u8>, serde_json::Error> {
    let provenance_json = serde_json::to_vec(provenance)?;
    let reports_json = serde_json::to_vec(reports)?;
    let mut out = Vec::with_capacity(24 + provenance_json.len() + reports_json.len());
    out.extend_from_slice(BUNDLE_MAGIC);
    out.extend_from_slice(&(provenance_json.len() as u64).to_le_bytes());
    out.extend_from_slice(&provenance_json);
    out.extend_from_slice(&(reports_json.len() as u64).to_le_bytes());
    out.extend_from_slice(&reports_json);
    Ok(out)
}
