//! Independent verification of a build artifact.
//!
//! [`verify_artifact`] is the counterpart to the build: it accepts an artifact
//! only when four things all hold — the package passes the SVS-02 verifier, the
//! provenance+report bundle signature verifies against the trust root (so the
//! provenance and reports could not have been altered), the bundle is bound to
//! this exact package (its recorded content hash matches), and the reports agree
//! with what the package actually contains when independently decoded. The last
//! check is derived by [`decode_package_reports`], which decodes the produced
//! tiles rather than trusting the pipeline's own counters, so a package that
//! disagrees with its reports is caught.

#[cfg(test)]
mod tests;

use std::collections::BTreeSet;

use ed25519_dalek::{Signature, VerifyingKey};
use pilotage_geo::{DatumRealizationId, HorizontalDatum};
use pilotage_svs_db::{
    ActiveDbId, DayNumber, FeatureClass, TrustRoot, UsePolicy, manifest_content_hash,
    verify_package,
};

use crate::bundle::canonical_bundle_bytes;
use crate::chain::{BuildArtifact, tile_of};
use crate::error::VerifyError;
use crate::payload::{
    decode_aerodrome_count, decode_obstacles, decode_runway_count, decode_terrain,
};

/// The report fields re-derived by decoding the produced package, independent of
/// the pipeline's own counters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DecodedReports {
    /// Terrain tiles found in the package.
    pub terrain_tiles: u32,
    /// Obstacle tiles found.
    pub obstacle_tiles: u32,
    /// Aerodrome tiles found.
    pub aerodrome_tiles: u32,
    /// Runway tiles found.
    pub runway_tiles: u32,
    /// Terrain posts decoded.
    pub terrain_posts: u32,
    /// Obstacles decoded.
    pub obstacles: u32,
    /// Populated grid nodes (equal to decoded terrain posts).
    pub covered_nodes: u32,
    /// Whether every decoded terrain post lands in the tile that holds it and no
    /// node appears in more than one tile.
    pub seam_ok: bool,
}

/// Verifies an artifact end to end.
///
/// # Errors
///
/// A [`VerifyError`] for a rejected package, an invalid or unbound bundle
/// signature, or reports that disagree with the decoded package.
pub fn verify_artifact(
    artifact: &BuildArtifact,
    trust: &TrustRoot,
    now: DayNumber,
    active: Option<ActiveDbId>,
    policy: UsePolicy,
) -> Result<(), VerifyError> {
    verify_package(&artifact.package, trust, now, active, policy)
        .map_err(|source| VerifyError::Package { source })?;
    verify_bundle(artifact, trust)?;
    verify_binding(artifact)?;
    verify_reports(artifact)?;
    Ok(())
}

/// Checks the bundle signature over the canonical provenance+report bytes.
fn verify_bundle(artifact: &BuildArtifact, trust: &TrustRoot) -> Result<(), VerifyError> {
    let bytes = canonical_bundle_bytes(&artifact.provenance, &artifact.reports)
        .map_err(|source| VerifyError::BundleSerialization { source })?;
    let key_id = artifact.package.manifest.signature.key_id;
    let public_key = trust
        .public_key(key_id)
        .ok_or(VerifyError::BundleSignatureInvalid)?;
    let verifying_key =
        VerifyingKey::from_bytes(&public_key).map_err(|_| VerifyError::BundleSignatureInvalid)?;
    let signature = Signature::from_bytes(&artifact.bundle_signature);
    verifying_key
        .verify_strict(&bytes, &signature)
        .map_err(|_| VerifyError::BundleSignatureInvalid)
}

/// Checks the provenance names this exact package by content hash.
fn verify_binding(artifact: &BuildArtifact) -> Result<(), VerifyError> {
    let actual = manifest_content_hash(&artifact.package.manifest);
    if artifact.provenance.package_content_hash == actual {
        Ok(())
    } else {
        Err(VerifyError::BundleBindingMismatch)
    }
}

/// Re-derives the reports by decoding the package and checks them against the
/// artifact's reports.
fn verify_reports(artifact: &BuildArtifact) -> Result<(), VerifyError> {
    let decoded = decode_package_reports(artifact)?;
    let claimed = &artifact.reports.coverage;
    check_eq(
        "terrain_tiles",
        claimed.terrain_tiles,
        decoded.terrain_tiles,
    )?;
    check_eq(
        "obstacle_tiles",
        claimed.obstacle_tiles,
        decoded.obstacle_tiles,
    )?;
    check_eq(
        "aerodrome_tiles",
        claimed.aerodrome_tiles,
        decoded.aerodrome_tiles,
    )?;
    check_eq("runway_tiles", claimed.runway_tiles, decoded.runway_tiles)?;
    check_eq(
        "terrain_posts",
        claimed.terrain_posts,
        decoded.terrain_posts,
    )?;
    check_eq("obstacles", claimed.obstacles, decoded.obstacles)?;
    check_eq(
        "covered_nodes",
        claimed.covered_nodes,
        decoded.covered_nodes,
    )?;
    if !decoded.seam_ok || artifact.reports.seam.ok != decoded.seam_ok {
        return Err(VerifyError::ReportMismatch { field: "seam" });
    }
    Ok(())
}

/// Fails with a [`VerifyError::ReportMismatch`] when two counts disagree.
fn check_eq(field: &'static str, claimed: u32, decoded: u32) -> Result<(), VerifyError> {
    if claimed == decoded {
        Ok(())
    } else {
        Err(VerifyError::ReportMismatch { field })
    }
}

