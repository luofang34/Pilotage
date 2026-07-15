//! Bilinear resampling of an output grid node from prepared source grids.
//!
//! A node is populated only when all four bracketing source posts are present in
//! some prepared grid; the elevation is their bilinear blend and the node traces
//! to those four source records. When no grid brackets the node with four
//! present corners, the node is a hole and the resampler returns `None` so the
//! caller records a void rather than inventing a value.

use crate::chain::terrain::prepare::{Prepared, PreparedGrid};
use crate::element::OutputPost;

/// Resamples node `(i, j)` at `(lat_deg, lon_deg)` from the first prepared grid
/// that brackets it, or `None` when none does.
pub(crate) fn resample_node(
    prepared: &[Prepared],
    i: u32,
    j: u32,
    lat_deg: f64,
    lon_deg: f64,
) -> Option<OutputPost> {
    for candidate in prepared {
        if let Some(post) = resample_from(&candidate.grid, i, j, lat_deg, lon_deg) {
            return Some(post);
        }
    }
    None
}

/// Resamples a node from one prepared grid, or `None` if the grid does not
/// bracket the node with four present corners.
fn resample_from(
    grid: &PreparedGrid,
    i: u32,
    j: u32,
    lat_deg: f64,
    lon_deg: f64,
) -> Option<OutputPost> {
    let fr = (lat_deg - grid.origin_lat_t) / grid.step_deg;
    let fc = (lon_deg - grid.origin_lon_t) / grid.step_deg;
    if !(fr.is_finite() && fc.is_finite()) {
        return None;
    }
    let r0f = fr.floor();
    let c0f = fc.floor();
    if r0f < 0.0 || c0f < 0.0 || r0f >= f64::from(grid.rows) || c0f >= f64::from(grid.cols) {
        return None;
    }
    let r0 = r0f as u32;
    let c0 = c0f as u32;
    let (h00, h01) = (grid.at(r0, c0)?, grid.at(r0, c0 + 1)?);
    let (h10, h11) = (grid.at(r0 + 1, c0)?, grid.at(r0 + 1, c0 + 1)?);
    let tr = fr - r0f;
    let tc = fc - c0f;
    let top = h00 * (1.0 - tc) + h01 * tc;
    let bottom = h10 * (1.0 - tc) + h11 * tc;
    let elevation_m = top * (1.0 - tr) + bottom * tr;
    // Cite the records that actually contribute each corner value: a
    // gap-filled corner resolves to its bounding posts, so a rejected or
    // void input record never appears in output lineage.
    let mut sources = Vec::with_capacity(8);
    grid.contributor_refs(r0, c0, &mut sources);
    grid.contributor_refs(r0, c0 + 1, &mut sources);
    grid.contributor_refs(r0 + 1, c0, &mut sources);
    grid.contributor_refs(r0 + 1, c0 + 1, &mut sources);
    sources.sort();
    sources.dedup();
    Some(OutputPost {
        i,
        j,
        lat_deg,
        lon_deg,
        elevation_m,
        sources,
    })
}
