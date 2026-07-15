//! Aggregating the pipeline outputs into the reports and the structured
//! provenance.
//!
//! Both are pure roll-ups of what the pipelines recorded, with everything sorted
//! so a reproduced build yields byte-identical provenance and reports. The seam
//! check is the independent partition check: every terrain node must be
//! accounted for exactly once (populated or void), and the terrain tiles' post
//! counts must sum to the populated total.

use crate::chain::{Metrics, PipelineOutput, STAGE_INGEST, STAGE_SERIALIZE};
use crate::config::BuildConfig;
use crate::provenance::{
    BuildProvenance, Disposition, ParamSnapshot, RecordDisposition, RecordLineage, SourceSummary,
    StageRecord, TOOL_ID, TOOL_VERSION, TileLineage,
};
use crate::report::{BuildReports, CoverageReport, HoleCheck, QualityReport, SeamCheck};
use crate::source::{SourceDataset, SourceRecordRef, source_record_refs};

/// Sums the scalar metrics and collects the voids across pipelines.
fn merge_metrics(outputs: &[&PipelineOutput]) -> Metrics {
    let mut m = Metrics::default();
    for output in outputs {
        let o = &output.metrics;
        m.outliers = m.outliers.wrapping_add(o.outliers);
        m.clipped = m.clipped.wrapping_add(o.clipped);
        m.holes = m.holes.wrapping_add(o.holes);
        m.merged = m.merged.wrapping_add(o.merged);
        m.terrain_posts = m.terrain_posts.wrapping_add(o.terrain_posts);
        m.covered_nodes = m.covered_nodes.wrapping_add(o.covered_nodes);
        m.total_nodes = m.total_nodes.wrapping_add(o.total_nodes);
        m.obstacles = m.obstacles.wrapping_add(o.obstacles);
        m.terrain_tiles = m.terrain_tiles.wrapping_add(o.terrain_tiles);
        m.obstacle_tiles = m.obstacle_tiles.wrapping_add(o.obstacle_tiles);
        m.aerodrome_tiles = m.aerodrome_tiles.wrapping_add(o.aerodrome_tiles);
        m.runway_tiles = m.runway_tiles.wrapping_add(o.runway_tiles);
        m.voids.extend(o.voids.iter().copied());
    }
    m
}

/// Builds the coverage, quality, seam, and hole reports.
pub(crate) fn build_reports(config: &BuildConfig, outputs: &[&PipelineOutput]) -> BuildReports {
    let metrics = merge_metrics(outputs);
    let terrain_post_sum = terrain_element_sum(outputs);
    let accounted = metrics.covered_nodes.wrapping_add(metrics.holes);
    let seam_ok = accounted == metrics.total_nodes && terrain_post_sum == metrics.covered_nodes;
    let mut voids = metrics.voids.clone();
    voids.sort_by_key(|v| (v.i, v.j));
    let coverage = CoverageReport {
        min_lat_deg: config.coverage.min_lat_deg,
        max_lat_deg: config.coverage.max_lat_deg,
        min_lon_deg: config.coverage.min_lon_deg,
        max_lon_deg: config.coverage.max_lon_deg,
        terrain_tiles: metrics.terrain_tiles,
        obstacle_tiles: metrics.obstacle_tiles,
        aerodrome_tiles: metrics.aerodrome_tiles,
        runway_tiles: metrics.runway_tiles,
        terrain_posts: metrics.terrain_posts,
        obstacles: metrics.obstacles,
        covered_nodes: metrics.covered_nodes,
        void_nodes: metrics.holes,
        total_nodes: metrics.total_nodes,
        coverage_fraction: coverage_fraction(metrics.covered_nodes, metrics.total_nodes),
    };
    BuildReports {
        coverage,
        quality: QualityReport {
            outliers_rejected: metrics.outliers,
            clipped: metrics.clipped,
            holes: metrics.holes,
            obstacles_merged: metrics.merged,
        },
        seam: SeamCheck {
            ok: seam_ok,
            conflicts: terrain_post_sum.saturating_sub(metrics.covered_nodes),
        },
        hole: HoleCheck { voids },
    }
}

