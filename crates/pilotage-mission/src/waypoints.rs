//! A mission plan given as a list of waypoints: exact positions, one
//! altitude for each waypoint, and the turn at each one. No route string
//! and no navigation data take part.

use pilotage_mission_core::{Digest, MissionIdentity};
use sha2::{Digest as _, Sha256};

const WAYPOINT_LIST_DIGEST_DOMAIN: &[u8] = b"pilotage.waypoint-list.v1\0";

/// The smallest capture radius derived from a speed, metres: below it a
/// position estimate of ordinary quality can circle a fix.
pub const MIN_CAPTURE_RADIUS_M: f64 = 2.0;

/// The distance to the last waypoint at which a vehicle that can stop
/// completes a waypoint list, unless the plan sets another, metres.
pub const DEFAULT_ARRIVAL_RADIUS_M: f64 = 1.0;

/// The bank angle a vehicle turns at on a route unless its configuration
/// names another, degrees: a moderate coordinated turn. It sizes the turn
/// ahead of a fly-by waypoint and the orbit after a plan for a vehicle
/// that cannot stop.
pub const DEFAULT_ORBIT_BANK_DEG: f64 = 25.0;

/// The capture radius for a vehicle at `speed_mps`: the distance it
/// flies in `capture_time_s`, and at least [`MIN_CAPTURE_RADIUS_M`]. The
/// turn ahead of a fly-by waypoint is Navigate's turn anticipation, and a
/// waypoint that the vehicle passes outside the radius still sequences,
/// so the radius need not hold a turn. A non-finite or negative input
/// gives the minimum.
#[must_use]
pub fn capture_radius_for(speed_mps: f64, capture_time_s: f64) -> f64 {
    at_least(speed_mps * capture_time_s, MIN_CAPTURE_RADIUS_M)
}

/// The coordinated turn radius `v² / (g tan φ)` at `speed_mps` and a bank
/// of `bank_deg`, and at least `floor_m`. A non-finite input, or a bank
/// outside 0° to 90°, gives `floor_m`.
#[must_use]
pub fn turn_radius_for(speed_mps: f64, bank_deg: f64, floor_m: f64) -> f64 {
    if !(bank_deg > 0.0 && bank_deg < 90.0) {
        return floor_m;
    }
    let lateral_mps2 = crate::body::SURFACE_GRAVITY_MPS2 * bank_deg.to_radians().tan();
    at_least(speed_mps * speed_mps / lateral_mps2, floor_m)
}

/// The true airspeed of an indicated airspeed `indicated_mps` at
/// `altitude_m`, in the ISA standard atmosphere with no wind: the speed
/// over the ground a fixed wing flies when its flight controller holds
/// that indicated airspeed. Thinner air needs more true speed for the same
/// dynamic pressure, `TAS = IAS / √σ`; the difference between indicated and
/// equivalent airspeed is below a percent at a light aircraft's speeds.
/// Above the 11 km tropopause the density falls exponentially. A
/// non-finite input gives the indicated airspeed.
#[must_use]
pub fn true_airspeed_for(indicated_mps: f64, altitude_m: f64) -> f64 {
    if !(indicated_mps.is_finite() && altitude_m.is_finite()) {
        return indicated_mps;
    }
    indicated_mps / isa_density_ratio(altitude_m).sqrt()
}

/// The indicated airspeed of a true airspeed `true_mps` at `altitude_m`,
/// the inverse of [`true_airspeed_for`]. A non-finite input gives the true
/// airspeed.
#[must_use]
pub fn indicated_airspeed_for(true_mps: f64, altitude_m: f64) -> f64 {
    if !(true_mps.is_finite() && altitude_m.is_finite()) {
        return true_mps;
    }
    true_mps * isa_density_ratio(altitude_m).sqrt()
}

/// The ISA density at `altitude_m` over the sea-level density.
fn isa_density_ratio(altitude_m: f64) -> f64 {
    /// Temperature lapse over the sea-level temperature, 1/m.
    const LAPSE_PER_M: f64 = 0.0065 / 288.15;
    /// `g / (L·R) − 1` for dry air: the density exponent of the ISA
    /// troposphere.
    const DENSITY_EXPONENT: f64 = 4.255_876;
    const TROPOPAUSE_M: f64 = 11_000.0;
    /// Density scale height of the isothermal layer above it, metres.
    const STRATOSPHERE_SCALE_M: f64 = 6_341.62;
    let troposphere = |height_m: f64| (1.0 - LAPSE_PER_M * height_m).powf(DENSITY_EXPONENT);
    if altitude_m <= TROPOPAUSE_M {
        troposphere(altitude_m)
    } else {
        troposphere(TROPOPAUSE_M) * (-(altitude_m - TROPOPAUSE_M) / STRATOSPHERE_SCALE_M).exp()
    }
}

fn at_least(value: f64, floor: f64) -> f64 {
    if value.is_finite() && value > floor {
        value
    } else {
        floor
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

/// A list of waypoints in fly order and the distance at which each one
/// captures. A vehicle that can stop completes the plan at the last
/// waypoint, inside the arrival radius; a vehicle that cannot stop
/// completes it when the last waypoint sequences.
#[derive(Debug, Clone, PartialEq)]
pub struct WaypointPlan {
    /// The waypoints in fly order.
    pub waypoints: Vec<PlannedWaypoint>,
    /// The distance to a waypoint at which it captures, metres. `None`
    /// derives it from the mission's cruise speed with
    /// [`capture_radius_for`] and the mission's capture time.
    pub capture_radius_m: Option<f64>,
    /// The distance to the last waypoint, horizontally and, for an
    /// aircraft, vertically, at which a vehicle that can stop completes
    /// the plan, metres.
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
