//! Area survey planning: a polygon, a height above the anchor, and a
//! camera footprint become a flight plan of parallel lines.
//!
//! The planner is sans-IO and deterministic. It projects the polygon onto
//! the tangent plane at the mission anchor, sweeps parallel lines across
//! the polygon at the spacing the footprint and sidelap give, and returns
//! the line ends as fly-by waypoints in boustrophedon order. A concave
//! polygon is covered by the full span of each sweep line through it; the
//! planner does not split a line at a concavity.

mod geometry;
pub mod objective_change;
#[cfg(test)]
mod tests;

use core::f64::consts::PI;

use navigate_contract::{
    AltitudeConstraint, FlightPlan, GeodeticPosition, PlanRole, TurnType, Waypoint,
};
use navigate_fpl::{ExecutionConfig, turn_radius_m};
use navigate_geodesy::{GeodesyError, LocalTangentPlane, NedOffset};
use pilotage_mission_core::{Digest, MissionIdentity};
use sha2::{Digest as _, Sha256};

pub use geometry::sweep_offsets;
use geometry::{Line, Planar};

/// Upper bound on the waypoints one survey plan may hold.
pub const MAX_SURVEY_WAYPOINTS: usize = 400;
/// Smallest polygon area the planner accepts, square meters.
const MIN_AREA_M2: f64 = 1.0;
/// Smallest line spacing the planner accepts, meters.
const MIN_SPACING_M: f64 = 1.0;
/// Largest sidelap fraction the planner accepts.
const MAX_SIDELAP: f64 = 0.95;
const DIGEST_DOMAIN: &[u8] = b"pilotage.survey-request.v1\0";
/// Seconds of cruise a line end's capture radius spans.
const CAPTURE_SECONDS: f64 = 2.0;

/// What to survey and how the camera sees the ground.
#[derive(Debug, Clone, PartialEq)]
pub struct SurveyRequest {
    /// Polygon vertices as `[latitude_deg, longitude_deg]`, in order,
    /// without a repeated closing vertex.
    pub polygon_deg: Vec<[f64; 2]>,
    /// The camera's across-track field of view for a nadir camera,
    /// radians. The default is the 4:3 photo width of a consumer
    /// multicopter camera, 69.6 degrees.
    pub across_track_fov_rad: f64,
    /// The fraction of the footprint two adjacent lines share, in
    /// `[0, 0.95]`. Default 0.6.
    pub sidelap: f64,
    /// The distance each line extends past the polygon at both ends,
    /// meters, so the camera is on the line before the polygon edge.
    /// Default 10.
    pub overshoot_m: f64,
    /// The line bearing, radians clockwise from north. `None` follows the
    /// polygon's longest edge.
    pub line_bearing_rad: Option<f64>,
    /// The distance to a line end at which the executor captures it,
    /// meters. Default 5.
    pub capture_radius_m: f64,
    /// A measured ground footprint width, meters, in place of the one the
    /// field of view and the height give. `None` derives the footprint.
    pub footprint_width_m: Option<f64>,
    /// Whether the plan ends over the anchor at the survey height, so the
    /// vehicle comes back before the mission completes and the pilot
    /// lands it. Default true.
    pub return_to_launch: bool,
}

impl SurveyRequest {
    /// A request over the documented defaults.
    #[must_use]
    pub fn new(polygon_deg: Vec<[f64; 2]>) -> Self {
        Self {
            polygon_deg,
            // 69.6 degrees.
            across_track_fov_rad: 1.214_7,
            sidelap: 0.6,
            overshoot_m: 10.0,
            line_bearing_rad: None,
            capture_radius_m: 5.0,
            footprint_width_m: None,
            return_to_launch: true,
        }
    }

    /// Sizes the capture radius and the overshoot for a vehicle that
    /// cruises at `cruise_mps`. A fast vehicle that lags its setpoint needs
    /// a wider capture circle to sequence a line end on the first pass. A
    /// fly-by line end starts its turn up to one turn radius before the
    /// end, so the line extends that far past the area: the turn stays
    /// outside, and the camera stays on the line across the area.
    pub fn fit_to_cruise(&mut self, cruise_mps: f64) {
        if !(cruise_mps.is_finite() && cruise_mps > 0.0) {
            return;
        }
        self.capture_radius_m = self.capture_radius_m.max(CAPTURE_SECONDS * cruise_mps);
        let bank = ExecutionConfig::default().bank_limit_rad;
        self.overshoot_m = self.overshoot_m.max(turn_radius_m(cruise_mps, bank));
    }

