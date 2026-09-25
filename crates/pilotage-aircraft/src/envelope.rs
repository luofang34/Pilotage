//! The center-of-gravity envelope as a polygon of (weight kg, arm m)
//! vertices.

/// Arm tolerance for a point on an envelope edge, in metres. A flight manual
/// counts a point on the limit as inside it.
const ON_EDGE_M: f64 = 1e-9;

/// Area, as a part of the bounding box, below which a polygon is flat.
const FLAT_RATIO: f64 = 1e-9;

type Point = [f64; 2];

/// Whether the point is inside the envelope or on its boundary.
pub(crate) fn contains(polygon: &[Point], point: Point) -> bool {
    edges(polygon).any(|edge| on_edge(edge, point)) || odd_crossings(polygon, point)
}

/// Whether the polygon has an area and no two edges cross or touch except
/// adjacent edges at their shared vertex. The even-odd test has no meaning
/// for a polygon that crosses itself.
pub(crate) fn is_simple(polygon: &[Point]) -> bool {
    let n = polygon.len();
    let twice_area: f64 = edges(polygon)
        .map(|([w1, a1], [w2, a2])| w1 * a2 - w2 * a1)
        .sum();
    let span = |axis: usize| {
        let values = polygon.iter().map(|p| p[axis]);
        values.clone().fold(f64::MIN, f64::max) - values.fold(f64::MAX, f64::min)
    };
    // Relative to the bounding box, so rounding cannot make a flat polygon
    // look like it has an area.
    if n < 3 || twice_area.abs() <= FLAT_RATIO * span(0) * span(1) {
        return false;
    }
    let all: Vec<(Point, Point)> = edges(polygon).collect();
    all.iter().enumerate().all(|(i, &first)| {
        all.iter()
            .enumerate()
            .skip(i + 2)
            .filter(|&(j, _)| !(i == 0 && j == n - 1))
            .all(|(_, &second)| !segments_meet(first, second))
    })
}

fn edges(polygon: &[Point]) -> impl Iterator<Item = (Point, Point)> + '_ {
    polygon
        .iter()
        .copied()
        .zip(polygon.iter().copied().cycle().skip(1))
}

fn on_edge(([w1, a1], [w2, a2]): (Point, Point), [w, a]: Point) -> bool {
    if w < w1.min(w2) || w > w1.max(w2) {
        return false;
    }
    if w1 == w2 {
        return a >= a1.min(a2) - ON_EDGE_M && a <= a1.max(a2) + ON_EDGE_M;
    }
    let arm = a1 + (a2 - a1) * (w - w1) / (w2 - w1);
    (a - arm).abs() <= ON_EDGE_M
}

fn odd_crossings(polygon: &[Point], [w, a]: Point) -> bool {
    edges(polygon)
        .filter(|&([wi, ai], [wj, aj])| {
            (wi > w) != (wj > w) && a < (aj - ai) * (w - wi) / (wj - wi) + ai
        })
        .count()
        % 2
        == 1
}

fn orientation(p: Point, q: Point, r: Point) -> f64 {
    (q[0] - p[0]) * (r[1] - p[1]) - (q[1] - p[1]) * (r[0] - p[0])
}

fn segments_meet((p1, p2): (Point, Point), (q1, q2): (Point, Point)) -> bool {
    let d1 = orientation(q1, q2, p1);
    let d2 = orientation(q1, q2, p2);
    let d3 = orientation(p1, p2, q1);
    let d4 = orientation(p1, p2, q2);
    if d1 * d2 < 0.0 && d3 * d4 < 0.0 {
        return true;
    }
    let within = |p: Point, q: Point, r: Point| {
        r[0] >= p[0].min(q[0])
            && r[0] <= p[0].max(q[0])
            && r[1] >= p[1].min(q[1])
            && r[1] <= p[1].max(q[1])
    };
    (d1 == 0.0 && within(q1, q2, p1))
        || (d2 == 0.0 && within(q1, q2, p2))
        || (d3 == 0.0 && within(p1, p2, q1))
        || (d4 == 0.0 && within(p1, p2, q2))
}

#[cfg(test)]
mod tests;
