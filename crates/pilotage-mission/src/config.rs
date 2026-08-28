//! Mission configuration with documented defaults.

use navigate_contract::{ClockDomainId, DurationNanos, GeodeticPosition};

/// Ceilings every emitted intent is clamped within.
///
/// The defaults mirror the reference adapters' advertised
/// `vehicle.motion` limits; the host MAY tighten them from the actually
/// advertised capabilities before constructing the engine. The engine
/// never emits beyond them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MissionLimits {
    /// Ceiling on the commanded horizontal speed, m/s. Default 2.5.
    /// An over-ceiling horizontal vector is scaled, never clipped per
    /// axis, so the cap cannot rotate the commanded direction.
    pub max_horizontal_mps: f64,
    /// Ceiling on the magnitude of the commanded vertical rate, m/s.
    /// Default 1.0.
    pub max_vertical_mps: f64,
    /// Ceiling on the magnitude of the commanded yaw rate, rad/s.
    /// Default 0.8.
    pub max_yaw_rate_rps: f64,
}

impl Default for MissionLimits {
    fn default() -> Self {
        Self {
            max_horizontal_mps: 2.5,
            max_vertical_mps: 1.0,
            max_yaw_rate_rps: 0.8,
        }
    }
}

/// Configuration of a [`crate::MissionEngine`].
///
/// The engine is sans-IO: `clock` names the single clock domain every
/// `now`, every sample stamp, and every fusion judgment lives on; the
/// engine itself never reads a clock.
#[derive(Debug, Clone, PartialEq)]
pub struct MissionConfig {
    /// Route string expanded against the navdata snapshot at build.
    pub route: String,
    /// Mission anchor in radians/meters ([`GeodeticPosition`]). The host
    /// converts from the snapshot's degrees exactly once, at plan build
    /// (ADR-0030); nothing downstream ever sees degrees.
    pub anchor: GeodeticPosition,
    /// Cruise height above the anchor altitude, meters. Default 15.0.
    /// `0.0` disables climb behavior. The mission starts enroute guidance
    /// after the arm receipt.
    pub cruise_height_m: f64,
    /// Commanded climb rate during the climb phase, m/s. Default 0.8.
    pub climb_rate_mps: f64,
    /// Along-track cruise speed asked of guidance, m/s. Default 2.0.
    pub cruise_mps: f64,
    /// Intent ceilings; see [`MissionLimits`] for the defaults.
    pub limits: MissionLimits,
    /// Yaw alignment gain, inverse seconds: radians of heading error to
    /// rad/s of commanded yaw rate. Default 1.0.
    pub yaw_gain_per_s: f64,
    /// 1-sigma of the synthesized GNSS position fix per NED axis,
    /// meters. Default 1.5.
    pub gnss_sigma_m: f64,
    /// Hint to the host for how often to call [`crate::MissionEngine::tick`]
    /// and frame the result. Default 50 ms — well inside the session
    /// holder-silence watchdog. The engine itself never schedules.
    pub frame_interval: DurationNanos,
    /// The clock domain all mission time lives on.
    pub clock: ClockDomainId,
}

impl MissionConfig {
    /// A config over the documented defaults; the route, anchor, and
    /// clock domain have no meaningful default and are always the
    /// caller's.
    #[must_use]
    pub fn new(route: String, anchor: GeodeticPosition, clock: ClockDomainId) -> Self {
        Self {
            route,
            anchor,
            cruise_height_m: 15.0,
            climb_rate_mps: 0.8,
            cruise_mps: 2.0,
            limits: MissionLimits::default(),
            yaw_gain_per_s: 1.0,
            gnss_sigma_m: 1.5,
            frame_interval: DurationNanos::from_millis(50),
            clock,
        }
    }
}
