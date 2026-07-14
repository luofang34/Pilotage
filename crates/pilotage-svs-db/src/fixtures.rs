//! Deterministic in-memory fixtures for the tests.
//!
//! Signing lives here, not in the library: these fixtures play the offline
//! publisher that produces a signed package, while the crate under test only
//! verifies. Keys are derived from fixed seeds, so every signature and hash is
//! reproducible with no randomness and no I/O.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use ed25519_dalek::{Signer, SigningKey};
use pilotage_geo::{
    BaroSettingId, DatumRealizationId, GeoTile, GeodeticPosition, GeoidModelId, HorizontalDatum,
    IntegrityLevel, LocalOriginId, TerrainRefId, VerticalDatum, VerticalPosition,
};

use crate::canonical::{manifest_canonical_bytes, tile_canonical_bytes};
use crate::feature::{FeatureClass, FeatureSet};
use crate::identity::{DatasetId, DayNumber, PackageVersion, ProviderId};
use crate::manifest::{
    Accuracy, ContentSpec, Coverage, CoverageBox, Effectivity, MANIFEST_SCHEMA_VERSION,
    PackageManifest, ProcessingChain, ProcessingStep, Provenance, Resolution, UseRestrictions,
};
use crate::merkle::{TileRoot, merkle_root, tile_leaf_hash};
use crate::tile::{CandidatePackage, Tile, TileKey};
use crate::trust::{PackageSignature, TrustAnchor, TrustKeyId, TrustRoot};

/// The current day the valid fixtures are effective on.
pub(crate) const NOW: DayNumber = DayNumber(150);

/// The trust key id the fixtures sign under.
pub(crate) const KEY_ID: TrustKeyId = TrustKeyId(0xA1);

/// The publisher's signing key, from a fixed seed.
pub(crate) fn signing_key() -> SigningKey {
    SigningKey::from_bytes(&[7u8; 32])
}

/// A trust root that trusts the fixture publisher's key under [`KEY_ID`].
pub(crate) fn trust_root() -> TrustRoot {
    TrustRoot::new(vec![TrustAnchor {
        key_id: KEY_ID,
        public_key: signing_key().verifying_key().to_bytes(),
    }])
}

/// A trust root that binds [`KEY_ID`] to a *different* public key, so a
/// correctly-signed fixture fails signature verification against it.
pub(crate) fn wrong_key_trust_root() -> TrustRoot {
    TrustRoot::new(vec![TrustAnchor {
        key_id: KEY_ID,
        public_key: SigningKey::from_bytes(&[9u8; 32])
            .verifying_key()
            .to_bytes(),
    }])
}

/// A geodetic position at `lat`/`lon` on WGS-84 with an ellipsoidal height, for
/// coverage queries.
pub(crate) fn position(lat_deg: f64, lon_deg: f64) -> GeodeticPosition {
    let vertical = VerticalPosition::new(
        0.0,
        VerticalDatum::Ellipsoid,
        GeoidModelId::UNDECLARED,
        TerrainRefId::UNDECLARED,
        BaroSettingId::UNDECLARED,
        LocalOriginId::UNDECLARED,
    )
    .expect("ellipsoidal vertical position is valid");
    GeodeticPosition::new(
        lat_deg,
        lon_deg,
        HorizontalDatum::Wgs84,
        DatumRealizationId::UNDECLARED,
        vertical,
    )
    .expect("wgs84 position is valid")
}

/// Three terrain tiles with distinct keys and payloads.
pub(crate) fn tiles() -> Vec<Tile> {
    (0..3)
        .map(|i| Tile {
            key: TileKey {
                class: FeatureClass::Terrain,
                level: 0,
                tile: GeoTile {
                    lat_index: i,
                    lon_index: 0,
                },
            },
            bytes: vec![i as u8; 16],
        })
        .collect()
}

/// Three terrain tiles whose payloads are uniformly `tag`, so two versions built
/// with different tags have completely disjoint content and any mix is visible.
pub(crate) fn tiles_tagged(tag: u8) -> Vec<Tile> {
    (0..3)
        .map(|i| Tile {
            key: TileKey {
                class: FeatureClass::Terrain,
                level: 0,
                tile: GeoTile {
                    lat_index: i,
                    lon_index: 0,
                },
            },
            bytes: vec![tag; 16],
        })
        .collect()
}

/// The tile-root over `tiles`, computed the same way verification recomputes it.
pub(crate) fn tile_root(tiles: &[Tile]) -> TileRoot {
    let mut entries: Vec<(TileKey, [u8; 32])> = tiles
        .iter()
        .map(|t| (t.key, tile_leaf_hash(&tile_canonical_bytes(t))))
        .collect();
    entries.sort_by_key(|entry| entry.0);
    let leaves: Vec<[u8; 32]> = entries.into_iter().map(|(_, leaf)| leaf).collect();
    merkle_root(&leaves)
}

