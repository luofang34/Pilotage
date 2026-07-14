//! The canonical byte layout the manifest signature and content hash are taken
//! over, and the canonical bytes of a tile.
//!
//! The layout is fixed, little-endian, and free of timestamps, padding, or
//! platform-dependent ordering, so the same package always produces the same
//! bytes and therefore the same hash and the same signature verdict. Following
//! the controlled-artifact pattern used elsewhere in the tree (a canonical
//! serialization, a hash the build records, and a verifier that recomputes),
//! [`manifest_content_hash`] lets a test pin the encoding: any change to the
//! byte layout changes the recorded hash and fails that test.
//!
//! The manifest's signature *bytes* are deliberately excluded from the canonical
//! form (they are the signature *over* it); the signing key id is included, so
//! the intended signer is bound into what is signed.

use sha2::{Digest, Sha256};

use crate::manifest::{ContentSpec, Coverage, Effectivity, PackageManifest, Provenance};
use crate::tile::Tile;

/// Domain-separating magic for a manifest's canonical bytes.
const MANIFEST_MAGIC: &[u8; 8] = b"SVSDBPKG";
/// Domain-separating magic for a tile's canonical bytes.
const TILE_MAGIC: &[u8; 8] = b"SVSDBTIL";

/// The canonical bytes of a manifest, excluding the signature bytes.
#[must_use]
pub fn manifest_canonical_bytes(manifest: &PackageManifest) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(MANIFEST_MAGIC);
    out.extend_from_slice(&manifest.schema_version.to_le_bytes());
    append_provenance(&mut out, &manifest.provenance);
    append_effectivity(&mut out, &manifest.effectivity);
    append_coverage(&mut out, &manifest.coverage);
    append_content(&mut out, &manifest.content);
    out.extend_from_slice(&manifest.tile_count.to_le_bytes());
    out.extend_from_slice(&manifest.tile_root.0);
    out.push(u8::from(manifest.simulation_only));
    out.extend_from_slice(&manifest.signature.key_id.0.to_le_bytes());
    out
}

/// Appends provenance: dataset, provider, version, the ordered processing
/// chain, and the use restrictions.
fn append_provenance(out: &mut Vec<u8>, provenance: &Provenance) {
    out.extend_from_slice(&provenance.dataset.0.to_le_bytes());
    out.extend_from_slice(&provenance.provider.0.to_le_bytes());
    out.extend_from_slice(&provenance.version.major.to_le_bytes());
    out.extend_from_slice(&provenance.version.minor.to_le_bytes());
    out.extend_from_slice(&provenance.version.patch.to_le_bytes());
    let steps = provenance.processing.steps();
    out.extend_from_slice(&(steps.len() as u64).to_le_bytes());
    for step in steps {
        out.extend_from_slice(&step.code.to_le_bytes());
        out.extend_from_slice(&step.tool_id.to_le_bytes());
    }
    out.extend_from_slice(&provenance.restrictions.bits().to_le_bytes());
}

/// Appends the three effectivity day numbers.
fn append_effectivity(out: &mut Vec<u8>, effectivity: &Effectivity) {
    out.extend_from_slice(&effectivity.release.0.to_le_bytes());
    out.extend_from_slice(&effectivity.effective.0.to_le_bytes());
    out.extend_from_slice(&effectivity.expiry.0.to_le_bytes());
}

/// Appends coverage: the bounding box (as IEEE-754 bit patterns), the datum and
/// its declared identities, and the resolution.
fn append_coverage(out: &mut Vec<u8>, coverage: &Coverage) {
    out.extend_from_slice(&coverage.region.min_lat_deg.to_bits().to_le_bytes());
    out.extend_from_slice(&coverage.region.max_lat_deg.to_bits().to_le_bytes());
    out.extend_from_slice(&coverage.region.min_lon_deg.to_bits().to_le_bytes());
    out.extend_from_slice(&coverage.region.max_lon_deg.to_bits().to_le_bytes());
    out.push(coverage.horizontal_datum.to_u8());
    out.extend_from_slice(&coverage.realization.0.to_le_bytes());
    out.push(coverage.vertical_datum.to_u8());
    out.extend_from_slice(&coverage.geoid.0.to_le_bytes());
    out.extend_from_slice(&coverage.resolution.post_spacing_mm.to_le_bytes());
}

/// Appends content: the feature set, accuracy, and integrity level.
fn append_content(out: &mut Vec<u8>, content: &ContentSpec) {
    out.extend_from_slice(&content.features.bits().to_le_bytes());
    out.extend_from_slice(&content.accuracy.horizontal_mm.to_le_bytes());
    out.extend_from_slice(&content.accuracy.vertical_mm.to_le_bytes());
    out.push(content.integrity.to_u8());
}

/// The canonical bytes of a tile: its key bound in front of its payload, so a
/// tile cannot be relabelled to another key without changing its leaf hash.
#[must_use]
pub fn tile_canonical_bytes(tile: &Tile) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(TILE_MAGIC);
    out.push(tile.key.class.to_u8());
    out.push(tile.key.level);
    out.extend_from_slice(&tile.key.tile.lat_index.to_le_bytes());
    out.extend_from_slice(&tile.key.tile.lon_index.to_le_bytes());
    out.extend_from_slice(&(tile.bytes.len() as u64).to_le_bytes());
    out.extend_from_slice(&tile.bytes);
    out
}

/// The SHA-256 of a manifest's canonical bytes — the recorded content hash a
/// test can pin the encoding against.
#[must_use]
pub fn manifest_content_hash(manifest: &PackageManifest) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(manifest_canonical_bytes(manifest));
    hasher.finalize().into()
}

#[cfg(test)]
mod tests;
