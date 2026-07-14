//! The obstacle pipeline: reject invalid obstacles, convert coordinates, clip to
//! coverage, tile, and merge co-located obstacles of the same kind.
//!
//! An obstacle's above-ground-level height is a length and needs no vertical
//! datum conversion; only its position is converted. Merging is deterministic:
//! within a tile, obstacles are clustered greedily in a fixed sort order, the
//! tallest of a cluster is kept, and every obstacle that merged into it is
//! recorded in the change report with the position it merged to.

mod merge;

use std::collections::BTreeMap;

use pilotage_geo::GeoTile;
use pilotage_svs_db::{CoverageBox, FeatureClass, Tile, TileKey};

use crate::chain::{Metrics, PipelineOutput, geo_tile_for};
use crate::config::BuildConfig;
use crate::datum::convert_horizontal;
use crate::element::OutputObstacle;
use crate::error::BuildError;
use crate::payload::encode_obstacles;
use crate::provenance::{Disposition, RecordDisposition, StageRecord, TileLineage};
use crate::source::{Obstacle, ObstacleKind, SourceDataset, SourceRecordRef};

use merge::merge_tile;

/// An obstacle that survived rejection and clipping, in the target datum.
pub(crate) struct TileObstacle {
    /// Latitude, target datum, degrees.
    pub lat_deg: f64,
    /// Longitude, target datum, degrees.
    pub lon_deg: f64,
    /// Above-ground-level height, meters.
    pub height_m: f64,
    /// The obstacle kind.
    pub kind: ObstacleKind,
    /// The source record.
    pub source: SourceRecordRef,
}

/// Runs the obstacle pipeline over the dataset.
///
/// # Errors
///
/// A [`BuildError`] from datum conversion or tiling.
pub(crate) fn process(
    config: &BuildConfig,
    source: &SourceDataset,
) -> Result<PipelineOutput, BuildError> {
    let mut dispositions: Vec<RecordDisposition> = Vec::new();
    let mut groups: BTreeMap<(i32, i32), Vec<TileObstacle>> = BTreeMap::new();
    let mut outliers = 0u32;
    let mut clipped = 0u32;
    let inputs = source.obstacles.len() as u32;
    for obstacle in &source.obstacles {
        match place(config, source, obstacle, &mut dispositions)? {
            Placement::Placed(tile, item) => groups.entry(tile).or_default().push(item),
            Placement::Rejected => outliers = outliers.wrapping_add(1),
            Placement::Clipped => clipped = clipped.wrapping_add(1),
        }
    }
    let mut merged = 0u32;
    let (tiles, lineages) = build_tiles(config, groups, &mut dispositions, &mut merged);
    let emitted = tiles_element_count(&lineages);
    let stages = obstacle_stages(inputs, outliers, clipped, emitted, merged);
    let metrics = Metrics {
        outliers,
        clipped,
        merged,
        obstacles: emitted,
        obstacle_tiles: tiles.len() as u32,
        ..Metrics::default()
    };
    Ok(PipelineOutput {
        tiles,
        lineages,
        dispositions,
        stages,
        metrics,
    })
}

/// Where an obstacle ended up: placed in a tile, rejected as an outlier, or
/// clipped for leaving coverage.
enum Placement {
    Placed((i32, i32), TileObstacle),
    Rejected,
    Clipped,
}

/// Rejects, converts, clips, and tiles a single obstacle.
fn place(
    config: &BuildConfig,
    source: &SourceDataset,
    obstacle: &Obstacle,
    dispositions: &mut Vec<RecordDisposition>,
) -> Result<Placement, BuildError> {
    if let Some(reason) = outlier_reason(config, obstacle) {
        dispositions.push(RecordDisposition {
            source: obstacle.source,
            disposition: Disposition::RejectedOutlier { reason },
        });
        return Ok(Placement::Rejected);
    }
    let meta =
        source
            .meta_for(obstacle.source.source)
            .ok_or(BuildError::UndeclaredSourceIdentity {
                source_id: obstacle.source.source.0,
                reason: "no source metadata declared",
            })?;
    let (lat, lon) = convert_horizontal(
        obstacle.lat_deg,
        obstacle.lon_deg,
        meta.horizontal_datum,
        config.target.horizontal,
    )?;
    if !within(&config.coverage, lat, lon) {
        dispositions.push(RecordDisposition {
            source: obstacle.source,
            disposition: Disposition::Clipped,
        });
        return Ok(Placement::Clipped);
    }
    let tile = geo_tile_for(config, obstacle.source.source.0, lat, lon)?;
    Ok(Placement::Placed(
        (tile.lat_index, tile.lon_index),
        TileObstacle {
            lat_deg: lat,
            lon_deg: lon,
            height_m: obstacle.height_m,
            kind: obstacle.kind,
            source: obstacle.source,
        },
    ))
}