/// An unsigned manifest over `tiles` with the given dataset, version, and
/// simulation flag. Coverage is a small WGS-84 box; effectivity is `[100, 200]`.
pub(crate) fn base_manifest(
    tiles: &[Tile],
    dataset: DatasetId,
    version: PackageVersion,
    simulation_only: bool,
    restrictions: UseRestrictions,
) -> PackageManifest {
    PackageManifest {
        schema_version: MANIFEST_SCHEMA_VERSION,
        provenance: Provenance {
            dataset,
            provider: ProviderId(0xB2),
            version,
            processing: ProcessingChain(vec![ProcessingStep {
                code: 1,
                tool_id: 42,
            }]),
            restrictions,
        },
        effectivity: Effectivity {
            release: DayNumber(100),
            effective: DayNumber(100),
            expiry: DayNumber(200),
        },
        coverage: Coverage {
            region: CoverageBox {
                min_lat_deg: 40.0,
                max_lat_deg: 41.0,
                min_lon_deg: -75.0,
                max_lon_deg: -74.0,
            },
            horizontal_datum: HorizontalDatum::Wgs84,
            realization: DatumRealizationId::UNDECLARED,
            vertical_datum: VerticalDatum::Ellipsoid,
            geoid: GeoidModelId::UNDECLARED,
            resolution: Resolution {
                post_spacing_mm: 30_000,
            },
        },
        content: ContentSpec {
            features: FeatureSet::empty().with(FeatureClass::Terrain),
            accuracy: Accuracy {
                horizontal_mm: 3_000,
                vertical_mm: 5_000,
            },
            integrity: IntegrityLevel::Monitored,
        },
        tile_count: tiles.len() as u32,
        tile_root: tile_root(tiles),
        simulation_only,
        signature: PackageSignature {
            key_id: KEY_ID,
            bytes: [0u8; 64],
        },
    }
}

/// Signs (or re-signs) a manifest in place with the fixture key, over its
/// current canonical bytes.
pub(crate) fn sign(manifest: &mut PackageManifest) {
    let bytes = manifest_canonical_bytes(manifest);
    manifest.signature.bytes = signing_key().sign(&bytes).to_bytes();
}

/// A default valid, signed candidate package: dataset `1`, version `1.0.0`,
/// three terrain tiles, `simulation_only = true`, `NO_OPERATIONAL_USE`.
pub(crate) fn candidate() -> CandidatePackage {
    candidate_with(DatasetId(1), PackageVersion::new(1, 0, 0), true)
}

/// A valid, signed candidate package with the given dataset, version, and
/// simulation flag, carrying the `NO_OPERATIONAL_USE` restriction (the SIM
/// default).
pub(crate) fn candidate_with(
    dataset: DatasetId,
    version: PackageVersion,
    simulation_only: bool,
) -> CandidatePackage {
    let tiles = tiles();
    let mut manifest = base_manifest(
        &tiles,
        dataset,
        version,
        simulation_only,
        UseRestrictions::NO_OPERATIONAL_USE,
    );
    sign(&mut manifest);
    CandidatePackage { manifest, tiles }
}

/// A valid, signed candidate package with `tag`-uniform tiles, so distinct tags
/// give content-disjoint versions for the no-mixing test.
pub(crate) fn candidate_tagged(
    dataset: DatasetId,
    version: PackageVersion,
    tag: u8,
) -> CandidatePackage {
    let tiles = tiles_tagged(tag);
    let mut manifest = base_manifest(&tiles, dataset, version, false, UseRestrictions::NONE);
    sign(&mut manifest);
    CandidatePackage { manifest, tiles }
}

/// A signed package that is genuinely operational: not `simulation_only` and
/// with no use restrictions.
pub(crate) fn operational_candidate() -> CandidatePackage {
    let tiles = tiles();
    let mut manifest = base_manifest(
        &tiles,
        DatasetId(1),
        PackageVersion::new(1, 0, 0),
        false,
        UseRestrictions::NONE,
    );
    sign(&mut manifest);
    CandidatePackage { manifest, tiles }
}

/// A signed package that is not `simulation_only` but carries the
/// `NO_OPERATIONAL_USE` restriction, so operational use must still be refused.
pub(crate) fn restricted_candidate() -> CandidatePackage {
    let tiles = tiles();
    let mut manifest = base_manifest(
        &tiles,
        DatasetId(1),
        PackageVersion::new(1, 0, 0),
        false,
        UseRestrictions::NO_OPERATIONAL_USE,
    );
    sign(&mut manifest);
    CandidatePackage { manifest, tiles }
}

/// A signed package carrying an unknown use-restriction bit (outside
/// `KNOWN_MASK`), which must be refused at validation rather than assumed
/// permissive.
pub(crate) fn unknown_restriction_candidate() -> CandidatePackage {
    let tiles = tiles();
    let mut manifest = base_manifest(
        &tiles,
        DatasetId(1),
        PackageVersion::new(1, 0, 0),
        false,
        UseRestrictions(1 << 31),
    );
    sign(&mut manifest);
    CandidatePackage { manifest, tiles }
}
