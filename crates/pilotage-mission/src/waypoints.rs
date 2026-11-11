//! A mission plan given as a list of waypoints: exact positions, one
//! altitude for each waypoint, and the turn at each one. No route string
//! and no navigation data take part.

use pilotage_mission_core::{Digest, MissionIdentity};
use sha2::{Digest as _, Sha256};

const WAYPOINT_LIST_DIGEST_DOMAIN: &[u8] = b"pilotage.waypoint-list.v1\0";

/// The capture radius of a waypoint list unless the plan sets one,
/// metres: the default of Navigate's `ExecutionConfig`.
pub const DEFAULT_CAPTURE_RADIUS_M: f64 = 100.0;

/// How the vehicle passes a waypoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WaypointTurn {
    /// The leg turns before the waypoint onto the next leg.
    #[default]
    FlyBy,
    /// The vehicle flies over the waypoint, then turns.
    FlyOver,
}

/// One waypoint of a list.
#[derive(Debug, Clone, PartialEq)]
pub struct PlannedWaypoint {
    /// Latitude, degrees north, on WGS84.
    pub latitude_deg: f64,
    /// Longitude, degrees east, on WGS84.
    pub longitude_deg: f64,
    /// Height above the WGS84 ellipsoid that the vehicle holds at this
    /// waypoint, metres.
    pub altitude_m: f64,
    /// How the vehicle passes the waypoint.
    pub turn: WaypointTurn,
    /// The waypoint name, at most 32 bytes; `WP<n>` when absent.
    pub name: Option<String>,
}

impl PlannedWaypoint {
    /// A fly-by waypoint with a generated name.
    #[must_use]
    pub const fn new(latitude_deg: f64, longitude_deg: f64, altitude_m: f64) -> Self {
        Self {
            latitude_deg,
            longitude_deg,
            altitude_m,
            turn: WaypointTurn::FlyBy,
            name: None,
        }
    }
}

/// A list of waypoints in fly order and the distance at which each one
/// captures.
#[derive(Debug, Clone, PartialEq)]
pub struct WaypointPlan {
    /// The waypoints in fly order.
    pub waypoints: Vec<PlannedWaypoint>,
    /// The distance to a waypoint at which it captures, metres.
    pub capture_radius_m: f64,
}

impl WaypointPlan {
    /// A plan with the default capture radius.
    #[must_use]
    pub const fn new(waypoints: Vec<PlannedWaypoint>) -> Self {
        Self {
            waypoints,
            capture_radius_m: DEFAULT_CAPTURE_RADIUS_M,
        }
    }

    /// The content digest of the plan: every value of every waypoint and
    /// the capture radius.
    #[must_use]
    pub fn digest(&self) -> Digest {
        let mut hasher = Sha256::new();
        hasher.update(WAYPOINT_LIST_DIGEST_DOMAIN);
        hasher.update((self.waypoints.len() as u64).to_le_bytes());
        for waypoint in &self.waypoints {
            for value in [
                waypoint.latitude_deg,
                waypoint.longitude_deg,
                waypoint.altitude_m,
            ] {
                hasher.update(value.to_le_bytes());
            }
            hasher.update([match waypoint.turn {
                WaypointTurn::FlyBy => 1,
                WaypointTurn::FlyOver => 2,
            }]);
            let name = waypoint.name.as_deref().unwrap_or_default();
            hasher.update((name.len() as u64).to_le_bytes());
            hasher.update(name.as_bytes());
        }
        hasher.update(self.capture_radius_m.to_le_bytes());
        Digest::from_bytes(hasher.finalize().into())
    }
}

/// What a mission built from a waypoint list records for its operator.
#[derive(Debug, Clone, PartialEq)]
pub struct WaypointPlanRecord {
    /// The content digest of the waypoint list.
    pub digest: Digest,
    /// The waypoint names in fly order.
    pub idents: Vec<String>,
    /// Identity of the mission document that owns the plan.
    pub mission_identity: MissionIdentity,
}