    /// The content digest of the request: what the plan was derived from.
    #[must_use]
    pub fn digest(&self) -> Digest {
        let mut hasher = Sha256::new();
        hasher.update(DIGEST_DOMAIN);
        hasher.update((self.polygon_deg.len() as u64).to_le_bytes());
        for [latitude_deg, longitude_deg] in &self.polygon_deg {
            hasher.update(latitude_deg.to_le_bytes());
            hasher.update(longitude_deg.to_le_bytes());
        }
        hasher.update(self.across_track_fov_rad.to_le_bytes());
        hasher.update(self.sidelap.to_le_bytes());
        hasher.update(self.overshoot_m.to_le_bytes());
        hasher.update([u8::from(self.line_bearing_rad.is_some())]);
        hasher.update(self.line_bearing_rad.unwrap_or(0.0).to_le_bytes());
        hasher.update(self.capture_radius_m.to_le_bytes());
        hasher.update([u8::from(self.footprint_width_m.is_some())]);
        hasher.update(self.footprint_width_m.unwrap_or(0.0).to_le_bytes());
        hasher.update([u8::from(self.return_to_launch)]);
        Digest::from_bytes(hasher.finalize().into())
    }
}

/// A planned survey: the plan and the figures an operator checks.
#[derive(Debug, Clone, PartialEq)]
pub struct SurveyPlan {
    /// The waypoints in fly order, two fly-by turns per line, then home
    /// as a fly-over.
    pub plan: FlightPlan,
    /// The content digest of the request the plan came from.
    pub digest: Digest,
    /// Number of survey lines.
    pub line_count: usize,
    /// Distance between adjacent lines, meters.
    pub line_spacing_m: f64,
    /// Ground width the camera sees at the survey height, meters.
    pub footprint_width_m: f64,
    /// Length of the path from the first waypoint to the last, meters.
    pub path_length_m: f64,
    /// The capture radius the plan can be flown with, meters: the
    /// requested radius, or half the shortest leg when that is smaller,
    /// because a leg shorter than its capture radius cannot be activated.
    pub capture_radius_m: f64,
}

/// The record returned when a survey mission is built: what the plan
/// came from and what it covers, for evidence and operator display.
#[derive(Debug, Clone, PartialEq)]
pub struct SurveyPlanRecord {
    /// The content digest of the survey request.
    pub digest: Digest,
    /// Number of survey lines.
    pub line_count: usize,
    /// Distance between adjacent lines, meters.
    pub line_spacing_m: f64,
    /// Ground width the camera sees at the survey height, meters.
    pub footprint_width_m: f64,
    /// Length of the path from the first waypoint to the last, meters.
    pub path_length_m: f64,
    /// The capture radius the plan is flown with, meters.
    pub capture_radius_m: f64,
    /// Number of waypoints in the built plan.
    pub waypoint_count: usize,
    /// Waypoint identifiers in fly order.
    pub idents: Vec<String>,
    /// Identity of the mission document that owns the plan execution.
    pub mission_identity: MissionIdentity,
}

