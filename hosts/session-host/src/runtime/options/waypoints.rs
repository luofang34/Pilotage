//! `PILOTAGE_MISSION_WAYPOINTS`: a JSON file with a waypoint list.
//!
//! ```json
//! {
//!   "capture_radius_m": 5.0,
//!   "waypoints": [
//!     { "lat_deg": 47.0006, "lon_deg": 8.0, "altitude": { "at": 512.0 } },
//!     { "lat_deg": 47.0006, "lon_deg": 8.0008,
//!       "altitude": { "window": { "lower_m": 500.0, "upper_m": 540.0 } },
//!       "path": { "course-to-fix": { "course_deg": 90.0, "reference": "true" } },
//!       "max_speed_mps": 25.0, "gradient": 0.05,
//!       "turn": "fly-over", "name": "CORNER" },
//!     { "lat_deg": 47.0012, "lon_deg": 8.0008,
//!       "altitude": { "at-or-above": 520.0 }, "path": "direct-to-fix" }
//!   ]
//! }
//! ```

use std::path::{Path, PathBuf};

use pilotage_mission::{
    LegRole, PlannedAltitude, PlannedCourseReference, PlannedHold, PlannedHoldLength, PlannedPath,
    PlannedWaypoint, WaypointPlan, WaypointTurn, WaypointTurnDirection,
};
use serde::Deserialize;

