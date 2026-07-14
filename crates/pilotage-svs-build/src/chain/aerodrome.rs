//! The aerodrome pipeline: reference points and their runways.
//!
//! An aerodrome reference point is rejected on a bad elevation or coordinate,
//! horizontally converted, vertically converted at its position, clipped to
//! coverage, and tiled. Its runways run through the runway sub-pipeline. The
//! pipeline emits two feature classes — aerodromes and runways — from one pass.

mod runway;

use std::collections::BTreeMap;

use pilotage_geo::GeoTile;
use pilotage_svs_db::{CoverageBox, FeatureClass, Tile, TileKey};

use crate::chain::{Metrics, PipelineOutput, geo_tile_for};
use crate::config::BuildConfig;
use crate::datum::{convert_horizontal, convert_vertical};
use crate::element::OutputAerodrome;
use crate::error::BuildError;
use crate::payload::encode_aerodromes;
use crate::provenance::{Disposition, RecordDisposition, StageRecord, TileLineage};
use crate::source::{Aerodrome, SourceDataset, SourceMeta, SourceRecordRef};

use runway::{RunwayGroups, build_tiles as build_runway_tiles, place as place_runway};

/// The map from tile index to the aerodromes placed in it.
type AeroGroups = BTreeMap<(i32, i32), Vec<OutputAerodrome>>;

/// Runs the aerodrome pipeline (reference points and runways).
///
/// # Errors
///
/// A [`BuildError`] from datum conversion or tiling.
pub(crate) fn process(
    config: &BuildConfig,
    source: &SourceDataset,
) -> Result<PipelineOutput, BuildError> {
    let mut dispositions: Vec<RecordDisposition> = Vec::new();
    let mut aero_groups: AeroGroups = BTreeMap::new();
    let mut rwy_groups: RunwayGroups = BTreeMap::new();
    let mut outliers = 0u32;
    let mut clipped = 0u32;
    let inputs = source.aerodromes.len() as u32;
    for aerodrome in &source.aerodromes {
        let meta = source.meta_for(aerodrome.source.source).ok_or(
            BuildError::UndeclaredSourceIdentity {
                source_id: aerodrome.source.source.0,
                reason: "no source metadata declared",
            },
        )?;
        place_aerodrome(
            config,
            meta,
            aerodrome,
            &mut aero_groups,
            &mut dispositions,
            (&mut outliers, &mut clipped),
        )?;
        for rwy in &aerodrome.runways {
            place_runway(
                config,
                meta,
                rwy,
                &mut rwy_groups,
                &mut dispositions,
                &mut outliers,
                &mut clipped,
            )?;
        }
    }
    Ok(assemble(
        config,
        aero_groups,
        rwy_groups,
        dispositions,
        (inputs, outliers, clipped),
    ))
}

/// Assembles the aerodrome and runway tiles into one pipeline output.
fn assemble(
    _config: &BuildConfig,
    aero_groups: AeroGroups,
    rwy_groups: RunwayGroups,
    dispositions: Vec<RecordDisposition>,
    counts: (u32, u32, u32),
) -> PipelineOutput {
    let (inputs, outliers, clipped) = counts;
    let (mut tiles, mut lineages) = build_aero_tiles(aero_groups);
    let aerodrome_tiles = tiles.len() as u32;
    let (rwy_tiles, rwy_lineages) = build_runway_tiles(rwy_groups);
    let runway_tiles = rwy_tiles.len() as u32;
    tiles.extend(rwy_tiles);
    lineages.extend(rwy_lineages);
    let emitted = lineages
        .iter()
        .fold(0u32, |acc, l| acc.wrapping_add(l.element_count));
    let stages = aero_stages(inputs, outliers, clipped, emitted);
    let metrics = Metrics {
        outliers,
        clipped,
        aerodrome_tiles,
        runway_tiles,
        ..Metrics::default()
    };
    PipelineOutput {
        tiles,
        lineages,
        dispositions,
        stages,
        metrics,
    }
}

