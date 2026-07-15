//! The processing chain: the deterministic, fail-closed path from a source
//! dataset to a signed, verifiable SVS-02 package.
//!
//! [`build_package`] runs the stages in a fixed order — validate, reject
//! outliers, convert datums, interpolate terrain (with the hole policy), clip to
//! coverage, tile (seam handling by flooring), merge obstacles, and serialize —
//! and then assembles, signs, and self-verifies the package. Any stage failure
//! returns a [`BuildError`] and emits nothing, so a tool fault can never
//! activate a partial package. Every stage records its lineage, so the emitted
//! provenance traces every output tile back to its source records.
//!
//! Geospatial tiling reuses [`pilotage_geo::GeodeticPosition::tile`], so seam,
//! anti-meridian, and pole handling are exactly the contract SVS-02 consumes.

mod aerodrome;
mod assemble;
mod obstacle;
mod terrain;

#[cfg(test)]
mod tests;

use ed25519_dalek::SigningKey;
use pilotage_geo::{
    BaroSettingId, DatumRealizationId, GeoTile, GeodeticPosition, GeoidModelId, HorizontalDatum,
    LocalOriginId, TerrainRefId, VerticalDatum, VerticalPosition,
};
use pilotage_svs_db::{CandidatePackage, Tile, TrustAnchor, TrustRoot, UsePolicy};

use crate::config::BuildConfig;
use crate::error::BuildError;
use crate::provenance::{
    BuildProvenance, RecordDisposition, RecordLineage, StageRecord, TileLineage,
};
use crate::report::{BuildReports, VoidNode};
use crate::source::{SourceDataset, SourceId};

/// Ingest stage code.
pub(crate) const STAGE_INGEST: u16 = 1;
/// Outlier-rejection stage code.
pub(crate) const STAGE_OUTLIER: u16 = 2;
/// Datum-conversion stage code.
pub(crate) const STAGE_DATUM: u16 = 3;
/// Interpolation stage code.
pub(crate) const STAGE_INTERPOLATE: u16 = 4;
/// Clipping stage code.
pub(crate) const STAGE_CLIP: u16 = 5;
/// Tiling stage code.
pub(crate) const STAGE_TILE: u16 = 6;
/// Obstacle-merge stage code.
pub(crate) const STAGE_MERGE: u16 = 7;
/// Serialize stage code.
pub(crate) const STAGE_SERIALIZE: u16 = 8;

/// The signed processing lineage, in execution order, every step performed by
/// this build tool.
pub(crate) const STAGE_CODES: [u16; 8] = [
    STAGE_INGEST,
    STAGE_OUTLIER,
    STAGE_DATUM,
    STAGE_INTERPOLATE,
    STAGE_CLIP,
    STAGE_TILE,
    STAGE_MERGE,
    STAGE_SERIALIZE,
];

/// The output of a per-feature pipeline: the tiles it produced and the lineage,
/// change-report, and stage records it accumulated.
pub(crate) struct PipelineOutput {
    /// The tiles the pipeline produced.
    pub tiles: Vec<Tile>,
    /// Per-tile lineage.
    pub lineages: Vec<TileLineage>,
    /// Per-record lineage for the records this pipeline emitted.
    pub records: Vec<RecordLineage>,
    /// The change report for the pipeline's records.
    pub dispositions: Vec<RecordDisposition>,
    /// The stages the pipeline ran.
    pub stages: Vec<StageRecord>,
    /// The metrics the reports aggregate.
    pub metrics: Metrics,
}

/// Per-pipeline counters the reports roll up. Fields irrelevant to a pipeline
/// stay zero.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Metrics {
    /// Records rejected as outliers.
    pub outliers: u32,
    /// Records clipped for leaving coverage.
    pub clipped: u32,
    /// Terrain nodes left void.
    pub holes: u32,
    /// Source obstacles merged into another.
    pub merged: u32,
    /// Terrain posts emitted.
    pub terrain_posts: u32,
    /// Grid nodes populated.
    pub covered_nodes: u32,
    /// Grid nodes the coverage implies.
    pub total_nodes: u32,
    /// Obstacles emitted after merging.
    pub obstacles: u32,
    /// Terrain tiles emitted.
    pub terrain_tiles: u32,
    /// Obstacle tiles emitted.
    pub obstacle_tiles: u32,
    /// Aerodrome tiles emitted.
    pub aerodrome_tiles: u32,
    /// Runway tiles emitted.
    pub runway_tiles: u32,
    /// The voided nodes, for the hole check.
    pub voids: Vec<VoidNode>,
}

