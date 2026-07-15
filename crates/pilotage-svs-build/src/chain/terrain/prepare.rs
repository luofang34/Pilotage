//! Preparing a source terrain grid: validation, outlier rejection, the row-wise
//! hole-fill policy, and conversion into the target frame.
//!
//! A prepared grid holds heights already in the target vertical datum, with
//! out-of-bounds posts removed and small voids filled by linear interpolation
//! along a row (bounded by the hole-span policy). Wide voids and edge voids stay
//! `None` so the resampler leaves them as holes rather than inventing terrain.

use crate::config::BuildConfig;
use crate::datum::{convert_horizontal, convert_vertical};
use crate::error::BuildError;
use crate::provenance::{Disposition, RecordDisposition};
use crate::source::{SourceId, SourceMeta, SourceRecordRef, TerrainGrid};

/// A source terrain grid converted into the target frame, ready to resample.
pub(crate) struct PreparedGrid {
    /// The source this grid came from.
    pub source: SourceId,
    /// The source-frame origin latitude, degrees (the grid's identity).
    pub src_origin_lat_deg: f64,
    /// The source-frame origin longitude, degrees (the grid's identity).
    pub src_origin_lon_deg: f64,
    /// Target-frame latitude of node row 0.
    pub origin_lat_t: f64,
    /// Target-frame longitude of node column 0.
    pub origin_lon_t: f64,
    /// Node step, degrees (unchanged by the translation-only horizontal shift).
    pub step_deg: f64,
    /// Node rows.
    pub rows: u32,
    /// Node columns.
    pub cols: u32,
    /// Row-major heights in the target vertical datum; `None` is a void.
    heights_t: Vec<Option<f64>>,
}

impl PreparedGrid {
    /// The target-vertical height at node `(r, c)`, or `None` for a void or an
    /// out-of-range index.
    pub(crate) fn at(&self, r: u32, c: u32) -> Option<f64> {
        if r >= self.rows || c >= self.cols {
            return None;
        }
        let idx = (r as usize)
            .checked_mul(self.cols as usize)?
            .checked_add(c as usize)?;
        self.heights_t.get(idx).copied().flatten()
    }

    /// The unambiguous source record reference of node `(r, c)`: the source, the
    /// grid (by its source origin), and the node position.
    pub(crate) fn record_ref(&self, r: u32, c: u32) -> SourceRecordRef {
        SourceRecordRef::terrain(
            self.source,
            self.src_origin_lat_deg,
            self.src_origin_lon_deg,
            r,
            c,
        )
    }
}

/// The result of preparing one grid: the grid plus the count of outliers it
/// rejected (each also pushed to the change report).
pub(crate) struct Prepared {
    /// The prepared grid.
    pub grid: PreparedGrid,
    /// Outliers rejected while preparing it.
    pub outliers: u32,
    /// Source posts that survived outlier rejection.
    pub survived: u32,
}

/// Validates and prepares a source grid, rejecting outliers into `dispositions`.
///
/// # Errors
///
/// [`BuildError::InvalidTerrainGrid`] for a malformed grid, or a conversion
/// error propagated from datum conversion.
pub(crate) fn prepare_grid(
    grid: &TerrainGrid,
    meta: &SourceMeta,
    config: &BuildConfig,
    dispositions: &mut Vec<RecordDisposition>,
) -> Result<Prepared, BuildError> {
    validate_grid(grid)?;
    let (mut src_heights, outliers, survived) = reject_outliers(grid, config, dispositions);
    row_gap_fill(
        &mut src_heights,
        grid.rows,
        grid.cols,
        config.params.max_hole_span,
    );
    let (origin_lat_t, origin_lon_t) = convert_horizontal(
        grid.origin_lat_deg,
        grid.origin_lon_deg,
        meta.horizontal_datum,
        config.target.horizontal,
    )?;
    let heights_t = convert_heights(&src_heights, grid, meta, config, origin_lat_t, origin_lon_t)?;
    Ok(Prepared {
        grid: PreparedGrid {
            source: grid.source,
            src_origin_lat_deg: grid.origin_lat_deg,
            src_origin_lon_deg: grid.origin_lon_deg,
            origin_lat_t,
            origin_lon_t,
            step_deg: grid.step_deg,
            rows: grid.rows,
            cols: grid.cols,
            heights_t,
        },
        outliers,
        survived,
    })
}

