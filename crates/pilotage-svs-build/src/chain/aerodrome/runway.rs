//! The runway sub-pipeline: convert both endpoints, clip by end A, and tile.
//!
//! A runway carries no elevation, so only its endpoints are converted. It is
//! tiled by end A, so a runway belongs to exactly one tile even when it straddles
//! a seam; a runway whose end A leaves coverage is clipped and recorded.

use std::collections::BTreeMap;

use pilotage_geo::GeoTile;
use pilotage_svs_db::{CoverageBox, FeatureClass, Tile, TileKey};

use crate::chain::geo_tile_for;
use crate::config::BuildConfig;
use crate::datum::convert_horizontal;
use crate::element::OutputRunway;
use crate::error::BuildError;
use crate::payload::encode_runways;
use crate::provenance::{Disposition, RecordDisposition, RecordKey, RecordLineage, TileLineage};
use crate::source::{Runway, SourceMeta, SourceRecordRef};

/// The map from tile index to the runways placed in it.
pub(crate) type RunwayGroups = BTreeMap<(i32, i32), Vec<OutputRunway>>;

/// Rejects, converts, clips, and tiles a single runway.
///
/// # Errors
///
/// A [`BuildError`] from datum conversion or tiling.
pub(crate) fn place(
    config: &BuildConfig,
    meta: &SourceMeta,
    runway: &Runway,
    groups: &mut RunwayGroups,
    dispositions: &mut Vec<RecordDisposition>,
    outliers: &mut u32,
    clipped: &mut u32,
) -> Result<(), BuildError> {
    if let Some(reason) = outlier_reason(runway) {
        dispositions.push(RecordDisposition {
            source: runway.source,
            disposition: Disposition::RejectedOutlier { reason },
        });
        *outliers = outliers.wrapping_add(1);
        return Ok(());
    }
    let (a_lat, a_lon) = convert_horizontal(
        runway.end_a_lat_deg,
        runway.end_a_lon_deg,
        meta.horizontal_datum,
        config.target.horizontal,
    )?;
    let (b_lat, b_lon) = convert_horizontal(
        runway.end_b_lat_deg,
        runway.end_b_lon_deg,
        meta.horizontal_datum,
        config.target.horizontal,
    )?;
    if !coverage_contains(&config.coverage, a_lat, a_lon) {
        dispositions.push(RecordDisposition {
            source: runway.source,
            disposition: Disposition::Clipped,
        });
        *clipped = clipped.wrapping_add(1);
        return Ok(());
    }
    let tile = geo_tile_for(config, runway.source.source.0, a_lat, a_lon)?;
    groups
        .entry((tile.lat_index, tile.lon_index))
        .or_default()
        .push(OutputRunway {
            designator: runway.designator,
            end_a_lat_deg: a_lat,
            end_a_lon_deg: a_lon,
            end_b_lat_deg: b_lat,
            end_b_lon_deg: b_lon,
            source: runway.source,
        });
    Ok(())
}

/// Whether `(lat, lon)` is inside coverage.
fn coverage_contains(coverage: &CoverageBox, lat: f64, lon: f64) -> bool {
    coverage.contains_lat_lon(lat, lon)
}

/// Why a runway is an outlier, or `None` if valid.
fn outlier_reason(runway: &Runway) -> Option<&'static str> {
    let finite = runway.end_a_lat_deg.is_finite()
        && runway.end_a_lon_deg.is_finite()
        && runway.end_b_lat_deg.is_finite()
        && runway.end_b_lon_deg.is_finite();
    if finite {
        None
    } else {
        Some("non-finite runway endpoint")
    }
}

/// Builds runway tiles and lineage from the grouped runways.
pub(crate) fn build_tiles(
    groups: RunwayGroups,
) -> (Vec<Tile>, Vec<TileLineage>, Vec<RecordLineage>) {
    let mut tiles = Vec::with_capacity(groups.len());
    let mut lineages = Vec::with_capacity(groups.len());
    let mut records: Vec<RecordLineage> = Vec::new();
    for ((lat_index, lon_index), runways) in groups {
        let key = TileKey {
            class: FeatureClass::Runways,
            level: 0,
            tile: GeoTile {
                lat_index,
                lon_index,
            },
        };
        tiles.push(Tile {
            key,
            bytes: encode_runways(&runways),
        });
        let mut sources: Vec<SourceRecordRef> = runways.iter().map(|r| r.source).collect();
        sources.sort();
        sources.dedup();
        for runway in &runways {
            records.push(RecordLineage {
                class: FeatureClass::Runways.to_u8(),
                lat_index,
                lon_index,
                key: RecordKey::Runway {
                    designator: runway.designator,
                    end_a_lat_bits: runway.end_a_lat_deg.to_bits(),
                    end_a_lon_bits: runway.end_a_lon_deg.to_bits(),
                },
                sources: vec![runway.source],
            });
        }
        lineages.push(TileLineage {
            class: FeatureClass::Runways.to_u8(),
            lat_index,
            lon_index,
            element_count: runways.len() as u32,
            sources,
        });
    }
    (tiles, lineages, records)
}