/// A build's deliverables: the signed package, its structured provenance, the
/// coverage/quality/seam/hole reports, and the Ed25519 signature that binds the
/// provenance+report bundle so it cannot be altered without detection.
#[derive(Debug, Clone, PartialEq)]
pub struct BuildArtifact {
    /// The signed, verifiable SVS-02 candidate package.
    pub package: CandidatePackage,
    /// The structured input-to-output provenance.
    pub provenance: BuildProvenance,
    /// The coverage, quality, seam, and hole reports.
    pub reports: BuildReports,
    /// The Ed25519 signature over the canonical provenance+report bundle, under
    /// the same key that signed the package. Any mutation of the provenance or
    /// reports invalidates it (see [`crate::verify_artifact`]).
    pub bundle_signature: [u8; 64],
}

/// Runs the full chain and returns the package, provenance, and reports.
///
/// # Errors
///
/// A [`BuildError`] from the first failing stage; on error nothing is emitted.
pub fn build_package(
    config: &BuildConfig,
    source: &SourceDataset,
) -> Result<BuildArtifact, BuildError> {
    config.validate()?;
    validate_sources(config, source)?;
    let terrain = terrain::process(config, source)?;
    let obstacles = obstacle::process(config, source)?;
    let aero = aerodrome::process(config, source)?;
    let mut tiles: Vec<Tile> = Vec::new();
    tiles.extend(terrain.tiles.iter().cloned());
    tiles.extend(obstacles.tiles.iter().cloned());
    tiles.extend(aero.tiles.iter().cloned());
    if tiles.is_empty() {
        return Err(BuildError::EmptyOutput);
    }
    let signed = crate::package::assemble_and_sign(config, source, &tiles)?;
    let outputs = [&terrain, &obstacles, &aero];
    let reports = assemble::build_reports(config, &outputs);
    let provenance = assemble::build_provenance(config, source, &outputs, signed.content_hash);
    let bundle_signature =
        crate::package::sign_bundle(&config.signing.signing_seed, &provenance, &reports)?;
    let artifact = BuildArtifact {
        package: signed.package,
        provenance,
        reports,
        bundle_signature,
    };
    // The build never emits an artifact the full verifier would reject: the
    // finished artifact is verified end to end — package, bundle binding,
    // provenance decode, lineage bijection, AND source digests — under a trust
    // root derived from its own signing key, at the package's effectivity
    // start, before it is returned.
    let key = SigningKey::from_bytes(&config.signing.signing_seed);
    let trust = TrustRoot::new(vec![TrustAnchor {
        key_id: config.signing.key_id,
        public_key: key.verifying_key().to_bytes(),
    }]);
    let policy = if config.identity.simulation_only {
        UsePolicy::SimulatorPermitted
    } else {
        UsePolicy::OperationalRequired
    };
    crate::verify::verify_artifact_with_sources(
        &artifact,
        source,
        &trust,
        config.identity.effectivity.effective,
        None,
        policy,
    )
    .map_err(|source| BuildError::ArtifactSelfVerification { source })?;
    Ok(artifact)
}

/// Validates that every source referenced has metadata and that its datums are
/// known and carry the identities they need. An unknown or under-declared source
/// datum aborts the build rather than being guessed.
fn validate_sources(config: &BuildConfig, source: &SourceDataset) -> Result<(), BuildError> {
    validate_target_datum(config)?;
    // A duplicated source id makes the governing metadata (datum, license,
    // version) ambiguous; the build refuses rather than picking the first.
    let mut declared = std::collections::BTreeSet::new();
    for meta in &source.meta {
        if !declared.insert(meta.id.0) {
            return Err(BuildError::DuplicateSourceIdentity {
                source_id: meta.id.0,
            });
        }
    }
    let mut ids: Vec<SourceId> = source.terrain.iter().map(|g| g.source).collect();
    ids.extend(source.obstacles.iter().map(|o| o.source.source));
    ids.extend(source.aerodromes.iter().map(|a| a.source.source));
    for id in ids {
        let meta = source
            .meta_for(id)
            .ok_or(BuildError::UndeclaredSourceIdentity {
                source_id: id.0,
                reason: "no source metadata declared",
            })?;
        check_datum_declared(
            meta.id.0,
            meta.horizontal_datum,
            meta.realization.is_declared(),
        )?;
        check_vertical_declared(meta.id.0, meta.vertical_datum, meta.geoid.is_declared())?;
    }
    Ok(())
}

