//! The terrain pipeline: prepare each source grid, resample onto the output
//! grid, and group the resampled posts into tiles.
//!
//! Resampling walks the output grid the coverage box implies at the configured
//! post spacing. Each populated node becomes a terrain post traced to its four
//! source corners; each unpopulated node is a recorded void. Nodes are assigned
//! to tiles by [`crate::chain::geo_tile_for`], so a node on a tile seam lands in
//! exactly one tile.

mod prepare;
mod resample;

use std::collections::BTreeMap;

use pilotage_geo::GeoTile;
use pilotage_svs_db::{FeatureClass, Tile, TileKey};

use crate::chain::{Metrics, PipelineOutput, geo_tile_for};
use crate::config::BuildConfig;
use crate::element::OutputPost;
use crate::error::BuildError;
use crate::payload::encode_terrain;
use crate::provenance::{RecordDisposition, RecordKey, RecordLineage, StageRecord, TileLineage};
use crate::report::VoidNode;
use crate::source::{SourceDataset, SourceRecordRef};

use prepare::{Prepared, prepare_grid};
use resample::resample_node;

/// A safety cap on nodes per axis, so a mis-scaled coverage/spacing pair fails
/// closed instead of allocating without bound.
const AXIS_NODE_CAP: u32 = 4096;

/// Runs the terrain pipeline over the dataset.
///
/// # Errors
///
/// A [`BuildError`] from grid preparation, datum conversion, or an over-large
/// output grid.
pub(crate) fn process(
    config: &BuildConfig,
    source: &SourceDataset,
) -> Result<PipelineOutput, BuildError> {
    let mut dispositions: Vec<RecordDisposition> = Vec::new();
    let prepared = prepare_all(config, source, &mut dispositions)?;
    let outliers = prepared
        .iter()
        .fold(0u32, |acc, p| acc.wrapping_add(p.outliers));
    let survived = prepared
        .iter()
        .fold(0u32, |acc, p| acc.wrapping_add(p.survived));
    let source_posts = source
        .terrain
        .iter()
        .fold(0u32, |acc, g| acc.wrapping_add(g.rows.wrapping_mul(g.cols)));
    let (n_lat, n_lon) = output_grid_dims(config)?;
    let total_nodes = u64::from(n_lat).saturating_mul(u64::from(n_lon)) as u32;
    let resampled = resample_grid(config, &prepared, n_lat, n_lon)?;
    let (tiles, lineages, records, terrain_tiles) = build_tiles(resampled.groups);
    let stages = terrain_stages(
        source_posts,
        survived,
        outliers,
        resampled.covered,
        &resampled.voids,
    );
    let metrics = Metrics {
        outliers,
        holes: resampled.voids.len() as u32,
        terrain_posts: resampled.covered,
        covered_nodes: resampled.covered,
        total_nodes,
        terrain_tiles,
        voids: resampled.voids,
        ..Metrics::default()
    };
    Ok(PipelineOutput {
        tiles,
        lineages,
        records,
        dispositions,
        stages,
        metrics,
    })
}

/// Prepares every source grid in a deterministic order.
fn prepare_all(
    config: &BuildConfig,
    source: &SourceDataset,
    dispositions: &mut Vec<RecordDisposition>,
) -> Result<Vec<Prepared>, BuildError> {
    let mut grids: Vec<&crate::source::TerrainGrid> = source.terrain.iter().collect();
    grids.sort_by(|a, b| {
        a.source
            .0
            .cmp(&b.source.0)
            .then(a.origin_lat_deg.total_cmp(&b.origin_lat_deg))
            .then(a.origin_lon_deg.total_cmp(&b.origin_lon_deg))
    });
    let mut prepared = Vec::with_capacity(grids.len());
    for grid in grids {
        let meta = source
            .meta_for(grid.source)
            .ok_or(BuildError::UndeclaredSourceIdentity {
                source_id: grid.source.0,
                reason: "no source metadata declared",
            })?;
        prepared.push(prepare_grid(grid, meta, config, dispositions)?);
    }
    Ok(prepared)
}

/// The resampled output grid: populated posts grouped by tile, and the voids.
struct Resampled {
    groups: BTreeMap<(i32, i32), Vec<OutputPost>>,
    voids: Vec<VoidNode>,
    covered: u32,
}

