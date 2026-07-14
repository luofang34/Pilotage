//! Assembling, signing, and self-verifying the SVS-02 package.
//!
//! This module builds a [`pilotage_svs_db::PackageManifest`] from the emitted
//! tiles and the build configuration, computes the tile-root and signs the
//! canonical manifest bytes with the configured Ed25519 seed, then runs the
//! package back through [`pilotage_svs_db::verify_package`] before returning it.
//! A package that does not verify is a [`BuildError::SelfVerification`], so the
//! chain never hands back bytes that would fail to activate. All package types
//! are consumed from `pilotage-svs-db`; none is re-minted here.

#[cfg(test)]
mod tests;

use ed25519_dalek::{Signer, SigningKey};
use pilotage_svs_db::{
    Accuracy, CandidatePackage, ContentSpec, Coverage, FeatureSet, MANIFEST_SCHEMA_VERSION,
    PackageManifest, PackageSignature, ProcessingChain, ProcessingStep, Provenance, Resolution,
    Tile, TileKey, TileRoot, TrustAnchor, TrustRoot, UsePolicy, UseRestrictions,
    manifest_canonical_bytes, manifest_content_hash, merkle_root, tile_canonical_bytes,
    tile_leaf_hash, verify_package,
};

use crate::config::BuildConfig;
use crate::error::BuildError;
use crate::provenance::TOOL_ID;
use crate::source::SourceDataset;

/// A signed package together with the content hash the provenance binds to.
pub(crate) struct SignedPackage {
    /// The signed, verified candidate package.
    pub package: CandidatePackage,
    /// The content hash of the signed manifest.
    pub content_hash: [u8; 32],
}

/// Assembles the manifest over `tiles`, signs it, and self-verifies it.
///
/// # Errors
///
/// [`BuildError::SelfVerification`] if the built package does not pass the
/// SVS-02 verifier.
pub(crate) fn assemble_and_sign(
    config: &BuildConfig,
    source: &SourceDataset,
    tiles: &[Tile],
) -> Result<SignedPackage, BuildError> {
    let mut manifest = build_manifest(config, source, tiles);
    sign(&mut manifest, &config.signing.signing_seed);
    let content_hash = manifest_content_hash(&manifest);
    let package = CandidatePackage {
        manifest,
        tiles: tiles.to_vec(),
    };
    self_verify(config, &package)?;
    Ok(SignedPackage {
        package,
        content_hash,
    })
}

/// Builds the manifest (with a placeholder signature) from the tiles and config.
fn build_manifest(config: &BuildConfig, source: &SourceDataset, tiles: &[Tile]) -> PackageManifest {
    let processing = ProcessingChain(
        crate::chain::STAGE_CODES
            .iter()
            .map(|&code| ProcessingStep {
                code,
                tool_id: TOOL_ID,
            })
            .collect(),
    );
    PackageManifest {
        schema_version: MANIFEST_SCHEMA_VERSION,
        provenance: Provenance {
            dataset: config.identity.dataset,
            provider: config.identity.provider,
            version: config.identity.version,
            processing,
            restrictions: restrictions(config, source),
        },
        effectivity: config.identity.effectivity,
        coverage: Coverage {
            region: config.coverage,
            horizontal_datum: config.target.horizontal,
            realization: config.target.realization,
            vertical_datum: config.target.vertical,
            geoid: config.target.geoid,
            resolution: Resolution {
                post_spacing_mm: config.params.post_spacing_mm,
            },
        },
        content: ContentSpec {
            features: features(tiles),
            accuracy: worst_accuracy(source),
            integrity: config.params.integrity,
        },
        tile_count: tiles.len() as u32,
        tile_root: tile_root(tiles),
        simulation_only: config.identity.simulation_only,
        signature: PackageSignature {
            key_id: config.signing.key_id,
            bytes: [0u8; 64],
        },
    }
}

/// The union of the base restrictions and every source's license restrictions,
/// so one restricted source restricts the whole package.
fn restrictions(config: &BuildConfig, source: &SourceDataset) -> UseRestrictions {
    let mut bits = config.identity.base_restrictions.bits();
    for meta in &source.meta {
        bits |= meta.license.restrictions().bits();
    }
    UseRestrictions(bits)
}

/// The feature set the emitted tiles cover.
fn features(tiles: &[Tile]) -> FeatureSet {
    tiles
        .iter()
        .fold(FeatureSet::empty(), |set, tile| set.with(tile.key.class))
}

/// The most conservative (largest) accuracy across contributing sources.
fn worst_accuracy(source: &SourceDataset) -> Accuracy {
    source.meta.iter().fold(
        Accuracy {
            horizontal_mm: 0,
            vertical_mm: 0,
        },
        |acc, meta| Accuracy {
            horizontal_mm: acc.horizontal_mm.max(meta.accuracy.horizontal_mm),
            vertical_mm: acc.vertical_mm.max(meta.accuracy.vertical_mm),
        },
    )
}

/// The tile-root over the tiles, computed exactly as the verifier recomputes it
/// (leaves in key order).
fn tile_root(tiles: &[Tile]) -> TileRoot {
    let mut entries: Vec<(TileKey, [u8; 32])> = tiles
        .iter()
        .map(|tile| (tile.key, tile_leaf_hash(&tile_canonical_bytes(tile))))
        .collect();
    entries.sort_by_key(|entry| entry.0);
    let leaves: Vec<[u8; 32]> = entries.into_iter().map(|(_, leaf)| leaf).collect();
    merkle_root(&leaves)
}

/// Signs the manifest in place over its current canonical bytes.
fn sign(manifest: &mut PackageManifest, seed: &[u8; 32]) {
    let signing_key = SigningKey::from_bytes(seed);
    let bytes = manifest_canonical_bytes(manifest);
    manifest.signature.bytes = signing_key.sign(&bytes).to_bytes();
}

/// Verifies the built package against a trust root holding the configured key,
/// at the effective day, under the simulator-permitted policy.
fn self_verify(config: &BuildConfig, package: &CandidatePackage) -> Result<(), BuildError> {
    let signing_key = SigningKey::from_bytes(&config.signing.signing_seed);
    let trust = TrustRoot::new(vec![TrustAnchor {
        key_id: config.signing.key_id,
        public_key: signing_key.verifying_key().to_bytes(),
    }]);
    verify_package(
        package,
        &trust,
        config.identity.effectivity.effective,
        None,
        UsePolicy::SimulatorPermitted,
    )
    .map(|_| ())
    .map_err(|source| BuildError::SelfVerification { source })
}

/// The canonical bytes of a signed package: the manifest canonical bytes, the
/// signature, then every tile's canonical bytes in key order. This is the
/// artifact a reproducibility check compares — a byte-identical value across two
/// builds proves the whole signed package reproduced.
#[must_use]
pub fn canonical_package_bytes(package: &CandidatePackage) -> Vec<u8> {
    let mut out = manifest_canonical_bytes(&package.manifest);
    out.extend_from_slice(&package.manifest.signature.bytes);
    let mut tiles: Vec<&Tile> = package.tiles.iter().collect();
    tiles.sort_by_key(|tile| tile.key);
    for tile in tiles {
        out.extend_from_slice(&tile_canonical_bytes(tile));
    }
    out
}
