//! `PILOTAGE_MISSION_WAYPOINTS`: a JSON file with a waypoint list.
//!
//! ```json
//! {
//!   "capture_radius_m": 5.0,
//!   "waypoints": [
//!     { "lat_deg": 47.0006, "lon_deg": 8.0, "alt_m": 512.0 },
//!     { "lat_deg": 47.0006, "lon_deg": 8.0008, "alt_m": 518.0, "turn": "fly-over", "name": "CORNER" }
//!   ]
//! }
//! ```
//!
//! `alt_m` is the height above the WGS84 ellipsoid. `turn` is `fly-by`
//! (the default) or `fly-over`. `capture_radius_m` is optional: without
//! it the radius follows the mission's cruise speed. The plan completes
//! when the vehicle passes the last waypoint.

use std::path::{Path, PathBuf};

use pilotage_mission::{PlannedWaypoint, WaypointPlan, WaypointTurn};
use serde::Deserialize;

use crate::error::HostError;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    #[serde(default)]
    capture_radius_m: Option<f64>,
    waypoints: Vec<Point>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Point {
    lat_deg: f64,
    lon_deg: f64,
    alt_m: f64,
    #[serde(default)]
    turn: Turn,
    #[serde(default)]
    name: Option<String>,
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
/// not a waypoint list. The mission build refuses a list with no
/// waypoint or with a position that is not on the Earth.
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
    let waypoints = file
        .waypoints
        .into_iter()
        .map(|point| PlannedWaypoint {
            latitude_deg: point.lat_deg,
            longitude_deg: point.lon_deg,
            altitude_m: point.alt_m,
            turn: match point.turn {
                Turn::FlyBy => WaypointTurn::FlyBy,
                Turn::FlyOver => WaypointTurn::FlyOver,
            },
            name: point.name,
        })
        .collect();
    let mut plan = WaypointPlan::new(waypoints);
    plan.capture_radius_m = file
        .capture_radius_m
        .map(|radius| positive("capture_radius_m", radius))
        .transpose()?;
    Ok(plan)
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
