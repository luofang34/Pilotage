//! Independent verification of a build artifact.
//!
//! [`verify_artifact`] accepts an artifact only when the package passes the
//! SVS-02 verifier, the provenance+report bundle signature verifies (so the
//! provenance and reports could not have been altered), the bundle is bound to
//! this exact package, the reports agree with the independently decoded package,
//! and the decoded package records and the lineage records are in exact 1:1
//! correspondence — no untraceable output record and no lineage entry without a
//! record. [`verify_source_digests`] additionally checks the recorded source
//! content digests against the source input, catching a provenance whose sources
//! were substituted or altered.

mod bijection;
mod sources;

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
use crate::payload::{decode_aerodromes, decode_obstacles, decode_runways, decode_terrain};
use crate::provenance::RecordKey;
use crate::source::SourceDataset;

use bijection::check_bijection;
use sources::check_lineage_sources;
pub use sources::verify_source_digests;

/// The identity of one emitted record: its class, tile, and decodable key.
pub(crate) type RecordIdentity = (u8, i32, i32, RecordKey);

/// The report fields re-derived by decoding the produced package.
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
    /// Whether every decoded record lands in the tile that holds it and no
    /// terrain node appears in more than one tile.
    pub seam_ok: bool,
}

/// The full result of decoding a package: its reports and every record identity.
struct DecodedPackage {
    reports: DecodedReports,
    identities: Vec<RecordIdentity>,
}

/// The datum and tile size used to recompute tile assignment.
type TilingContext = (HorizontalDatum, DatumRealizationId, f64);

/// Verifies an artifact end to end AND its recorded source digests against
/// the source dataset, in one call. This is the entry point to use whenever
/// the source input is at hand: source verification cannot be skipped by
/// forgetting a second call.
///
/// # Errors
///
/// Every [`verify_artifact`] error, plus the source-digest errors of
/// [`verify_source_digests`].
pub fn verify_artifact_with_sources(
    artifact: &BuildArtifact,
    source: &SourceDataset,
    trust: &TrustRoot,
    now: DayNumber,
    active: Option<ActiveDbId>,
    policy: UsePolicy,
) -> Result<(), VerifyError> {
    verify_artifact(artifact, trust, now, active, policy)?;
    verify_source_digests(artifact, source)
}