/// Decodes the package and re-derives the package-observable report fields,
/// including an independent seam check that each terrain post lands in the tile
/// holding it.
///
/// # Errors
///
/// [`VerifyError::PayloadDecode`] for a malformed tile payload.
pub fn decode_package_reports(artifact: &BuildArtifact) -> Result<DecodedReports, VerifyError> {
    let package = &artifact.package;
    let (horizontal, realization, tile_deg) = tiling_context(artifact);
    let (min_lat, min_lon) = (
        artifact.reports.coverage.min_lat_deg,
        artifact.reports.coverage.min_lon_deg,
    );
    let spacing = artifact.provenance.params.post_spacing_deg;
    let mut counts = ClassCounts::default();
    let mut seam_ok = true;
    let mut seen: BTreeSet<(u32, u32)> = BTreeSet::new();
    for tile in &package.tiles {
        match tile.key.class {
            FeatureClass::Terrain => {
                let posts = decode_terrain(&tile.bytes)
                    .ok_or(VerifyError::PayloadDecode { reason: "terrain" })?;
                counts.terrain_tiles += 1;
                counts.terrain_posts += posts.len() as u32;
                seam_ok &= terrain_tile_ok(
                    tile,
                    &posts,
                    (horizontal, realization, tile_deg),
                    (min_lat, min_lon, spacing),
                    &mut seen,
                );
            }
            FeatureClass::Obstacles => {
                let obstacles =
                    decode_obstacles(&tile.bytes).ok_or(VerifyError::PayloadDecode {
                        reason: "obstacles",
                    })?;
                counts.obstacle_tiles += 1;
                counts.obstacles += obstacles.len() as u32;
                seam_ok &= obstacle_tile_ok(tile, &obstacles, (horizontal, realization, tile_deg));
            }
            FeatureClass::Aerodromes => {
                decode_aerodrome_count(&tile.bytes).ok_or(VerifyError::PayloadDecode {
                    reason: "aerodromes",
                })?;
                counts.aerodrome_tiles += 1;
            }
            FeatureClass::Runways => {
                decode_runway_count(&tile.bytes)
                    .ok_or(VerifyError::PayloadDecode { reason: "runways" })?;
                counts.runway_tiles += 1;
            }
            // This build never emits taxiway tiles; the reports carry no taxiway
            // field, so there is nothing to cross-check for one.
            FeatureClass::Taxiways => {}
        }
    }
    Ok(counts.into_reports(seam_ok))
}

/// The datum and tile size to recompute tile assignment, from the provenance
/// parameters. Falls back to WGS-84 if the recorded datum code is unknown, which
/// the report cross-check then surfaces as a seam mismatch.
fn tiling_context(artifact: &BuildArtifact) -> (HorizontalDatum, DatumRealizationId, f64) {
    let params = &artifact.provenance.params;
    let horizontal =
        HorizontalDatum::from_u8(params.target_horizontal).unwrap_or(HorizontalDatum::Wgs84);
    (
        horizontal,
        DatumRealizationId(params.target_realization),
        params.tile_deg,
    )
}

/// Whether every post in a terrain tile lands in that tile and is globally
/// unique.
fn terrain_tile_ok(
    tile: &pilotage_svs_db::Tile,
    posts: &[crate::payload::DecodedPost],
    datum: (HorizontalDatum, DatumRealizationId, f64),
    grid: (f64, f64, f64),
    seen: &mut BTreeSet<(u32, u32)>,
) -> bool {
    let (horizontal, realization, tile_deg) = datum;
    let (min_lat, min_lon, spacing) = grid;
    for post in posts {
        if !post.elevation_m.is_finite() || !seen.insert((post.i, post.j)) {
            return false;
        }
        let lat = min_lat + f64::from(post.i) * spacing;
        let lon = min_lon + f64::from(post.j) * spacing;
        match tile_of(horizontal, realization, tile_deg, 0, lat, lon) {
            Ok(computed) => {
                if computed.lat_index != tile.key.tile.lat_index
                    || computed.lon_index != tile.key.tile.lon_index
                {
                    return false;
                }
            }
            Err(_) => return false,
        }
    }
    true
}

/// Whether every obstacle in a tile lands in that tile.
fn obstacle_tile_ok(
    tile: &pilotage_svs_db::Tile,
    obstacles: &[crate::payload::DecodedObstacle],
    datum: (HorizontalDatum, DatumRealizationId, f64),
) -> bool {
    let (horizontal, realization, tile_deg) = datum;
    for obstacle in obstacles {
        match tile_of(
            horizontal,
            realization,
            tile_deg,
            0,
            obstacle.lat_deg,
            obstacle.lon_deg,
        ) {
            Ok(computed) => {
                if computed.lat_index != tile.key.tile.lat_index
                    || computed.lon_index != tile.key.tile.lon_index
                {
                    return false;
                }
            }
            Err(_) => return false,
        }
    }
    true
}

/// Accumulates decoded tile and element counts.
#[derive(Default)]
struct ClassCounts {
    terrain_tiles: u32,
    obstacle_tiles: u32,
    aerodrome_tiles: u32,
    runway_tiles: u32,
    terrain_posts: u32,
    obstacles: u32,
}

impl ClassCounts {
    /// Finishes into a [`DecodedReports`] with the seam verdict.
    fn into_reports(self, seam_ok: bool) -> DecodedReports {
        DecodedReports {
            terrain_tiles: self.terrain_tiles,
            obstacle_tiles: self.obstacle_tiles,
            aerodrome_tiles: self.aerodrome_tiles,
            runway_tiles: self.runway_tiles,
            terrain_posts: self.terrain_posts,
            obstacles: self.obstacles,
            covered_nodes: self.terrain_posts,
            seam_ok,
        }
    }
}