/// Rejects an unusable grid geometry.
fn validate_grid(grid: &TerrainGrid) -> Result<(), BuildError> {
    let reason = if !(grid.step_deg.is_finite() && grid.step_deg > 0.0) {
        Some("step must be positive and finite")
    } else if grid.rows == 0 || grid.cols == 0 {
        Some("grid must have at least one row and column")
    } else if !(grid.origin_lat_deg.is_finite() && grid.origin_lon_deg.is_finite()) {
        Some("origin must be finite")
    } else if grid.posts.len() != (grid.rows as usize).saturating_mul(grid.cols as usize) {
        Some("post count does not equal rows * cols")
    } else {
        None
    };
    match reason {
        Some(reason) => Err(BuildError::InvalidTerrainGrid {
            source_id: grid.source.0,
            reason,
        }),
        None => Ok(()),
    }
}

/// Filters out-of-bounds and non-finite posts, recording each as an outlier.
fn reject_outliers(
    grid: &TerrainGrid,
    config: &BuildConfig,
    dispositions: &mut Vec<RecordDisposition>,
) -> (Vec<Option<f64>>, u32, u32) {
    let (emin, emax) = (config.params.elevation_min_m, config.params.elevation_max_m);
    let mut heights = Vec::with_capacity(grid.posts.len());
    let mut outliers = 0u32;
    let mut survived = 0u32;
    for r in 0..grid.rows {
        for c in 0..grid.cols {
            match grid.post(r, c) {
                Some(h) if h.is_finite() && h >= emin && h <= emax => {
                    heights.push(Some(h));
                    survived = survived.wrapping_add(1);
                }
                Some(_) => {
                    dispositions.push(RecordDisposition {
                        source: SourceRecordRef::terrain(
                            grid.source,
                            grid.origin_lat_deg,
                            grid.origin_lon_deg,
                            r,
                            c,
                        ),
                        disposition: Disposition::RejectedOutlier {
                            reason: "terrain elevation out of bounds",
                        },
                    });
                    outliers = outliers.wrapping_add(1);
                    heights.push(None);
                }
                None => heights.push(None),
            }
        }
    }
    (heights, outliers, survived)
}

/// Fills a run of voids of length `<= max_span` bounded by present posts on both
/// sides of a row by linear interpolation. Edge runs (a run reaching a row edge
/// with no bounding post) and runs wider than the policy stay `None`. Runs never
/// cross a row boundary, so a fill is always between two posts of the same row.
fn row_gap_fill(heights: &mut [Option<f64>], rows: u32, cols: u32, max_span: u32) {
    let cols = cols as usize;
    if max_span == 0 || cols < 3 {
        return;
    }
    for r in 0..rows {
        let base = (r as usize) * cols;
        let mut c = 1usize;
        while c + 1 < cols {
            if heights[base + c].is_some() {
                c += 1;
                continue;
            }
            let run_end = gap_end(heights, base, c, cols);
            let span = (run_end - c) as u32;
            if run_end < cols
                && span <= max_span
                && let (Some(left), Some(right)) = (heights[base + c - 1], heights[base + run_end])
            {
                fill_run(heights, base, c, run_end, left, right);
            }
            c = run_end.max(c + 1);
        }
    }
}

/// The index one past the end of the void run starting at `c` within a row.
fn gap_end(heights: &[Option<f64>], base: usize, c: usize, cols: usize) -> usize {
    let mut end = c;
    while end < cols && heights[base + end].is_none() {
        end += 1;
    }
    end
}

/// Linearly fills `[c, run_end)` between `left` and `right`.
fn fill_run(
    heights: &mut [Option<f64>],
    base: usize,
    c: usize,
    run_end: usize,
    left: f64,
    right: f64,
) {
    let span = (run_end - c + 1) as f64;
    for (k, slot) in (c..run_end).enumerate() {
        let t = (k as f64 + 1.0) / span;
        heights[base + slot] = Some(left + (right - left) * t);
    }
}

/// Converts every surviving post's height to the target vertical datum at its
/// target-frame coordinates.
fn convert_heights(
    src_heights: &[Option<f64>],
    grid: &TerrainGrid,
    meta: &SourceMeta,
    config: &BuildConfig,
    origin_lat_t: f64,
    origin_lon_t: f64,
) -> Result<Vec<Option<f64>>, BuildError> {
    let mut out = Vec::with_capacity(src_heights.len());
    for r in 0..grid.rows {
        for c in 0..grid.cols {
            let idx = (r as usize) * (grid.cols as usize) + (c as usize);
            match src_heights[idx] {
                Some(h) => {
                    let lat_t = origin_lat_t + f64::from(r) * grid.step_deg;
                    let lon_t = origin_lon_t + f64::from(c) * grid.step_deg;
                    let converted = convert_vertical(
                        h,
                        meta.vertical_datum,
                        config.target.vertical,
                        lat_t,
                        lon_t,
                    )?;
                    out.push(Some(converted));
                }
                None => out.push(None),
            }
        }
    }
    Ok(out)
}
