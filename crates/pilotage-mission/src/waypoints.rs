//! A mission plan given as a list of waypoints: exact positions, one
//! altitude for each waypoint, and the turn at each one. No route string
//! and no navigation data take part.

use pilotage_mission_core::{Digest, MissionIdentity};
use sha2::{Digest as _, Sha256};

const WAYPOINT_LIST_DIGEST_DOMAIN: &[u8] = b"pilotage.waypoint-list.v1\0";

/// The lateral acceleration that sizes a capture radius from a cruise
/// speed when the plan sets none, m/s²: a bank of about 14°, a gentle
/// turn for a multicopter or a light aircraft.
pub const DEFAULT_TURN_ACCEL_MPS2: f64 = 2.5;

/// The smallest capture radius derived from a speed, metres: below it a
/// position estimate of ordinary quality can circle a fix.
pub const MIN_CAPTURE_RADIUS_M: f64 = 2.0;

/// The distance to the last waypoint at which a waypoint list completes
/// unless the plan sets another, metres.
pub const DEFAULT_ARRIVAL_RADIUS_M: f64 = 1.0;

/// The capture radius for a vehicle that turns at `turn_accel_mps2` at
/// `speed_mps`: its turn radius `v² / a`, and at least
/// [`MIN_CAPTURE_RADIUS_M`]. A non-finite or non-positive input gives
/// the minimum.
#[must_use]
pub fn capture_radius_for(speed_mps: f64, turn_accel_mps2: f64) -> f64 {
    let radius = speed_mps * speed_mps / turn_accel_mps2;
    if radius.is_finite() && radius > MIN_CAPTURE_RADIUS_M {
        radius
    } else {
        MIN_CAPTURE_RADIUS_M
    }
}

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

/// A list of waypoints in fly order, the distance at which each one
/// captures, and the distance at which the last one completes the plan.
#[derive(Debug, Clone, PartialEq)]
pub struct WaypointPlan {
    /// The waypoints in fly order.
    pub waypoints: Vec<PlannedWaypoint>,
    /// The distance to a waypoint at which it captures, metres. `None`
    /// derives it from the mission's cruise speed with
    /// [`capture_radius_for`] and [`DEFAULT_TURN_ACCEL_MPS2`].
    pub capture_radius_m: Option<f64>,
    /// The distance to the last waypoint, horizontally and vertically,
    /// at which the plan completes, metres. The vehicle flies on to the
    /// last waypoint after it captures until it is this close.
    pub arrival_radius_m: f64,
}

impl WaypointPlan {
    /// A plan whose capture radius follows the mission's cruise speed,
    /// with the default arrival radius.
    #[must_use]
    pub const fn new(waypoints: Vec<PlannedWaypoint>) -> Self {
        Self {
            waypoints,
            capture_radius_m: None,
            arrival_radius_m: DEFAULT_ARRIVAL_RADIUS_M,
        }
    }

    /// A plan for a vehicle that cruises at `cruise_mps` and turns at
    /// `turn_accel_mps2`: the capture radius is [`capture_radius_for`].
    #[must_use]
    pub fn for_vehicle(
        waypoints: Vec<PlannedWaypoint>,
        cruise_mps: f64,
        turn_accel_mps2: f64,
    ) -> Self {
        Self {
            capture_radius_m: Some(capture_radius_for(cruise_mps, turn_accel_mps2)),
            ..Self::new(waypoints)
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
        match self.capture_radius_m {
            Some(radius) => {
                hasher.update([1]);
                hasher.update(radius.to_le_bytes());
            }
            None => hasher.update([0]),
        }
        hasher.update(self.arrival_radius_m.to_le_bytes());
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
    /// The capture radius the plan flies with, metres.
    pub capture_radius_m: f64,
    /// Identity of the mission document that owns the plan.
    pub mission_identity: MissionIdentity,
}