/// Why a survey request cannot become a plan.
#[derive(Debug, thiserror::Error)]
pub enum SurveyError {
    /// A polygon needs at least three vertices.
    #[error("survey polygon has {count} vertices; at least 3 are required")]
    TooFewVertices {
        /// The vertex count given.
        count: usize,
    },
    /// A vertex is not a finite position on the Earth.
    #[error("survey polygon vertex {index} is not a position on the Earth: {vertex:?}")]
    ImplausibleVertex {
        /// Index of the vertex in the request.
        index: usize,
        /// The rejected vertex.
        vertex: [f64; 2],
    },
    /// The polygon encloses no usable area.
    #[error("survey polygon encloses {area_m2} m²; at least {MIN_AREA_M2} m² is required")]
    DegenerateArea {
        /// The enclosed area, square meters.
        area_m2: f64,
    },
    /// A parameter is outside its range.
    #[error("survey {parameter} {value} is outside {range}")]
    ParameterOutOfRange {
        /// The parameter name.
        parameter: &'static str,
        /// The rejected value.
        value: f64,
        /// The accepted range.
        range: &'static str,
    },
    /// The footprint and sidelap give lines too close together.
    #[error("survey line spacing {spacing_m} m is below {MIN_SPACING_M} m")]
    SpacingTooSmall {
        /// The computed spacing, meters.
        spacing_m: f64,
    },
    /// The polygon needs more waypoints than one plan may hold.
    #[error("survey needs more than {MAX_SURVEY_WAYPOINTS} waypoints")]
    TooManyWaypoints,
    /// The anchor is not a plausible tangent-plane origin.
    #[error("survey anchor rejected by geodesy")]
    Geodesy(#[from] GeodesyError),
}

/// Plans the survey of `request` at `height_above_anchor_m` above the
/// anchor altitude.
///
/// # Errors
///
/// A [`SurveyError`] for a malformed polygon, a parameter outside its
/// range, a spacing below the floor, or a plan beyond the waypoint bound.
pub fn plan_survey(
    request: &SurveyRequest,
    anchor: &GeodeticPosition,
    height_above_anchor_m: f64,
) -> Result<SurveyPlan, SurveyError> {
    validate_parameters(request, height_above_anchor_m)?;
    let plane = LocalTangentPlane::new(*anchor)?;
    let vertices = project(request, anchor, &plane)?;
    let area_m2 = geometry::area_m2(&vertices);
    if area_m2 < MIN_AREA_M2 {
        return Err(SurveyError::DegenerateArea { area_m2 });
    }
    let footprint_width_m = request.footprint_width_m.unwrap_or_else(|| {
        2.0 * height_above_anchor_m * (request.across_track_fov_rad / 2.0).tan()
    });
    let line_spacing_m = footprint_width_m * (1.0 - request.sidelap);
    if line_spacing_m < MIN_SPACING_M {
        return Err(SurveyError::SpacingTooSmall {
            spacing_m: line_spacing_m,
        });
    }
    let direction = match request.line_bearing_rad {
        Some(bearing_rad) => geometry::direction_from_bearing(bearing_rad),
        None => geometry::longest_edge_direction(&vertices),
    };
    let (across_min, width_m) = geometry::across_extent(&vertices, direction);
    let offsets = geometry::sweep_offsets(
        width_m,
        footprint_width_m,
        line_spacing_m,
        MAX_SURVEY_WAYPOINTS / 2,
    )
    .ok_or(SurveyError::TooManyWaypoints)?;
    // The lines are spread evenly, so they may sit closer than the
    // sidelap asks; the plan reports the spacing it flies.
    let line_spacing_m = match offsets.as_slice() {
        [first, second, ..] => second - first,
        _ => line_spacing_m,
    };
    let lines = geometry::sweep_lines(
        &vertices,
        direction,
        &offsets,
        across_min,
        request.overshoot_m,
    );
    let digest = request.digest();
    let altitude_m = anchor.altitude_m + height_above_anchor_m;
    // The plane's origin is the anchor: the launch point.
    let home = request.return_to_launch.then_some([0.0, 0.0]);
    let plan = FlightPlan::new(
        format!("survey:{digest}"),
        PlanRole::Mission,
        waypoints(&lines, home, &plane, height_above_anchor_m, altitude_m),
    );
    Ok(SurveyPlan {
        plan,
        digest,
        line_count: lines.len(),
        line_spacing_m,
        footprint_width_m,
        path_length_m: path_length_m(&lines, home),
        capture_radius_m: request
            .capture_radius_m
            .min(shortest_leg_m(&lines, home) / 2.0),
    })
}

/// The shortest leg of the path: a line, the gap between two lines, or
/// the way home.
fn shortest_leg_m(lines: &[Line], home: Option<Planar>) -> f64 {
    let mut shortest = f64::INFINITY;
    for (index, line) in lines.iter().enumerate() {
        shortest = shortest.min(line.length_m());
        if let Some(next) = lines.get(index + 1) {
            shortest = shortest.min(geometry::distance_m(line.end, next.start));
        }
    }
    if let (Some(last), Some(home)) = (lines.last(), home) {
        shortest = shortest.min(geometry::distance_m(last.end, home));
    }
    shortest
}

fn validate_parameters(request: &SurveyRequest, height_m: f64) -> Result<(), SurveyError> {
    let check = |parameter: &'static str, value: f64, ok: bool, range: &'static str| {
        if value.is_finite() && ok {
            Ok(())
        } else {
            Err(SurveyError::ParameterOutOfRange {
                parameter,
                value,
                range,
            })
        }
    };
    match request.footprint_width_m {
        // A measured footprint stands on its own; the height may be zero
        // for a planar vehicle.
        Some(width_m) => {
            check("footprint width", width_m, width_m > 0.0, "(0, inf) m")?;
            check("height", height_m, height_m >= 0.0, "[0, inf) m")?;
        }
        None => check("height", height_m, height_m > 0.0, "(0, inf) m")?,
    }
    let fov = request.across_track_fov_rad;
    check(
        "across-track field of view",
        fov,
        fov > 0.0 && fov < PI,
        "(0, pi) rad",
    )?;
    let sidelap = request.sidelap;
    check(
        "sidelap",
        sidelap,
        (0.0..=MAX_SIDELAP).contains(&sidelap),
        "[0, 0.95]",
    )?;
    check(
        "overshoot",
        request.overshoot_m,
        request.overshoot_m >= 0.0,
        "[0, inf) m",
    )?;
    check(
        "capture radius",
        request.capture_radius_m,
        request.capture_radius_m > 0.0,
        "(0, inf) m",
    )?;
    if let Some(bearing) = request.line_bearing_rad {
        check("line bearing", bearing, true, "finite rad")?;
    }
    Ok(())
}

/// Projects the polygon onto the tangent plane at the anchor, at the
/// anchor altitude.
fn project(
    request: &SurveyRequest,
    anchor: &GeodeticPosition,
    plane: &LocalTangentPlane,
) -> Result<Vec<Planar>, SurveyError> {
    if request.polygon_deg.len() < 3 {
        return Err(SurveyError::TooFewVertices {
            count: request.polygon_deg.len(),
        });
    }
    request
        .polygon_deg
        .iter()
        .enumerate()
        .map(|(index, vertex)| {
            let [latitude_deg, longitude_deg] = *vertex;
            let position = GeodeticPosition::new(
                latitude_deg.to_radians(),
                longitude_deg.to_radians(),
                anchor.altitude_m,
            );
            if !position.is_plausible() {
                return Err(SurveyError::ImplausibleVertex {
                    index,
                    vertex: *vertex,
                });
            }
            let offset = plane.to_ned(&position);
            Ok([offset.north_m, offset.east_m])
        })
        .collect()
}

/// Two fly-by waypoints per line, `L01A`, `L01B`, `L02A`, ... in fly
/// order, all at the survey altitude. A fly-by line end turns onto the
/// next leg before the end instead of past it; the overshoot keeps that
/// turn outside the area. Home is flown over.
fn waypoints(
    lines: &[Line],
    home: Option<Planar>,
    plane: &LocalTangentPlane,
    height_above_anchor_m: f64,
    altitude_m: f64,
) -> Vec<Waypoint> {
    // The plane's down axis leaves the ellipsoid normal away from the
    // anchor by millimetres; the waypoint states the survey altitude.
    let place = |point: Planar| {
        let mut position =
            plane.from_ned(&NedOffset::new(point[0], point[1], -height_above_anchor_m));
        position.altitude_m = altitude_m;
        position
    };
    lines
        .iter()
        .enumerate()
        .flat_map(|(index, line)| {
            [("A", line.start), ("B", line.end)]
                .into_iter()
                .map(move |(end, point)| {
                    Waypoint::new(format!("L{:02}{end}", index + 1), place(point))
                        .with_altitude(AltitudeConstraint::At(altitude_m))
                        .with_turn(TurnType::FlyBy)
                })
        })
        .chain(home.map(|point| {
            Waypoint::new("HOME".to_owned(), place(point))
                .with_altitude(AltitudeConstraint::At(altitude_m))
                .with_turn(TurnType::FlyOver)
        }))
        .collect()
}

fn path_length_m(lines: &[Line], home: Option<Planar>) -> f64 {
    let along: f64 = lines
        .iter()
        .enumerate()
        .map(|(index, line)| {
            line.length_m()
                + lines
                    .get(index + 1)
                    .map_or(0.0, |next| geometry::distance_m(line.end, next.start))
        })
        .sum();
    along
        + match (lines.last(), home) {
            (Some(last), Some(home)) => geometry::distance_m(last.end, home),
            _ => 0.0,
        }
}