/// Why an obstacle is an outlier, or `None` if it is valid.
fn outlier_reason(config: &BuildConfig, obstacle: &Obstacle) -> Option<&'static str> {
    if !(obstacle.lat_deg.is_finite() && obstacle.lon_deg.is_finite()) {
        return Some("non-finite obstacle coordinate");
    }
    if !obstacle.height_m.is_finite() {
        return Some("non-finite obstacle height");
    }
    if obstacle.height_m <= 0.0 {
        return Some("non-positive obstacle height");
    }
    if obstacle.height_m > config.params.max_obstacle_height_m {
        return Some("obstacle height exceeds maximum");
    }
    None
}

/// Whether `(lat, lon)` is inside the coverage box (inclusive lower, exclusive
/// upper).
fn within(coverage: &CoverageBox, lat: f64, lon: f64) -> bool {
    coverage.contains_lat_lon(lat, lon)
}

/// Merges each tile's obstacles and builds the tiles and lineage.
fn build_tiles(
    config: &BuildConfig,
    groups: BTreeMap<(i32, i32), Vec<TileObstacle>>,
    dispositions: &mut Vec<RecordDisposition>,
    merged: &mut u32,
) -> (Vec<Tile>, Vec<TileLineage>) {
    let mut tiles = Vec::with_capacity(groups.len());
    let mut lineages = Vec::with_capacity(groups.len());
    for ((lat_index, lon_index), items) in groups {
        let obstacles = merge_tile(
            items,
            config.params.merge_tolerance_deg,
            dispositions,
            merged,
        );
        let key = TileKey {
            class: FeatureClass::Obstacles,
            level: 0,
            tile: GeoTile {
                lat_index,
                lon_index,
            },
        };
        tiles.push(Tile {
            key,
            bytes: encode_obstacles(&obstacles),
        });
        lineages.push(lineage(lat_index, lon_index, &obstacles));
    }
    (tiles, lineages)
}

/// The lineage of an obstacle tile.
fn lineage(lat_index: i32, lon_index: i32, obstacles: &[OutputObstacle]) -> TileLineage {
    let mut sources: Vec<SourceRecordRef> = obstacles
        .iter()
        .flat_map(|o| o.sources.iter().copied())
        .collect();
    sources.sort();
    sources.dedup();
    TileLineage {
        class: FeatureClass::Obstacles.to_u8(),
        lat_index,
        lon_index,
        element_count: obstacles.len() as u32,
        sources,
    }
}

/// The total emitted obstacle count across tiles.
fn tiles_element_count(lineages: &[TileLineage]) -> u32 {
    lineages
        .iter()
        .fold(0u32, |acc, l| acc.wrapping_add(l.element_count))
}

/// The stage records for the obstacle pipeline.
fn obstacle_stages(
    inputs: u32,
    outliers: u32,
    clipped: u32,
    emitted: u32,
    merged: u32,
) -> Vec<StageRecord> {
    let after_outlier = inputs.wrapping_sub(outliers);
    let after_clip = after_outlier.wrapping_sub(clipped);
    vec![
        StageRecord {
            code: crate::chain::STAGE_OUTLIER,
            name: "obstacle-outlier",
            inputs,
            outputs: after_outlier,
            rejected: outliers,
        },
        StageRecord {
            code: crate::chain::STAGE_CLIP,
            name: "obstacle-clip",
            inputs: after_outlier,
            outputs: after_clip,
            rejected: clipped,
        },
        StageRecord {
            code: crate::chain::STAGE_MERGE,
            name: "obstacle-merge",
            inputs: after_clip,
            outputs: emitted,
            rejected: merged,
        },
    ]
}