/// The fraction of grid nodes populated, or zero when there is no grid.
fn coverage_fraction(covered: u32, total: u32) -> f64 {
    if total == 0 {
        0.0
    } else {
        f64::from(covered) / f64::from(total)
    }
}

/// The sum of terrain tile post counts across pipelines.
fn terrain_element_sum(outputs: &[&PipelineOutput]) -> u32 {
    let mut sum = 0u32;
    for output in outputs {
        for lineage in &output.lineages {
            if lineage.class == pilotage_svs_db::FeatureClass::Terrain.to_u8() {
                sum = sum.wrapping_add(lineage.element_count);
            }
        }
    }
    sum
}

/// Builds the structured provenance, binding it to the package content hash.
pub(crate) fn build_provenance(
    config: &BuildConfig,
    source: &SourceDataset,
    outputs: &[&PipelineOutput],
    package_content_hash: [u8; 32],
) -> BuildProvenance {
    BuildProvenance {
        tool_id: TOOL_ID,
        tool_version: TOOL_VERSION,
        params: param_snapshot(config),
        sources: source_summaries(source),
        stages: stage_records(source, outputs),
        dispositions: complete_dispositions(source, outputs),
        tiles: tile_lineages(outputs),
        records: record_lineages(outputs),
        package_content_hash,
    }
}

/// Every per-record lineage, sorted by class then tile then record key.
fn record_lineages(outputs: &[&PipelineOutput]) -> Vec<RecordLineage> {
    let mut all: Vec<RecordLineage> = Vec::new();
    for output in outputs {
        all.extend(output.records.iter().cloned());
    }
    all.sort_by(|a, b| {
        a.class
            .cmp(&b.class)
            .then(a.lat_index.cmp(&b.lat_index))
            .then(a.lon_index.cmp(&b.lon_index))
            .then(a.key.cmp(&b.key))
    });
    all
}

/// The numeric parameter snapshot from the configuration.
fn param_snapshot(config: &BuildConfig) -> ParamSnapshot {
    let p = &config.params;
    ParamSnapshot {
        tile_deg: p.tile_deg,
        post_spacing_deg: p.post_spacing_deg,
        post_spacing_mm: p.post_spacing_mm,
        elevation_min_m: p.elevation_min_m,
        elevation_max_m: p.elevation_max_m,
        max_obstacle_height_m: p.max_obstacle_height_m,
        max_hole_span: p.max_hole_span,
        merge_tolerance_deg: p.merge_tolerance_deg,
        integrity: p.integrity.to_u8(),
        target_horizontal: config.target.horizontal.to_u8(),
        target_realization: config.target.realization.0,
        target_vertical: config.target.vertical.to_u8(),
        target_geoid: config.target.geoid.0,
        effective_day: config.identity.effectivity.effective.0,
        expiry_day: config.identity.effectivity.expiry.0,
        release_day: config.identity.effectivity.release.0,
    }
}

/// One summary per source, sorted by id, with its immutable content digest and
/// record count.
fn source_summaries(source: &SourceDataset) -> Vec<SourceSummary> {
    let mut summaries: Vec<SourceSummary> = source
        .meta
        .iter()
        .map(|meta| SourceSummary {
            id: meta.id,
            version: meta.version,
            content_digest: crate::source::source_content_digest(source, meta),
            license: meta.license,
            horizontal_datum: meta.horizontal_datum.to_u8(),
            vertical_datum: meta.vertical_datum.to_u8(),
            accuracy_h_mm: meta.accuracy.horizontal_mm,
            accuracy_v_mm: meta.accuracy.vertical_mm,
            record_count: record_count(source, meta.id),
        })
        .collect();
    summaries.sort_by_key(|s| s.id.0);
    summaries
}

