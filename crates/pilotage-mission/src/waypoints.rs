//! A mission plan given as a list of waypoints: exact positions, the
//! altitude constraint, turn, mandatory turn direction, leg path, speed
//! limit, gradient, procedure role, and RNP at each one. No route string
//! and no navigation data take part.

use pilotage_mission_core::{Digest, MissionIdentity};
use sha2::{Digest as _, Sha256};

mod legs;

pub use legs::{
    PlannedAltitude, PlannedCourseReference, PlannedHold, PlannedHoldLength, PlannedPath,
    WaypointTurnDirection,
};

use crate::engine::LegRole;

const WAYPOINT_LIST_DIGEST_DOMAIN: &[u8] = b"pilotage.waypoint-list.v2\0";

/// The smallest capture radius derived from a speed, metres: below it a
/// position estimate of ordinary quality can circle a fix.
pub const MIN_CAPTURE_RADIUS_M: f64 = 2.0;

/// The distance to the last waypoint at which a vehicle that can stop
/// completes a waypoint list, unless the plan sets another, metres.
pub const DEFAULT_ARRIVAL_RADIUS_M: f64 = 1.0;

/// The bank angle a vehicle turns at on a route unless its configuration
/// names another, degrees: a moderate coordinated turn. It sizes the turn
/// ahead of a fly-by waypoint, and it limits the bank of the holding
/// pattern after a plan for a vehicle that cannot stop.
pub const DEFAULT_ROUTE_BANK_DEG: f64 = 25.0;

/// The capture radius for a vehicle at `speed_mps`: the distance it
/// flies in `capture_time_s`, and at least [`MIN_CAPTURE_RADIUS_M`].
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
/// `altitude_m`, in the ISA standard atmosphere with no wind.
#[must_use]
pub fn true_airspeed_for(indicated_mps: f64, altitude_m: f64) -> f64 {
    if !(indicated_mps.is_finite() && altitude_m.is_finite()) {
        return indicated_mps;
    }
    indicated_mps / isa_density_ratio(altitude_m).sqrt()
}

/// The indicated airspeed of a true airspeed `true_mps` at `altitude_m`,
/// the inverse of [`true_airspeed_for`].
#[must_use]
pub fn indicated_airspeed_for(true_mps: f64, altitude_m: f64) -> f64 {
    if !(true_mps.is_finite() && altitude_m.is_finite()) {
        return true_mps;
    }
    true_mps * isa_density_ratio(altitude_m).sqrt()
}

/// The ISA density at `altitude_m` over the sea-level density.
fn isa_density_ratio(altitude_m: f64) -> f64 {
    const LAPSE_PER_M: f64 = 0.0065 / 288.15;
    const DENSITY_EXPONENT: f64 = 4.255_876;
    const TROPOPAUSE_M: f64 = 11_000.0;
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
    /// Altitude constraint at this waypoint; its
    /// [`PlannedAltitude::reference_m`] is the waypoint's 3D altitude.
    pub altitude: PlannedAltitude,
    /// How the vehicle passes the waypoint.
    pub turn: WaypointTurn,
    /// Optional mandatory turn direction (`L` or `R`) onto this leg.
    pub turn_direction: Option<WaypointTurnDirection>,
    /// The waypoint name, at most 32 bytes; `WP<n>` when absent.
    pub name: Option<String>,
    /// The path of the leg that ends here; `None` for a track from the
    /// waypoint before (or `InitialFix` at index `0`).
    pub path: Option<PlannedPath>,
    /// Maximum speed along the leg that ends here, m/s; `None` for no
    /// limit beyond the mission's.
    pub max_speed_mps: Option<f64>,
    /// Vertical gradient of the leg that ends here, metres of height per
    /// metre along track, negative descending; `None` for none.
    pub gradient: Option<f64>,
    /// Operational flight-phase role of this leg (`Departure`, `Enroute`,
    /// `Arrival`, `Approach`, `MissedApproach`).
    pub role: Option<LegRole>,
    /// Procedure-published RNP on this leg, nautical miles (`> 0`).
    pub rnp_nm: Option<f64>,
}