/// Resamples every output node, grouping populated posts by tile.
fn resample_grid(
    config: &BuildConfig,
    prepared: &[Prepared],
    n_lat: u32,
    n_lon: u32,
) -> Result<Resampled, BuildError> {
    let (min_lat, min_lon) = (config.coverage.min_lat_deg, config.coverage.min_lon_deg);
    let step = config.params.post_spacing_deg;
    let mut groups: BTreeMap<(i32, i32), Vec<OutputPost>> = BTreeMap::new();
    let mut voids: Vec<VoidNode> = Vec::new();
    let mut covered = 0u32;
    for i in 0..n_lat {
        for j in 0..n_lon {
            let lat = min_lat + f64::from(i) * step;
            let lon = min_lon + f64::from(j) * step;
            match resample_node(prepared, i, j, lat, lon) {
                Some(post) => {
                    let tile = geo_tile_for(config, post.sources[0].source.0, lat, lon)?;
                    groups
                        .entry((tile.lat_index, tile.lon_index))
                        .or_default()
                        .push(post);
                    covered = covered.wrapping_add(1);
                }
                None => voids.push(VoidNode { i, j }),
            }
        }
    }
    Ok(Resampled {
        groups,
        voids,
        covered,
    })
}

/// Builds terrain tiles, their tile-level lineage, and per-record lineage from
/// the grouped posts.
fn build_tiles(
    groups: BTreeMap<(i32, i32), Vec<OutputPost>>,
) -> (Vec<Tile>, Vec<TileLineage>, Vec<RecordLineage>, u32) {
    let mut tiles = Vec::with_capacity(groups.len());
    let mut lineages = Vec::with_capacity(groups.len());
    let mut records: Vec<RecordLineage> = Vec::new();
    let mut count = 0u32;
    for ((lat_index, lon_index), posts) in groups {
        let key = TileKey {
            class: FeatureClass::Terrain,
            level: 0,
            tile: GeoTile {
                lat_index,
                lon_index,
            },
        };
        tiles.push(Tile {
            key,
            bytes: encode_terrain(&posts),
        });
        let mut sources: Vec<SourceRecordRef> = posts
            .iter()
            .flat_map(|p| p.sources.iter().copied())
            .collect();
        sources.sort();
        sources.dedup();
        for post in &posts {
            records.push(RecordLineage {
                class: FeatureClass::Terrain.to_u8(),
                lat_index,
                lon_index,
                key: RecordKey::TerrainNode {
                    i: post.i,
                    j: post.j,
                },
                sources: post.sources.clone(),
            });
        }
        lineages.push(TileLineage {
            class: FeatureClass::Terrain.to_u8(),
            lat_index,
            lon_index,
            element_count: posts.len() as u32,
            sources,
        });
        count = count.wrapping_add(1);
    }
    (tiles, lineages, records, count)
}

/// The per-axis node count the coverage box implies at the post spacing.
fn output_grid_dims(config: &BuildConfig) -> Result<(u32, u32), BuildError> {
    let step = config.params.post_spacing_deg;
    let n_lat = count_axis(
        config.coverage.min_lat_deg,
        config.coverage.max_lat_deg,
        step,
    )?;
    let n_lon = count_axis(
        config.coverage.min_lon_deg,
        config.coverage.max_lon_deg,
        step,
    )?;
    Ok((n_lat, n_lon))
}

/// The number of nodes on one axis: the count of `min + n*step < max`.
fn count_axis(min: f64, max: f64, step: f64) -> Result<u32, BuildError> {
    let mut n = 0u32;
    while min + f64::from(n) * step < max {
        n = n.wrapping_add(1);
        if n > AXIS_NODE_CAP {
            return Err(BuildError::InvalidConfig {
                reason: "output grid exceeds the per-axis node cap",
            });
        }
    }
    Ok(n)
}

/// The stage records for the terrain pipeline.
fn terrain_stages(
    source_posts: u32,
    survived: u32,
    outliers: u32,
    covered: u32,
    voids: &[VoidNode],
) -> Vec<StageRecord> {
    vec![
        StageRecord {
            code: crate::chain::STAGE_OUTLIER,
            name: "terrain-outlier",
            inputs: source_posts,
            outputs: survived,
            rejected: outliers,
        },
        StageRecord {
            code: crate::chain::STAGE_DATUM,
            name: "terrain-datum",
            inputs: survived,
            outputs: survived,
            rejected: 0,
        },
        StageRecord {
            code: crate::chain::STAGE_INTERPOLATE,
            name: "terrain-interpolate",
            inputs: survived,
            outputs: covered,
            rejected: voids.len() as u32,
        },
    ]
}
