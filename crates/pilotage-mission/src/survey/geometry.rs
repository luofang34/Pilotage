//! Planar sweep geometry for the survey planner, in anchor-relative
//! meters: polygon area, the line direction, and the parallel lines that
//! cover the polygon at a given footprint and spacing.

/// A point or vector in the tangent plane: north meters, east meters.
pub(super) type Planar = [f64; 2];

/// One survey line in fly order, with the overshoot applied.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Line {
    pub(super) start: Planar,
    pub(super) end: Planar,
}

impl Line {
    pub(super) fn length_m(&self) -> f64 {
        distance_m(self.start, self.end)
    }
}

pub(super) fn distance_m(a: Planar, b: Planar) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

fn dot(a: Planar, b: Planar) -> f64 {
    a[0] * b[0] + a[1] * b[1]
}

/// The polygon's area by the shoelace formula, square meters, always
/// non-negative. Vertices are in order without a repeated closing vertex.
pub(super) fn area_m2(vertices: &[Planar]) -> f64 {
    let twice: f64 = vertices
        .iter()
        .enumerate()
        .map(|(index, a)| {
            let b = vertices[(index + 1) % vertices.len()];
            a[0] * b[1] - b[0] * a[1]
        })
        .sum();
    twice.abs() / 2.0
}

/// The unit direction of the polygon's longest edge: the sweep direction
/// that needs the fewest turns on a typical field.
pub(super) fn longest_edge_direction(vertices: &[Planar]) -> Planar {
    let mut best = [1.0, 0.0];
    let mut best_length = -1.0;
    for (index, a) in vertices.iter().enumerate() {
        let b = vertices[(index + 1) % vertices.len()];
        let length = distance_m(*a, b);
        if length > best_length {
            best_length = length;
            best = [(b[0] - a[0]) / length, (b[1] - a[1]) / length];
        }
    }
    best
}

/// A unit direction from a bearing in radians clockwise from north.
pub(super) fn direction_from_bearing(bearing_rad: f64) -> Planar {
    [bearing_rad.cos(), bearing_rad.sin()]
}

/// The across-track offsets of the sweep lines that cover the polygon's
/// width: the first swath edge sits on one side of the polygon, the last
/// on the far side, and the lines between are spread evenly at no more
/// than `spacing_m`. Even spacing keeps every gap between two lines wide
/// enough to turn in; pulling only the last line back could leave a gap
/// too narrow for the turn. A polygon narrower than the footprint gets one
/// line through its middle. Returns `None` when more than `max_lines`
/// would be needed.
pub fn sweep_offsets(
    width_m: f64,
    footprint_m: f64,
    spacing_m: f64,
    max_lines: usize,
) -> Option<Vec<f64>> {
    let half = footprint_m / 2.0;
    if width_m <= footprint_m {
        return Some(vec![width_m / 2.0]);
    }
    let span = width_m - footprint_m;
    let steps = (span / spacing_m).ceil().max(1.0);
    if !steps.is_finite() || steps >= max_lines as f64 {
        return None;
    }
    let step = span / steps;
    Some(
        (0..=steps as usize)
            .map(|index| half + index as f64 * step)
            .collect(),
    )
}

/// The along-track span of the polygon on the line at `across`, as the
/// extreme intersections of that line with the polygon edges. `None` when
/// the line only touches the polygon.
fn span_at(
    vertices: &[Planar],
    direction: Planar,
    across_axis: Planar,
    across: f64,
) -> Option<[f64; 2]> {
    let mut min = f64::INFINITY;
    let mut max = f64::NEG_INFINITY;
    for (index, a) in vertices.iter().enumerate() {
        let b = vertices[(index + 1) % vertices.len()];
        let (ta, tb) = (dot(*a, across_axis), dot(b, across_axis));
        let (sa, sb) = (dot(*a, direction), dot(b, direction));
        if (ta - across) * (tb - across) > 0.0 {
            continue;
        }
        if ta == tb {
            min = min.min(sa.min(sb));
            max = max.max(sa.max(sb));
            continue;
        }
        let s = sa + (across - ta) / (tb - ta) * (sb - sa);
        min = min.min(s);
        max = max.max(s);
    }
    (max - min > 1e-6).then_some([min, max])
}

/// The survey lines in boustrophedon order: each line spans the polygon
/// at its across-track offset, extended by `overshoot_m` at both ends, and
/// consecutive lines are flown in opposite directions.
pub(super) fn sweep_lines(
    vertices: &[Planar],
    direction: Planar,
    offsets: &[f64],
    across_min: f64,
    overshoot_m: f64,
) -> Vec<Line> {
    let across_axis = [-direction[1], direction[0]];
    let point = |along: f64, across: f64| -> Planar {
        [
            direction[0] * along + across_axis[0] * across,
            direction[1] * along + across_axis[1] * across,
        ]
    };
    let mut lines = Vec::new();
    for offset in offsets {
        let across = across_min + offset;
        let Some([min, max]) = span_at(vertices, direction, across_axis, across) else {
            continue;
        };
        let (from, to) = (min - overshoot_m, max + overshoot_m);
        let forward = lines.len() % 2 == 0;
        let (start, end) = if forward { (from, to) } else { (to, from) };
        lines.push(Line {
            start: point(start, across),
            end: point(end, across),
        });
    }
    lines
}

/// The smallest across-track coordinate of the polygon and its width
/// along `direction`'s perpendicular.
pub(super) fn across_extent(vertices: &[Planar], direction: Planar) -> (f64, f64) {
    let across_axis = [-direction[1], direction[0]];
    let mut min = f64::INFINITY;
    let mut max = f64::NEG_INFINITY;
    for vertex in vertices {
        let across = dot(*vertex, across_axis);
        min = min.min(across);
        max = max.max(across);
    }
    (min, max - min)
}