/// The number of source records `id` supplied across all feature kinds.
fn record_count(source: &SourceDataset, id: crate::source::SourceId) -> u32 {
    let mut count = 0u32;
    for grid in source.terrain.iter().filter(|g| g.source == id) {
        count = count.wrapping_add(grid.rows.wrapping_mul(grid.cols));
    }
    count = count.wrapping_add(
        source
            .obstacles
            .iter()
            .filter(|o| o.source.source == id)
            .count() as u32,
    );
    count = count.wrapping_add(
        source
            .aerodromes
            .iter()
            .filter(|a| a.source.source == id)
            .count() as u32,
    );
    // Runways are counted for THEIR OWN source, not the aerodrome's.
    count = count.wrapping_add(
        source
            .aerodromes
            .iter()
            .flat_map(|a| a.runways.iter())
            .filter(|r| r.source.source == id)
            .count() as u32,
    );
    count
}

/// The ordered stage records: ingest, every pipeline's stages, then serialize.
fn stage_records(source: &SourceDataset, outputs: &[&PipelineOutput]) -> Vec<StageRecord> {
    let inputs = total_source_records(source);
    let tiles = outputs
        .iter()
        .fold(0u32, |acc, o| acc.wrapping_add(o.tiles.len() as u32));
    let mut stages = vec![StageRecord {
        code: STAGE_INGEST,
        name: "ingest",
        inputs,
        outputs: inputs,
        rejected: 0,
    }];
    for output in outputs {
        stages.extend(output.stages.iter().copied());
    }
    stages.push(StageRecord {
        code: STAGE_SERIALIZE,
        name: "serialize",
        inputs: tiles,
        outputs: tiles,
        rejected: 0,
    });
    stages
}

/// The total source record count across every kind.
fn total_source_records(source: &SourceDataset) -> u32 {
    let mut count = 0u32;
    for grid in &source.terrain {
        count = count.wrapping_add(grid.rows.wrapping_mul(grid.cols));
    }
    count = count.wrapping_add(source.obstacles.len() as u32);
    for aerodrome in &source.aerodromes {
        count = count
            .wrapping_add(1)
            .wrapping_add(aerodrome.runways.len() as u32);
    }
    count
}

/// Every change-report disposition, sorted by source record.
fn dispositions(outputs: &[&PipelineOutput]) -> Vec<RecordDisposition> {
    let mut all: Vec<RecordDisposition> = Vec::new();
    for output in outputs {
        all.extend(output.dispositions.iter().copied());
    }
    all.sort_by_key(|d| d.source);
    all
}

/// The change report with every input record's fate made explicit: a record
/// the pipelines neither traced into output lineage nor recorded a
/// disposition for (every derived node void, or every candidate lost
/// deterministic resolution) is given a [`Disposition::NoContribution`]
/// entry, so a consumed source can never look like a phantom summary.
fn complete_dispositions(
    source: &SourceDataset,
    outputs: &[&PipelineOutput],
) -> Vec<RecordDisposition> {
    let mut all = dispositions(outputs);
    let mut fated: std::collections::BTreeSet<SourceRecordRef> =
        all.iter().map(|d| d.source).collect();
    for output in outputs {
        for record in &output.records {
            fated.extend(record.sources.iter().copied());
        }
    }
    for source_ref in source_record_refs(source) {
        if fated.insert(source_ref) {
            all.push(RecordDisposition {
                source: source_ref,
                disposition: Disposition::NoContribution {
                    reason: "no surviving output",
                },
            });
        }
    }
    all.sort_by_key(|d| d.source);
    all
}

/// Every tile lineage, sorted by class then tile index.
fn tile_lineages(outputs: &[&PipelineOutput]) -> Vec<TileLineage> {
    let mut all: Vec<TileLineage> = Vec::new();
    for output in outputs {
        all.extend(output.lineages.iter().cloned());
    }
    all.sort_by(|a, b| {
        a.class
            .cmp(&b.class)
            .then(a.lat_index.cmp(&b.lat_index))
            .then(a.lon_index.cmp(&b.lon_index))
    });
    all
}
