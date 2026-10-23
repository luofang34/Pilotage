//! Allocation: one contiguous band of the team's survey lines per member.
//! Each band is the area clipped to the swaths of its lines, so a member's
//! own planner, given the same height, camera, overlap and line bearing,
//! lays exactly the team's lines inside it.

use pilotage_protocol::wire;

use crate::geometry::{Local, clip_to_strip, dot, longest_edge_bearing, sweep_offsets};

/// One member's share of a survey.
#[derive(Debug, Clone, PartialEq)]
pub struct Assignment {
    /// The member's name.
    pub member: String,
    /// The band the member surveys.
    pub band: Vec<wire::GeoPoint>,
    /// The team's line numbers in the band, first line 1.
    pub lines: Vec<usize>,
}

/// The parameters allocation needs from the objective.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Sweep {
    pub bearing_deg: f64,
    pub footprint_m: f64,
    pub spacing_m: f64,
}

/// The line bearing and swath geometry of `survey` at its height.
pub(crate) fn sweep_of(
    survey: &wire::SurveyObjective,
    height_m: f64,
    fov_deg: f64,
    sidelap: f64,
) -> Sweep {
    let local = Local::around(&survey.area);
    let vertices: Vec<[f64; 2]> = survey.area.iter().map(|p| local.project(p)).collect();
    let bearing_deg = survey
        .line_bearing_deg
        .unwrap_or_else(|| longest_edge_bearing(&vertices).to_degrees());
    let footprint_m = 2.0 * height_m * (fov_deg.to_radians() / 2.0).tan();
    Sweep {
        bearing_deg,
        footprint_m,
        spacing_m: footprint_m * (1.0 - sidelap),
    }
}

/// Splits `area` into one band per member, in the order of the members'
/// launch points across the lines, with line counts as even as possible.
/// A member gets no band when there are fewer lines than members.
#[must_use]
pub fn allocate(
    area: &[wire::GeoPoint],
    members: &[(String, wire::GeoPoint)],
    bearing_deg: f64,
    footprint_m: f64,
    spacing_m: f64,
) -> Vec<Assignment> {
    let local = Local::around(area);
    let vertices: Vec<[f64; 2]> = area.iter().map(|p| local.project(p)).collect();
    let bearing = bearing_deg.to_radians();
    let axis = [-bearing.sin(), bearing.cos()];
    let across: Vec<f64> = vertices.iter().map(|v| dot(*v, axis)).collect();
    let low = across.iter().copied().fold(f64::INFINITY, f64::min);
    let high = across.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let offsets = sweep_offsets(high - low, footprint_m, spacing_m);
    let mut ordered: Vec<&(String, wire::GeoPoint)> = members.iter().collect();
    ordered
        .sort_by(|a, b| dot(local.project(&a.1), axis).total_cmp(&dot(local.project(&b.1), axis)));
    let count = ordered.len().max(1);
    let (base, extra) = (offsets.len() / count, offsets.len() % count);
    let mut next = 0;
    let mut assignments = Vec::new();
    for (index, (member, _)) in ordered.iter().enumerate() {
        let take = base + usize::from(index < extra);
        if take == 0 {
            continue;
        }
        let (Some(first), Some(last)) = (offsets.get(next), offsets.get(next + take - 1)) else {
            break;
        };
        let half = footprint_m / 2.0;
        let band = clip_to_strip(&vertices, axis, low + first - half, low + last + half);
        assignments.push(Assignment {
            member: member.clone(),
            band: band.iter().map(|p| local.unproject(*p)).collect(),
            lines: (next + 1..=next + take).collect(),
        });
        next = next.saturating_add(take);
    }
    assignments
}

/// A thin rectangle around one flown line, narrower than a swath, so the
/// planner lays exactly that line through it.
pub(crate) fn line_strip(path: &[wire::GeoPoint], spacing_m: f64) -> Option<Vec<wire::GeoPoint>> {
    let (first, last) = (path.first()?, path.last()?);
    let local = Local::around(path);
    let (a, b) = (local.project(first), local.project(last));
    let length = (b[0] - a[0]).hypot(b[1] - a[1]);
    if length <= 1.0 {
        return None;
    }
    let along = [(b[0] - a[0]) / length, (b[1] - a[1]) / length];
    let side = [-along[1] * spacing_m / 4.0, along[0] * spacing_m / 4.0];
    let corners = [
        [a[0] - side[0], a[1] - side[1]],
        [b[0] - side[0], b[1] - side[1]],
        [b[0] + side[0], b[1] + side[1]],
        [a[0] + side[0], a[1] + side[1]],
    ];
    Some(corners.iter().map(|c| local.unproject(*c)).collect())
}
