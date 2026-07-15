//! Coverage, data-quality, seam, and hole reports over a build.
//!
//! These reports are the independently derived check on what the chain
//! produced: the coverage report counts what was emitted against the grid the
//! coverage box implies, the quality report counts every rejection and merge,
//! the seam check confirms each grid node landed in exactly one tile (so tiles
//! neither overlap nor leave a gap on a seam), and the hole check lists every
//! void the interpolation left. They serialize deterministically alongside the
//! provenance as engineering evidence.

#[cfg(test)]
mod tests;

use serde::Serialize;

/// How much of the coverage grid the build populated.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct CoverageReport {
    /// Southern coverage bound, degrees.
    pub min_lat_deg: f64,
    /// Northern coverage bound, degrees.
    pub max_lat_deg: f64,
    /// Western coverage bound, degrees.
    pub min_lon_deg: f64,
    /// Eastern coverage bound, degrees.
    pub max_lon_deg: f64,
    /// Terrain tiles emitted.
    pub terrain_tiles: u32,
    /// Obstacle tiles emitted.
    pub obstacle_tiles: u32,
    /// Aerodrome tiles emitted.
    pub aerodrome_tiles: u32,
    /// Runway tiles emitted.
    pub runway_tiles: u32,
    /// Terrain posts emitted.
    pub terrain_posts: u32,
    /// Obstacles emitted (after merging).
    pub obstacles: u32,
    /// Grid nodes with a populated elevation.
    pub covered_nodes: u32,
    /// Grid nodes left void.
    pub void_nodes: u32,
    /// Total grid nodes the coverage box implies.
    pub total_nodes: u32,
    /// Fraction of grid nodes populated, `covered / total`.
    pub coverage_fraction: f64,
}

/// The data-quality tally: rejections, clips, holes, and merges.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct QualityReport {
    /// Records rejected as outliers.
    pub outliers_rejected: u32,
    /// Records clipped for falling outside coverage.
    pub clipped: u32,
    /// Terrain nodes left void by the hole policy.
    pub holes: u32,
    /// Source obstacles that merged into another.
    pub obstacles_merged: u32,
}

/// The seam check: every grid node must belong to exactly one tile.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct SeamCheck {
    /// Whether no node was claimed by two tiles and none straddled a seam.
    pub ok: bool,
    /// The number of nodes found in more than one tile (zero when `ok`).
    pub conflicts: u32,
}

/// A voided grid node the interpolation could not populate.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct VoidNode {
    /// Global grid row index.
    pub i: u32,
    /// Global grid column index.
    pub j: u32,
}

/// The hole check: the recorded voids left after the hole policy ran.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct HoleCheck {
    /// The voided nodes, sorted by `(i, j)`.
    pub voids: Vec<VoidNode>,
}

/// The full report set emitted alongside a build.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BuildReports {
    /// The coverage report.
    pub coverage: CoverageReport,
    /// The data-quality report.
    pub quality: QualityReport,
    /// The seam check.
    pub seam: SeamCheck,
    /// The hole check.
    pub hole: HoleCheck,
}
