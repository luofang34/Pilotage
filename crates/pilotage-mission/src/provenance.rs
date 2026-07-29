//! Snapshot provenance (ADR-0030): which authority and cycle a flight is
//! packed against, bound to the blob's verified content hash.

use aerocontext_core::NavDataSnapshot;
use aerocontext_navdata::blob;
use chrono::NaiveDate;

use crate::error::MissionBuildError;

/// The provenance record of one decoded navdata snapshot.
///
/// `sha256_hex` is the blob container's verified payload checksum — a
/// content fingerprint, not a re-hash — so two hosts holding the same
/// record hold byte-identical navdata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotProvenance {
    /// Publishing authority slug, e.g. `"faa-nasr"`.
    pub authority: String,
    /// First date the snapshot's cycle is effective.
    pub effective_on: NaiveDate,
    /// First date a successor cycle is effective.
    pub next_effective_on: NaiveDate,
    /// Verified SHA-256 of the blob payload, lowercase hex.
    pub sha256_hex: String,
    /// Whether the snapshot is a generated fixture rather than published
    /// data; a fixture-built plan must never be mistaken for one packed
    /// from an authority's cycle.
    pub fixture: bool,
}

/// Decodes a navdata blob and derives its provenance record. Pure: the
/// bytes are the caller's, and `fixture` is the caller's honest claim
/// about where they came from.
///
/// # Errors
///
/// [`MissionBuildError::Blob`] when the container checks or the payload
/// decode fail.
pub fn decode_snapshot(
    bytes: &[u8],
    fixture: bool,
) -> Result<(NavDataSnapshot, SnapshotProvenance), MissionBuildError> {
    let info = blob::inspect(bytes)?;
    let provenance = SnapshotProvenance {
        authority: info.snapshot.cycle.authority.slug().to_owned(),
        effective_on: info.snapshot.cycle.effective_on,
        next_effective_on: info.snapshot.cycle.next_effective_on,
        sha256_hex: info.sha256_hex(),
        fixture,
    };
    Ok((info.snapshot, provenance))
}

/// The pack-for-flight record (ADR-0030) returned when a mission is
/// built: what data the plan came from and what the route expanded to,
/// for evidence and operator display.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MissionPlanRecord {
    /// Provenance of the snapshot the plan was expanded against.
    pub provenance: SnapshotProvenance,
    /// The route string as given to the engine.
    pub route_input: String,
    /// Waypoint identifiers in fly order, exactly as planned (anonymous
    /// lat/lon points carry their generated identifiers).
    pub expanded_idents: Vec<String>,
    /// Number of waypoints in the built plan.
    pub waypoint_count: usize,
}
