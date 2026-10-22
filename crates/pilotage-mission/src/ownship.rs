//! Ownship samples as the host converts them, and the source-role
//! honesty gate the engine holds them to.

use navigate_contract::MonotonicNanos;

/// The LINK-04 source-role vocabulary as the mission engine consumes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TruthRole {
    /// Ground truth exported by a simulator.
    SimulationTruth,
    /// State exported by the flight controller's own estimator.
    FcState,
    /// The host's operational estimate.
    OperationalEstimate,
}

/// One ownship kinematic sample, converted by the host task from
/// vehicle telemetry.
///
/// **The engine admits one role, the one its configured source names.**
/// Under [`crate::OwnshipSource::SimulationTruth`] only
/// [`TruthRole::SimulationTruth`] samples become observations: the engine
/// synthesizes a GNSS-class position fix from truth, a simulation-only
/// stand-in for a real receiver. Under
/// [`crate::OwnshipSource::VehicleEstimate`] only
/// [`TruthRole::OperationalEstimate`] samples are admitted, and the engine
/// flies them as a declared estimator-derived solution without fusion
/// (ADR-0024 correlation rule). Every other role is a counted refusal,
/// never an aid.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OwnshipSample {
    /// Position as NED meters from the mission anchor.
    pub ned: [f64; 3],
    /// Velocity in NED, m/s. The synthesized observation is
    /// position-only; velocity aiding is a designed extension.
    pub ned_velocity: [f64; 3],
    /// Heading, radians, zero north, positive toward east. Absent when
    /// the source carried no attitude group: the engine can still feed
    /// fusion from position, but it cannot rotate NED into the body
    /// frame honestly, so a tick without a known heading emits no
    /// intent (zero would silently rotate commands to due north).
    pub yaw_rad: Option<f64>,
    /// Which estimator family produced this sample.
    pub role: TruthRole,
    /// Monotonic acquisition time on the engine's clock domain.
    pub acquired_at: MonotonicNanos,
    /// Per-source sample sequence, wrap-aware.
    pub sequence: u32,
}

/// The vertical reading a host converts for the vehicle-estimate source.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OwnshipAltitude {
    /// Height above the WGS-84 ellipsoid, meters.
    Ellipsoid(f64),
    /// Height above the mission anchor, meters. A vehicle that measures
    /// height from its launch point reports this reading when the anchor
    /// is that point.
    AboveAnchor(f64),
}