use crate::error::HostError;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    #[serde(default)]
    capture_radius_m: Option<f64>,
    #[serde(default)]
    arrival_radius_m: Option<f64>,
    waypoints: Vec<Point>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Point {
    lat_deg: f64,
    lon_deg: f64,
    altitude: Altitude,
    #[serde(default)]
    path: Option<Leg>,
    #[serde(default)]
    turn_direction: Option<Direction>,
    #[serde(default)]
    max_speed_mps: Option<f64>,
    #[serde(default)]
    gradient: Option<f64>,
    #[serde(default)]
    role: Option<Role>,
    #[serde(default)]
    rnp_nm: Option<f64>,
    #[serde(default)]
    turn: Turn,
    #[serde(default)]
    name: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
enum Altitude {
    At(f64),
    AtOrAbove(f64),
    AtOrBelow(f64),
    Window { lower_m: f64, upper_m: f64 },
    Unconstrained { reference_m: f64 },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HoldSpec {
    inbound_course_deg: f64,
    reference: Reference,
    turn: Direction,
    length: HoldLength,
}

#[derive(Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
enum HoldLength {
    Minutes(f64),
    DistanceM(f64),
}

#[derive(Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
enum Leg {
    InitialFix,
    DirectToFix,
    CourseToFix {
        course_deg: f64,
        reference: Reference,
    },
    CourseToAltitude {
        course_deg: f64,
        reference: Reference,
    },
    HoldToAltitude(HoldSpec),
    HoldToFix(HoldSpec),
    HoldManual(HoldSpec),
}

#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "kebab-case")]
enum Reference {
    True,
    Magnetic,
}

#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "kebab-case")]
enum Direction {
    Left,
    Right,
}

#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "kebab-case")]
enum Role {
    Departure,
    Enroute,
    Arrival,
    Approach,
    MissedApproach,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
enum Turn {
    #[default]
    FlyBy,
    FlyOver,
}

/// Reads the waypoint list at `path`.
///
/// # Errors
///
/// [`HostError::MissionWaypoints`] when the file cannot be read or is
/// not a waypoint list.
pub(super) fn read_blocking(path: &Path) -> Result<WaypointPlan, HostError> {
    let failed = |reason: String| HostError::MissionWaypoints {
        path: path.to_path_buf(),
        reason,
    };
    let text = std::fs::read_to_string(path).map_err(|error| failed(error.to_string()))?;
    parse(&text).map_err(failed)
}

/// Parses a waypoint list.
pub(super) fn parse(text: &str) -> Result<WaypointPlan, String> {
    let file: File = serde_json::from_str(text).map_err(|error| error.to_string())?;
    let waypoints = file.waypoints.into_iter().map(planned).collect();
    let mut plan = WaypointPlan::new(waypoints);
    plan.capture_radius_m = file
        .capture_radius_m
        .map(|radius| positive("capture_radius_m", radius))
        .transpose()?;
    if let Some(radius) = file.arrival_radius_m {
        plan.arrival_radius_m = positive("arrival_radius_m", radius)?;
    }
    Ok(plan)
}

fn planned(point: Point) -> PlannedWaypoint {
    PlannedWaypoint {
        latitude_deg: point.lat_deg,
        longitude_deg: point.lon_deg,
        altitude: match point.altitude {
            Altitude::At(value) => PlannedAltitude::At(value),
            Altitude::AtOrAbove(value) => PlannedAltitude::AtOrAbove(value),
            Altitude::AtOrBelow(value) => PlannedAltitude::AtOrBelow(value),
            Altitude::Window { lower_m, upper_m } => PlannedAltitude::Window { lower_m, upper_m },
            Altitude::Unconstrained { reference_m } => {
                PlannedAltitude::Unconstrained { reference_m }
            }
        },
        turn: match point.turn {
            Turn::FlyBy => WaypointTurn::FlyBy,
            Turn::FlyOver => WaypointTurn::FlyOver,
        },
        turn_direction: point.turn_direction.map(direction),
        name: point.name,
        path: point.path.map(planned_path),
        max_speed_mps: point.max_speed_mps,
        gradient: point.gradient,
        role: point.role.map(|role| match role {
            Role::Departure => LegRole::Departure,
            Role::Enroute => LegRole::Enroute,
            Role::Arrival => LegRole::Arrival,
            Role::Approach => LegRole::Approach,
            Role::MissedApproach => LegRole::MissedApproach,
        }),
        rnp_nm: point.rnp_nm,
    }
}

fn planned_path(leg: Leg) -> PlannedPath {
    match leg {
        Leg::InitialFix => PlannedPath::InitialFix,
        Leg::DirectToFix => PlannedPath::DirectToFix,
        Leg::CourseToFix {
            course_deg,
            reference,
        } => PlannedPath::CourseToFix {
            course_rad: course_deg.to_radians(),
            reference: course_reference(reference),
        },
        Leg::CourseToAltitude {
            course_deg,
            reference,
        } => PlannedPath::CourseToAltitude {
            course_rad: course_deg.to_radians(),
            reference: course_reference(reference),
        },
        Leg::HoldToAltitude(spec) => PlannedPath::HoldToAltitude(planned_hold(spec)),
        Leg::HoldToFix(spec) => PlannedPath::HoldToFix(planned_hold(spec)),
        Leg::HoldManual(spec) => PlannedPath::HoldManual(planned_hold(spec)),
    }
}

fn planned_hold(spec: HoldSpec) -> PlannedHold {
    PlannedHold {
        inbound_course_rad: spec.inbound_course_deg.to_radians(),
        reference: course_reference(spec.reference),
        turn: direction(spec.turn),
        length: match spec.length {
            HoldLength::Minutes(minutes) => PlannedHoldLength::Minutes(minutes),
            HoldLength::DistanceM(meters) => PlannedHoldLength::DistanceM(meters),
        },
    }
}

const fn course_reference(reference: Reference) -> PlannedCourseReference {
    match reference {
        Reference::True => PlannedCourseReference::True,
        Reference::Magnetic => PlannedCourseReference::Magnetic,
    }
}

const fn direction(dir: Direction) -> WaypointTurnDirection {
    match dir {
        Direction::Left => WaypointTurnDirection::Left,
        Direction::Right => WaypointTurnDirection::Right,
    }
}

fn positive(name: &str, radius: f64) -> Result<f64, String> {
    if radius.is_finite() && radius > 0.0 {
        Ok(radius)
    } else {
        Err(format!("{name} must be a positive distance, got {radius}"))
    }
}

/// The path that `PILOTAGE_MISSION_WAYPOINTS` names.
pub(super) fn path_of(value: String) -> PathBuf {
    PathBuf::from(value)
}

#[cfg(test)]
mod tests;
