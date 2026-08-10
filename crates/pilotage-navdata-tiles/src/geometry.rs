//! Deterministic clipping for vector tile geometry.

use crate::mercator::{TileBounds, WorldPoint};

pub(crate) fn point_bounds(points: &[WorldPoint]) -> Option<(WorldPoint, WorldPoint)> {
    let first = *points.first()?;
    let mut min = first;
    let mut max = first;
    for point in &points[1..] {
        min.x = min.x.min(point.x);
        min.y = min.y.min(point.y);
        max.x = max.x.max(point.x);
        max.y = max.y.max(point.y);
    }
    Some((min, max))
}

pub(crate) fn clip_segment(
    start: WorldPoint,
    end: WorldPoint,
    bounds: TileBounds,
) -> Option<(WorldPoint, WorldPoint)> {
    let delta_x = end.x - start.x;
    let delta_y = end.y - start.y;
    let mut lower = 0.0;
    let mut upper = 1.0;
    for (p, q) in [
        (-delta_x, start.x - bounds.min_x),
        (delta_x, bounds.max_x - start.x),
        (-delta_y, start.y - bounds.min_y),
        (delta_y, bounds.max_y - start.y),
    ] {
        if p.abs() <= f64::EPSILON {
            if q < 0.0 {
                return None;
            }
            continue;
        }
        let ratio = q / p;
        if p < 0.0 {
            lower = f64::max(lower, ratio);
        } else {
            upper = f64::min(upper, ratio);
        }
        if lower > upper {
            return None;
        }
    }
    Some((
        interpolate(start, end, lower),
        interpolate(start, end, upper),
    ))
}

pub(crate) fn clip_polygon(points: &[WorldPoint], bounds: TileBounds) -> Vec<WorldPoint> {
    let mut output = points.to_vec();
    output = clip_edge(
        &output,
        |point| point.x >= bounds.min_x,
        |a, b| intersect_vertical(a, b, bounds.min_x),
    );
    output = clip_edge(
        &output,
        |point| point.x <= bounds.max_x,
        |a, b| intersect_vertical(a, b, bounds.max_x),
    );
    output = clip_edge(
        &output,
        |point| point.y >= bounds.min_y,
        |a, b| intersect_horizontal(a, b, bounds.min_y),
    );
    clip_edge(
        &output,
        |point| point.y <= bounds.max_y,
        |a, b| intersect_horizontal(a, b, bounds.max_y),
    )
}

fn clip_edge(
    input: &[WorldPoint],
    inside: impl Fn(WorldPoint) -> bool,
    intersection: impl Fn(WorldPoint, WorldPoint) -> WorldPoint,
) -> Vec<WorldPoint> {
    let Some(mut previous) = input.last().copied() else {
        return Vec::new();
    };
    let mut output = Vec::new();
    for &current in input {
        let current_inside = inside(current);
        let previous_inside = inside(previous);
        if current_inside {
            if !previous_inside {
                output.push(intersection(previous, current));
            }
            output.push(current);
        } else if previous_inside {
            output.push(intersection(previous, current));
        }
        previous = current;
    }
    output
}

fn intersect_vertical(start: WorldPoint, end: WorldPoint, x: f64) -> WorldPoint {
    let delta = end.x - start.x;
    let ratio = if delta.abs() <= f64::EPSILON {
        0.0
    } else {
        (x - start.x) / delta
    };
    WorldPoint {
        x,
        y: start.y + ratio * (end.y - start.y),
    }
}

fn intersect_horizontal(start: WorldPoint, end: WorldPoint, y: f64) -> WorldPoint {
    let delta = end.y - start.y;
    let ratio = if delta.abs() <= f64::EPSILON {
        0.0
    } else {
        (y - start.y) / delta
    };
    WorldPoint {
        x: start.x + ratio * (end.x - start.x),
        y,
    }
}

fn interpolate(start: WorldPoint, end: WorldPoint, ratio: f64) -> WorldPoint {
    WorldPoint {
        x: start.x + ratio * (end.x - start.x),
        y: start.y + ratio * (end.y - start.y),
    }
}