/// Rejects, converts, clips, and tiles a single aerodrome reference point.
fn place_aerodrome(
    config: &BuildConfig,
    meta: &SourceMeta,
    aerodrome: &Aerodrome,
    groups: &mut AeroGroups,
    dispositions: &mut Vec<RecordDisposition>,
    counters: (&mut u32, &mut u32),
) -> Result<(), BuildError> {
    let (outliers, clipped) = counters;
    if let Some(reason) = outlier_reason(config, aerodrome) {
        dispositions.push(RecordDisposition {
            source: aerodrome.source,
            disposition: Disposition::RejectedOutlier { reason },
        });
        *outliers = outliers.wrapping_add(1);
        return Ok(());
    }
    let (lat, lon) = convert_horizontal(
        aerodrome.ref_lat_deg,
        aerodrome.ref_lon_deg,
        meta.horizontal_datum,
        config.target.horizontal,
    )?;
    let elevation_m = convert_vertical(
        aerodrome.elevation_m,
        meta.vertical_datum,
        config.target.vertical,
        lat,
        lon,
    )?;
    if !coverage_contains(&config.coverage, lat, lon) {
        dispositions.push(RecordDisposition {
            source: aerodrome.source,
            disposition: Disposition::Clipped,
        });
        *clipped = clipped.wrapping_add(1);
        return Ok(());
    }
    let tile = geo_tile_for(config, aerodrome.source.source.0, lat, lon)?;
    groups
        .entry((tile.lat_index, tile.lon_index))
        .or_default()
        .push(OutputAerodrome {
            ident: aerodrome.ident,
            lat_deg: lat,
            lon_deg: lon,
            elevation_m,
            source: aerodrome.source,
        });
    Ok(())
}

/// Why an aerodrome is an outlier, or `None` if valid.
fn outlier_reason(config: &BuildConfig, aerodrome: &Aerodrome) -> Option<&'static str> {
    if !(aerodrome.ref_lat_deg.is_finite() && aerodrome.ref_lon_deg.is_finite()) {
        return Some("non-finite aerodrome coordinate");
    }
    if !aerodrome.elevation_m.is_finite() {
        return Some("non-finite aerodrome elevation");
    }
    if aerodrome.elevation_m < config.params.elevation_min_m
        || aerodrome.elevation_m > config.params.elevation_max_m
    {
        return Some("aerodrome elevation out of bounds");
    }
    None
}

/// Whether `(lat, lon)` is inside coverage.
fn coverage_contains(coverage: &CoverageBox, lat: f64, lon: f64) -> bool {
    coverage.contains_lat_lon(lat, lon)
}

/// Builds aerodrome tiles and lineage from the grouped reference points.
fn build_aero_tiles(groups: AeroGroups) -> (Vec<Tile>, Vec<TileLineage>) {
    let mut tiles = Vec::with_capacity(groups.len());
    let mut lineages = Vec::with_capacity(groups.len());
    for ((lat_index, lon_index), aerodromes) in groups {
        let key = TileKey {
            class: FeatureClass::Aerodromes,
            level: 0,
            tile: GeoTile {
                lat_index,
                lon_index,
            },
        };
        tiles.push(Tile {
            key,
            bytes: encode_aerodromes(&aerodromes),
        });
        let mut sources: Vec<SourceRecordRef> = aerodromes.iter().map(|a| a.source).collect();
        sources.sort();
        sources.dedup();
        lineages.push(TileLineage {
            class: FeatureClass::Aerodromes.to_u8(),
            lat_index,
            lon_index,
            element_count: aerodromes.len() as u32,
            sources,
        });
    }
    (tiles, lineages)
}

/// The stage records for the aerodrome pipeline.
fn aero_stages(inputs: u32, outliers: u32, clipped: u32, emitted: u32) -> Vec<StageRecord> {
    let after_outlier = inputs.wrapping_sub(outliers);
    vec![
        StageRecord {
            code: crate::chain::STAGE_OUTLIER,
            name: "aerodrome-outlier",
            inputs,
            outputs: after_outlier,
            rejected: outliers,
        },
        StageRecord {
            code: crate::chain::STAGE_CLIP,
            name: "aerodrome-clip",
            inputs: after_outlier,
            outputs: emitted,
            rejected: clipped,
        },
    ]
}