/// Verifies an artifact end to end: package, bundle signature and binding,
/// decoded reports, and the record-lineage bijection. Prefer
/// [`verify_artifact_with_sources`] whenever the source dataset is at hand —
/// this variant alone cannot check the recorded source digests.
///
/// # Errors
///
/// A [`VerifyError`] for a rejected package, an invalid or unbound bundle
/// signature, reports that disagree with the decoded package, or a broken
/// record-lineage bijection.
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
    let decoded = decode_package(artifact)?;
    check_reports(artifact, &decoded.reports)?;
    check_bijection(artifact, &decoded.identities)?;
    check_lineage_sources(artifact)?;
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
fn check_reports(artifact: &BuildArtifact, decoded: &DecodedReports) -> Result<(), VerifyError> {
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

/// Decodes the package and re-derives the report fields (with an independent seam
/// check).
///
/// # Errors
///
/// [`VerifyError::PayloadDecode`] for a malformed tile payload.
pub fn decode_package_reports(artifact: &BuildArtifact) -> Result<DecodedReports, VerifyError> {
    Ok(decode_package(artifact)?.reports)
}

/// One decode pass over the package producing both the reports and the record
/// identities.
fn decode_package(artifact: &BuildArtifact) -> Result<DecodedPackage, VerifyError> {
    let ctx = tiling_context(artifact);
    let grid = (
        artifact.reports.coverage.min_lat_deg,
        artifact.reports.coverage.min_lon_deg,
        artifact.provenance.params.post_spacing_deg,
    );
    let mut state = DecodeState::default();
    for tile in &artifact.package.tiles {
        decode_tile(tile, ctx, grid, &mut state)?;
    }
    Ok(DecodedPackage {
        reports: state.counts.into_reports(state.seam_ok),
        identities: state.identities,
    })
}

/// The mutable accumulator threaded through the decode pass. `seam_ok` starts
/// `true` and is cleared by any record that fails its tile or uniqueness check.
struct DecodeState {
    counts: ClassCounts,
    identities: Vec<RecordIdentity>,
    seam_ok: bool,
    seen: BTreeSet<(u32, u32)>,
}

impl Default for DecodeState {
    fn default() -> Self {
        Self {
            counts: ClassCounts::default(),
            identities: Vec::new(),
            seam_ok: true,
            seen: BTreeSet::new(),
        }
    }
}

/// Decodes one tile into the accumulator.
fn decode_tile(
    tile: &pilotage_svs_db::Tile,
    ctx: TilingContext,
    grid: (f64, f64, f64),
    state: &mut DecodeState,
) -> Result<(), VerifyError> {
    let (li, lo) = (tile.key.tile.lat_index, tile.key.tile.lon_index);
    match tile.key.class {
        FeatureClass::Terrain => decode_terrain_tile(tile, li, lo, ctx, grid, state),
        FeatureClass::Obstacles => decode_obstacle_tile(tile, li, lo, ctx, state),
        FeatureClass::Aerodromes => decode_aerodrome_tile(tile, li, lo, ctx, state),
        FeatureClass::Runways => decode_runway_tile(tile, li, lo, ctx, state),
        // This build never emits taxiway tiles.
        FeatureClass::Taxiways => Ok(()),
    }
}

/// Decodes a terrain tile: counts, seam check, node identities.
fn decode_terrain_tile(
    tile: &pilotage_svs_db::Tile,
    li: i32,
    lo: i32,
    ctx: TilingContext,
    grid: (f64, f64, f64),
    state: &mut DecodeState,
) -> Result<(), VerifyError> {
    let (min_lat, min_lon, spacing) = grid;
    let posts =
        decode_terrain(&tile.bytes).ok_or(VerifyError::PayloadDecode { reason: "terrain" })?;
    state.counts.terrain_tiles += 1;
    state.counts.terrain_posts += posts.len() as u32;
    for post in &posts {
        let lat = min_lat + f64::from(post.i) * spacing;
        let lon = min_lon + f64::from(post.j) * spacing;
        state.seam_ok &= post.elevation_m.is_finite()
            && state.seen.insert((post.i, post.j))
            && lands_in(li, lo, ctx, lat, lon);
        state.identities.push((
            FeatureClass::Terrain.to_u8(),
            li,
            lo,
            RecordKey::TerrainNode {
                i: post.i,
                j: post.j,
            },
        ));
    }
    Ok(())
}

/// Decodes an obstacle tile: count, seam check, obstacle identities.
fn decode_obstacle_tile(
    tile: &pilotage_svs_db::Tile,
    li: i32,
    lo: i32,
    ctx: TilingContext,
    state: &mut DecodeState,
) -> Result<(), VerifyError> {
    let obstacles = decode_obstacles(&tile.bytes).ok_or(VerifyError::PayloadDecode {
        reason: "obstacles",
    })?;
    state.counts.obstacle_tiles += 1;
    state.counts.obstacles += obstacles.len() as u32;
    for obstacle in &obstacles {
        state.seam_ok &= lands_in(li, lo, ctx, obstacle.lat_deg, obstacle.lon_deg);
        state.identities.push((
            FeatureClass::Obstacles.to_u8(),
            li,
            lo,
            RecordKey::Obstacle {
                lat_bits: obstacle.lat_deg.to_bits(),
                lon_bits: obstacle.lon_deg.to_bits(),
                kind: obstacle.kind,
            },
        ));
    }
    Ok(())
}

/// Decodes an aerodrome tile: count, seam check, aerodrome identities.
fn decode_aerodrome_tile(
    tile: &pilotage_svs_db::Tile,
    li: i32,
    lo: i32,
    ctx: TilingContext,
    state: &mut DecodeState,
) -> Result<(), VerifyError> {
    let aerodromes = decode_aerodromes(&tile.bytes).ok_or(VerifyError::PayloadDecode {
        reason: "aerodromes",
    })?;
    state.counts.aerodrome_tiles += 1;
    for aerodrome in &aerodromes {
        state.seam_ok &= lands_in(li, lo, ctx, aerodrome.lat_deg, aerodrome.lon_deg);
        state.identities.push((
            FeatureClass::Aerodromes.to_u8(),
            li,
            lo,
            RecordKey::Aerodrome {
                ident: aerodrome.ident,
            },
        ));
    }
    Ok(())
}

/// Decodes a runway tile: count, seam check, runway identities.
fn decode_runway_tile(
    tile: &pilotage_svs_db::Tile,
    li: i32,
    lo: i32,
    ctx: TilingContext,
    state: &mut DecodeState,
) -> Result<(), VerifyError> {
    let runways =
        decode_runways(&tile.bytes).ok_or(VerifyError::PayloadDecode { reason: "runways" })?;
    state.counts.runway_tiles += 1;
    for runway in &runways {
        state.seam_ok &= lands_in(li, lo, ctx, runway.end_a_lat_deg, runway.end_a_lon_deg);
        state.identities.push((
            FeatureClass::Runways.to_u8(),
            li,
            lo,
            RecordKey::Runway {
                designator: runway.designator,
                end_a_lat_bits: runway.end_a_lat_deg.to_bits(),
                end_a_lon_bits: runway.end_a_lon_deg.to_bits(),
            },
        ));
    }
    Ok(())
}

/// Whether `(lat, lon)` tiles to `(lat_index, lon_index)` under `ctx`.
fn lands_in(lat_index: i32, lon_index: i32, ctx: TilingContext, lat: f64, lon: f64) -> bool {
    let (horizontal, realization, tile_deg) = ctx;
    match tile_of(horizontal, realization, tile_deg, 0, lat, lon) {
        Ok(tile) => tile.lat_index == lat_index && tile.lon_index == lon_index,
        Err(_) => false,
    }
}

/// The datum and tile size to recompute tile assignment, from the provenance
/// parameters. An unknown recorded datum code falls back to WGS-84, which the
/// seam cross-check then surfaces as a mismatch.
fn tiling_context(artifact: &BuildArtifact) -> TilingContext {
    let params = &artifact.provenance.params;
    let horizontal =
        HorizontalDatum::from_u8(params.target_horizontal).unwrap_or(HorizontalDatum::Wgs84);
    (
        horizontal,
        DatumRealizationId(params.target_realization),
        params.tile_deg,
    )
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