impl PlannedWaypoint {
    /// A fly-by waypoint with a generated name, crossed at `altitude_m`,
    /// on a track from the waypoint before.
    #[must_use]
    pub const fn new(latitude_deg: f64, longitude_deg: f64, altitude_m: f64) -> Self {
        Self {
            latitude_deg,
            longitude_deg,
            altitude: PlannedAltitude::At(altitude_m),
            turn: WaypointTurn::FlyBy,
            turn_direction: None,
            name: None,
            path: None,
            max_speed_mps: None,
            gradient: None,
            role: None,
            rnp_nm: None,
        }
    }

    /// Sets the waypoint's [`PlannedAltitude`].
    #[must_use]
    pub const fn with_altitude(mut self, altitude: PlannedAltitude) -> Self {
        self.altitude = altitude;
        self
    }

    /// Sets how the vehicle passes the waypoint (`FlyBy` / `FlyOver`).
    #[must_use]
    pub const fn with_turn(mut self, turn: WaypointTurn) -> Self {
        self.turn = turn;
        self
    }

    /// Sets a mandatory turn direction (`Left` / `Right`) onto this leg.
    #[must_use]
    pub const fn with_turn_direction(mut self, direction: WaypointTurnDirection) -> Self {
        self.turn_direction = Some(direction);
        self
    }

    /// Sets the waypoint identifier string.
    #[must_use]
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Sets the leg path ending at this waypoint.
    #[must_use]
    pub const fn with_path(mut self, path: PlannedPath) -> Self {
        self.path = Some(path);
        self
    }

    /// Sets a speed constraint on the leg ending at this waypoint, m/s.
    #[must_use]
    pub const fn with_max_speed_mps(mut self, max_speed_mps: f64) -> Self {
        self.max_speed_mps = Some(max_speed_mps);
        self
    }

    /// Sets a vertical gradient on the leg ending at this waypoint.
    #[must_use]
    pub const fn with_gradient(mut self, gradient: f64) -> Self {
        self.gradient = Some(gradient);
        self
    }

    /// Sets the flight-phase [`LegRole`] of this leg.
    #[must_use]
    pub const fn with_role(mut self, role: LegRole) -> Self {
        self.role = Some(role);
        self
    }

    /// Sets a procedure-published RNP value in nautical miles on this leg.
    #[must_use]
    pub const fn with_rnp_nm(mut self, rnp_nm: f64) -> Self {
        self.rnp_nm = Some(rnp_nm);
        self
    }
}

/// A list of waypoints in fly order and the distance at which each one
/// captures.
#[derive(Debug, Clone, PartialEq)]
pub struct WaypointPlan {
    /// The waypoints in fly order.
    pub waypoints: Vec<PlannedWaypoint>,
    /// The distance to a waypoint at which it captures, metres. `None`
    /// derives it from the mission's cruise speed with
    /// [`capture_radius_for`] and the mission's capture time.
    pub capture_radius_m: Option<f64>,
    /// The distance to the last waypoint at which a vehicle that can stop
    /// completes the plan, metres.
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
    /// the capture and arrival radii.
    #[must_use]
    pub fn digest(&self) -> Digest {
        let mut hasher = Sha256::new();
        hasher.update(WAYPOINT_LIST_DIGEST_DOMAIN);
        hasher.update((self.waypoints.len() as u64).to_le_bytes());
        for waypoint in &self.waypoints {
            for value in [waypoint.latitude_deg, waypoint.longitude_deg] {
                hasher.update(value.to_le_bytes());
            }
            waypoint.altitude.hash(&mut hasher);
            PlannedPath::hash(waypoint.path.as_ref(), &mut hasher);
            legs::hash_turn_direction(waypoint.turn_direction, &mut hasher);
            legs::hash_optional(waypoint.max_speed_mps, &mut hasher);
            legs::hash_optional(waypoint.gradient, &mut hasher);
            legs::hash_role(waypoint.role, &mut hasher);
            legs::hash_optional(waypoint.rnp_nm, &mut hasher);
            hasher.update([match waypoint.turn {
                WaypointTurn::FlyBy => 1,
                WaypointTurn::FlyOver => 2,
            }]);
            let name = waypoint.name.as_deref().unwrap_or_default();
            hasher.update((name.len() as u64).to_le_bytes());
            hasher.update(name.as_bytes());
        }
        legs::hash_optional(self.capture_radius_m, &mut hasher);
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