/// Validates the configured target datum is fully declared, so every conversion
/// has a well-defined destination frame.
fn validate_target_datum(config: &BuildConfig) -> Result<(), BuildError> {
    let t = &config.target;
    check_datum_declared(0, t.horizontal, t.realization.is_declared())?;
    check_vertical_declared(0, t.vertical, t.geoid.is_declared())
}

/// Rejects an unknown or under-declared horizontal datum.
fn check_datum_declared(
    source_id: u32,
    datum: HorizontalDatum,
    realization_declared: bool,
) -> Result<(), BuildError> {
    if datum == HorizontalDatum::Unknown {
        return Err(BuildError::UnknownSourceDatum {
            source_id,
            axis: "horizontal",
        });
    }
    if datum.needs_realization() && !realization_declared {
        return Err(BuildError::UndeclaredSourceIdentity {
            source_id,
            reason: "horizontal datum needs a declared realization",
        });
    }
    Ok(())
}

/// Rejects an unknown or under-declared vertical datum.
fn check_vertical_declared(
    source_id: u32,
    datum: VerticalDatum,
    geoid_declared: bool,
) -> Result<(), BuildError> {
    if datum == VerticalDatum::Unknown {
        return Err(BuildError::UnknownSourceDatum {
            source_id,
            axis: "vertical",
        });
    }
    if datum == VerticalDatum::Msl && !geoid_declared {
        return Err(BuildError::UndeclaredSourceIdentity {
            source_id,
            reason: "geometric-MSL datum needs a declared geoid",
        });
    }
    Ok(())
}

/// The tile a target-datum position falls in, reusing the SVS-02 geospatial
/// contract so seam, anti-meridian, and pole handling match the consumer.
///
/// # Errors
///
/// [`BuildError::InvalidGeodetic`] when the coordinates are not a valid geodetic
/// position at the configured tile size.
pub(crate) fn geo_tile_for(
    config: &BuildConfig,
    source_id: u32,
    lat_deg: f64,
    lon_deg: f64,
) -> Result<GeoTile, BuildError> {
    tile_of(
        config.target.horizontal,
        config.target.realization,
        config.params.tile_deg,
        source_id,
        lat_deg,
        lon_deg,
    )
}

/// The core tiling primitive: the tile a position falls in, in the given target
/// datum at the given tile size. Reuses [`pilotage_geo::GeodeticPosition::tile`]
/// so the build and any independent verifier agree on tile assignment.
///
/// # Errors
///
/// [`BuildError::InvalidGeodetic`] when the coordinates are not a valid geodetic
/// position at `tile_deg`.
pub(crate) fn tile_of(
    horizontal: HorizontalDatum,
    realization: DatumRealizationId,
    tile_deg: f64,
    source_id: u32,
    lat_deg: f64,
    lon_deg: f64,
) -> Result<GeoTile, BuildError> {
    let vertical = VerticalPosition::new(
        0.0,
        VerticalDatum::Ellipsoid,
        GeoidModelId::UNDECLARED,
        TerrainRefId::UNDECLARED,
        BaroSettingId::UNDECLARED,
        LocalOriginId::UNDECLARED,
    )
    .map_err(|source| BuildError::InvalidGeodetic { source_id, source })?;
    let position = GeodeticPosition::new(lat_deg, lon_deg, horizontal, realization, vertical)
        .map_err(|source| BuildError::InvalidGeodetic { source_id, source })?;
    position
        .tile(tile_deg)
        .map_err(|source| BuildError::InvalidGeodetic { source_id, source })
}
