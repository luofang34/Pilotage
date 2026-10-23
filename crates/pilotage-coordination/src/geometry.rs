//! Planar geometry on a local tangent plane: equirectangular around a
//! reference point, adequate for areas of a few kilometres.

use pilotage_protocol::wire;

/// The WGS-84 equatorial radius, metres.
const RADIUS_M: f64 = 6_378_137.0;

/// A local north-east frame around a reference point.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Local {
    latitude_deg: f64,
    longitude_deg: f64,
}

impl Local {
    /// The frame around the centroid of `points`.
    #[must_use]
    pub fn around(points: &[wire::GeoPoint]) -> Self {
        let count = points.len().max(1) as f64;
        Self {
            latitude_deg: points.iter().map(|p| p.latitude_deg).sum::<f64>() / count,
            longitude_deg: points.iter().map(|p| p.longitude_deg).sum::<f64>() / count,
        }
    }

    /// `point` as north and east metres.
    #[must_use]
    pub fn project(&self, point: &wire::GeoPoint) -> [f64; 2] {
        [
            (point.latitude_deg - self.latitude_deg).to_radians() * RADIUS_M,
            (point.longitude_deg - self.longitude_deg).to_radians()
                * RADIUS_M
                * self.latitude_deg.to_radians().cos(),
        ]
    }

    /// North and east metres as a position.
    #[must_use]
    pub fn unproject(&self, [north, east]: [f64; 2]) -> wire::GeoPoint {
        wire::GeoPoint {
            latitude_deg: self.latitude_deg + (north / RADIUS_M).to_degrees(),
            longitude_deg: self.longitude_deg
                + (east / (RADIUS_M * self.latitude_deg.to_radians().cos())).to_degrees(),
        }
    }
}

pub(crate) fn dot(a: [f64; 2], b: [f64; 2]) -> f64 {
    a[0] * b[0] + a[1] * b[1]
}

/// The bearing, radians clockwise from north, of the polygon's longest
/// edge: the survey planner's default line direction.
pub(crate) fn longest_edge_bearing(vertices: &[[f64; 2]]) -> f64 {
    let mut best = (0.0, 0.0);
    for (index, a) in vertices.iter().enumerate() {
        let Some(b) = vertices.get((index + 1) % vertices.len()) else {
            continue;
        };
        let (dn, de) = (b[0] - a[0], b[1] - a[1]);
        let length = dn.hypot(de);
        if length > best.0 {
            best = (length, de.atan2(dn));
        }
    }
    best.1
}

/// The polygon clipped to the strip `low ≤ p·axis ≤ high` (convex clip of
/// each half-plane in turn).
pub(crate) fn clip_to_strip(
    vertices: &[[f64; 2]],
    axis: [f64; 2],
    low: f64,
    high: f64,
) -> Vec<[f64; 2]> {
    let lower = clip_half_plane(vertices, |p| dot(p, axis) - low);
    clip_half_plane(&lower, |p| high - dot(p, axis))
}

/// Keeps the part of the polygon where `inside(p) ≥ 0`.
fn clip_half_plane(vertices: &[[f64; 2]], inside: impl Fn([f64; 2]) -> f64) -> Vec<[f64; 2]> {
    let mut out = Vec::with_capacity(vertices.len() + 2);
    for (index, &current) in vertices.iter().enumerate() {
        let Some(&next) = vertices.get((index + 1) % vertices.len()) else {
            continue;
        };
        let (a, b) = (inside(current), inside(next));
        if a >= 0.0 {
            out.push(current);
        }
        if (a >= 0.0) != (b >= 0.0) {
            let t = a / (a - b);
            out.push([
                current[0] + t * (next[0] - current[0]),
                current[1] + t * (next[1] - current[1]),
            ]);
        }
    }
    out
}

/// The across-track offsets of the lines that cover `width_m`, as the
/// survey planner lays them: the first swath edge on one side, a fixed
/// spacing, the last line pulled back to the far side.
pub(crate) fn sweep_offsets(width_m: f64, footprint_m: f64, spacing_m: f64) -> Vec<f64> {
    let half = footprint_m / 2.0;
    if width_m <= footprint_m || spacing_m <= 0.0 {
        return vec![width_m / 2.0];
    }
    let last = width_m - half;
    let steps = ((width_m - footprint_m) / spacing_m).ceil().max(0.0) as usize;
    let mut offsets: Vec<f64> = (0..steps)
        .map(|index| half + index as f64 * spacing_m)
        .filter(|offset| *offset < last)
        .collect();
    offsets.push(last);
    offsets
}
